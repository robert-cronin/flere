#!/usr/bin/env python3
"""Offline eight-asset release fixtures; no payload execution or publication."""
import copy
import io
import json
import lzma
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import unittest
from unittest import mock

import test_release_automation as fixtures

release = fixtures.release
packages = release.debian.packages
VERSION, COMMIT, RUN, WORKFLOW = fixtures.VERSION, fixtures.COMMIT, fixtures.RUN, fixtures.WORKFLOW


class DebianRelease(unittest.TestCase):
    def setUp(self):
        fixtures.ReleaseAutomation.setUp(self)
        self.api = fixtures.FakeGitHub(schema=2)
        source = release.manifests(self.directory, VERSION, COMMIT)
        archive = release.source_archive.inspect(self.source_archive, VERSION, source)
        self.lock, self.binaries, self.licenses = release.debian_inputs(self.directory, VERSION, COMMIT, source, archive)
        self.artifact = self.directory / release.debian.name(VERSION)
        self.write_deb()

    def tar(self, files, change=None, extra=None):
        directories = {""}
        for name in files:
            parts = name.split("/")
            directories.update("/".join(parts[:n]) for n in range(1, len(parts)))
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w", format=tarfile.USTAR_FORMAT) as archive:
            for name in sorted(directories):
                member = tarfile.TarInfo("./" + name if name else ".")
                member.type, member.mode = tarfile.DIRTYPE, 0o755
                member.uid = member.gid = 0
                member.uname = member.gname = "root"
                member.mtime = self.lock["source_date_epoch"]
                archive.addfile(member)
            for name, (data, mode) in sorted(files.items()):
                member = tarfile.TarInfo("./" + name)
                member.mode, member.size = mode, len(data)
                member.uid = member.gid = 0
                member.uname = member.gname = "root"
                member.mtime = self.lock["source_date_epoch"]
                if change:
                    change(member)
                archive.addfile(member, io.BytesIO(data) if member.isfile() else None)
            if extra:
                member = tarfile.TarInfo(extra)
                member.size, member.mode = 4, 0o644
                member.uid = member.gid = 0
                member.uname = member.gname = "root"
                member.mtime = self.lock["source_date_epoch"]
                archive.addfile(member, io.BytesIO(b"hook"))
        return lzma.compress(stream.getvalue(), format=lzma.FORMAT_XZ)

    def write_deb(self, *, file_change=None, control_change=None, extra=None, data_files=None):
        files = packages.deb_files(self.binaries, self.licenses)
        control = packages.deb_control_files(files, self.lock)
        if control_change:
            control_change(control)
        members = [("debian-binary", b"2.0\n"),
                   ("control.tar.xz", self.tar(control)),
                   ("data.tar.xz", self.tar(files if data_files is None else data_files, file_change, extra))]
        raw = bytearray(b"!<arch>\n")
        for name, data in members:
            header = f"{name:<16}{self.lock['source_date_epoch']:<12}{0:<6}{0:<6}{'100644':<8}{len(data):<10}`\n"
            assert len(header) == 60
            raw += header.encode() + data + (b"\n" if len(data) % 2 else b"")
        self.artifact.write_bytes(raw)

    def seal(self):
        return release.seal(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.evidence, schema=2)

    def test_eight_assets_seal_publish_and_retry_without_rebuild_or_extra_writes(self):
        proof = self.seal()
        self.assertEqual(set(proof["files"]), release.allowlist(VERSION, 2))
        self.assertEqual(len(proof["files"]), 8)
        descriptor = json.loads((self.directory / "release.json").read_bytes())
        self.assertEqual(descriptor["schema_version"], 2)
        self.assertEqual(descriptor["debian"]["source_commit"], COMMIT)
        self.assertEqual(set(descriptor["debian"]["files"]), set(packages.deb_files(self.binaries, self.licenses)))
        pin = proof["descriptor_sha256"]
        release.publish(self.directory, VERSION, COMMIT, RUN, WORKFLOW, pin, self.api)
        writes = list(self.api.writes)
        release.publish(self.directory, VERSION, COMMIT, RUN, WORKFLOW, pin, self.api)
        self.assertEqual(self.api.writes, writes)
        self.assertEqual(set(self.api.files), release.allowlist(VERSION, 2))
        self.assertEqual(self.api.release["make_latest"], "false")
        self.assertIn("does not update external package catalogues or the latest-release pointer", self.api.release["body"])
        self.assertNotIn("does not advance package-manager channels", self.api.release["body"])
        calls = []
        def read(url):
            name = url.rsplit("/", 1)[1]
            calls.append(name)
            return self.api.files[name]
        with mock.patch.object(release, "public_download", side_effect=read):
            result = release.verify_public(self.directory, VERSION, COMMIT, RUN, WORKFLOW, pin)
        self.assertEqual(set(calls), release.allowlist(VERSION, 2))
        self.assertEqual(result["channels"], release.DEBIAN_CHANNELS)
        lock = packages.lock_from_release(self.directory, pin)
        self.assertEqual(set(lock["assets"]), release.candidate_names(VERSION))
        self.assertNotIn(self.artifact.name, lock["assets"])
        self.assertNotIn(pin.encode(), b"".join(data for data, _ in packages.deb_files(self.binaries, self.licenses).values()))

    def test_derived_lock_and_recipes_are_identical_across_hash_seeds(self):
        proof = self.seal()
        script = """
import importlib.util, json, sys
from pathlib import Path
spec = importlib.util.spec_from_file_location("packages", sys.argv[1])
packages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packages)
lock = packages.lock_from_release(Path(sys.argv[2]), sys.argv[3])
recipes = {**packages.aur_files(lock), **packages.rpm_files(lock)}
print(json.dumps({"lock": lock, "recipes": {name: data.decode() for name, data in recipes.items()}}))
"""
        outputs = [subprocess.check_output(
            [sys.executable, "-c", script, packages.__file__, str(self.directory), proof["descriptor_sha256"]],
            env=dict(os.environ, PYTHONHASHSEED=seed, PYTHONDONTWRITEBYTECODE="1"), timeout=30)
            for seed in ("0", "1")]
        self.assertEqual(outputs[0], outputs[1])
        result = json.loads(outputs[0])
        self.assertEqual(list(result["lock"]["assets"]), sorted(release.candidate_names(VERSION)))
        self.assertEqual(set(result["recipes"]), {"PKGBUILD", ".SRCINFO", "flere.spec"})

    def test_payload_manifest_license_or_control_change_fails_before_seal(self):
        expected = packages.deb_files(self.binaries, self.licenses)
        for name in ("usr/bin/flere", "usr/share/flere/flere-connect.manifest.json", "usr/share/doc/flere/copyright"):
            with self.subTest(name=name):
                changed = copy.deepcopy(expected)
                changed[name] = (changed[name][0] + b"changed", changed[name][1])
                self.write_deb(data_files=changed)
                with self.assertRaisesRegex(ValueError, "file bytes/mode"):
                    self.seal()
                self.assertFalse((self.directory / "release.json").exists())
        def control_change(control):
            control["control"] = (control["control"][0].replace(b"Architecture: amd64", b"Architecture: arm64"), 0o644)
        self.write_deb(control_change=control_change)
        with self.assertRaisesRegex(ValueError, "file bytes/mode"):
            self.seal()

    def test_links_hooks_duplicate_modes_and_owners_are_rejected(self):
        def link(member):
            if member.name == "./usr/bin/flere":
                member.type, member.linkname, member.size = tarfile.SYMTYPE, "/private", 0
        for mutation in (link, lambda m: setattr(m, "mode", 0o777), lambda m: setattr(m, "uid", 1000)):
            with self.subTest(mutation=mutation):
                self.write_deb(file_change=mutation)
                with self.assertRaises(ValueError):
                    self.seal()
        for extra in ("./usr/bin/flere", "../../private", "./postinst"):
            with self.subTest(extra=extra):
                self.write_deb(extra=extra)
                with self.assertRaises(ValueError):
                    self.seal()
        self.write_deb(control_change=lambda c: c.update(postinst=(b"#!/bin/sh\n", 0o755)))
        with self.assertRaises(ValueError):
            self.seal()

    def test_missing_deb_wrong_format_or_trailing_content_fails(self):
        raw = self.artifact.read_bytes()
        for data in (raw + b"trailing", b"invalid", raw[:-2]):
            with self.subTest(data_bytes=len(data)):
                self.artifact.write_bytes(data)
                with self.assertRaises(ValueError):
                    self.seal()
        self.artifact.unlink()
        with self.assertRaisesRegex(ValueError, "candidate must contain exactly"):
            self.seal()

    def test_xz_bomb_concatenation_and_hidden_tar_tail_are_bounded(self):
        valid = self.tar({})
        raw = lzma.decompress(valid)
        for data in (lzma.compress(b"x" * 70000), valid + valid,
                     lzma.compress(raw + b"hidden" + b"\0" * 506), b"invalid xz"):
            with self.subTest(compressed_bytes=len(data)):
                with self.assertRaises(ValueError):
                    release.debian.inspect_tar(data, {}, self.lock["source_date_epoch"])

    def test_new_pin_cannot_bypass_debian_content_or_provenance_checks(self):
        proof = self.seal()
        original = self.artifact.read_bytes()
        self.artifact.write_bytes(original + b"modified")
        with self.assertRaisesRegex(ValueError, "final bytes"):
            release.publish(self.directory, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], self.api)
        self.assertEqual(self.api.writes, [])
        self.artifact.write_bytes(original)
        descriptor_path = self.directory / "release.json"
        descriptor = json.loads(descriptor_path.read_bytes())
        descriptor["debian"]["source_commit"] = "d" * 40
        descriptor_path.write_bytes(release.json_bytes(descriptor))
        sums = "".join(f"{release.sha((self.directory / name).read_bytes())}  {name}\n"
                       for name in sorted(release.candidate_names(VERSION, 2) | {"release.json"}))
        (self.directory / "SHA256SUMS").write_text(sums)
        with self.assertRaisesRegex(ValueError, "Debian wrapper provenance"):
            release.publish(self.directory, VERSION, COMMIT, RUN, WORKFLOW,
                            release.sha(descriptor_path.read_bytes()), self.api)
        self.assertEqual(self.api.writes, [])

    def test_rehashed_tampered_wrapper_still_fails_before_github_writes(self):
        self.seal()
        files = packages.deb_files(self.binaries, self.licenses)
        files["usr/bin/flere"] = (files["usr/bin/flere"][0] + b"modified", 0o755)
        self.write_deb(data_files=files)
        descriptor_path = self.directory / "release.json"
        descriptor = json.loads(descriptor_path.read_bytes())
        data = self.artifact.read_bytes()
        descriptor["assets"][self.artifact.name] = {"bytes": len(data), "sha256": release.sha(data)}
        descriptor_path.write_bytes(release.json_bytes(descriptor))
        (self.directory / "SHA256SUMS").write_text("".join(
            f"{release.sha((self.directory / name).read_bytes())}  {name}\n"
            for name in sorted(release.candidate_names(VERSION, 2) | {"release.json"})))
        with self.assertRaisesRegex(ValueError, "file bytes/mode"):
            release.publish(self.directory, VERSION, COMMIT, RUN, WORKFLOW,
                            release.sha(descriptor_path.read_bytes()), self.api)
        self.assertEqual(self.api.writes, [])

    def test_schema_one_stays_seven_assets_and_rejects_undeclared_wrapper(self):
        self.assertEqual(release.sha(release.release_body(VERSION, COMMIT, RUN, "c" * 64).encode()),
                         "e25e973e28b1bc2416dc668203698a3f8c609f14c65c41cab07018ae5a4c1991")
        with self.assertRaises(ValueError):
            release.seal(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.evidence)
        self.artifact.unlink()
        proof = release.seal(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.evidence)
        self.assertEqual(set(proof["files"]), release.allowlist(VERSION))
        self.assertEqual(len(proof["files"]), 7)
        descriptor = json.loads((self.directory / "release.json").read_bytes())
        self.assertEqual(descriptor["schema_version"], 1)
        self.assertNotIn("debian", descriptor)
        release.validate(self.directory, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"])
        with self.assertRaises(ValueError):
            release.candidate_names(VERSION, True)


if __name__ == "__main__":
    unittest.main()
