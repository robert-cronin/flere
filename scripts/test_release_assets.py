#!/usr/bin/env python3
"""Offline release preparation checks using inert, locally generated packages."""
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("flere_release_assets", Path(__file__).with_name("release-assets.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseAssets(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / ".cache/flere/tmp"
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.temporary = tempfile.TemporaryDirectory(prefix="release-assets-", dir=parent)
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)

    def package(self, component, target, *, version="0.3.0", source="a" * 64, accepts=None):
        directory = self.root / f"{component}-{target}"
        directory.mkdir(exist_ok=True, mode=0o700)
        payload = f"inert {component} {target}\n".encode()
        (directory / component).write_bytes(payload)
        manifest = {
            "schema_version": 1,
            "build": {"component": component, "target": target, "package_version": version,
                      "compatibility": {"remote_protocol": {
                          "current": "flere-remote-v6",
                          "accepts": ["flere-remote-v6"] if accepts is None else accepts}}},
            "payload": {"file_name": component, "download_file": directory.name,
                        "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()},
            "source": {"source_sha256": source},
        }
        (directory / "manifest.json").write_text(json.dumps(manifest))
        return directory

    def prepare(self, output, packages):
        argv = ["release-assets.py", str(output), *(str(package) for package in packages)]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
            release.main()

    def test_inconsistent_releases_fail_before_creating_output(self):
        core = self.package("flere", "x86_64-unknown-linux-gnu")
        cases = [
            ("flere-connect", "x86_64-unknown-linux-gnu", {"version": "0.4.0"}, "same version"),
            ("flere-connect", "x86_64-unknown-linux-gnu", {"source": "b" * 64}, "source SHA-256"),
            ("flere", "aarch64-apple-darwin", {"version": "0.4.0"}, "same version"),
            ("flere", "aarch64-apple-darwin", {"source": "b" * 64}, "source SHA-256"),
            ("flere-connect", "x86_64-unknown-linux-gnu", {"accepts": ["flere-remote-v5"]}, "paired core protocol"),
            ("flere-connect", "x86_64-unknown-linux-gnu", {"source": None}, "recorded source SHA-256"),
        ]
        for index, (component, target, changes, error) in enumerate(cases):
            with self.subTest(component=component, target=target, changes=changes):
                other = self.package(component, target, **changes)
                output = self.root / f"rejected-{index}"
                with self.assertRaisesRegex(ValueError, error):
                    self.prepare(output, [core, other])
                self.assertFalse(output.exists())

    def test_multi_target_release_allows_unpaired_server_and_windows_companion(self):
        packages = [self.package(component, target) for component, target in [
            ("flere", "x86_64-unknown-linux-gnu"),
            ("flere-connect", "x86_64-unknown-linux-gnu"),
            ("flere", "aarch64-apple-darwin"),
            ("flere-connect", "x86_64-pc-windows-gnu"),
        ]]
        output = self.root / "prepared"
        self.prepare(output, packages)
        plan = json.loads((output / "publication-plan.json").read_bytes())
        self.assertEqual(plan["status"], "prepared_not_published")
        self.assertEqual(len(plan["assets"]), 4)
        sums = (output / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(sums), 8)
        for checksum in sums:
            expected, name = checksum.split("  ", 1)
            self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), expected)
        for package in packages:
            original = json.loads((package / "manifest.json").read_bytes())
            self.assertEqual(json.loads((output / (package.name + ".manifest.json")).read_bytes()), original)
            self.assertEqual((output / package.name).read_bytes(), (package / original["build"]["component"]).read_bytes())


if __name__ == "__main__":
    unittest.main()
