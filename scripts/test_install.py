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
        self.target = installer.platform_target()
        self.core_url = installer.default_manifest(self.target)
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

    def add_package(self, component, url, *, version="0.3.0", embedded_version=None, source="a" * 64):
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

    def fetch(self, url, limit, output=None):
        self.requested.append(url)
        payload = self.responses[url]
        if len(payload) > limit:
            raise ValueError("fixture HTTPS response exceeds bound")
        if output:
            output.write(payload)
        return b"" if output else payload, len(payload), hashlib.sha256(payload).hexdigest()

    def run_install(self, args):
        with mock.patch.object(installer, "fetch", side_effect=self.fetch), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
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
                    self.run_install([])
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

    def test_core_only_does_not_fetch_or_install_companion(self):
        custom = "https://example.invalid/server/manifest.json"
        self.add_package("flere", custom)
        self.run_install([custom, "--core-only"])
        self.assertEqual([event["component"] for event in self.events() if event["args"][0] == "install"], ["flere"])
        self.assertNotIn(self.companion_url, self.requested)

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


if __name__ == "__main__":
    unittest.main()
