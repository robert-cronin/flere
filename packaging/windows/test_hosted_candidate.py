#!/usr/bin/env python3
"""Offline source/ZIP rejection checks; no payload, package manager or build runs."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("candidate", Path(__file__).with_name("hosted-candidate.py"))
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)


class CandidateChecks(unittest.TestCase):
    def setUp(self):
        cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "flere/tests"
        cache.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(prefix="windows-candidate-", dir=cache)
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def git(self, *args):
        return subprocess.check_output(["git", "-C", str(self.root), *args], stderr=subprocess.PIPE).decode().strip()

    def source(self):
        self.git("init", "--initial-branch=main")
        self.git("config", "core.autocrlf", "false")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "user.name", "Fixture")
        self.git("config", "user.email", "fixture@example.invalid")
        source = candidate.module("release-source")
        for name in source.REQUIRED:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"fixture\n")
        for prefix, name, build in (("", "flere", "build-support/build.rs"),
                                    ("companion/", "flere-connect", "../build-support/build.rs")):
            (self.root / (prefix + "Cargo.toml")).write_text(
                f'[package]\nname = "{name}"\nversion = "0.3.4"\nrust-version = "1.98"\nbuild = "{build}"\n')
            (self.root / (prefix + "Cargo.lock")).write_text(
                f'[[package]]\nname = "{name}"\nversion = "0.3.4"\n')
        self.git("add", "."); self.git("commit", "-qm", "fixture")
        commit = self.git("rev-parse", "HEAD")
        self.git("update-ref", "refs/remotes/origin/main", commit)
        return commit

    def test_exact_git_bytes_reject_hidden_crlf_and_stale_public_identity(self):
        commit = self.source()
        with mock.patch.object(candidate, "COMMIT", commit):
            original = candidate.source_snapshot(self.root)
            self.assertEqual(original["commit"], commit)
            # Git status can hide this change. The actual build bytes must still
            # be checked, including Windows newline conversion.
            self.git("update-index", "--assume-unchanged", "src/main.rs")
            (self.root / "src/main.rs").write_bytes(b"fixture\r\n")
            self.assertEqual(self.git("status", "--porcelain"), "")
            with self.assertRaisesRegex(ValueError, "checkout file type/size differs"):
                candidate.source_snapshot(self.root)
            (self.root / "src/main.rs").write_bytes(b"FIXTURE\n")
            with self.assertRaisesRegex(ValueError, "checkout bytes differ"):
                candidate.source_snapshot(self.root)
            (self.root / "src/main.rs").write_bytes(b"fixture\n")
            self.assertEqual(candidate.source_snapshot(self.root), original)
        with mock.patch.object(candidate, "COMMIT", "a" * 40):
            with self.assertRaises(ValueError):
                candidate.source_snapshot(self.root)

    def test_newer_main_does_not_change_selected_source_and_lock_version_is_checked(self):
        commit = self.source()
        (self.root / "README.md").write_text("later documentation\n")
        self.git("add", "."); self.git("commit", "-qm", "docs")
        self.git("update-ref", "refs/remotes/origin/main", "HEAD")
        # Only this disposable test repo is reset; its sole branch stays main.
        self.git("reset", "--hard", commit)
        with mock.patch.object(candidate, "COMMIT", commit):
            self.assertEqual(candidate.source_snapshot(self.root)["commit"], commit)
        path = self.root / "companion/Cargo.lock"
        path.write_text(path.read_text().replace('"0.3.4"', '"0.3.3"'))
        self.git("add", "."); self.git("commit", "-qm", "wrong version")
        commit = self.git("rev-parse", "HEAD")
        self.git("update-ref", "refs/remotes/origin/main", commit)
        with mock.patch.object(candidate, "COMMIT", commit):
            with self.assertRaisesRegex(ValueError, "lock"):
                candidate.source_snapshot(self.root)

    def test_generated_zip_rejects_extra_file_changed_alias_and_manifest(self):
        generator = candidate.module("windows-manifests")
        binary = bytearray(128); binary[:2] = b"MZ"
        struct.pack_into("<I", binary, 0x3c, 64); binary[64:70] = b"PE\0\0\x64\x86"
        binary = bytes(binary)
        manifest = {"payload": {"bytes": len(binary), "sha256": hashlib.sha256(binary).hexdigest()}}
        entries = {"flere.exe": binary, "flere-connect.exe": binary,
                   "manifest.json": json.dumps(manifest).encode(), "LICENSE": b"MIT fixture"}
        good = self.root / "good.zip"; generator.write_zip(good, entries)
        candidate.verify_zip(good, manifest, b"MIT fixture")
        for label, changed in (("extra", {**entries, "private.log": b"private"}),
                               ("alias", {**entries, "flere.exe": binary + b"x"}),
                               ("manifest", {**entries, "manifest.json": b"{}"})):
            with self.subTest(label=label):
                path = self.root / (label + ".zip"); generator.write_zip(path, changed)
                with self.assertRaises(ValueError):
                    candidate.verify_zip(path, manifest, b"MIT fixture")
        with zipfile.ZipFile(self.root / "duplicate.zip", "w") as archive:
            for name, data in entries.items():
                archive.writestr(name, data)
            with self.assertWarns(UserWarning):
                archive.writestr("flere.exe", binary)
        with self.assertRaisesRegex(ValueError, "inventory"):
            candidate.verify_zip(self.root / "duplicate.zip", manifest, b"MIT fixture")

    def test_hosted_guard_rejects_nonmanual_wrong_source_context_and_architecture(self):
        env = {"GITHUB_ACTIONS": "true", "FLERE_RUNNER_ENVIRONMENT": "github-hosted",
               "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REPOSITORY": "robert-cronin/flere",
               "GITHUB_REF": "refs/heads/main", "RUNNER_OS": "Windows", "RUNNER_ARCH": "X64",
               "FLERE_WORKFLOW_SHA": "a" * 40, "GITHUB_RUN_ID": "123", "GITHUB_RUN_ATTEMPT": "1"}
        candidate.hosted(env)
        for key, value in (("FLERE_RUNNER_ENVIRONMENT", "self-hosted"), ("GITHUB_EVENT_NAME", "push"),
                           ("GITHUB_REF", "refs/heads/other"), ("RUNNER_ARCH", "ARM64"),
                           ("FLERE_WORKFLOW_SHA", "main"), ("GITHUB_RUN_ID", "0")):
            with self.subTest(key=key), self.assertRaises(ValueError):
                candidate.hosted({**env, key: value})

    def test_nupkg_rejects_executable_wrong_version_and_changed_script(self):
        path = self.root / "flere-connect.0.3.4.nupkg"
        spec = b'<package xmlns="urn:fixture"><metadata><id>flere-connect</id><version>0.3.4</version></metadata></package>'
        entries = {"_rels/.rels": b"<Relationships/>", "[Content_Types].xml": b"<Types/>",
                   "flere-connect.nuspec": spec, "tools/chocolateyInstall.ps1": b"reviewed fixture",
                   "package/services/metadata/core-properties/" + "a" * 32 + ".psmdcp": b"<coreProperties/>"}
        for label, values in (("valid", entries), ("exe", {**entries, "tools/private.exe": b"MZ"}),
                              ("version", {**entries, "flere-connect.nuspec": spec.replace(b"0.3.4", b"0.3.3")}),
                              ("script", {**entries, "tools/chocolateyInstall.ps1": b"different"})):
            with self.subTest(label=label):
                with zipfile.ZipFile(path, "w") as archive:
                    for name, data in values.items():
                        archive.writestr(name, data)
                if label == "valid":
                    self.assertEqual(len(candidate.verify_nupkg(path, b"reviewed fixture")), 5)
                else:
                    with self.assertRaises(ValueError):
                        candidate.verify_nupkg(path, b"reviewed fixture")


if __name__ == "__main__":
    unittest.main()
