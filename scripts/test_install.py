#!/usr/bin/env python3
"""Offline public-installer fixtures; HTTPS responses and executables are local stand-ins."""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("flere_install", Path(__file__).with_name("install.py"))
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class Installation(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / ".cache/flere/tmp"
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.temporary = tempfile.TemporaryDirectory(prefix="installer-pair-", dir=parent)
        self.root = Path(self.temporary.name)
        self.target = "x86_64-unknown-linux-gnu"
        self.platform = mock.patch.object(installer, "platform_target", side_effect=lambda: self.target)
        self.platform.start()
        self.addCleanup(self.platform.stop)
        self.core_url = installer.release_manifest("0.3.6", self.target, "flere")
        self.companion_url = installer.companion_manifest(self.core_url, self.target)
        self.responses = {}
        self.requested = []
        self.environment = mock.patch.dict(os.environ, {
            "HOME": str(self.root), "XDG_CACHE_HOME": str(self.root / "cache"),
            "FLERE_INSTALL_FIXTURE_EVENTS": str(self.root / "events"),
            "FLERE_INSTALL_FIXTURE_FAIL": "",
        })
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.addCleanup(self.temporary.cleanup)
        self.builds = {}
        self.add_package("flere", self.core_url)
        self.add_package("flere-connect", self.companion_url)
        self.channel()

    def add_package(self, component, url, *, version="0.3.6", embedded_version=None, source="a" * 64):
        build = {"schema_version": 1, "identity_kind": "cargo_generation_stamp", "component": component,
                 "target": self.target, "package_version": version, "build_id": component + "-fixture",
                 "profile": "release", "rustc": "fixture", "compatibility": {
                     "remote_protocol": {"current": "flere-remote-v6", "accepts": ["flere-remote-v6"]}}}
        embedded = dict(build, package_version=embedded_version or version)
        script = f"#!{sys.executable}\n" + '''import hashlib,json,os,pathlib,shutil,sys
build=json.loads(BUILD)
component=build['component']
args=sys.argv[1:]
with open(os.environ['FLERE_INSTALL_FIXTURE_EVENTS'],'a') as log:
    log.write(json.dumps({'component':component,'args':args})+'\\n')
if args==['--build-info']:
    print(json.dumps(build))
elif args[0]=='verify-package':
    directory=pathlib.Path(args[1]); manifest=json.loads((directory/'manifest.json').read_bytes())
    payload=(directory/manifest['payload']['file_name']).read_bytes()
    assert len(payload)==manifest['payload']['bytes']
    assert hashlib.sha256(payload).hexdigest()==manifest['payload']['sha256']
    print(json.dumps({'schema_version':1,'status':'verified','manifest':manifest}))
elif args[0]=='install':
    if os.environ.get('FLERE_INSTALL_FIXTURE_FAIL')==component: sys.exit(7)
    directory=pathlib.Path(args[1]); manifest=json.loads((directory/'manifest.json').read_bytes())
    destination=pathlib.Path.home()/'.local/bin'/component
    destination.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
    shutil.copyfile(directory/component,destination)
    print(json.dumps({'component':component,'current':{'manifest':manifest},'destination':str(destination)}))
else: sys.exit(8)
'''.replace("BUILD", repr(json.dumps(embedded)))
        payload = script.encode()
        asset = f"{component}-{self.target}"
        manifest = {"schema_version": 1, "build": build, "payload": {
            "file_name": component, "download_file": asset, "bytes": len(payload),
            "sha256": hashlib.sha256(payload).hexdigest()}, "source": {
                "source_sha256": source, "git_commit": "b" * 40, "checks": ["fixture"]}}
        self.responses[url] = json.dumps(manifest).encode()
        self.responses[url.rsplit("/", 1)[0] + "/" + asset] = payload
        self.builds[component] = manifest

    def channel(self, *, version="0.3.6", policy="current"):
        pins = {component: {"bytes": len(self.responses[url]),
                            "sha256": hashlib.sha256(self.responses[url]).hexdigest()}
                for component, url in [("flere", self.core_url), ("flere-connect", self.companion_url)]}
        channel = {"schema_version": 1, "targets": {self.target: {
            "policy": policy, "version": version, "manifests": pins}}}
        self.responses[installer.CHANNEL_URL] = json.dumps(channel).encode()
        return channel

    def fetch(self, url, limit, output=None):
        self.requested.append(url)
        payload = self.responses[url]
        if len(payload) > limit:
            raise ValueError("fixture HTTPS response exceeds bound")
        if output:
            output.write(payload)
        return b"" if output else payload, len(payload), hashlib.sha256(payload).hexdigest()

    def run_install(self, args):
        self.stderr = io.StringIO()
        with mock.patch.object(installer, "fetch", side_effect=self.fetch), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(self.stderr):
            installer.main(args)

    def events(self):
        path = self.root / "events"
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def reports(self):
        return [json.loads(path.read_bytes()) for path in (self.root / "cache/flere/downloads").glob("installation-result-*.json")]

    def test_default_pair_verifies_both_before_install_and_records_each_source(self):
        self.run_install(["--adopt"])
        events = self.events()
        first_install = next(index for index, event in enumerate(events) if event["args"][0] == "install")
        self.assertEqual(first_install, 4)
        self.assertEqual([event["component"] for event in events[:4] if event["args"] == ["--build-info"]], ["flere", "flere-connect"])
        self.assertEqual(len([event for event in events[:4] if event["args"][0] == "verify-package"]), 2)
        installed = [event for event in events if event["args"][0] == "install"]
        for event, url in zip(installed, [self.core_url, self.companion_url]):
            self.assertEqual(event["args"][-3:], ["--source-url", url, "--adopt"])
        self.assertEqual(self.reports()[0]["status"], "installed")
        self.assertFalse(self.reports()[0]["atomic_pair"])
        self.assertTrue((self.root / ".local/bin/flere-connect").is_file())
        self.assertEqual(self.requested.count(installer.CHANNEL_URL), 1)
        self.assertEqual(self.reports()[0]["discovery"], {
            "url": installer.CHANNEL_URL, "sha256": hashlib.sha256(self.responses[installer.CHANNEL_URL]).hexdigest(),
            "target": self.target, "policy": "current", "version": "0.3.6"})
        self.assertEqual(self.reports()[0]["sources"], {"flere": self.core_url, "flere-connect": self.companion_url})
        self.assertFalse(any("/latest/" in url for url in self.requested))

    def test_corrupt_second_download_never_mutates_installation(self):
        asset = self.companion_url.rsplit("/", 1)[0] + f"/flere-connect-{self.target}"
        self.responses[asset] = self.responses[asset][:-1] + b"!"
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            self.run_install([])
        self.assertEqual(self.events(), [])
        self.assertFalse((self.root / ".local/bin").exists())

    def test_release_source_and_embedded_identity_mismatches_prevent_both_installs(self):
        for kwargs in [{"version": "0.4.0"}, {"source": "c" * 64}, {"embedded_version": "0.4.0"}]:
            with self.subTest(kwargs=kwargs):
                self.add_package("flere-connect", self.companion_url, **kwargs)
                with self.assertRaises(ValueError):
                    self.run_install([self.core_url])
                self.assertFalse(any(event["args"][0] == "install" for event in self.events()))
        self.assertFalse((self.root / ".local/bin").exists())

    def test_custom_channel_needs_explicit_pair_or_core_only(self):
        custom = "https://example.invalid/private/manifest.json"
        pair = "https://example.invalid/companion/manifest.json"
        with self.assertRaisesRegex(ValueError, "--companion-url"):
            self.run_install([custom])
        self.assertEqual(self.requested, [])
        self.add_package("flere", custom)
        self.add_package("flere-connect", pair)
        self.run_install([custom, "--companion-url", pair])
        self.assertEqual(self.reports()[0]["sources"], {"flere": custom, "flere-connect": pair})
        self.assertNotIn(installer.CHANNEL_URL, self.requested)
        self.assertNotIn("discovery", self.reports()[0])

    def test_core_only_does_not_fetch_or_install_companion(self):
        custom = "https://example.invalid/server/manifest.json"
        self.add_package("flere", custom)
        self.run_install([custom, "--core-only"])
        self.assertEqual([event["component"] for event in self.events() if event["args"][0] == "install"], ["flere"])
        self.assertNotIn(self.companion_url, self.requested)
        self.assertNotIn(installer.CHANNEL_URL, self.requested)

    def test_default_core_only_never_fetches_companion(self):
        self.run_install(["--core-only"])
        self.assertEqual(self.requested, [installer.CHANNEL_URL, self.core_url,
                         self.core_url.rsplit("/", 1)[0] + f"/flere-{self.target}"])
        self.assertEqual([entry["component"] for entry in self.reports()[0]["installed"]], ["flere"])

    def test_default_core_allows_explicit_companion_with_unchanged_pair_checks(self):
        custom = "https://example.invalid/companion/manifest.json"
        self.add_package("flere-connect", custom, source="c" * 64)
        with self.assertRaisesRegex(ValueError, "different source"):
            self.run_install(["--companion-url", custom])
        self.assertNotIn(self.companion_url, self.requested)
        self.assertFalse(any(event["args"][0] == "install" for event in self.events()))
        self.add_package("flere-connect", custom)
        self.run_install(["--companion-url", custom])
        self.assertEqual(self.reports()[0]["sources"]["flere-connect"], custom)

    def test_bad_or_unavailable_index_stops_before_staging_without_fallback(self):
        cases = [b"not JSON", b" " * (installer.MAX_CHANNEL_BYTES + 1),
                 b'{"schema_version":1,"targets":{}}',
                 json.dumps({"schema_version": 1, "targets": {self.target: {"policy": "unavailable"}}}).encode()]
        # Even an unselected target must validate before any package is fetched.
        invalid_other = self.channel()
        invalid_other["targets"]["x86_64-apple-darwin"] = {"policy": "unavailable", "url": "bad"}
        cases.append(json.dumps(invalid_other).encode())
        for raw in cases:
            with self.subTest(raw=raw[:100]):
                self.responses[installer.CHANNEL_URL] = raw
                self.requested.clear()
                with self.assertRaises(ValueError):
                    self.run_install([])
                self.assertEqual(self.requested, [installer.CHANNEL_URL])
                self.assertFalse((self.root / "cache").exists())
        self.assertEqual(self.events(), [])

    def test_offline_index_stops_without_fallback(self):
        with mock.patch.object(installer, "fetch", side_effect=OSError("offline")) as fetch:
            with self.assertRaisesRegex(ValueError, "no fallback"):
                installer.main([])
        fetch.assert_called_once_with(installer.CHANNEL_URL, installer.MAX_CHANNEL_BYTES)
        self.assertFalse((self.root / "cache").exists())

    def test_manifest_raw_pin_and_selection_are_checked_before_payload(self):
        original = self.responses[self.core_url]
        for case in ["bytes", "sha256", "version", "component", "target"]:
            with self.subTest(case=case):
                self.responses[self.core_url] = original
                channel = self.channel()
                pin = channel["targets"][self.target]["manifests"]["flere"]
                if case == "bytes":
                    pin["bytes"] += 1
                elif case == "sha256":
                    # Whitespace preserves parsed JSON; the raw hash must still reject it.
                    self.responses[self.core_url] = original.replace(b": ", b":\t", 1)
                else:
                    manifest = json.loads(original)
                    field = "package_version" if case == "version" else case
                    manifest["build"][field] = "0.3.5" if case == "version" else "wrong"
                    self.responses[self.core_url] = json.dumps(manifest).encode()
                    channel = self.channel()  # Matching hash alone cannot bless wrong metadata.
                self.responses[installer.CHANNEL_URL] = json.dumps(channel).encode()
                self.requested.clear()
                with self.assertRaisesRegex(ValueError, "manifest"):
                    self.run_install(["--core-only"])
                self.assertEqual(self.requested, [installer.CHANNEL_URL, self.core_url])
                self.assertEqual(self.events(), [])
                self.assertFalse((self.root / ".local/bin").exists())

    def test_companion_manifest_pin_is_checked_before_either_executable_runs(self):
        self.responses[self.companion_url] = self.responses[self.companion_url].replace(b": ", b":\t", 1)
        with self.assertRaisesRegex(ValueError, "manifest.*SHA-256"):
            self.run_install([])
        self.assertEqual(self.requested, [installer.CHANNEL_URL, self.core_url,
                         self.core_url.rsplit("/", 1)[0] + f"/flere-{self.target}", self.companion_url])
        self.assertEqual(self.events(), [])
        self.assertFalse((self.root / ".local/bin").exists())

    def test_legacy_macos_uses_fixed_notice_and_pinned_030(self):
        self.target = "aarch64-apple-darwin"
        self.core_url = installer.release_manifest("0.3.0", self.target, "flere")
        self.companion_url = installer.companion_manifest(self.core_url, self.target)
        self.add_package("flere", self.core_url, version="0.3.0")
        self.add_package("flere-connect", self.companion_url, version="0.3.0")
        self.channel(version="0.3.0", policy="legacy_unsigned")
        self.run_install(["--core-only"])
        self.assertIn(installer.LEGACY_NOTICE + "\n", self.stderr.getvalue())
        self.assertEqual(self.stderr.getvalue().count(installer.LEGACY_NOTICE), 1)
        self.assertEqual(self.reports()[0]["discovery"]["policy"], "legacy_unsigned")

    def test_second_install_failure_records_partial_and_keeps_installed_core(self):
        with mock.patch.dict(os.environ, {"FLERE_INSTALL_FIXTURE_FAIL": "flere-connect"}):
            with self.assertRaisesRegex(ValueError, "partial installation"):
                self.run_install([])
        report = self.reports()[0]
        self.assertEqual(report["status"], "partial")
        self.assertEqual(report["attempted"], "flere-connect")
        self.assertEqual([entry["component"] for entry in report["installed"]], ["flere"])
        self.assertTrue((self.root / ".local/bin/flere").is_file())
        self.assertFalse((self.root / ".local/bin/flere-connect").exists())
        self.assertFalse(list((self.root / "cache/flere/downloads").glob("bootstrap-*")))


class ChannelSchema(unittest.TestCase):
    def test_shared_schema_vectors(self):
        vectors = json.loads((Path(__file__).parent / "fixtures/channel-v1.json").read_bytes())
        for vector in vectors:
            with self.subTest(name=vector["name"]):
                raw = vector["raw"].encode("utf-8")
                if vector["valid"]:
                    self.assertEqual(installer.parse_channel(raw), json.loads(raw))
                else:
                    with self.assertRaises(ValueError):
                        installer.parse_channel(raw)

    def test_utf8_depth_and_byte_bounds(self):
        for raw in [b"\xff", b"[" * 1100 + b"]" * 1100, b" " * 8193]:
            with self.subTest(raw=raw[:30]), self.assertRaises(ValueError):
                installer.parse_channel(raw)

    def test_canonical_version_tuple_bounds(self):
        self.assertEqual(installer.version_tuple("0.3.6"), (0, 3, 6))
        self.assertEqual(installer.version_tuple("2147483647.0.1"), (2147483647, 0, 1))
        for value in [None, True, 0.3, "0.03.6", "01.0.0", "1.2", "1.2.3.4", "1.2.3-rc1", "1.2.3\n",
                      "1.+2.3", "1.２.3", "2147483648.0.0", "9" * 5000 + ".0.0"]:
            with self.subTest(value=str(value)[:40]), self.assertRaises(ValueError):
                installer.version_tuple(value)


if __name__ == "__main__":
    unittest.main()
