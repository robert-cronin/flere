#!/usr/bin/env python3
"""Offline packaging checks; the synthetic PE header is never executed."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import struct
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import xml.etree.ElementTree as ET
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

    def test_scoop_retains_public_zip_and_only_adds_fixed_metadata_rename(self):
        result = packager.prepare(self.assets, self.root / "scoop", self.target)
        recipe = json.loads((self.root / "scoop/scoop/bucket/flere.json").read_text())
        self.assertEqual(recipe["pre_install"], packager.SCOOP_PRE_INSTALL)
        self.assertEqual(recipe["bin"], ["flere.exe", "flere-connect.exe"])
        self.assertEqual(recipe, packager.scoop_manifest("0.3.0", result["url"], result["sha256"]))
        with zipfile.ZipFile(self.root / "scoop" / result["asset"]) as archive:
            self.assertEqual(archive.read("manifest.json"), (self.assets / (self.name + ".manifest.json")).read_bytes())
            self.assertNotIn("flere-release.manifest.json", archive.namelist())

    @unittest.skipUnless(shutil.which("pwsh"), "actual rename requires PowerShell; hosted Windows runs this check")
    def test_actual_scoop_hook_preserves_bytes_and_fails_closed(self):
        source = b'{"build":"fixture"}\r\n'
        for case in ("normal", "missing", "collision"):
            folder = self.root / case
            folder.mkdir()
            if case != "missing":
                (folder / "manifest.json").write_bytes(source)
            if case == "collision":
                (folder / "flere-release.manifest.json").write_bytes(b"existing record")
            command = "$ErrorActionPreference='Stop'; $dir=" + packager.powershell_literal(str(folder)) + "; " + packager.SCOOP_PRE_INSTALL
            result = subprocess.run([shutil.which("pwsh"), "-NoLogo", "-NoProfile", "-Command", command],
                                    stdin=subprocess.DEVNULL, capture_output=True, timeout=15)
            self.assertLess(len(result.stdout) + len(result.stderr), 65536)
            if case == "normal":
                self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
                self.assertFalse((folder / "manifest.json").exists())
                (folder / "manifest.json").write_bytes(b'{"version":"0.3.0"}')
                self.assertEqual((folder / "flere-release.manifest.json").read_bytes(), source)
                self.assertEqual(json.loads((folder / "manifest.json").read_bytes()), {"version":"0.3.0"})
            else:
                self.assertNotEqual(result.returncode, 0)
                if case == "collision":
                    self.assertEqual((folder / "flere-release.manifest.json").read_bytes(), b"existing record")
                    self.assertEqual((folder / "manifest.json").read_bytes(), source)

    def test_winget_headers_match_each_type_and_portable_has_no_scope(self):
        packager.prepare(self.assets, self.root / "out", self.target)
        manifests = list((self.root / "out/winget").rglob("*.yaml"))
        self.assertEqual(len(manifests), 3)
        types = set()
        for path in manifests:
            text = path.read_text()
            fields = dict(line.split(": ", 1) for line in text.splitlines()
                          if line.startswith(("ManifestType: ", "ManifestVersion: ")))
            kind = fields["ManifestType"]
            types.add(kind)
            self.assertEqual(text.splitlines()[0],
                             "# yaml-language-server: $schema=https://aka.ms/winget-manifest."
                             + kind + "." + fields["ManifestVersion"] + ".schema.json")
            if kind == "installer":
                self.assertIn("NestedInstallerType: portable\n", text)
                self.assertNotIn("Scope:", text)
                self.assertIn("PortableCommandAlias: flere\n", text)
                self.assertIn("PortableCommandAlias: flere-connect\n", text)
        self.assertEqual(types, {"version", "defaultLocale", "installer"})

    def test_chocolatey_binds_same_zip_and_allows_only_install_script(self):
        result = packager.prepare(self.assets, self.root / "out", self.target)
        package = self.root / "out" / result["chocolatey"]["directory"]
        expected = {"flere-connect.nuspec", "tools/chocolateyInstall.ps1"}
        self.assertEqual({p.relative_to(package).as_posix() for p in package.rglob("*") if p.is_file()}, expected)
        self.assertEqual(set(result["chocolatey"]["files"]), expected)
        for name, identity in result["chocolatey"]["files"].items():
            data = (package / name).read_bytes()
            self.assertEqual(identity, {"bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
        ns = {"n": "http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd"}
        spec = ET.parse(package / "flere-connect.nuspec").getroot()
        self.assertEqual(spec.findtext("n:metadata/n:id", namespaces=ns), "flere-connect")
        self.assertEqual(spec.findtext("n:metadata/n:version", namespaces=ns), result["version"])
        self.assertIn("not a Windows core", spec.findtext("n:metadata/n:description", namespaces=ns))
        self.assertEqual([e.attrib for e in spec.findall("n:files/n:file", ns)],
                         [{"src": "tools\\chocolateyInstall.ps1", "target": "tools"}])
        # The explicit file map still excludes a nearby file when a maintainer packs.
        (package / "tools/private.log").write_text("PRIVATE FIXTURE - EXCLUDE")
        mapped = {e.attrib["src"].replace("\\", "/") for e in spec.findall("n:files/n:file", ns)}
        self.assertEqual(mapped, {"tools/chocolateyInstall.ps1"})
        raw = (package / "tools/chocolateyInstall.ps1").read_bytes()
        self.assertTrue(raw.startswith(b"\xef\xbb\xbf"))
        script = raw.decode("utf-8-sig")
        self.assertIn("url64bit      = " + packager.powershell_literal(result["url"]), script)
        self.assertIn("checksum64    = '" + result["sha256"] + "'", script)
        self.assertIn("checksumType64 = 'sha256'", script)
        self.assertIn("unzipLocation = Join-Path $toolsDir 'app'", script)
        guard = script.split("$toolsDir =", 1)[0]
        self.assertIn("$architecture = $env:PROCESSOR_ARCHITECTURE", guard)
        self.assertIn("if ($env:PROCESSOR_ARCHITEW6432) {\n"
                      "    $architecture = $env:PROCESSOR_ARCHITEW6432\n}", guard)
        self.assertIn("$architecture -ne 'AMD64' -or", guard)
        self.assertLess(guard.index("$architecture = $env:PROCESSOR_ARCHITEW6432"),
                        guard.index("$architecture -ne 'AMD64'"))
        self.assertNotIn("Get-OSArchitectureWidth", script)
        self.assertIn("$env:ChocolateyForceX86 -eq 'true'", script)
        self.assertEqual(script.count("Install-ChocolateyZipPackage @packageArgs"), 1)
        for forbidden in ("Start-Process", "Invoke-Expression", "Remove-Item", "Stop-Process",
                          "Install-BinFile", "$env:APPDATA", "$env:USERPROFILE", "--adopt"):
            self.assertNotIn(forbidden, script)
        self.assertFalse(list(package.rglob("*.exe")))

    def test_chocolatey_xml_and_powershell_literals_do_not_interpolate_metadata(self):
        url = "https://example.invalid/a'b?x=$value&y=`whoami`"
        with mock.patch.object(packager, "REPOSITORY", url):
            files = packager.chocolatey_files("0.3.0", url, "c" * 64)
        spec = ET.fromstring(files["flere-connect.nuspec"])
        ns = {"n": "http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd"}
        self.assertEqual(spec.findtext("n:metadata/n:projectUrl", namespaces=ns), url)
        script = files["tools/chocolateyInstall.ps1"].decode("utf-8-sig")
        literal = script.split("url64bit      = ", 1)[1].splitlines()[0]
        self.assertEqual(literal, "'https://example.invalid/a''b?x=$value&y=`whoami`'")
        self.assertEqual(literal[1:-1].replace("''", "'"), url)

    def test_release_versions_are_exact_and_cannot_inject_or_normalize(self):
        for version in ("0.3.0", "1.20.300"):
            self.manifest["build"]["package_version"] = version
            self.save_manifest()
            result = packager.prepare(self.assets, self.root / version, self.target)
            self.assertIn(f"/v{version}/flere-connect-{version}-", result["url"])
        for version in ("00.3.0", "0.03.0", "0.3.00", "0.3", "0.3.0.1", "0.3.0-beta",
                        "0.3.0+build", "2147483648.0.0", "9" * 1000 + ".0.0",
                        "0.3.0'; Stop-Process -Name test; #", "0.3.0\n", 3, None):
            with self.subTest(version=str(version)[:60]):
                self.manifest["build"]["package_version"] = version
                self.save_manifest()
                with self.assertRaises(ValueError):
                    packager.prepare(self.assets, self.root / "rejected", self.target)
                self.assertFalse((self.root / "rejected").exists())

    def test_malformed_manifest_objects_and_boolean_schema_are_rejected(self):
        original = json.dumps(self.manifest)
        invalid = [[], None, True]
        for section in ("build", "payload", "source"):
            for value in (None, [], "wrong"):
                candidate = json.loads(original)
                candidate[section] = value
                invalid.append(candidate)
        candidate = json.loads(original)
        candidate["schema_version"] = True
        invalid.append(candidate)
        for candidate in invalid:
            with self.subTest(manifest=candidate):
                (self.assets / (self.name + ".manifest.json")).write_text(json.dumps(candidate))
                with self.assertRaises(ValueError):
                    packager.prepare(self.assets, self.root / "rejected", self.target)
                self.assertFalse((self.root / "rejected").exists())

    def test_gnu_companion_uses_its_exact_target_without_windows_core(self):
        target = "x86_64-pc-windows-gnu"
        (self.assets / self.name).rename(self.assets / f"flere-connect-{target}")
        self.manifest["build"]["target"] = target
        self.manifest["payload"]["download_file"] = f"flere-connect-{target}"
        (self.assets / f"flere-connect-{target}.manifest.json").write_text(json.dumps(self.manifest))
        result = packager.prepare(self.assets, self.root / "gnu", target)
        self.assertIn(target, result["url"])
        self.assertEqual(result["target"], target)
        with self.assertRaisesRegex(ValueError, "only the Windows x86_64 companion"):
            packager.prepare(self.assets, self.root / "linux", "x86_64-unknown-linux-gnu")

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
