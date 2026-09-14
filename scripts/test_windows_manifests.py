#!/usr/bin/env python3
"""Offline packaging checks; the synthetic PE header is never executed."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zipfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("windows_manifests", Path(__file__).with_name("windows-manifests.py"))
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class WindowsPackaging(unittest.TestCase):
    def setUp(self):
        cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home() / ".cache")) / "flere/tests"
        cache.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.temp = tempfile.TemporaryDirectory(prefix="windows-packaging-", dir=cache)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.assets = self.root / "assets"
        self.assets.mkdir()
        self.target = "x86_64-pc-windows-msvc"
        self.name = f"flere-connect-{self.target}"
        binary = bytearray(128)
        binary[:2] = b"MZ"
        struct.pack_into("<I", binary, 0x3c, 64)
        binary[64:70] = b"PE\0\0\x64\x86"
        self.binary = bytes(binary)
        (self.assets / self.name).write_bytes(self.binary)
        self.manifest = {
            "schema_version": 1,
            "build": {"component": "flere-connect", "target": self.target,
                      "package_version": "0.3.0", "profile": "release"},
            "payload": {"file_name": "flere-connect", "download_file": self.name,
                        "bytes": len(binary), "sha256": hashlib.sha256(binary).hexdigest()},
            "source": {"git_commit": "a" * 40, "source_sha256": "b" * 64, "dirty": False},
        }
        self.save_manifest()
        # Extra local files must never be swept into a redistributable package.
        (self.assets / "private.log").write_text("PRIVATE FIXTURE - EXCLUDE")

    def save_manifest(self):
        (self.assets / (self.name + ".manifest.json")).write_text(json.dumps(self.manifest))

    def test_allowlisted_reproducible_archive_and_catalogue_hashes(self):
        result = packager.prepare(self.assets, self.root / "one", self.target)
        second = packager.prepare(self.assets, self.root / "two", self.target)
        self.assertEqual(result["sha256"], second["sha256"])
        archive_path = self.root / "one" / result["asset"]
        with zipfile.ZipFile(archive_path) as archive:
            self.assertEqual(set(archive.namelist()), {"flere.exe", "flere-connect.exe", "manifest.json", "LICENSE"})
            for name in ("flere.exe", "flere-connect.exe"):
                self.assertEqual(archive.read(name), self.binary)
            self.assertTrue(all(info.date_time == (1980, 1, 1, 0, 0, 0) for info in archive.infolist()))
        scoop = json.loads((self.root / "one/scoop/bucket/flere.json").read_text())
        self.assertEqual(scoop["architecture"]["64bit"]["hash"], hashlib.sha256(archive_path.read_bytes()).hexdigest())
        self.assertIn("/releases/download/v0.3.0/", scoop["architecture"]["64bit"]["url"])
        installer = next((self.root / "one/winget").rglob("*.installer.yaml")).read_text()
        self.assertIn(result["sha256"].upper(), installer)
        self.assertIn("PortableCommandAlias: flere-connect", installer)
        self.assertEqual(result["status"], "prepared_not_published")

    def test_corrupt_payload_is_rejected_before_output(self):
        (self.assets / self.name).write_bytes(self.binary + b"corruption")
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            packager.prepare(self.assets, self.root / "out", self.target)
        self.assertFalse((self.root / "out").exists())

    def test_unsupported_identity_injection_and_dirty_source_are_rejected(self):
        original = json.dumps(self.manifest)
        for section, key, value in [
                ("build", "component", "flere"), ("build", "package_version", "0.3.0\nInjected: yes"),
                ("payload", "download_file", "../private.log"), ("source", "dirty", True)]:
            with self.subTest(key=key):
                self.manifest = json.loads(original)
                self.manifest[section][key] = value
                self.save_manifest()
                with self.assertRaises(ValueError):
                    packager.prepare(self.assets, self.root / "out", self.target)
                self.assertFalse((self.root / "out").exists())

    def test_wrong_pe_architecture_and_symlink_are_rejected(self):
        wrong = bytearray(self.binary)
        wrong[68:70] = b"\x4c\x01"
        (self.assets / self.name).write_bytes(wrong)
        self.manifest["payload"]["sha256"] = hashlib.sha256(wrong).hexdigest()
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, "x86_64 PE"):
            packager.prepare(self.assets, self.root / "out", self.target)
        (self.assets / self.name).unlink()
        (self.root / "elsewhere").write_bytes(wrong)
        (self.assets / self.name).symlink_to(self.root / "elsewhere")
        with self.assertRaisesRegex(ValueError, "regular input"):
            packager.prepare(self.assets, self.root / "out", self.target)


if __name__ == "__main__":
    unittest.main()
