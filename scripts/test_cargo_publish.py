#!/usr/bin/env python3
"""Offline registry reconciliation fixtures; never run Cargo or contact a registry."""
import copy
import gzip
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("cargo_publish", Path(__file__).with_name("cargo-publish.py"))
publish = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publish)
VERSION = "0.4.0"
API = f"https://crates.io/api/v1/crates/flere/{VERSION}"
INDEX = "https://index.crates.io/fl/er/flere"
DOWNLOAD = f"https://static.crates.io/crates/flere/flere-{VERSION}.crate"


class CargoPublication(unittest.TestCase):
    def setUp(self):
        cache = Path.home() / ".cache/flere/tmp"
        cache.mkdir(parents=True, exist_ok=True, mode=0o700)
        temporary = tempfile.TemporaryDirectory(prefix="cargo-publish-", dir=cache)
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.checkout = self.directory / "source"
        self.checkout.mkdir()
        self.output = self.directory / "proof"
        self.output.mkdir()
        self.original = ('[package]\nname = "flere"\nversion = "0.4.0"\n'
                         'edition = "2024"\nbuild = "build-support/build.rs"\n'
                         'include = ' + json.dumps(publish.INCLUDES) + '\n'
                         '[dependencies]\nserde_json = "1"\n')
        self.normalized = self.original.replace('[dependencies]',
            'autolib = false\nautobins = false\nautoexamples = false\n'
            'autotests = false\nautobenches = false\n[dependencies]')
        self.normalized = self.normalized.replace('serde_json = "1"',
                                                  'serde_json = { version = "1" }')
        self.normalized += ('[lib]\nname = "flere"\npath = "src/lib.rs"\n'
                            '[[bin]]\nname = "flere"\npath = "src/main.rs"\n')
        self.files = {
            "Cargo.toml.orig": self.original.encode(),
            "Cargo.toml": self.normalized.encode(),
            "Cargo.lock": b'version = 4\n[[package]]\nname = "flere"\nversion = "0.4.0"\n',
            "LICENSE": b"Fixture license\n",
            "src/main.rs": b"fn main() {}\n",
            "src/lib.rs": b"pub mod sixel;\n",
            "src/sixel.rs": b"// fixture shared source\n",
            "src/terminal/hyperlinks.rs": b"// fixture required inventory coverage\n",
            "src/assets/fonts/OFL.txt": b"Fixture font license\n",
            "src/assets/fonts/LICENSE-Nerd-Fonts": b"Fixture font license\n",
            "build-support/build.rs": b"fn main() {}\n",
            "packaging/cargo/README.md": b"Fixture readme\n",
            "tests/fixtures/local-image.png": b"fixture png\n",
            "tests/fixtures/local-image.jpg": b"fixture jpg\n",
        }
        for name, data in self.files.items():
            if name == "Cargo.toml":
                continue
            path = self.checkout / ("Cargo.toml" if name == "Cargo.toml.orig" else name)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        (self.checkout / "companion").mkdir()
        (self.checkout / "companion/Cargo.toml").write_text(
            '[package]\nname = "flere-companion"\nversion = "0.4.0"\n')
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        def git(*args):
            return subprocess.check_output(["git", "-C", str(self.checkout), *args], env=env,
                                           stderr=subprocess.PIPE, timeout=10)
        git("init", "--quiet", "--initial-branch=main")
        git("add", ".")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
            "-c", "commit.gpgsign=false", "commit", "--quiet", "-m", "fixture")
        self.commit = git("rev-parse", "HEAD").decode().strip()
        self.files[".cargo_vcs_info.json"] = json.dumps(
            {"git": {"sha1": self.commit}, "path_in_vcs": ""}).encode()
        self.calls = []
        self.sleeps = []
        self.set_registry(self.archive())

    def archive(self, files=None):
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w") as archive:
            for name, data in (self.files if files is None else files).items():
                entry = tarfile.TarInfo(f"flere-{VERSION}/{name}")
                entry.size, entry.mode = len(data), 0o644
                archive.addfile(entry, io.BytesIO(data))
        return gzip.compress(stream.getvalue(), mtime=0)

    def set_registry(self, data):
        self.data = data
        self.api = {"version": {"crate": "flere", "num": VERSION, "yanked": False,
                                "crate_size": len(data), "checksum": publish.sha(data),
                                "dl_path": "https://untrusted.invalid/ignored"}}
        self.index = {"name": "flere", "vers": VERSION, "yanked": False,
                      "cksum": publish.sha(data)}

    def read(self, url, maximum, missing=False):
        self.calls.append(url)
        if url == API:
            return json.dumps(self.api).encode() if self.api else None
        if url == INDEX:
            return json.dumps(self.index).encode() if self.index else None
        self.assertEqual(url, DOWNLOAD)
        return self.data

    def reconcile(self, **kwargs):
        return publish.reconcile(self.checkout, VERSION, self.commit, self.output,
                                 read=self.read, sleep=self.sleeps.append, **kwargs)

    def test_matching_existing_crate_proves_checksum_complete_git_inventory_and_vcs(self):
        proof = self.reconcile()
        self.assertEqual(proof["status"], "published_verified")
        self.assertEqual(proof["crate_sha256"], publish.sha(self.data))
        self.assertEqual(proof["source_commit"], self.commit)
        self.assertEqual(proof["source_files_verified"], len(self.files))
        self.assertEqual(self.calls, [API, INDEX, DOWNLOAD])
        self.assertEqual(self.sleeps, [])

    def test_absent_version_requires_both_public_sources_and_never_downloads(self):
        self.api = self.index = None
        self.assertEqual(self.reconcile()["status"], "not_published")
        self.assertEqual(self.calls, [API, INDEX])

    def test_partial_visibility_waits_bounded_and_cannot_authorize_publish(self):
        self.index = None
        with self.assertRaisesRegex(ValueError, "visibility did not settle"):
            self.reconcile()
        self.assertEqual(self.calls, [API, INDEX] * 6)
        self.assertEqual(self.sleeps, [3] * 5)
        self.assertFalse((self.output / "extracted").exists())

    def test_required_verification_retries_absence_then_verifies_normal_cargo_archive(self):
        ready_api, ready_index = self.api, self.index
        self.api = self.index = None
        def settle(delay):
            self.sleeps.append(delay)
            self.api, self.index = ready_api, ready_index
        local = self.directory / "cargo-output.crate"
        local.write_bytes(self.data)
        proof = publish.reconcile(self.checkout, VERSION, self.commit, self.output,
            required=True, local_crate=local, read=self.read, sleep=settle)
        self.assertEqual(proof["status"], "published_verified")
        self.assertEqual(self.sleeps, [3])

    def test_registry_conflicts_or_yanks_and_corrupt_downloads_fail_closed(self):
        for field, value in (("cksum", "0" * 64), ("yanked", True), ("name", "another")):
            with self.subTest(field=field):
                self.set_registry(self.archive())
                self.index[field] = value
                with self.assertRaisesRegex(ValueError, "registry version"):
                    self.reconcile()
        self.set_registry(self.archive())
        self.data += b"tampered"
        with self.assertRaisesRegex(ValueError, "checksum/size"):
            self.reconcile()

    def test_source_vcs_manifest_lock_and_inventory_corruption_are_rejected(self):
        cases = {
            "source bytes": ("src/lib.rs", b"changed source"),
            "missing new module": ("src/terminal/hyperlinks.rs", None),
            "wrong commit": (".cargo_vcs_info.json", json.dumps(
                {"git": {"sha1": "a" * 40}, "path_in_vcs": ""}).encode()),
            "dirty archive": (".cargo_vcs_info.json", json.dumps(
                {"git": {"sha1": self.commit, "dirty": True}, "path_in_vcs": ""}).encode()),
            "wrong source root": (".cargo_vcs_info.json", json.dumps(
                {"git": {"sha1": self.commit}, "path_in_vcs": "companion"}).encode()),
            "injected build script": ("Cargo.toml", self.normalized.replace(
                'build-support/build.rs', 'src/main.rs').encode()),
            "changed lock": ("Cargo.lock", self.files["Cargo.lock"] + b'checksum = "changed"\n'),
        }
        for label, (name, data) in cases.items():
            with self.subTest(label=label):
                files = self.files.copy()
                if data is None:
                    del files[name]
                else:
                    files[name] = data
                self.set_registry(self.archive(files))
                with self.assertRaises((ValueError, RuntimeError)):
                    self.reconcile()

    def test_changed_local_cargo_archive_stops_before_registry_download(self):
        local = self.directory / "cargo-output.crate"
        local.write_bytes(self.data + b"changed")
        with self.assertRaisesRegex(ValueError, "archive produced by Cargo publish"):
            self.reconcile(required=True, local_crate=local)
        self.assertEqual(self.calls, [API, INDEX])

    def test_wrong_selected_commit_and_dirty_source_stop_before_network(self):
        with self.assertRaisesRegex(ValueError, "clean selected source"):
            publish.reconcile(self.checkout, VERSION, "a" * 40, self.output, read=self.read)
        (self.checkout / "src/lib.rs").write_text("changed")
        with self.assertRaisesRegex(ValueError, "clean selected source"):
            self.reconcile()
        self.assertEqual(self.calls, [])

    def test_invalid_identity_and_unsupported_manifest_stop_before_publish(self):
        for version in ("v0.4.0", "0.04.0", "0.4.0-rc1", "0.4.0\nkey=bad", "../0.4.0"):
            with self.subTest(version=version), self.assertRaises(ValueError):
                publish.identity(version, self.commit)
        original = tomllib.loads(self.original)
        for mutation in ({"workspace": {}}, {"bin": [{"name": "other"}]},
                         {"dependencies": {"other": {"path": "../other"}}}):
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                publish.normalized_manifest(dict(copy.deepcopy(original), **mutation))

    def test_http_errors_are_not_treated_as_missing_or_retried(self):
        with mock.patch.object(publish.urllib.request, "build_opener") as opener:
            opener.return_value.open.side_effect = publish.urllib.error.HTTPError(
                API, 403, "forbidden", {}, None)
            with self.assertRaisesRegex(ValueError, "HTTP 403"):
                publish.visible_version(VERSION, read=publish.public_read, sleep=self.sleeps.append)
            self.assertEqual(opener.return_value.open.call_count, 1)
        self.assertEqual(self.sleeps, [])
        self.assertIsNone(publish.NoRedirect().redirect_request(None, None, 302, "", {}, DOWNLOAD))

    def test_workflow_publishes_core_only_after_verification_with_job_local_oidc(self):
        workflow = (publish.ROOT / ".github/workflows/release.yml").read_text()
        prefix, job = workflow.split("\n  publish-core:\n")
        self.assertNotIn("id-token: write", prefix)
        self.assertIn("needs: [select, verify-public]", job)
        self.assertIn("github.event_name == 'workflow_dispatch'", job)
        self.assertIn("github.ref == 'refs/heads/main'", job)
        self.assertIn("environment: release", job)
        self.assertIn("permissions:\n      contents: read\n      id-token: write", job)
        self.assertIn("ref: ${{ needs.select.outputs.commit }}", job)
        self.assertIn("rust-lang/crates-io-auth-action@c6f97d42243bad5fab37ca0427f495c86d5b1a18", job)
        self.assertIn("CARGO_REGISTRY_TOKEN: ${{ steps.crates-auth.outputs.token }}", job)
        self.assertEqual(job.count("if: steps.registry.outputs.needs_publish == 'true'"), 4)
        self.assertLess(job.index("cargo-publish.py check"), job.index("rustup toolchain install"))
        self.assertIn("cargo publish --locked --registry crates-io -p flere\n", job)
        for forbidden in ("--no-verify", "--allow-dirty", "flere-companion", "contents: write"):
            self.assertNotIn(forbidden, job)


if __name__ == "__main__":
    unittest.main()
