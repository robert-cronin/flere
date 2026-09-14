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

    @unittest.skipUnless(shutil.which("bash"), "requires Bash")
    def test_aur_package_function_preserves_license_metadata(self):
        recipe = self.root / "PKGBUILD"
        recipe.write_bytes(packages.aur_files(self.lock)["PKGBUILD"])
        # Run the real generated function without depending on GNU install or
        # executing a payload. Bash loop assignments can overwrite array index 0.
        result = subprocess.run(["bash", "-euc", '''source "$1"
install() { :; }
package
printf '%s\\n' "${license[@]}"
''', "aur-license-check", str(recipe)], check=True, capture_output=True, timeout=5,
                                env=dict(os.environ, srcdir=str(self.assets),
                                         pkgdir=str(self.root / "unused-package")))
        self.assertEqual(result.stdout.splitlines(), [b"MIT", b"OFL-1.1"])
        self.assertEqual(result.stderr, b"")

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

    def test_rpm_recipe_and_preparation_include_only_reviewed_metadata(self):
        for name, data in packages.rpm_files(packages.load_lock()).items():
            self.assertEqual((packages.ROOT / "packaging/linux/rpm" / name).read_bytes(), data)
        (self.assets / "private-token").write_bytes(b"DO-NOT-PACKAGE")
        output = self.root / "prepared"
        with mock.patch.object(packages, "load_lock", return_value=self.lock), contextlib.redirect_stdout(io.StringIO()):
            packages.main(["--assets", str(self.assets), "--output", str(output)])
        self.assertEqual({p.relative_to(output).as_posix() for p in output.rglob("*") if p.is_file()},
                         {"aur/flere-bin/PKGBUILD", "aur/flere-bin/.SRCINFO", "rpm/flere.spec",
                          "package-provenance.json", "SHA256SUMS"})
        for path in output.rglob("*"):
            if path.is_file():
                self.assertNotIn(b"DO-NOT-PACKAGE", path.read_bytes())
        for line in (output / "SHA256SUMS").read_text().splitlines():
            checksum, name = line.split("  ", 1)
            self.assertEqual(checksum, packages.digest((output / name).read_bytes()))

    def test_rpm_rejects_macro_shell_paths_before_invoking_rpmbuild(self):
        inputs, licenses = self.load()
        for name in ("percent%name", "quote'name", "dollar$name", "line\nbreak", "with space", "semi;colon"):
            output = self.root / name
            with self.subTest(name=name), mock.patch.object(packages.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "RPM output path"):
                    packages.build_rpm(output, inputs, licenses, self.lock)
                run.assert_not_called()
                self.assertFalse(output.exists())

    def require_rpm_coreutils(self):
        # Ubuntu can provide rpmbuild without an RPM database containing the
        # spec's BuildRequires. Probe that database, not just commands on PATH.
        home = self.root / "rpm-probe-home"
        home.mkdir(mode=0o700, exist_ok=True)
        options = dict(capture_output=True, text=True, timeout=10,
                       env=dict(os.environ, HOME=str(home), LC_ALL="C",
                                XDG_CONFIG_HOME=str(home / ".config"),
                                XDG_CACHE_HOME=str(home / ".cache"), TMPDIR=str(home)))
        database = subprocess.run(["rpm", "--eval", "%{_dbpath}"], check=True, **options)
        database_path = Path(database.stdout.strip())
        if (len(database.stdout.splitlines()) != 1 or not database_path.is_absolute()
                or database.stderr.strip()):
            raise ValueError("RPM database path query returned an unexpected result")
        try:
            database_path.stat()
        except FileNotFoundError:
            self.skipTest("requires an existing RPM dependency database")
        result = subprocess.run(["rpm", "-q", "--whatprovides", "coreutils"], **options)
        if (result.returncode == 1 and result.stdout.strip() == "no package provides coreutils"
                and not result.stderr.strip()):
            self.skipTest("requires an RPM database provider for BuildRequires: coreutils")
        result.check_returncode()

    def test_rpm_coreutils_probe_accepts_database_provider(self):
        result = subprocess.CompletedProcess([], 0, "coreutils-single-9.10-1.fc44.x86_64\n", "")
        database = subprocess.CompletedProcess([], 0, str(self.root), "")
        with mock.patch.object(subprocess, "run", side_effect=[database, result]) as run:
            self.require_rpm_coreutils()
        self.assertEqual([call.args for call in run.call_args_list],
                         [(["rpm", "--eval", "%{_dbpath}"],),
                          (["rpm", "-q", "--whatprovides", "coreutils"],)])
        options = run.call_args.kwargs
        self.assertEqual(options["timeout"], 10)
        self.assertTrue(options["capture_output"])
        self.assertEqual(options["env"]["LC_ALL"], "C")
        self.assertEqual(options["env"]["HOME"], str(self.root / "rpm-probe-home"))
        self.assertEqual(options["env"]["TMPDIR"], options["env"]["HOME"])

    def test_rpm_coreutils_probe_skips_absent_database_without_querying(self):
        database = subprocess.CompletedProcess([], 0, str(self.root / "absent-db"), "")
        with mock.patch.object(subprocess, "run", return_value=database) as run:
            with self.assertRaisesRegex(unittest.SkipTest, "existing RPM dependency database"):
                self.require_rpm_coreutils()
        run.assert_called_once()
        self.assertFalse((self.root / "absent-db").exists())

    def test_rpm_coreutils_probe_skips_missing_database_provider(self):
        result = subprocess.CompletedProcess([], 1, "no package provides coreutils\n", "")
        database = subprocess.CompletedProcess([], 0, str(self.root), "")
        with mock.patch.object(subprocess, "run", side_effect=[database, result]):
            with self.assertRaisesRegex(unittest.SkipTest, "RPM database provider"):
                self.require_rpm_coreutils()

    def test_rpm_coreutils_probe_preserves_unexpected_failures(self):
        database = subprocess.CompletedProcess([], 0, str(self.root), "")
        for code, output, error in ((1, "no package provides coreutils\n", "error: cannot open database\n"),
                                    (1, "unexpected query failure\n", ""),
                                    (2, "no package provides coreutils\n", "")):
            result = subprocess.CompletedProcess([], code, output, error)
            with self.subTest(code=code, output=output, error=error):
                with mock.patch.object(subprocess, "run", side_effect=[database, result]):
                    with self.assertRaises(subprocess.CalledProcessError):
                        self.require_rpm_coreutils()
        with mock.patch.object(subprocess, "run", side_effect=[database, subprocess.TimeoutExpired("rpm", 10)]):
            with self.assertRaises(subprocess.TimeoutExpired):
                self.require_rpm_coreutils()

    @unittest.skipUnless(sys.platform == "linux" and shutil.which("rpmbuild") and shutil.which("rpm"),
                         "requires Linux rpmbuild and rpm")
    def test_real_rpm_checks_hashes_modes_licenses_and_rejects_changed_input(self):
        self.require_rpm_coreutils()
        inputs, licenses = self.load()
        output = self.root / "rpm-output"
        output.mkdir()
        artifact = packages.build_rpm(output, inputs, licenses, self.lock)
        subprocess.run(["rpm", "--checksig", str(artifact)], check=True, stdout=subprocess.PIPE)
        metadata = subprocess.check_output(["rpm", "-qp", "--qf", "%{NAME} %{VERSION} %{RELEASE} %{ARCH}\n%{LICENSE}\n", str(artifact)]).decode()
        self.assertEqual(metadata, "flere 0.3.0 1 x86_64\nMIT AND OFL-1.1\n")
        for option in ("--scripts", "--triggers"):
            self.assertEqual(subprocess.check_output(["rpm", "-qp", option, str(artifact)]), b"")
        requires = subprocess.check_output(["rpm", "-qp", "--requires", str(artifact)]).decode()
        for dependency in ("glibc(x86-64) >= 2.39", "libgcc(x86-64)", "libz.so.1()(64bit)", "git-core"):
            self.assertIn(dependency, requires.splitlines())
        query = "[%{FILENAMES}\t%{FILEMODES}\t%{FILEDIGESTS}\t%{FILEUSERNAME}\t%{FILEGROUPNAME}\t%{FILEFLAGS:fflags}\n]"
        rows = subprocess.check_output(["rpm", "-qp", "--qf", query, str(artifact)]).decode().splitlines()
        expected = {}
        for component in packages.COMPONENTS:
            name = f"{component}-{packages.TARGET}"
            expected[f"/usr/bin/{component}"] = (inputs[name], 0o755)
            expected[f"/usr/share/flere/{component}.manifest.json"] = (inputs[name + ".manifest.json"], 0o644)
        expected.update({f"/usr/share/licenses/flere/{name}": (data, 0o644) for name, data in licenses.items()})
        files = set()
        for row in rows:
            name, mode, checksum, user, group, flags = row.split("\t")
            self.assertEqual((user, group), ("root", "root"))
            if checksum:
                data, wanted_mode = expected[name]
                self.assertEqual((checksum, int(mode) & 0o777), (packages.digest(data), wanted_mode))
                if name.startswith("/usr/share/licenses/"):
                    self.assertIn("l", flags)
                files.add(name)
            else:
                self.assertIn(name, ("/usr/share/flere", "/usr/share/licenses/flere"))
                self.assertEqual(int(mode) & 0o777, 0o755)
        self.assertEqual(files, set(expected))
        self.assertFalse((output / ".rpm-staging").exists())
        broken = dict(inputs)
        broken[f"flere-{packages.TARGET}"] += b"unexpected change"
        rejected = self.root / "rejected-rpm"
        rejected.mkdir()
        with self.assertRaises(subprocess.CalledProcessError):
            packages.build_rpm(rejected, broken, licenses, self.lock)
        self.assertEqual(list(rejected.iterdir()), [])

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
