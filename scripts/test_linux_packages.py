#!/usr/bin/env python3
"""Offline package integrity checks with inert ELF fixtures and real package tools."""
import copy
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("linux_packages", Path(__file__).with_name("linux-packages.py"))
packages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packages)


class LinuxPackages(unittest.TestCase):
    def setUp(self):
        parent = Path(os.environ.get("FLERE_TEST_CACHE", Path.home() / ".cache/flere/tmp"))
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.temporary = tempfile.TemporaryDirectory(prefix="linux-packages-", dir=parent)
        self.root = Path(self.temporary.name)
        self.addCleanup(self.temporary.cleanup)
        self.assets = self.root / "assets"
        self.assets.mkdir(mode=0o700)
        self.lock = copy.deepcopy(packages.load_lock())
        self.licenses = {Path(name).name: f"Fixture license {name}\n".encode() for name in packages.LICENSES}
        self.manifests = {}
        for component in packages.COMPONENTS:
            name = f"{component}-{packages.TARGET}"
            binary = b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x3e\x00" + f"inert {component}".encode()
            self.pin(name, binary)
            manifest = {"schema_version": 1, "build": {
                "component": component, "target": packages.TARGET,
                "package_version": self.lock["version"], "profile": "release",
                "compatibility": {"remote_protocol": {"current": "v6", "accepts": ["v6"]}}},
                "source": {"dirty": False, "git_commit": self.lock["source_commit"],
                           "source_sha256": self.lock["source_sha256"]},
                "payload": dict(self.lock["assets"][name], file_name=component, download_file=name)}
            self.manifests[component] = manifest
            self.pin(name + ".manifest.json", json.dumps(manifest).encode())
        self.source_archive()
        self.sums()

    def pin(self, name, data):
        (self.assets / name).write_bytes(data)
        self.lock["assets"][name] = {"bytes": len(data), "sha256": packages.digest(data)}

    def source_archive(self, bad_member=None):
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            for name in packages.LICENSES:
                data = self.licenses[Path(name).name]
                member = tarfile.TarInfo(f"flere-{self.lock['version']}/{name}")
                member.size = len(data)
                if name == bad_member:
                    member.type = tarfile.SYMTYPE
                    member.linkname = "../../private-token"
                    member.size = 0
                archive.addfile(member, io.BytesIO(data))
                self.lock["licenses"][name] = {"bytes": len(data), "sha256": packages.digest(data)}
            # The real source archive contains a whole checkout. Nothing except
            # the three fixed license members may enter a binary package.
            private = tarfile.TarInfo(f"flere-{self.lock['version']}/unrelated-file")
            data = b"DO-NOT-PACKAGE"
            private.size = len(data)
            archive.addfile(private, io.BytesIO(data))
        self.pin(f"flere-{self.lock['version']}-source.tar.gz", buffer.getvalue())

    def sums(self):
        (self.assets / "SHA256SUMS").write_text("".join(
            f"{entry['sha256']}  {name}\n" for name, entry in self.lock["assets"].items()))

    def load(self):
        return packages.load_inputs(self.assets, self.lock)

    def repin_manifest(self, component):
        self.pin(f"{component}-{packages.TARGET}.manifest.json", json.dumps(self.manifests[component]).encode())
        self.sums()

    def test_corruption_and_checksum_substitution_are_rejected(self):
        name = f"flere-{packages.TARGET}"
        original = (self.assets / name).read_bytes()
        (self.assets / name).write_bytes(original + b"modified")
        with self.assertRaisesRegex(ValueError, "pinned release"):
            self.load()
        (self.assets / name).write_bytes(original)
        sums = self.assets / "SHA256SUMS"
        sums.write_text(sums.read_text().replace(self.lock["assets"][name]["sha256"], "0" * 64))
        with self.assertRaisesRegex(ValueError, "SHA256SUMS does not match"):
            self.load()

    def test_cli_rejects_corruption_before_creating_output(self):
        binary = self.assets / f"flere-{packages.TARGET}"
        binary.write_bytes(binary.read_bytes() + b"modified")
        output = self.root / "must-not-exist"
        with mock.patch.object(packages, "load_lock", return_value=self.lock), contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(ValueError, "pinned release"):
                packages.main(["--assets", str(self.assets), "--output", str(output)])
        self.assertFalse(output.exists())

    def test_mixed_identity_and_protocol_are_rejected(self):
        original = copy.deepcopy(self.manifests["flere-connect"])
        changes = [("source", "source_sha256", "0" * 64),
                   ("source", "git_commit", "0" * 40), ("source", "dirty", True),
                   ("build", "target", "aarch64-unknown-linux-gnu"),
                   ("build", "package_version", "0.4.0"),
                   ("payload", "file_name", "../private")]
        for section, key, value in changes:
            with self.subTest(section=section, key=key):
                self.manifests["flere-connect"] = copy.deepcopy(original)
                self.manifests["flere-connect"][section][key] = value
                self.repin_manifest("flere-connect")
                with self.assertRaisesRegex(ValueError, "manifest identity"):
                    self.load()
        self.manifests["flere-connect"] = copy.deepcopy(original)
        self.manifests["flere-connect"]["build"]["compatibility"]["remote_protocol"]["accepts"] = ["v5"]
        self.repin_manifest("flere-connect")
        with self.assertRaisesRegex(ValueError, "core protocol"):
            self.load()

    def test_symlink_inputs_and_license_members_are_rejected(self):
        name = f"flere-{packages.TARGET}"
        (self.assets / name).rename(self.assets / "elsewhere")
        (self.assets / name).symlink_to("elsewhere")
        with self.assertRaisesRegex(ValueError, "invalid input file"):
            self.load()
        (self.assets / name).unlink()
        (self.assets / "elsewhere").rename(self.assets / name)
        self.source_archive(bad_member="LICENSE")
        self.sums()
        with self.assertRaisesRegex(ValueError, "source license member"):
            self.load()

    def test_allowlist_and_checked_in_aur_recipe(self):
        (self.assets / "private-token").write_bytes(b"DO-NOT-PACKAGE")
        inputs, licenses = self.load()
        self.assertEqual(set(inputs), {f"{c}-{packages.TARGET}{suffix}" for c in packages.COMPONENTS
                                      for suffix in ("", ".manifest.json")})
        self.assertEqual(licenses, self.licenses)
        for data, _ in packages.deb_files(inputs, licenses).values():
            self.assertNotIn(b"DO-NOT-PACKAGE", data)
        for name, data in packages.aur_files(packages.load_lock()).items():
            self.assertEqual((packages.ROOT / "packaging/linux/aur/flere-bin" / name).read_bytes(), data)

    @unittest.skipUnless(sys.platform == "linux" and shutil.which("bash") and shutil.which("sha256sum"),
                         "requires Linux bash and coreutils")
    def test_aur_integrity_arrays_and_package_function_preserve_payloads(self):
        inputs, licenses = self.load()
        sources = self.root / "aur-source"
        sources.mkdir()
        for name, data in dict(inputs, **licenses).items():
            (sources / name).write_bytes(data)
        recipe = self.root / "PKGBUILD"
        recipe.write_bytes(packages.aur_files(self.lock)["PKGBUILD"])
        destination = self.root / "aur-package"
        subprocess.run(["bash", "-euc", '''source "$1"
cd "$srcdir"
for i in "${!source[@]}"; do
  printf '%s  %s\n' "${sha256sums[$i]}" "${source[$i]%%::*}"
done | sha256sum --check
package
''', "aur-check", str(recipe)], check=True, stdout=subprocess.PIPE,
                       env=dict(os.environ, srcdir=str(sources), pkgdir=str(destination)))
        expected = {f"usr/bin/{c}" for c in packages.COMPONENTS}
        expected |= {f"usr/share/flere/{c}.manifest.json" for c in packages.COMPONENTS}
        expected |= {f"usr/share/licenses/flere-bin/{name}" for name in licenses}
        actual = {p.relative_to(destination).as_posix() for p in destination.rglob("*") if p.is_file()}
        self.assertEqual(actual, expected)
        for component in packages.COMPONENTS:
            binary = destination / "usr/bin" / component
            self.assertEqual(binary.read_bytes(), inputs[f"{component}-{packages.TARGET}"])
            self.assertEqual(binary.stat().st_mode & 0o777, 0o755)

    @unittest.skipUnless(shutil.which("dpkg-deb"), "requires dpkg-deb")
    def test_native_deb_directory_modes_do_not_depend_on_umask(self):
        inputs, licenses = self.load()
        artifacts = []
        for mask in (0o002, 0o077):
            with self.subTest(umask=oct(mask)):
                output = self.root / f"umask-{mask:o}"
                output.mkdir(mode=0o700)
                previous = os.umask(mask)
                try:
                    artifact = packages.build_deb(output, inputs, licenses, self.lock)
                finally:
                    os.umask(previous)
                artifacts.append(artifact)
                for section in ("--fsys-tarfile", "--ctrl-tarfile"):
                    data = subprocess.check_output(["dpkg-deb", section, str(artifact)])
                    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
                        directories = [member for member in archive if member.isdir()]
                        self.assertTrue(directories)
                        for member in directories:
                            self.assertEqual((member.uid, member.gid, member.mode),
                                             (0, 0, 0o755), f"{section}: {member.name}")
        if len(artifacts) == 2:  # Earlier subtest failures already fail this test.
            self.assertEqual(artifacts[0].read_bytes(), artifacts[1].read_bytes())

    @unittest.skipUnless(shutil.which("dpkg-deb"), "requires dpkg-deb")
    def test_native_deb_metadata_members_hashes_and_reproducibility(self):
        inputs, licenses = self.load()
        artifacts = []
        for suffix in ("first", "second"):
            output = self.root / suffix
            output.mkdir(mode=0o700)
            artifacts.append(packages.build_deb(output, inputs, licenses, self.lock))
        self.assertEqual(artifacts[0].read_bytes(), artifacts[1].read_bytes())
        control = subprocess.check_output(["dpkg-deb", "--field", str(artifacts[0])]).decode()
        self.assertIn("Architecture: amd64\n", control)
        self.assertIn("Depends: libc6 (>= 2.39), libgcc-s1, zlib1g, git\n", control)
        self.assertIn("Provides: flere-connect (= 0.3.0-1)\n", control)
        archive_bytes = subprocess.check_output(["dpkg-deb", "--fsys-tarfile", str(artifacts[0])])
        with tarfile.open(fileobj=io.BytesIO(archive_bytes)) as archive:
            files = {member.name.removeprefix("./"): member for member in archive if member.isfile()}
            expected = packages.deb_files(inputs, licenses)
            self.assertEqual(set(files), set(expected))
            for name, member in files.items():
                data, mode = expected[name]
                self.assertEqual(archive.extractfile(member).read(), data)
                self.assertEqual((member.uid, member.gid, member.mode), (0, 0, mode))
            for member in archive.getmembers():
                self.assertFalse(member.issym() or member.islnk())
        control_bytes = subprocess.check_output(["dpkg-deb", "--ctrl-tarfile", str(artifacts[0])])
        with tarfile.open(fileobj=io.BytesIO(control_bytes)) as archive:
            self.assertEqual({m.name.removeprefix("./") for m in archive if m.isfile()}, {"control", "md5sums"})
            for line in archive.extractfile("./md5sums").read().decode().splitlines():
                checksum, name = line.split("  ", 1)
                self.assertEqual(checksum, hashlib.md5(expected[name][0], usedforsecurity=False).hexdigest())


if __name__ == "__main__":
    unittest.main()
