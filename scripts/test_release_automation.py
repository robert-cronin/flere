#!/usr/bin/env python3
"""Offline adversarial release-boundary tests; no real GitHub writes or payload execution."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_automation", Path(__file__).with_name("release-automation.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
ci_spec = importlib.util.spec_from_file_location("release_linux_ci", Path(__file__).with_name("release-linux-ci.py"))
ci = importlib.util.module_from_spec(ci_spec)
ci_spec.loader.exec_module(ci)

VERSION = "0.4.0"
COMMIT = "a" * 40
WORKFLOW = "b" * 40
RUN = "1234"


class FakeGitHub:
    def __init__(self):
        self.release = None
        self.tag = None
        self.files = {}
        self.writes = []
        self.fail_upload = None
        self.immutable = True

    def get(self, path, **kwargs):
        if path == f"releases/tags/v{VERSION}":
            return copy.deepcopy(self.release)
        if path == f"git/ref/tags/v{VERSION}":
            return copy.deepcopy(self.tag)
        if path == "releases/1/assets?per_page=100":
            return [{"id": index, "name": name, "state": "uploaded", "size": len(data)}
                    for index, (name, data) in enumerate(self.files.items())]
        if path.startswith("releases/assets/"):
            return list(self.files.values())[int(path.rsplit("/", 1)[1])]
        raise AssertionError(path)

    def request(self, method, path, data, **kwargs):
        self.writes.append((method, path))
        if method == "POST" and path == "releases":
            self.release = dict(copy.deepcopy(data), id=1, immutable=False)
            return copy.deepcopy(self.release)
        if method == "POST" and path.startswith("releases/1/assets?name="):
            name = path.split("?name=")[1]
            if name == self.fail_upload:
                raise ValueError("simulated upload interruption")
            if name in self.files:
                raise AssertionError("publisher attempted an overwrite")
            self.files[name] = data
            return {"name": name}
        if method == "PATCH" and path == "releases/1":
            if set(self.files) != release.allowlist():
                raise AssertionError("publication before complete asset upload")
            self.release.update(data, immutable=self.immutable)
            self.tag = {"object": {"type": "commit", "sha": COMMIT,
                                  "url": f"https://api.github.com/repos/{release.REPOSITORY}/git/commits/{COMMIT}"}}
            return copy.deepcopy(self.release)
        raise AssertionError((method, path))


class ReleaseAutomation(unittest.TestCase):
    def setUp(self):
        cache = Path.home() / ".cache/flere/tmp"
        cache.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.temporary = tempfile.TemporaryDirectory(prefix="release-automation-", dir=cache)
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.api = FakeGitHub()
        self.evidence = {"checks": release.CHECKS, "payloads": {},
                         "elf_needed": {c: ["libc.so.6"] for c in release.COMPONENTS}, "runner": {
            "os": "ubuntu-24.04", "architecture": "x86_64", "image": "ubuntu24",
            "image_version": "fixture", "rustc": "rustc 1.98.0 (fixture)",
            "linker": "fixture", "glibc": "2.39"}}
        for component in release.COMPONENTS:
            name = f"{component}-{release.TARGET}"
            payload = b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x3e\x00" + component.encode()
            item = {"schema_version": 1, "build": {"component": component,
                    "target": release.TARGET, "package_version": VERSION, "profile": "release",
                    "compatibility": {"remote_protocol": {"current": "v6", "accepts": ["v6"]}}},
                    "source": {"git_commit": COMMIT, "dirty": False, "source_sha256": "c" * 64},
                    "payload": {"file_name": component, "download_file": name,
                                "bytes": len(payload), "sha256": release.sha(payload)}}
            (self.directory / name).write_bytes(payload)
            (self.directory / (name + ".manifest.json")).write_bytes(release.json_bytes(item))
            self.evidence["payloads"][component] = release.sha(payload)

    def seal(self):
        return release.seal(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.evidence)["descriptor_sha256"]

    def publish(self, digest):
        return release.publish(self.directory, VERSION, COMMIT, RUN, WORKFLOW, digest, self.api)

    def mutate_manifest(self, change):
        path = self.directory / f"flere-{release.TARGET}.manifest.json"
        item = json.loads(path.read_bytes())
        change(item)
        path.write_bytes(release.json_bytes(item))

    def test_input_injection_and_ambiguous_identities_are_rejected(self):
        for version in ("v0.4.0", "0.04.0", "0.4.0;echo bad", "0.4.0\nother=evil", "0.4.0-rc1", "../0.4.0"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.identity(version, COMMIT, RUN)
        for commit in ("main", "a" * 7, "A" * 40, "-" + "a" * 39):
            with self.subTest(commit=commit), self.assertRaises(ValueError):
                release.identity(VERSION, commit, RUN)

    def test_wrong_source_and_dirty_checkout_fail_before_sealing(self):
        for field, value in (("git_commit", "d" * 40), ("dirty", True)):
            with self.subTest(field=field):
                self.mutate_manifest(lambda item: item["source"].update({field: value}))
                with self.assertRaisesRegex(ValueError, "wrong-source"):
                    self.seal()
                self.assertFalse((self.directory / "release.json").exists())
                self.mutate_manifest(lambda item: item["source"].update(git_commit=COMMIT, dirty=False))

    def test_changed_binary_with_pre_signing_checksum_stops_before_writes(self):
        digest = self.seal()
        binary = self.directory / f"flere-{release.TARGET}"
        binary.write_bytes(binary.read_bytes() + b"byte-changing-signature")
        with self.assertRaisesRegex(ValueError, "stale checksum"):
            self.publish(digest)
        self.assertEqual(self.api.writes, [])

    def test_replaced_descriptor_or_wrong_run_cannot_be_promoted(self):
        digest = self.seal()
        with self.assertRaisesRegex(ValueError, "provenance"):
            release.validate(self.directory, VERSION, COMMIT, "5678", WORKFLOW, digest)
        descriptor = self.directory / "release.json"
        descriptor.write_bytes(descriptor.read_bytes() + b" ")
        with self.assertRaisesRegex(ValueError, "descriptor digest"):
            self.publish(digest)
        self.assertEqual(self.api.writes, [])

    def test_extra_asset_and_symlink_fail_before_github_writes(self):
        digest = self.seal()
        extra = self.directory / "private-token"
        extra.write_text("never publish")
        with self.assertRaisesRegex(ValueError, "allowlist"):
            self.publish(digest)
        extra.unlink()
        binary = self.directory / f"flere-{release.TARGET}"
        binary.unlink()
        binary.symlink_to("release.json")
        with self.assertRaisesRegex(ValueError, "regular release file"):
            self.publish(digest)
        self.assertEqual(self.api.writes, [])

    def test_missing_native_or_final_digest_acceptance_is_rejected(self):
        self.evidence["payloads"]["flere"] = "0" * 64
        with self.assertRaisesRegex(ValueError, "acceptance"):
            self.seal()
        self.evidence["payloads"]["flere"] = release.sha((self.directory / f"flere-{release.TARGET}").read_bytes())
        self.evidence["runner"]["architecture"] = "aarch64"
        with self.assertRaisesRegex(ValueError, "acceptance"):
            self.seal()

    def test_windows_and_macos_cannot_be_added_with_a_build_only_receipt(self):
        digest = self.seal()
        for target in release.BLOCKED:
            extra = self.directory / f"flere-connect-{target}"
            extra.write_bytes(b"compile-only")
            with self.assertRaisesRegex(ValueError, "allowlist"):
                self.publish(digest)
            extra.unlink()
        self.assertEqual(self.api.writes, [])

    def test_draft_upload_and_ambiguous_publication_retry_preserve_bytes(self):
        digest = self.seal()
        self.api.fail_upload = f"flere-{release.TARGET}"
        with self.assertRaisesRegex(ValueError, "interruption"):
            self.publish(digest)
        self.assertTrue(self.api.release["draft"])
        retained = copy.deepcopy(self.api.files)
        self.api.fail_upload = None
        result = self.publish(digest)
        self.assertEqual(result["status"], "published_immutable")
        self.assertFalse(self.api.release["draft"])
        self.assertEqual(self.api.release["make_latest"], "false")
        self.assertEqual(set(self.api.files), release.allowlist())
        for name, data in retained.items():
            self.assertEqual(self.api.files[name], data)
        writes = list(self.api.writes)
        self.publish(digest)
        self.assertEqual(self.api.writes, writes)

    def test_conflicting_draft_is_never_adopted_or_repaired(self):
        digest = self.seal()
        self.api.fail_upload = f"flere-{release.TARGET}"
        with self.assertRaises(ValueError):
            self.publish(digest)
        self.api.files["SHA256SUMS"] = b"foreign candidate"
        writes = list(self.api.writes)
        with self.assertRaisesRegex(ValueError, "wrong size"):
            self.publish(digest)
        self.assertEqual(self.api.writes, writes)
        self.api.release["body"] = "another run"
        with self.assertRaisesRegex(ValueError, "another candidate"):
            self.publish(digest)
        self.assertEqual(self.api.writes, writes)

    def test_tag_race_is_detected_without_overwrite(self):
        digest = self.seal()
        self.api.tag = {"object": {"type": "commit", "sha": "d" * 40}}
        with self.assertRaisesRegex(ValueError, "tag already exists"):
            self.publish(digest)
        self.assertEqual(self.api.writes, [])

    def test_nonimmutable_publication_is_reported_without_destructive_repair(self):
        digest = self.seal()
        self.api.immutable = False
        with self.assertRaisesRegex(ValueError, "immutable release"):
            self.publish(digest)
        self.assertFalse(self.api.release["draft"])
        self.assertFalse(any(method == "DELETE" for method, _ in self.api.writes))

    def test_public_download_failure_leaves_release_and_channels_unchanged(self):
        digest = self.seal()
        self.publish(digest)
        writes = list(self.api.writes)
        with mock.patch.object(release, "public_download", return_value=b"corrupted"):
            with self.assertRaisesRegex(ValueError, "public download"):
                release.verify_public(self.directory, VERSION, COMMIT, RUN, WORKFLOW, digest)
        self.assertEqual(self.api.writes, writes)
        self.assertFalse(self.api.release["draft"])

    def test_public_fetch_is_constrained_to_https_github_download_hosts(self):
        for url in ("http://github.com/asset", "https://github.com.evil.example/asset",
                    "https://user:password@github.com/asset", "https://github.com:443/asset"):
            with self.subTest(url=url), self.assertRaisesRegex(ValueError, "allowlisted HTTPS"):
                release.check_public_url(url)

    def test_select_rejects_existing_version_and_requires_main_dispatch(self):
        def git(*args):
            command = args[1:]
            if command == ("rev-parse", "HEAD"):
                return WORKFLOW
            if command[0] == "cat-file":
                return "commit"
            if command[0] == "show":
                return '[package]\nversion = "0.4.0"\n'
            raise AssertionError(command)
        environment = {"GITHUB_REF": "refs/heads/main", "GITHUB_EVENT_NAME": "workflow_dispatch",
                       "GITHUB_REPOSITORY": release.REPOSITORY}
        with mock.patch.dict(os.environ, environment), mock.patch.object(release, "git", side_effect=git), mock.patch.object(release.subprocess, "run"):
            self.api.release = {"draft": True}
            with self.assertRaisesRegex(ValueError, "already has"):
                release.select(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.api)
            self.api.release = None
            result = release.select(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.api)
            self.assertEqual(result["commit"], COMMIT)
            with mock.patch.dict(os.environ, {"GITHUB_REF": "refs/tags/v0.4.0"}):
                with self.assertRaisesRegex(ValueError, "trusted main"):
                    release.select(self.directory, VERSION, COMMIT, RUN, WORKFLOW, self.api)

    def test_newer_glibc_or_missing_dynamic_symbol_proof_is_rejected(self):
        ci.inspect_elf("Name: GLIBC_2.2.5\nName: GLIBC_2.39\n")
        for text in ("Name: GLIBC_2.40\n", "not an ELF version report"):
            with self.subTest(text=text), self.assertRaisesRegex(ValueError, "GLIBC"):
                ci.inspect_elf(text)

    def test_required_library_outside_declared_baseline_is_rejected(self):
        text = "0x0000000000000001 (NEEDED) Shared library: [libc.so.6]\n"
        self.assertEqual(ci.needed_libraries(text), ["libc.so.6"])
        with self.assertRaisesRegex(ValueError, "outside the declared"):
            ci.needed_libraries(text + "0x0000000000000001 (NEEDED) Shared library: [libmystery.so.1]\n")
        self.evidence["elf_needed"]["flere"] = ["libc.so.6", "libmystery.so.1"]
        with self.assertRaisesRegex(ValueError, "required ELF libraries"):
            self.seal()


if __name__ == "__main__":
    unittest.main()
