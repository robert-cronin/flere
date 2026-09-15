#!/usr/bin/env python3
"""Offline multi-target sealing fixtures; no executable or network is invoked."""
import copy
import json
import shutil
import struct
import unittest
from unittest import mock

import test_release_automation as fixtures
import test_release_debian as debian_fixtures

release = fixtures.release
aggregate = release.aggregate
VERSION, COMMIT, RUN, WORKFLOW = fixtures.VERSION, fixtures.COMMIT, fixtures.RUN, fixtures.WORKFLOW


class AggregateRelease(unittest.TestCase):
    tar = debian_fixtures.DebianRelease.tar
    write_deb = debian_fixtures.DebianRelease.write_deb

    def setUp(self):
        debian_fixtures.DebianRelease.setUp(self)
        self.linux_pin = release.seal(self.directory, VERSION, COMMIT, RUN, WORKFLOW,
                                      self.evidence, schema=2)["descriptor_sha256"]
        self.windows = self.directory.with_name(self.directory.name + "-windows")
        self.output = self.directory.with_name(self.directory.name + "-aggregate")
        for path in (self.windows, self.output):
            self.addCleanup(shutil.rmtree, path, True)
        (self.windows / "windows").mkdir(parents=True)
        (self.windows / "logs").mkdir()
        self.binary = bytearray(128)
        self.binary[:2] = b"MZ"
        struct.pack_into("<I", self.binary, 0x3c, 64)
        self.binary[64:70] = b"PE\0\0\x64\x86"
        self.manifest = {
            "schema_version": 1,
            "build": {"component": "flere-connect", "package_version": VERSION,
                      "profile": "release", "target": aggregate.TARGET, "build_id": "fixture",
                      "rustc": "rustc 1.98.0 (fixture)\nbinary: rustc\nhost: " + aggregate.TARGET + "\nrelease: 1.98.0",
                      "compatibility": {"remote_protocol": {"current": "v6", "accepts": ["v6"]}}},
            "source": {"git_commit": COMMIT, "source_sha256": self.source_digest, "dirty": False},
            "payload": {"file_name": "flere-connect", "download_file": aggregate.PAYLOAD,
                        "bytes": len(self.binary), "sha256": release.sha(self.binary)}}
        self.write_windows()

    def write_windows(self, *, entries_change=None, receipt_change=None):
        archive = self.windows / "windows" / aggregate.zip_name(VERSION)
        archive.unlink(missing_ok=True)
        entries = {"flere.exe": bytes(self.binary), "flere-connect.exe": bytes(self.binary),
                   "manifest.json": release.json_bytes(self.manifest), "LICENSE": self.licenses["LICENSE"]}
        if entries_change:
            entries_change(entries)
        aggregate.windows.write_zip(archive, entries)
        checks = []
        for name in aggregate.CHECKS:
            data = (self.manifest["build"]["rustc"] + "\n").encode() if name == "02-rustc" else b"fixture\n"
            if name.endswith("-build-info"):
                data = release.json_bytes(self.manifest["build"])
            (self.windows / "logs" / (name + ".log")).write_bytes(data)
            checks.append({"name": name, "argv": ["C:\\private-fixture\\source"], "status": "passed",
                           "exit": 0, "log_bytes": len(data), "log_sha256": release.sha(data)})
        raw = archive.read_bytes()
        self.receipt = {
            "schema": "flere-windows-candidate-v1", "status": "prepared_not_published",
            "commit": COMMIT, "version": VERSION, "target": aggregate.TARGET,
            "workflow_sha": WORKFLOW, "run_id": RUN, "run_attempt": "2",
            "source_sha256": self.source_digest, "source_unchanged": True,
            "stateless_state_unchanged": True, "stateless_checks": 6,
            "build": self.manifest["build"], "payload_sha256": self.manifest["payload"]["sha256"],
            "zip": {"name": archive.name, "bytes": len(raw), "sha256": release.sha(raw)},
            "generated_files": {archive.name: {"bytes": len(raw), "sha256": release.sha(raw)}},
            "machine": {"system": "Windows-2025Server-10.0.26100-SP0", "machine": "AMD64",
                        "image_os": "win25-vs2026", "image_version": "20260907.229.1"},
            "checks": checks, "winget": {"status": "blocked"}, "chocolatey": {"status": "blocked"},
            "private_fixture": "C:\\private-fixture\\do-not-publish"}
        if receipt_change:
            receipt_change(self.receipt)
        self.pin_receipt()

    def pin_receipt(self):
        raw = release.json_bytes(self.receipt)
        (self.windows / "candidate.json").write_bytes(raw)
        self.windows_pin = release.sha(raw)

    def prepare(self, **changes):
        args = dict(linux_directory=self.directory, windows_directory=self.windows, output=self.output,
                    version=VERSION, commit=COMMIT, run_id=RUN, workflow_sha=WORKFLOW,
                    linux_descriptor_sha256=self.linux_pin, windows_receipt_sha256=self.windows_pin)
        args.update(changes)
        return aggregate.prepare(**args)

    def reject(self, **changes):
        with self.assertRaises((ValueError, KeyError)):
            self.prepare(**changes)
        self.assertFalse(self.output.exists())

    def test_complete_eleven_assets_preserve_linux_and_exclude_private_evidence(self):
        old = {path.name: path.read_bytes() for path in self.directory.iterdir()}
        proof = self.prepare()
        self.assertEqual(set(proof["files"]), release.allowlist(VERSION, 3))
        self.assertEqual(len(proof["files"]), 11)
        self.assertEqual(old, {path.name: path.read_bytes() for path in self.directory.iterdir()})
        descriptor = json.loads((self.output / "release.json").read_bytes())
        self.assertEqual(descriptor["schema_version"], 3)
        self.assertEqual(descriptor["source_sha256"], self.source_digest)
        self.assertEqual(descriptor["evidence"]["inputs"], {
            "linux_descriptor_sha256": self.linux_pin, "windows_receipt_sha256": self.windows_pin})
        self.assertEqual(descriptor["evidence"]["windows"]["checks"], aggregate.CHECKS)
        self.assertNotIn(b"private-fixture", (self.output / "release.json").read_bytes())
        self.assertNotIn("aarch64-apple-darwin", " ".join(proof["files"]))
        self.assertEqual(descriptor["targets"]["aarch64-apple-darwin"], release.BLOCKED["aarch64-apple-darwin"])
        self.assertEqual((self.output / aggregate.PAYLOAD).read_bytes(), bytes(self.binary))
        lock = fixtures.packages.lock_from_release(self.output, proof["descriptor_sha256"])
        self.assertEqual(set(lock["assets"]), release.candidate_names(VERSION))
        self.assertEqual(fixtures.packages.aur_files(lock), fixtures.packages.aur_files(self.lock))
        recipes = aggregate.load("aggregate_recipes", aggregate.ROOT / "scripts/release-recipes.py")
        destination = self.output.with_name(self.output.name + "-recipes")
        self.addCleanup(shutil.rmtree, destination, True)
        prepared = recipes.prepare(self.output, destination, VERSION, COMMIT, RUN, WORKFLOW,
                                   proof["descriptor_sha256"])
        self.assertEqual(prepared["status"], "prepared_not_published")
        self.assertEqual(prepared["release_descriptor_sha256"], proof["descriptor_sha256"])
        self.assertEqual({p.relative_to(destination).as_posix() for p in destination.rglob("*") if p.is_file()},
                         recipes.ARTIFACT_FILES)

    def test_trusted_input_digests_and_same_run_source_are_required(self):
        for changes in ({"linux_descriptor_sha256": "0" * 64}, {"windows_receipt_sha256": "0" * 64},
                        {"commit": "c" * 40}, {"version": "0.4.1"}, {"run_id": "2345"},
                        {"workflow_sha": "d" * 40}):
            with self.subTest(changes=changes):
                self.reject(**changes)
        for key, value in (("commit", "c" * 40), ("version", "0.4.1"), ("run_id", "2345"),
                           ("workflow_sha", "d" * 40), ("source_sha256", "0" * 64)):
            with self.subTest(key=key):
                self.write_windows(receipt_change=lambda item: item.update({key: value}))
                self.reject()

    def test_missing_native_checks_failed_status_or_changed_logs_reject(self):
        for change in (lambda item: item.update(status="failed"),
                       lambda item: item["checks"].pop(),
                       lambda item: item["checks"][6].update(exit=1),
                       lambda item: item.update(stateless_checks=5),
                       lambda item: item.update(source_unchanged=False),
                       lambda item: item["winget"].update(status="failed")):
            self.write_windows(receipt_change=change)
            self.reject()
        self.write_windows()
        (self.windows / "logs/07-tests.log").write_bytes(b"changed log\n")
        self.reject()

    def test_windows_source_version_target_and_cross_platform_protocol_reject(self):
        original = copy.deepcopy(self.manifest)
        for change in (lambda item: item["source"].update(git_commit="c" * 40),
                       lambda item: item["source"].update(source_sha256="0" * 64),
                       lambda item: item["source"].update(dirty=True),
                       lambda item: item["build"].update(package_version="0.4.1"),
                       lambda item: item["build"].update(target="aarch64-pc-windows-msvc"),
                       lambda item: item["build"]["compatibility"]["remote_protocol"].update(accepts=["v5"])):
            self.manifest = copy.deepcopy(original)
            change(self.manifest)
            self.write_windows()
            self.reject()

    def test_wrong_pe_architecture_rejects_even_with_rebound_hashes(self):
        self.binary[68:70] = b"\x64\xaa"
        self.manifest["payload"]["sha256"] = release.sha(self.binary)
        self.write_windows()
        self.reject()

    def test_zip_alias_license_member_and_manifest_corruption_reject(self):
        for change in (lambda entries: entries.update({"flere.exe": b"different alias"}),
                       lambda entries: entries.update(LICENSE=b"wrong license"),
                       lambda entries: entries.update({"../extra.exe": b"extra"}),
                       lambda entries: entries.pop("flere.exe"),
                       lambda entries: entries.update({"manifest.json": b"{}"})):
            self.write_windows(entries_change=change)
            self.reject()

    def test_native_architecture_and_private_runner_strings_reject(self):
        for field, value in (("machine", "ARM64"), ("system", "Linux"),
                             ("image_os", "win25/secret"), ("image_version", "C:\\private")):
            self.write_windows(receipt_change=lambda item: item["machine"].update({field: value}))
            self.reject()
        self.write_windows()
        for name in ("release.json", "SHA256SUMS"):
            (self.directory / name).unlink()
        self.evidence["runner"]["linker"] = "/private/home/linker"
        self.linux_pin = release.seal(self.directory, VERSION, COMMIT, RUN, WORKFLOW,
                                      self.evidence, schema=2)["descriptor_sha256"]
        self.reject()

    def test_missing_extra_and_symlink_assets_reject_before_any_publication(self):
        proof = self.prepare()
        path = self.output / aggregate.PAYLOAD
        original = path.read_bytes()
        api = fixtures.FakeGitHub(schema=3)
        def reject_publication():
            with self.assertRaises(ValueError):
                release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
            self.assertEqual(api.writes, [])
        path.unlink()
        reject_publication()
        path.write_bytes(original)
        extra = self.output / "extra.txt"
        extra.write_bytes(b"extra")
        reject_publication()
        extra.unlink()
        path.unlink()
        path.symlink_to(self.windows / "candidate.json")
        reject_publication()

    def test_complete_publication_retry_is_read_only_and_conflicting_upload_never_overwrites(self):
        proof = self.prepare()
        api = fixtures.FakeGitHub(schema=3)
        api.fail_upload = aggregate.PAYLOAD
        with self.assertRaisesRegex(ValueError, "interruption"):
            release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertTrue(api.release["draft"])
        self.assertNotIn(("PATCH", "releases/1"), api.writes)
        api.fail_upload = None
        release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertEqual(set(api.files), release.allowlist(VERSION, 3))
        self.assertEqual(api.release["make_latest"], "false")
        writes = list(api.writes)
        release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertEqual(api.writes, writes)
        api.files[aggregate.PAYLOAD] += b"changed"
        with self.assertRaises(ValueError):
            release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertEqual(api.writes, writes)

    def test_public_verification_reads_every_asset_and_preserves_target_status(self):
        proof = self.prepare()
        fetched = []
        def download(url):
            name = url.rsplit("/", 1)[1]
            fetched.append(name)
            return (self.output / name).read_bytes()
        with mock.patch.object(release, "public_download", side_effect=download):
            checked = release.verify_public(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"])
        self.assertEqual(set(fetched), release.allowlist(VERSION, 3))
        self.assertEqual(checked["channels"], release.MULTI_CHANNELS)
        self.assertEqual(json.loads((self.output / "release.json").read_bytes())["targets"], release.MULTI_TARGETS)


if __name__ == "__main__":
    unittest.main()
