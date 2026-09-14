#!/usr/bin/env python3
"""Offline adversarial release-boundary tests; no real GitHub writes or payload execution."""
import copy
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
import urllib.error
from unittest import mock

from test_release_source import sample_files, source_digest, write_archive

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_automation", Path(__file__).with_name("release-automation.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
ci_spec = importlib.util.spec_from_file_location("release_linux_ci", Path(__file__).with_name("release-linux-ci.py"))
ci = importlib.util.module_from_spec(ci_spec)
ci_spec.loader.exec_module(ci)
packages_spec = importlib.util.spec_from_file_location("linux_packages", Path(__file__).with_name("linux-packages.py"))
packages = importlib.util.module_from_spec(packages_spec)
packages_spec.loader.exec_module(packages)

VERSION = "0.4.0"
COMMIT = "a" * 40
WORKFLOW = "b" * 40
RUN = "1234"


class FakeGitHub:
    def __init__(self, schema=1):
        self.schema = schema
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
            if set(self.files) != release.allowlist(VERSION, self.schema):
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
        files = sample_files(VERSION)
        self.source_digest = source_digest(files)
        self.source_archive = self.directory / release.source_archive.name(VERSION)
        write_archive(self.source_archive, files)
        for component in release.COMPONENTS:
            name = f"{component}-{release.TARGET}"
            payload = b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x3e\x00" + component.encode()
            item = {"schema_version": 1, "build": {"component": component,
                    "target": release.TARGET, "package_version": VERSION, "profile": "release",
                    "compatibility": {"remote_protocol": {"current": "v6", "accepts": ["v6"]}}},
                    "source": {"git_commit": COMMIT, "dirty": False, "source_sha256": self.source_digest},
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

    def linux_output(self):
        output = self.directory.with_name(self.directory.name + "-linux-packages")
        self.addCleanup(shutil.rmtree, output, True)
        return output

    def rewrite_channels(self, channels):
        path = self.directory / "release.json"
        descriptor = json.loads(path.read_bytes())
        descriptor["channels"] = channels
        path.write_bytes(release.json_bytes(descriptor))
        names = sorted(release.candidate_names(VERSION) | {"release.json"})
        (self.directory / "SHA256SUMS").write_text("".join(
            f"{release.sha((self.directory / name).read_bytes())}  {name}\n" for name in names))
        return release.sha(path.read_bytes())

    def test_new_and_pinned_historical_channel_snapshots_support_linux_derivation(self):
        digest = self.seal()
        descriptor = json.loads((self.directory / "release.json").read_bytes())
        self.assertEqual(descriptor["channels"], release.CHANNELS)
        self.assertEqual(descriptor["channels"]["crates_io"],
                         "pending: separate core Cargo job follows verified GitHub publication")
        for channels in (release.CHANNELS, release.LEGACY_CHANNELS):
            with self.subTest(channels=channels):
                digest = self.rewrite_channels(channels)
                proof = release.validate(self.directory, VERSION, COMMIT, RUN, WORKFLOW, digest)
                self.assertEqual(proof["descriptor_sha256"], digest)
                lock = packages.lock_from_release(self.directory, digest, 1)
                self.assertEqual(lock["version"], VERSION)
                self.assertEqual(lock["assets"], descriptor["assets"])
        with self.assertRaisesRegex(ValueError, "unexpected channel readiness"):
            release.validate(self.directory, VERSION, COMMIT, RUN, WORKFLOW)

    def test_pinned_unknown_channel_readiness_is_rejected(self):
        self.seal()
        for channels in (dict(release.CHANNELS, crates_io="published"),
                         dict(release.LEGACY_CHANNELS, windows="accepted")):
            with self.subTest(channels=channels):
                digest = self.rewrite_channels(channels)
                with self.assertRaisesRegex(ValueError, "unexpected channel readiness"):
                    release.validate(self.directory, VERSION, COMMIT, RUN, WORKFLOW, digest)
                with self.assertRaisesRegex(ValueError, "unexpected channel readiness"):
                    packages.lock_from_release(self.directory, digest, 1)

    def test_verified_release_drives_linux_recipes_lock_and_checksums(self):
        descriptor_digest = self.seal()
        checked_in_lock = (packages.ROOT / "packaging/linux/release.json").read_bytes()
        output = self.linux_output()
        with contextlib.redirect_stdout(io.StringIO()):
            packages.main(["--assets", str(self.directory), "--output", str(output),
                           "--descriptor-sha256", descriptor_digest, "--revision", "2"])
        lock = json.loads((output / "release-lock.json").read_bytes())
        descriptor = json.loads((self.directory / "release.json").read_bytes())
        self.assertEqual((lock["version"], lock["revision"], lock["source_commit"], lock["source_sha256"]),
                         (VERSION, 2, COMMIT, self.source_digest))
        self.assertEqual(lock["assets"], descriptor["assets"])
        self.assertEqual(lock["source_date_epoch"], 1700000000)
        for name in packages.LICENSES:
            self.assertEqual(lock["licenses"][name], {"bytes": 8, "sha256": release.sha(b"fixture\n")})
        inputs, licenses = packages.load_inputs(self.directory, lock)
        for component in packages.COMPONENTS:
            self.assertEqual(packages.deb_files(inputs, licenses)[f"usr/bin/{component}"][0],
                             (self.directory / f"{component}-{packages.TARGET}").read_bytes())
        for recipe in ("aur/flere-bin/PKGBUILD", "aur/flere-bin/.SRCINFO", "rpm/flere.spec"):
            text = (output / recipe).read_text()
            self.assertIn(f"/releases/download/v{VERSION}/", text)
            self.assertIn(f"/flere/{COMMIT}/LICENSE", text)
            for name, asset in lock["assets"].items():
                if not name.endswith(".tar.gz"):
                    self.assertIn(asset["sha256"], text)
        provenance = json.loads((output / "package-provenance.json").read_bytes())
        self.assertEqual(provenance["release_descriptor_sha256"], descriptor_digest)
        self.assertEqual(provenance["release"], lock)
        self.assertEqual(provenance["status"], "prepared_not_published")
        expected = {"aur/flere-bin/PKGBUILD", "aur/flere-bin/.SRCINFO", "rpm/flere.spec",
                    "release-lock.json", "package-provenance.json"}
        self.assertEqual({p.relative_to(output).as_posix() for p in output.rglob("*") if p.is_file()},
                         expected | {"SHA256SUMS"})
        checksums = dict(line.split("  ", 1)[::-1] for line in (output / "SHA256SUMS").read_text().splitlines())
        self.assertEqual(checksums, {name: release.sha((output / name).read_bytes()) for name in expected})
        self.assertEqual((packages.ROOT / "packaging/linux/release.json").read_bytes(), checked_in_lock)

    def test_linux_derivation_rejects_substituted_assets_before_output(self):
        descriptor_digest = self.seal()
        output = self.linux_output()
        for name in ("release.json", "SHA256SUMS", self.source_archive.name,
                     f"flere-{packages.TARGET}"):
            path = self.directory / name
            original = path.read_bytes()
            with self.subTest(name=name):
                path.write_bytes(original + b"changed")
                with self.assertRaises(ValueError), contextlib.redirect_stdout(io.StringIO()):
                    packages.main(["--assets", str(self.directory), "--output", str(output),
                                   "--descriptor-sha256", descriptor_digest])
                self.assertFalse(output.exists())
            path.write_bytes(original)

    def test_linux_derivation_requires_explicit_valid_pin_and_revision(self):
        descriptor_digest = self.seal()
        for pin, revision in (("0" * 64, 1), (descriptor_digest + "\n", 1),
                              (descriptor_digest, 0), (descriptor_digest, -1),
                              (descriptor_digest, True)):
            with self.subTest(pin=pin, revision=revision), self.assertRaises(ValueError):
                packages.lock_from_release(self.directory, pin, revision)
        output = self.linux_output()
        with self.assertRaises(SystemExit), contextlib.redirect_stderr(io.StringIO()):
            packages.main(["--assets", str(self.directory), "--output", str(output), "--revision", "2"])
        self.assertFalse(output.exists())

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
        self.assertEqual(retained[self.source_archive.name], self.source_archive.read_bytes())
        self.api.fail_upload = None
        result = self.publish(digest)
        self.assertEqual(result["status"], "published_immutable")
        self.assertFalse(self.api.release["draft"])
        self.assertEqual(self.api.release["make_latest"], "false")
        self.assertEqual(set(self.api.files), release.allowlist(VERSION))
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

    def test_post_publication_tag_visibility_retries_only_reads(self):
        digest = self.seal()
        original_get = self.api.get
        visibility_reads = []

        def get(path, **kwargs):
            if path == f"git/ref/tags/v{VERSION}" and self.api.release and not self.api.release["draft"]:
                visibility_reads.append(path)
                if len(visibility_reads) <= 2:
                    return None
            return original_get(path, **kwargs)

        with mock.patch.object(self.api, "get", side_effect=get), mock.patch.object(release.time, "sleep") as sleep:
            self.assertEqual(self.publish(digest)["status"], "published_immutable")
        self.assertEqual(len(visibility_reads), 3)
        self.assertEqual(sleep.call_args_list, [mock.call(2), mock.call(2)])
        expected_writes = [("POST", "releases")]
        expected_writes += [("POST", f"releases/1/assets?name={name}") for name in sorted(release.allowlist(VERSION))]
        expected_writes += [("PATCH", "releases/1")]
        self.assertEqual(self.api.writes, expected_writes)
        self.assertFalse(self.api.release["draft"])
        self.assertTrue(self.api.release["immutable"])
        self.assertEqual(self.api.release["make_latest"], "false")
        self.publish(digest)
        self.assertEqual(self.api.writes, expected_writes)

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

    def test_changed_or_wrong_source_archive_stops_before_publication(self):
        original = self.source_archive.read_bytes()
        digest = self.seal()
        self.source_archive.write_bytes(original + b"changed")
        with self.assertRaisesRegex(ValueError, "final bytes"):
            self.publish(digest)
        self.assertEqual(self.api.writes, [])
        self.source_archive.write_bytes(original)
        descriptor = self.directory / "release.json"
        value = json.loads(descriptor.read_bytes())
        value["source_archive"]["fingerprint_algorithm"] = "unrelated-algorithm"
        descriptor.write_bytes(release.json_bytes(value))
        with self.assertRaisesRegex(ValueError, "source archive provenance"):
            self.publish(release.sha(descriptor.read_bytes()))
        self.assertEqual(self.api.writes, [])

    def test_full_source_must_match_both_binary_manifests_before_sealing(self):
        files = sample_files(VERSION)
        files["LICENSE"] = (0o644, b"different source commit")
        write_archive(self.source_archive, files)
        with self.assertRaisesRegex(ValueError, "fingerprint differs"):
            self.seal()
        self.assertFalse((self.directory / "release.json").exists())
        self.assertEqual(self.api.writes, [])

    def test_conflicting_uploaded_source_archive_is_never_replaced(self):
        digest = self.seal()
        self.api.fail_upload = f"flere-{release.TARGET}"
        with self.assertRaisesRegex(ValueError, "interruption"):
            self.publish(digest)
        name = self.source_archive.name
        original = self.api.files[name]
        self.api.files[name] = original[:-1] + bytes([original[-1] ^ 1])
        writes = list(self.api.writes)
        with self.assertRaisesRegex(ValueError, "remote asset bytes differ"):
            self.publish(digest)
        self.assertEqual(self.api.writes, writes)
        self.assertTrue(self.api.release["draft"])

    def test_anonymous_source_archive_download_must_match_promoted_bytes(self):
        digest = self.seal()
        self.publish(digest)
        seen = []
        def download(url):
            name = url.rsplit("/", 1)[-1]
            seen.append(name)
            return self.api.files[name]
        with mock.patch.object(release, "public_download", side_effect=download):
            release.verify_public(self.directory, VERSION, COMMIT, RUN, WORKFLOW, digest)
        self.assertEqual(set(seen), release.allowlist(VERSION))
        self.api.files[self.source_archive.name] += b"changed-public-source"
        with mock.patch.object(release, "public_download", side_effect=download):
            with self.assertRaisesRegex(ValueError, "public download"):
                release.verify_public(self.directory, VERSION, COMMIT, RUN, WORKFLOW, digest)

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


class PublishedTagVisibility(unittest.TestCase):
    def setUp(self):
        self.api = release.GitHub("fixture-token")
        self.url = f"https://api.github.com/repos/{release.REPOSITORY}/git/ref/tags/v{VERSION}"
        self.ref = {"object": {"type": "commit", "sha": COMMIT,
                               "url": f"https://api.github.com/repos/{release.REPOSITORY}/git/commits/{COMMIT}"}}

    def response(self, value):
        response = mock.MagicMock()
        response.__enter__.return_value.read.return_value = json.dumps(value).encode()
        return response

    def error(self, status):
        error = urllib.error.HTTPError(self.url, status, "fixture", {}, None)
        self.addCleanup(error.close)
        return error

    def test_matching_tag_succeeds_immediately_or_after_404_visibility_delay(self):
        for missing in (0, 2):
            with self.subTest(missing=missing), mock.patch.object(release.urllib.request, "build_opener") as build, \
                    mock.patch.object(release.time, "sleep") as sleep:
                build.return_value.open.side_effect = [self.error(404) for _ in range(missing)] + [self.response(self.ref)]
                release.verify_tag(self.api, VERSION, COMMIT, wait_for_visibility=True)
                self.assertEqual(build.return_value.open.call_count, missing + 1)
                self.assertEqual(sleep.call_args_list, [mock.call(2)] * missing)
                self.assertTrue(all(call.args[0].method == "GET" and call.args[0].full_url == self.url
                                    for call in build.return_value.open.call_args_list))

    def test_missing_tag_fails_after_six_reads_and_five_waits(self):
        with mock.patch.object(release.urllib.request, "build_opener") as build, \
                mock.patch.object(release.time, "sleep") as sleep:
            build.return_value.open.side_effect = self.error(404)
            with self.assertRaisesRegex(ValueError, "unavailable after 6 checks"):
                release.verify_tag(self.api, VERSION, COMMIT, wait_for_visibility=True)
            self.assertEqual(build.return_value.open.call_count, 6)
            self.assertEqual(sleep.call_args_list, [mock.call(2)] * 5)

    def test_wrong_visible_tag_fails_without_wait_or_retry(self):
        wrong = copy.deepcopy(self.ref)
        wrong["object"]["sha"] = "d" * 40
        with mock.patch.object(release.urllib.request, "build_opener") as build, \
                mock.patch.object(release.time, "sleep") as sleep:
            build.return_value.open.return_value = self.response(wrong)
            with self.assertRaisesRegex(ValueError, "exact selected source commit"):
                release.verify_tag(self.api, VERSION, COMMIT, wait_for_visibility=True)
            self.assertEqual(build.return_value.open.call_count, 1)
            sleep.assert_not_called()

    def test_non404_api_errors_fail_without_wait_or_retry(self):
        for status in (403, 429, 500):
            with self.subTest(status=status), mock.patch.object(release.urllib.request, "build_opener") as build, \
                    mock.patch.object(release.time, "sleep") as sleep:
                build.return_value.open.side_effect = self.error(status)
                with self.assertRaisesRegex(ValueError, f"HTTP {status}"):
                    release.verify_tag(self.api, VERSION, COMMIT, wait_for_visibility=True)
                self.assertEqual(build.return_value.open.call_count, 1)
                sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
