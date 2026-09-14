#!/usr/bin/env python3
"""Offline source-archive boundary tests, using disposable Git repositories."""
import gzip
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_source", Path(__file__).with_name("release-source.py"))
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)
VERSION = "0.4.0"


def sample_files(version=VERSION):
    files = {name: (0o644, b"fixture\n") for name in source.REQUIRED}
    for prefix, component, build in (("", "flere", "build-support/build.rs"),
                                      ("companion/", "flere-connect", "../build-support/build.rs")):
        files[prefix + "Cargo.toml"] = (0o644, (
            f'[package]\nname = "{component}"\nversion = "{version}"\n'
            f'rust-version = "1.98"\nbuild = "{build}"\n').encode())
        files[prefix + "Cargo.lock"] = (0o644, (
            f'version = 4\n[[package]]\nname = "{component}"\nversion = "{version}"\n').encode())
    files["scripts/fixture"] = (0o755, b"#!/bin/sh\nexit 0\n")
    files[".gitignore"] = (0o644, b"ignored-private\n")
    return files


def source_digest(files):
    return source.fingerprint({name: {"mode": mode, "sha256": hashlib.sha256(data).hexdigest()}
                               for name, (mode, data) in files.items()})


def write_archive(path, files, *, members=None, transform=None):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name in sorted(files) if members is None else members:
            mode, data = files[name]
            member = tarfile.TarInfo(f"flere-{VERSION}/{name}")
            member.mode, member.size, member.mtime = mode, len(data), 1700000000
            if transform:
                transform(member)
            archive.addfile(member, io.BytesIO(data))
    with path.open("wb") as stream:
        with gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0, compresslevel=9) as compressed:
            compressed.write(raw.getvalue())


class ReleaseSource(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / ".cache/flere/tmp"
        parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(prefix="release-source-", dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.files = sample_files()
        self.digest = source_digest(self.files)
        self.archive = self.root / source.name(VERSION)

    def repo(self, name="checkout"):
        repo = self.root / name
        repo.mkdir()
        for name, (mode, data) in self.files.items():
            path = repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
            path.chmod(mode)
        source.git(repo, "init", "-q", "-b", "main")
        source.git(repo, "add", ".")
        source.git(repo, "-c", "user.name=Release Fixture", "-c", "user.email=release@example.invalid",
                   "-c", "commit.gpgsign=false", "commit", "-qm", "fixture")
        return repo, source.git(repo, "rev-parse", "HEAD").decode().strip()

    def inspect(self):
        return source.inspect(self.archive, VERSION, self.digest)

    def test_fingerprint_matches_fixed_unix_record_vector(self):
        # Independently fixed encoding: u64 path length 1, 'a', st_mode 0x81a4,
        # the ASCII SHA-256 of 'abc', NUL. File-type bits must not be discarded.
        entries = {"a": {"mode": 0o644, "sha256": "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"}}
        self.assertEqual(source.fingerprint(entries), "07ea4819688e6328ed87d60eb24231d436a19121bd7666c03a827df702ac5b4b")

    def test_selected_git_bytes_modes_and_ignored_private_files_across_umasks(self):
        repo, commit = self.repo()
        other, _ = self.repo("misleading-selector")
        (repo / "ignored-private").write_bytes(b"PRIVATE-FIXTURE-CONTENT")
        outputs = []
        for mask in (0o002, 0o077):
            directory = self.root / str(mask)
            directory.mkdir()
            archive = directory / source.name(VERSION)
            command = [sys.executable, "-c", "import os,runpy,sys; os.umask(int(sys.argv.pop(1))); path=sys.argv.pop(1); runpy.run_path(path,run_name='__main__')",
                       str(mask), str(Path(source.__file__)), "create", "--archive", str(archive),
                       "--checkout", str(repo), "--commit", commit, "--version", VERSION,
                       "--source-sha256", self.digest]
            env = dict(os.environ, GIT_DIR=str(other / ".git"), GIT_WORK_TREE=str(other),
                       GIT_INDEX_FILE=str(other / ".git/index"), PYTHONDONTWRITEBYTECODE="1")
            result = subprocess.run(command, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            outputs.append(archive.read_bytes())
        self.assertEqual(outputs[0], outputs[1])
        with tarfile.open(fileobj=io.BytesIO(outputs[0]), mode="r:gz") as archive:
            actual = {m.name.removeprefix(f"flere-{VERSION}/"): (m.mode, archive.extractfile(m).read())
                      for m in archive}
        self.assertEqual(actual, self.files)
        self.assertNotIn(b"PRIVATE-FIXTURE-CONTENT", gzip.decompress(outputs[0]))

    def test_dirty_untracked_or_wrong_commit_fail_before_archive_creation(self):
        repo, commit = self.repo()
        for change in ("wrong-commit", "untracked", "tracked"):
            with self.subTest(change=change):
                selected = "a" * 40 if change == "wrong-commit" else commit
                if change == "untracked":
                    (repo / "private-untracked").write_text("fixture")
                if change == "tracked":
                    (repo / "LICENSE").write_text("modified")
                with self.assertRaisesRegex(ValueError, "clean selected"):
                    source.create(repo, self.archive, VERSION, selected, self.digest)
                self.assertFalse(self.archive.exists())
                (repo / "private-untracked").unlink(missing_ok=True)

    def test_git_links_are_rejected_without_following_them(self):
        repo, _ = self.repo()
        (repo / "link").symlink_to("LICENSE")
        source.git(repo, "add", "link")
        source.git(repo, "-c", "user.name=Release Fixture", "-c", "user.email=release@example.invalid",
                   "-c", "commit.gpgsign=false", "commit", "-qm", "link fixture")
        commit = source.git(repo, "rev-parse", "HEAD").decode().strip()
        with self.assertRaisesRegex(ValueError, "only regular"):
            source.create(repo, self.archive, VERSION, commit, self.digest)
        self.assertFalse(self.archive.exists())

    def test_replacement_refs_cannot_change_the_selected_commit_archive(self):
        repo, original = self.repo()
        replacement_files = dict(self.files)
        replacement_files["LICENSE"] = (0o644, b"different replacement tree\n")
        (repo / "LICENSE").write_bytes(replacement_files["LICENSE"][1])
        source.git(repo, "add", "LICENSE")
        source.git(repo, "-c", "user.name=Release Fixture", "-c", "user.email=release@example.invalid",
                   "-c", "commit.gpgsign=false", "commit", "-qm", "replacement fixture")
        replacement = source.git(repo, "rev-parse", "HEAD").decode().strip()
        source.git(repo, "replace", original, replacement)
        # Keep replacement files/index while presenting the original commit name.
        # Replacement-aware Git would report this misleading checkout as clean.
        source.git(repo, "update-ref", "HEAD", original)
        with self.assertRaisesRegex(ValueError, "clean selected"):
            source.create(repo, self.archive, VERSION, original, source_digest(replacement_files))
        self.assertFalse(self.archive.exists())
        source.git(repo, "reset", "--hard", original)
        source.create(repo, self.archive, VERSION, original, self.digest)
        with tarfile.open(self.archive, "r:gz") as archive:
            self.assertEqual(archive.extractfile(f"flere-{VERSION}/LICENSE").read(), self.files["LICENSE"][1])

    def test_missing_shared_inputs_licenses_and_wrong_package_identity(self):
        for name in source.REQUIRED:
            with self.subTest(missing=name):
                files = dict(self.files)
                del files[name]
                write_archive(self.archive, files)
                with self.assertRaisesRegex(ValueError, "missing shared inputs or licenses"):
                    source.inspect(self.archive, VERSION, source_digest(files))
        for name in ("Cargo.toml", "Cargo.lock", "companion/Cargo.toml", "companion/Cargo.lock"):
            with self.subTest(wrong_version=name):
                files = dict(self.files)
                files[name] = (0o644, files[name][1].replace(b"0.4.0", b"0.4.1"))
                write_archive(self.archive, files)
                with self.assertRaisesRegex(ValueError, "version, lock or shared build"):
                    source.inspect(self.archive, VERSION, source_digest(files))
        files = dict(self.files)
        files["companion/Cargo.toml"] = (0o644, files["companion/Cargo.toml"][1].replace(b"../build-support", b"../../outside"))
        write_archive(self.archive, files)
        with self.assertRaisesRegex(ValueError, "shared build"):
            source.inspect(self.archive, VERSION, source_digest(files))

    def test_content_and_executable_mode_changes_break_source_provenance(self):
        for mode, data in ((0o644, b"different"), (0o755, self.files["LICENSE"][1])):
            files = dict(self.files)
            files["LICENSE"] = (mode, data)
            write_archive(self.archive, files)
            with self.assertRaisesRegex(ValueError, "fingerprint differs"):
                self.inspect()

    def test_unsafe_members_order_duplicates_and_metadata_are_rejected(self):
        def edit(field, value):
            return lambda member: setattr(member, field, value)
        mutations = [edit("name", "../outside"), edit("name", f"flere-{VERSION}/../outside"),
                     edit("name", f"flere-{VERSION}/a\\b"), edit("name", f"flere-{VERSION}/.git/config"),
                     edit("type", tarfile.SYMTYPE), edit("type", tarfile.LNKTYPE),
                     edit("type", tarfile.DIRTYPE), edit("type", tarfile.CHRTYPE),
                     edit("uid", 501), edit("uname", "private-owner"), edit("mode", 0o666)]
        for index, transform in enumerate(mutations):
            with self.subTest(mutation=index):
                write_archive(self.archive, self.files, transform=transform)
                with self.assertRaises(ValueError):
                    self.inspect()
        for members in (list(reversed(sorted(self.files))), sorted(self.files) + [sorted(self.files)[-1]]):
            write_archive(self.archive, self.files, members=members)
            with self.assertRaisesRegex(ValueError, "duplicate, unsorted"):
                self.inspect()

    def test_gzip_corruption_truncation_and_concatenation_are_rejected(self):
        write_archive(self.archive, self.files)
        original = self.archive.read_bytes()
        for data in (original[:-1], original + b"trailing", original + original,
                     original[:20] + b"corrupt" + original[27:], b"not gzip"):
            with self.subTest(size=len(data)):
                self.archive.write_bytes(data)
                with self.assertRaises(ValueError):
                    self.inspect()

    def test_hidden_tar_extensions_and_trailing_material_are_rejected(self):
        write_archive(self.archive, self.files)
        original = gzip.decompress(self.archive.read_bytes())
        extension = tarfile.TarInfo("pax")
        extension.type, extension.size = tarfile.XHDTYPE, 0
        for raw in (extension.tobuf(format=tarfile.USTAR_FORMAT) + original,
                    original[:-512] + b"private-tail" + bytes(500)):
            with self.archive.open("wb") as stream:
                with gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0) as compressed:
                    compressed.write(raw)
            with self.assertRaises(ValueError):
                self.inspect()

    def test_compressed_expanded_member_count_and_content_bounds(self):
        write_archive(self.archive, self.files)
        for constant in ("MAX_ARCHIVE", "MAX_TAR", "MAX_FILE", "MAX_FILES", "MAX_CONTENT"):
            with self.subTest(bound=constant), mock.patch.object(source, constant, 1):
                with self.assertRaisesRegex(ValueError, "limit|excessive"):
                    self.inspect()

    def test_create_refuses_wrong_fingerprint_and_never_overwrites_candidate(self):
        repo, commit = self.repo()
        with self.assertRaisesRegex(ValueError, "Git tree fingerprint"):
            source.create(repo, self.archive, VERSION, commit, "0" * 64)
        self.assertFalse(self.archive.exists())
        self.archive.write_bytes(b"retain exact earlier candidate")
        with self.assertRaises(FileExistsError):
            source.create(repo, self.archive, VERSION, commit, self.digest)
        self.assertEqual(self.archive.read_bytes(), b"retain exact earlier candidate")


if __name__ == "__main__":
    unittest.main()
