#!/usr/bin/env python3
"""Data-only complete-release fixtures; signatures/notary receipts are synthetic."""
import json
import shutil
import unittest
from unittest import mock
import zipfile

import test_release_aggregate as prior
import test_release_macos as mac_fixtures

release, aggregate = prior.release, prior.aggregate
VERSION, COMMIT, RUN, WORKFLOW = prior.VERSION, prior.COMMIT, prior.RUN, prior.WORKFLOW
mac = aggregate.macos()
SELECTED = mac.identity(VERSION, COMMIT, RUN, WORKFLOW)


class CompleteRelease(unittest.TestCase):
    tar = prior.AggregateRelease.tar
    write_deb = prior.AggregateRelease.write_deb
    write_windows = prior.AggregateRelease.write_windows
    pin_receipt = prior.AggregateRelease.pin_receipt
    reject = prior.AggregateRelease.reject

    def setUp(self):
        prior.AggregateRelease.setUp(self)
        self.mac = self.directory.with_name(self.directory.name + "-mac")
        self.notary = self.directory.with_name(self.directory.name + "-notary")
        self.accepted = self.directory.with_name(self.directory.name + "-accepted.json")
        for path in (self.mac, self.notary):
            path.mkdir()
            self.addCleanup(shutil.rmtree, path, True)
        self.addCleanup(self.accepted.unlink, missing_ok=True)
        self.write_mac()

    def write_mac(self, *, selected=None, source=None, licenses=None, manifests_change=None,
                  payload=None, zip_change=None, zip_metadata_change=None, signed_change=None, notary_change=None, accepted_change=None):
        for path in self.mac.iterdir():
            path.unlink()
        selected = selected or SELECTED
        source = source or self.source_digest
        licenses = licenses or self.licenses
        data = payload or mac_fixtures.executable()
        manifests = {}
        for component in mac.COMPONENTS:
            name = component + "-" + mac.TARGET
            manifests[component] = {"schema_version": 1,
                "build": {"component": component, "target": mac.TARGET, "profile": "release",
                          "package_version": selected["version"], "build_id": "fixture",
                          "compatibility": {"remote_protocol": {"current": "v6", "accepts": ["v6"]}}},
                "source": {"git_commit": selected["commit"], "source_sha256": source, "dirty": False},
                "payload": {"file_name": component, "download_file": name, "bytes": len(data), "sha256": mac.sha(data)}}
        if manifests_change:
            manifests_change(manifests)
        for component, item in manifests.items():
            name = component + "-" + mac.TARGET
            (self.mac / name).write_bytes(data)
            (self.mac / (name + ".manifest.json")).write_bytes(release.json_bytes(item))
        for name in mac.LICENSES:
            (self.mac / name).write_bytes(licenses[name])
        built = {"schema": "flere-macos-build-v1", "status": "built_not_signed", "identity": selected,
                 "source_sha256": source, "source_unchanged": True, "checks": mac.CHECKS,
                 "files": mac.inventory(self.mac, mac.payload_names() | set(mac.LICENSES)),
                 "manifests": manifests, "validation_log_sha256": mac.sha(b"fixture log"),
                 "private_fixture": "/private/phase/source"}
        build_pin = mac.save(self.mac / "build.json", built)
        # Only data fixtures: no codesign/notary operation or payload is invoked.
        for component, item in manifests.items():
            name = component + "-" + mac.TARGET
            signed_data = data + b"fixture signature"
            (self.mac / name).write_bytes(signed_data)
            (self.mac / (name + ".manifest.json")).write_bytes(release.json_bytes(mac.rebind(item, signed_data)))
        entries = mac.zip_entries(self.mac)
        if zip_change:
            zip_change(entries)
        with zipfile.ZipFile(self.mac / mac.zip_name(VERSION), "x", compression=zipfile.ZIP_DEFLATED) as archive:
            for name, raw in entries.items():
                info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)); info.create_system = 3
                info.external_attr = (0o100755 if name in mac.COMPONENTS else 0o100644) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                if zip_metadata_change:
                    zip_metadata_change(info)
                archive.writestr(info, raw)
        signed = {"schema": "flere-macos-signing-v1", "status": "signed_not_submitted", "identity": selected,
                  "source_sha256": source, "build_receipt_sha256": build_pin,
                  "team_id": mac_fixtures.TEAM, "certificate_sha1": mac_fixtures.CERT,
                  "signatures": {c: {"identifier": "io.github.robert-cronin." + c, "team_id": mac_fixtures.TEAM,
                                     "hardened_runtime": True, "secure_timestamp": True} for c in mac.COMPONENTS},
                  "keychain_removed": True, "search_list_restored": True,
                  "files": mac.inventory(self.mac, mac.payload_names() | set(mac.LICENSES) | {mac.zip_name(VERSION), "build.json"}),
                  "private_fixture": "/private/phase/keychain"}
        if signed_change:
            signed_change(signed)
        signed_pin = mac.save(self.mac / "signed.json", signed)
        archive_sha = mac.sha((self.mac / mac.zip_name(VERSION)).read_bytes())
        log_pin = mac.save(self.notary / "notary-log.json", {"jobId": mac_fixtures.SUBMISSION,
            "status": "Accepted", "statusCode": 0, "sha256": archive_sha, "private_fixture": "/private/notary/log"})
        notary = {"schema": "flere-macos-notary-v1", "status": "Accepted", "identity": selected,
                  "signed_receipt_sha256": signed_pin, "zip_sha256": archive_sha,
                  "submission_id": mac_fixtures.SUBMISSION, "log_sha256": log_pin}
        if notary_change:
            notary_change(notary)
        notary_pin = mac.save(self.notary / "notary.json", notary)
        accepted = {"schema": "flere-macos-accepted-v1", "status": "accepted_not_published", "identity": selected,
                    "source_sha256": source, "signed_receipt_sha256": signed_pin, "notary_receipt_sha256": notary_pin,
                    "submission_id": mac_fixtures.SUBMISSION, "checks": mac.FINAL_CHECKS,
                    "files": signed["files"], "runner_macos": "15.7.9", "private_fixture": "/private/final/runtime"}
        if accepted_change:
            accepted_change(accepted)
        accepted_pin = mac.save(self.accepted, accepted)
        self.mac_pins = dict(zip(aggregate.MAC_PINS, (signed_pin, notary_pin, accepted_pin)))

    def prepare(self, **changes):
        args = dict(schema=4, macos_directory=self.mac, macos_notary_state=self.notary,
                    macos_accepted_receipt=self.accepted, **self.mac_pins)
        args.update(changes)
        return prior.AggregateRelease.prepare(self, **args)

    def test_complete_sixteen_assets_bind_source_licenses_receipts_without_execution(self):
        old = {p.name: p.read_bytes() for p in self.mac.iterdir()}
        with mock.patch("subprocess.Popen", side_effect=AssertionError("aggregate executed a child")):
            proof = self.prepare()
            release.validate(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"])
        self.assertEqual(set(proof["files"]), release.allowlist(VERSION, 4))
        self.assertEqual(len(proof["files"]), 16)
        descriptor = json.loads((self.output / "release.json").read_bytes())
        self.assertEqual(descriptor["schema_version"], 4)
        self.assertEqual(descriptor["source_sha256"], self.source_digest)
        self.assertEqual(descriptor["evidence"]["inputs"], self.mac_pins)
        self.assertEqual(descriptor["evidence"]["macos"]["checks"], mac.FINAL_CHECKS)
        self.assertEqual(descriptor["targets"], release.COMPLETE_TARGETS)
        self.assertEqual(old, {p.name: p.read_bytes() for p in self.mac.iterdir()})
        self.assertNotIn(b"/private", (self.output / "release.json").read_bytes())
        self.assertNotIn(b"private_fixture", (self.output / "release.json").read_bytes())
        self.assertFalse({"signed.json", "build.json", "notary.json", "notary-log.json", "accepted.json"} & set(proof["files"]))
        lock = prior.fixtures.packages.lock_from_release(self.output, proof["descriptor_sha256"])
        self.assertEqual(set(lock["assets"]), release.candidate_names(VERSION))

    def test_explicit_complete_inputs_and_pins_cannot_fall_back_to_partial(self):
        for name in aggregate.MAC_PINS:
            self.reject(**{name: "0" * 64})
        for name in ("macos_directory", "macos_notary_state", "macos_accepted_receipt"):
            self.reject(**{name: None})
        self.reject(schema=3)
        self.reject(schema=True)

    def test_mac_source_version_run_workflow_and_full_archive_licenses_match(self):
        for key, value in (("version", "0.4.1"), ("commit", "c" * 40), ("run_id", "2345"), ("workflow_sha", "d" * 40)):
            self.write_mac(selected={**SELECTED, key: value})
            self.reject()
        self.write_mac(source="0" * 64)
        self.reject()
        for name in mac.LICENSES:
            self.write_mac(licenses={**self.licenses, name: b"unrelated license"})
            self.reject()

    def test_build_only_unsigned_unaccepted_notary_and_missing_final_checks_reject(self):
        for args in (
            {"signed_change": lambda x: x.update(status="built_not_signed")},
            {"signed_change": lambda x: x["signatures"]["flere"].update(hardened_runtime=False)},
            {"signed_change": lambda x: x["signatures"]["flere-connect"].update(secure_timestamp=False)},
            {"signed_change": lambda x: x.update(search_list_restored=False)},
            {"notary_change": lambda x: x.update(status="pending")},
            {"notary_change": lambda x: x.update(status="Rejected")},
            {"notary_change": lambda x: x.update(zip_sha256="0" * 64)},
            {"accepted_change": lambda x: x.update(status="signed_not_submitted")},
            {"accepted_change": lambda x: x.update(checks=mac.FINAL_CHECKS[:-1])},
            {"accepted_change": lambda x: x.update(runner_macos="/private/15.7")},
        ):
            self.write_mac(**args)
            self.reject()
        self.write_mac()
        (self.notary / "notary-log.json").write_bytes(b"{}")
        self.reject()

    def test_macho_architecture_zip_members_bytes_and_modes_reject(self):
        self.write_mac(payload=mac_fixtures.executable(cpu=0x01000007))
        self.reject()
        for change in (lambda x: x.pop("flere-connect"), lambda x: x.update({"../extra": b"extra"}),
                       lambda x: x.update({"flere": b"wrong signed bytes"}), lambda x: x.update(LICENSE=b"wrong license")):
            self.write_mac(zip_change=change)
            self.reject()
        # Wrong mode is pinned throughout otherwise valid phase receipts, so
        # final ZIP inspection, not a stale archive checksum, must reject it.
        def wrong_mode(info):
            if info.filename == "flere": info.external_attr = 0o100644 << 16
        self.write_mac(zip_metadata_change=wrong_mode)
        self.reject()

    def test_every_companion_accepts_every_published_core(self):
        # Both native pairs are internally compatible; Linux/Windows nevertheless
        # cannot read the Mac core's new protocol. Pair-only checks must not pass.
        def new_mac_protocol(items):
            items["flere"]["build"]["compatibility"]["remote_protocol"]["current"] = "v7"
            items["flere-connect"]["build"]["compatibility"]["remote_protocol"]["accepts"] = ["v6", "v7"]
        self.write_mac(manifests_change=new_mac_protocol)
        self.reject()
        # Conversely a Mac companion may read its own new core but not Linux.
        self.write_mac(manifests_change=lambda items: [x["build"]["compatibility"]["remote_protocol"].update(
            current="v7", accepts=["v7"]) for x in items.values()])
        self.reject()
        self.write_mac()
        self.prepare()
        for target in (release.TARGET, mac.TARGET, aggregate.TARGET):
            components = ("flere-connect",) if target == aggregate.TARGET else mac.COMPONENTS
            for component in components:
                path = self.output / (component + "-" + target + ".manifest.json")
                value = json.loads(path.read_bytes())
                value["build"]["compatibility"]["remote_protocol"] = {
                    "current": "v7" if target == mac.TARGET else "v6", "accepts": ["v6", "v7"]}
                path.write_bytes(release.json_bytes(value))
        aggregate.cross_protocols(self.output, release.TARGET)
        for target in (release.TARGET, mac.TARGET, aggregate.TARGET):
            path = self.output / ("flere-connect-" + target + ".manifest.json")
            original = path.read_bytes()
            value = json.loads(original)
            value["build"]["compatibility"]["remote_protocol"]["accepts"] = ["v7" if target == mac.TARGET else "v6"]
            path.write_bytes(release.json_bytes(value))
            with self.assertRaisesRegex(ValueError, "every published core"):
                aggregate.cross_protocols(self.output, release.TARGET)
            path.write_bytes(original)

    def test_missing_extra_symlink_or_altered_final_assets_never_publish(self):
        proof = self.prepare()
        api = prior.fixtures.FakeGitHub(schema=4)
        path = self.output / ("flere-" + mac.TARGET)
        original = path.read_bytes()
        for kind in ("missing", "extra", "symlink", "changed"):
            path.unlink(missing_ok=True)
            if kind == "extra":
                path.write_bytes(original); (self.output / "AuthKey.p8").write_bytes(b"not a key")
            elif kind == "symlink": path.symlink_to(self.mac / path.name)
            elif kind == "changed": path.write_bytes(original + b"changed")
            with self.assertRaises(ValueError):
                release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
            self.assertEqual(api.writes, [])
            (self.output / "AuthKey.p8").unlink(missing_ok=True)

    def test_mac_phase_inventory_extra_or_missing_fails_before_success_output(self):
        (self.mac / "AuthKey.p8").write_bytes(b"not a key")
        self.reject()
        (self.mac / "AuthKey.p8").unlink()
        (self.mac / ("flere-connect-" + mac.TARGET)).unlink()
        self.reject()

    def test_complete_publication_retry_is_immutable_and_publicly_verifiable(self):
        proof = self.prepare()
        api = prior.fixtures.FakeGitHub(schema=4)
        api.fail_upload = mac.zip_name(VERSION)
        with self.assertRaisesRegex(ValueError, "interruption"):
            release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertTrue(api.release["draft"])
        self.assertNotIn(("PATCH", "releases/1"), api.writes)
        api.fail_upload = None
        release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertEqual(set(api.files), release.allowlist(VERSION, 4))
        self.assertEqual(api.release["make_latest"], "false")
        self.assertIn("accepted Apple notarization", api.release["body"])
        writes = list(api.writes)
        release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertEqual(api.writes, writes)
        fetched = []
        def download(url):
            name = url.rsplit("/", 1)[1]; fetched.append(name)
            return api.files[name]
        with mock.patch.object(release, "public_download", side_effect=download):
            release.verify_public(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"])
        self.assertEqual(set(fetched), release.allowlist(VERSION, 4))
        api.files[mac.zip_name(VERSION)] += b"changed"
        with self.assertRaises(ValueError):
            release.publish(self.output, VERSION, COMMIT, RUN, WORKFLOW, proof["descriptor_sha256"], api)
        self.assertEqual(api.writes, writes)


if __name__ == "__main__":
    unittest.main()
