#!/usr/bin/env python3
"""Offline Homebrew recipe checks using inert packages in a disposable home cache."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("flere_homebrew", ROOT / "packaging/homebrew-prebuilt/render.py")
homebrew = importlib.util.module_from_spec(spec)
spec.loader.exec_module(homebrew)
sys.path.insert(0, str(ROOT / "packaging/homebrew"))
spec = importlib.util.spec_from_file_location("flere_homebrew_source", ROOT / "packaging/homebrew/render.py")
source_render = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source_render)
sys.path.pop(0)


class HomebrewDistribution(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / ".cache/flere/tmp"
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.temporary = tempfile.TemporaryDirectory(prefix="homebrew-", dir=parent)
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)
        self.assets = self.root / "assets"
        self.assets.mkdir()
        for component in homebrew.COMPONENTS:
            for target in homebrew.TARGETS:
                name = f"{component}-{target}"
                payload = f"inert {name}\n".encode()
                (self.assets / name).write_bytes(payload)
                manifest = {
                    "schema_version": 1,
                    "build": {"component": component, "target": target,
                              "package_version": "0.3.0", "profile": "release"},
                    "source": {"source_sha256": "a" * 64},
                    "payload": {"file_name": component, "download_file": name,
                                "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()},
                }
                (self.assets / f"{name}.manifest.json").write_text(json.dumps(manifest))

    def prepare(self, *extra):
        argv = ["render.py", str(self.assets), "--output", str(self.root / "tap"), *extra]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
            homebrew.main()

    def change_manifest(self, change):
        path = self.assets / "flere-aarch64-apple-darwin.manifest.json"
        manifest = json.loads(path.read_bytes())
        change(manifest)
        path.write_text(json.dumps(manifest))

    def test_release_checksums_urls_and_output_are_consistent(self):
        self.prepare()
        self.prepare("--check")
        for component in homebrew.COMPONENTS:
            recipe = (self.root / "tap/Casks" / f"{component}.rb").read_text()
            self.assertIn('version "0.3.0"', recipe)
            for target in homebrew.TARGETS:
                binary = self.assets / f"{component}-{target}"
                self.assertIn(hashlib.sha256(binary.read_bytes()).hexdigest(), recipe)
                self.assertIn(f"/releases/download/v#{{version}}/{binary.name}", recipe)
            self.assertNotIn(":no_check", recipe)
            self.assertNotIn("uninstall", recipe.split("caveats", 1)[0])
        path = self.root / "tap/Casks/flere.rb"
        path.write_text(path.read_text().replace('version "0.3.0"', 'version "0.4.0"'))
        with self.assertRaisesRegex(ValueError, "does not match"):
            self.prepare("--check")

    def test_rejects_modified_payload_before_writing(self):
        (self.assets / "flere-connect-x86_64-unknown-linux-gnu").write_bytes(b"tampered")
        with self.assertRaisesRegex(ValueError, "payload differs"):
            self.prepare()
        self.assertFalse((self.root / "tap").exists())

    def test_rejects_mixed_version_and_source_before_writing(self):
        for key, new, old in (("package_version", "0.4.0", "0.3.0"), ("source_sha256", "b" * 64, "a" * 64)):
            section = "build" if key == "package_version" else "source"
            self.change_manifest(lambda m: m[section].update({key: new}))
            with self.assertRaisesRegex(ValueError, "same release version and source"):
                self.prepare()
            self.assertFalse((self.root / "tap").exists())
            self.change_manifest(lambda m: m[section].update({key: old}))

    def test_rejects_version_code_injection(self):
        self.change_manifest(lambda m: m["build"].update(package_version='0.3.0#{system("false")}'))
        with self.assertRaisesRegex(ValueError, "invalid release version"):
            self.prepare()
        self.assertFalse((self.root / "tap").exists())

    def test_rejects_symlink_payload(self):
        binary = self.assets / "flere-aarch64-apple-darwin"
        renamed = binary.with_suffix(".original")
        binary.rename(renamed)
        binary.symlink_to(renamed.name)
        with self.assertRaisesRegex(ValueError, "payload differs"):
            self.prepare()

    def test_glibc_guard_accepts_minimum_and_rejects_old_or_unknown_runtime(self):
        # Substitute only the read-only runtime probe; execute the shipped guard in /bin/sh.
        probe = self.root / "getconf"
        probe.write_text('#!/bin/sh\nprintf "%s\\n" "$TEST_GLIBC"\n')
        probe.chmod(0o755)
        guard = self.root / "guard.sh"
        guard.write_text(homebrew.GLIBC_CHECK.replace("/usr/bin/getconf", str(probe)))
        for version, allowed in (("glibc 2.39", True), ("glibc 2.40", True), ("glibc 3.0", True),
                                 ("glibc 2.38", False), ("glibc 1.99", False), ("musl 1.2.5", False),
                                 ("glibc x.y", False), ("glibc 2", False), ("glibc 3", False), ("glibc 2.39.1", False),
                                 ("", False)):
            with self.subTest(version=version):
                result = subprocess.run(["/bin/sh", str(guard)], cwd=self.root,
                                        env={**os.environ, "TEST_GLIBC": version}, capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, allowed, result.stderr)


    def source_archive(self, *, omitted=None, companion_version="0.3.0"):
        archive = self.root / "flere-0.3.0-source.tar.gz"
        from verify import REQUIRED
        with tarfile.open(archive, "w:gz") as package:
            for name in REQUIRED:
                if name == omitted:
                    continue
                version = companion_version if name == "companion/Cargo.toml" else "0.3.0"
                data = (f'version = "{version}"\nrust-version = "1.98"\n'
                        if name.endswith("Cargo.toml") else "inert input\n").encode()
                entry = tarfile.TarInfo("flere-0.3.0/" + name)
                entry.size = len(data)
                package.addfile(entry, io.BytesIO(data))
        return archive, hashlib.sha256(archive.read_bytes()).hexdigest()

    def render_source(self, archive, checksum, *extra):
        argv = ["render.py", str(archive), "--sha256", checksum,
                "--output", str(self.root / "source-tap"), *extra]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
            source_render.main()

    def test_source_render_and_verifier_agree_on_both_formulas(self):
        archive, checksum = self.source_archive()
        self.render_source(archive, checksum)
        self.render_source(archive, checksum, "--check")
        from verify import verify
        tap = self.root / "source-tap"
        self.assertEqual(verify(archive, tap), "0.3.0")
        for component in ("flere", "flere-connect"):
            formula = (tap / "Formula" / f"{component}.rb").read_text()
            self.assertIn(checksum, formula)
            self.assertIn('"cargo", "fetch", "--locked"', formula)
            self.assertIn('"cargo", "install", "--offline"', formula)
        path = tap / "Formula/flere.rb"
        path.write_text(path.read_text().replace('version "0.3.0"', 'version "0.4.0"'))
        with self.assertRaisesRegex(ValueError, "differs from verified"):
            self.render_source(archive, checksum, "--check")

    def test_source_render_rejects_checksum_corruption_before_writing(self):
        archive, checksum = self.source_archive()
        archive.write_bytes(archive.read_bytes() + b"changed")
        with self.assertRaisesRegex(ValueError, "trusted SHA-256"):
            self.render_source(archive, checksum)
        self.assertFalse((self.root / "source-tap").exists())

    def test_source_render_requires_shared_inputs_and_consistent_versions(self):
        for changes, error in (({"omitted": "build-support/build.rs"}, "missing"),
                               ({"omitted": "src/assets/fonts/OFL.txt"}, "missing"),
                               ({"companion_version": "0.4.0"}, "version or Rust")):
            with self.subTest(changes=changes):
                archive, checksum = self.source_archive(**changes)
                with self.assertRaisesRegex(ValueError, error):
                    self.render_source(archive, checksum)
                self.assertFalse((self.root / "source-tap").exists())


if __name__ == "__main__":
    unittest.main()
