#!/usr/bin/env python3
"""Pure Mac release-boundary fixtures. No Apple tools, credentials or binaries run."""
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("release_macos", Path(__file__).with_name("release-macos.py"))
mac = importlib.util.module_from_spec(spec); spec.loader.exec_module(mac)
SELECTED = mac.identity("0.4.0", "a" * 40, "1234", "b" * 40)
SOURCE = "c" * 64
TEAM, CERT = "ABCDEFGHIJ", "D" * 40
SUBMISSION = "12345678-1234-1234-1234-123456789abc"
SETTINGS = {"MACOS_TEAM_ID": TEAM, "MACOS_DEVELOPER_ID_SHA1": CERT,
            "MACOS_CERTIFICATE_P12_BASE64": "Zml4dHVyZQ==", "MACOS_CERTIFICATE_PASSWORD": "fixture-password",
            "MACOS_NOTARY_KEY_ID": "0123456789", "MACOS_NOTARY_ISSUER_ID": SUBMISSION,
            "MACOS_NOTARY_PRIVATE_KEY": "-----BEGIN PRIVATE KEY-----\nfixture\n-----END PRIVATE KEY-----\n"}


def executable(library=b"/usr/lib/libSystem.B.dylib", cpu=0x0100000C, subtype=0):
    name = library + b"\0"
    size = (24 + len(name) + 7) // 8 * 8
    dylib = struct.pack("<6I", 0xC, size, 24, 0, 0, 0) + name
    dylib += b"\0" * (size - len(dylib))
    build = struct.pack("<6I", 0x32, 24, 1, 11 << 16, 15 << 16, 0)
    commands = dylib + build
    return struct.pack("<8I", 0xFEEDFACF, cpu, subtype, 2, 2, len(commands), 0, 0) + commands + b"fixture executable"


def display(component="flere", team=TEAM):
    return ("Executable=/private/not-public/" + component + "\nIdentifier=io.github.robert-cronin." + component
            + "\nCodeDirectory v=20500 size=100 flags=0x10000(runtime) hashes=5+7 location=embedded"
            + "\nAuthority=Developer ID Application: Fixture (" + team + ")\nAuthority=Developer ID Certification Authority"
            + "\nTeamIdentifier=" + team + "\nTimestamp=Sep 15, 2026 at 12:00:00 PM\n").encode()


class MacRelease(unittest.TestCase):
    def setUp(self):
        cache = Path.home() / ".cache/flere/tmp"; cache.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(prefix="mac-release-", dir=cache); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.built = self.root / "built"; self.built.mkdir()
        self.signed, self.state = self.root / "signed", self.root / "notary"
        self.calls = []
        manifests = {}
        for component in mac.COMPONENTS:
            name = component + "-" + mac.TARGET
            data = executable()
            item = {"schema_version": 1,
                    "build": {"component": component, "target": mac.TARGET, "profile": "release",
                              "package_version": SELECTED["version"], "build_id": "fixture",
                              "compatibility": {"remote_protocol": {"current": "v6", "accepts": ["v6"]}}},
                    "source": {"git_commit": SELECTED["commit"], "source_sha256": SOURCE, "dirty": False},
                    "payload": {"file_name": component, "download_file": name, "bytes": len(data), "sha256": mac.sha(data)}}
            (self.built / name).write_bytes(data)
            (self.built / (name + ".manifest.json")).write_bytes(mac.release.json_bytes(item))
            manifests[component] = item
        for name in mac.LICENSES:
            (self.built / name).write_bytes(b"fixture license\n")
        self.build = dict(schema="flere-macos-build-v1", status="built_not_signed", identity=SELECTED,
                          source_sha256=SOURCE, source_unchanged=True, checks=mac.CHECKS,
                          files=mac.inventory(self.built, mac.payload_names() | set(mac.LICENSES)), manifests=manifests,
                          validation_log_sha256=mac.sha(b"native fixture checks passed\n"))
        (self.built / "validation.log").write_bytes(b"native fixture checks passed\n")
        self.build_pin = mac.save(self.built / "build.json", self.build)

    def fake_command(self, argv, home, **kwargs):
        self.calls.append(list(map(str, argv)))
        if argv[0] == "/usr/bin/security" and argv[1] == "create-keychain":
            Path(argv[-1]).write_bytes(b"fixture keychain")
        if argv[0] == "/usr/bin/security" and argv[1] == "delete-keychain":
            Path(argv[-1]).unlink(missing_ok=True)
        if argv[0] == "/usr/bin/codesign" and "--sign" in argv:
            path = Path(argv[-1]); path.write_bytes(path.read_bytes() + b"fixture Developer ID signature")
        if argv[0] == "/usr/bin/codesign" and "--display" in argv:
            component = Path(argv[-1]).name.removesuffix("-" + mac.TARGET)
            return 0, display(component)
        if argv[1:] == ["list-keychains", "-d", "user"]:
            return 0, b'    "/private/fixture/login.keychain-db"\n    "/Library/Keychains/System.keychain"\n'
        if "notarytool" in argv:
            action = argv[2]
            if action == "submit":
                return 0, json.dumps({"id": SUBMISSION}).encode()
            if action in ("wait", "info"):
                return 0, json.dumps({"id": SUBMISSION, "status": "Accepted"}).encode()
            if action == "log":
                return 0, json.dumps({"jobId": SUBMISSION, "status": "Accepted", "statusCode": 0,
                                      "sha256": mac.sha((self.signed / mac.zip_name(SELECTED["version"])).read_bytes())}).encode()
        return 0, b""

    def signer(self, command=None):
        settings = {key: SETTINGS[key] if key in mac.SIGN_KEYS else "" for key in SETTINGS}
        with mock.patch.object(mac, "native"), mock.patch.dict(os.environ, settings), \
                mock.patch.object(mac, "command", side_effect=command or self.fake_command), \
                mock.patch.object(mac, "capture", return_value=(0, b"", b"")):
            result = mac.sign(self.built, self.signed, SELECTED, self.build_pin)
        self.signed_pin = result["signed_receipt_sha256"]
        return result

    def notary(self, command=None, **kwargs):
        settings = {key: SETTINGS[key] if key in mac.NOTARY_KEYS else "" for key in SETTINGS}
        with mock.patch.object(mac, "native"), mock.patch.dict(os.environ, settings), \
                mock.patch.object(mac, "command", side_effect=command or self.fake_command):
            return mac.notarize(self.signed, self.state, SELECTED, self.signed_pin, **kwargs)

    def test_missing_or_malformed_configuration_fails_without_exposing_values(self):
        with self.assertRaisesRegex(mac.PhaseError, "macOS release settings missing: MACOS_DEVELOPER_ID_SHA1"):
            mac.configuration({}, "preflight")
        mac.configuration(SETTINGS, "preflight")
        for phase in ("sign", "notarize"):
            with self.assertRaisesRegex(mac.PhaseError, "scoped to this release phase"):
                mac.configuration(SETTINGS, phase)
        for key, value in (("MACOS_TEAM_ID", 'BAD";team'), ("MACOS_DEVELOPER_ID_SHA1", "-"),
                           ("MACOS_CERTIFICATE_P12_BASE64", "!!!"), ("MACOS_NOTARY_KEY_ID", "../key"),
                           ("MACOS_NOTARY_ISSUER_ID", "not-an-issuer"), ("MACOS_NOTARY_PRIVATE_KEY", "secret-marker")):
            with self.subTest(key=key), self.assertRaises(ValueError) as result:
                mac.configuration(dict(SETTINGS, **{key: value}), "preflight")
            self.assertNotIn("secret-marker", str(result.exception))

    def test_build_and_runtime_refuse_apple_credentials(self):
        env = {"GITHUB_ACTIONS": "true", "FLERE_RUNNER_ENVIRONMENT": "github-hosted",
               "GITHUB_REPOSITORY": mac.release.REPOSITORY, "GITHUB_EVENT_NAME": "workflow_dispatch",
               "GITHUB_REF": "refs/heads/main", "GITHUB_RUN_ID": SELECTED["run_id"]}
        with mock.patch.object(mac.sys, "platform", "darwin"), mock.patch.object(mac.platform, "machine", return_value="arm64"), \
                mock.patch.object(mac.release, "git", return_value=SELECTED["workflow_sha"]), mock.patch.dict(os.environ, env, clear=True):
            mac.native(SELECTED)
            with mock.patch.dict(os.environ, SETTINGS), self.assertRaisesRegex(mac.PhaseError, "must not receive Apple"):
                mac.native(SELECTED)
            with mock.patch.dict(os.environ, {"GITHUB_REF": "refs/heads/other"}), self.assertRaises(mac.PhaseError):
                mac.native(SELECTED)

    def test_macho_rejects_wrong_architecture_linkage_and_truncated_commands(self):
        self.assertEqual(mac.macho(executable())["minimum_macos"], "11.0.0")
        for data in (executable(cpu=0x01000007), executable(subtype=2), executable(library=b"/opt/homebrew/lib/dylib"),
                     executable(library=b"/usr/lib/../private/dylib"), executable()[:40], b"\xca\xfe\xba\xbe" + executable()[4:]):
            with self.assertRaises(ValueError):
                mac.macho(data)

    def test_wrong_source_version_protocol_and_tampered_build_log_reject(self):
        for changes in ({"version": "0.4.1"}, {"commit": "d" * 40}, {"workflow_sha": "e" * 40}, {"run_id": "2345"}):
            with self.assertRaises(ValueError):
                mac.validate_built(self.built, self.build_pin, dict(SELECTED, **changes))
        path = self.built / ("flere-connect-" + mac.TARGET + ".manifest.json")
        original = path.read_bytes()
        item = json.loads(original); item["build"]["compatibility"]["remote_protocol"]["accepts"] = ["v5"]
        path.write_bytes(mac.release.json_bytes(item))
        with self.assertRaisesRegex(mac.PhaseError, "protocol"):
            mac.pair(self.built, SELECTED, SOURCE)
        path.write_bytes(original)
        (self.built / "validation.log").write_bytes(b"changed")
        with self.assertRaisesRegex(mac.PhaseError, "bytes/log"):
            mac.validate_built(self.built, self.build_pin, SELECTED)

    def test_signing_rebinds_final_bytes_and_zip_without_executing_payload_or_source(self):
        before = {p.name: p.read_bytes() for p in self.built.iterdir()}
        self.signer()
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.built.iterdir()})
        signed = mac.validate_signed(self.signed, self.signed_pin, SELECTED)
        self.assertTrue(signed["keychain_removed"])
        self.assertTrue(signed["search_list_restored"])
        self.assertEqual({c[0] for c in self.calls}, {"/usr/bin/security", "/usr/bin/codesign"})
        for call in [c for c in self.calls if "--sign" in c]:
            self.assertIn("--timestamp", call); self.assertIn("runtime", call); self.assertIn(CERT, call)
            self.assertIn("--keychain", call); self.assertNotIn("--deep", call)
        self.assertEqual(self.calls[-1], ["/usr/bin/security", "list-keychains", "-d", "user"])
        self.assertEqual(self.calls[-2][1], "delete-keychain")
        self.assertEqual(self.calls[-3], ["/usr/bin/security", "list-keychains", "-d", "user", "-s",
                                        "/private/fixture/login.keychain-db", "/Library/Keychains/System.keychain"])
        self.assertFalse(any(p.name.startswith("sign-key-") for p in self.root.iterdir()))
        for component in mac.COMPONENTS:
            original = self.build["manifests"][component]
            updated = json.loads((self.signed / (component + "-" + mac.TARGET + ".manifest.json")).read_bytes())
            self.assertNotEqual(updated["payload"]["sha256"], original["payload"]["sha256"])
            self.assertEqual(updated["build"], original["build"]); self.assertEqual(updated["source"], original["source"])
        self.assertNotIn(SETTINGS["MACOS_CERTIFICATE_PASSWORD"].encode(), (self.signed / "signed.json").read_bytes())
        with self.assertRaises(ValueError):
            self.signer()  # Existing signed bytes cannot be silently re-signed.

    def test_signing_failure_removes_owned_keychain_and_cannot_create_signed_receipt(self):
        def failure(argv, home, **kwargs):
            result = self.fake_command(argv, home, **kwargs)
            if "--sign" in argv:
                raise mac.PhaseError("fixture signing failure")
            return result
        with self.assertRaisesRegex(mac.PhaseError, "fixture signing failure"):
            self.signer(failure)
        self.assertEqual(self.calls[-1], ["/usr/bin/security", "list-keychains", "-d", "user"])
        self.assertEqual(self.calls[-2][1], "delete-keychain")
        self.assertEqual(self.calls[-3][-2:], ["/private/fixture/login.keychain-db", "/Library/Keychains/System.keychain"])
        self.assertFalse((self.signed / "signed.json").exists())
        failed = json.loads((self.signed / "signing-failure.json").read_bytes())
        self.assertTrue(failed["signing_failed"]); self.assertTrue(all(failed["cleanup"].values()))
        self.assertFalse(any(p.name.startswith("sign-key-") for p in self.root.iterdir()))

    def test_cleanup_success_exit_cannot_replace_search_list_and_keychain_readback(self):
        for wrong in ("search_list", "retained_keychain"):
            with self.subTest(wrong=wrong):
                reads = 0
                def incomplete(argv, home, **kwargs):
                    nonlocal reads
                    if wrong == "retained_keychain" and argv[1] == "delete-keychain":
                        self.calls.append(list(map(str, argv)))
                        return 0, b""  # Claimed success but left the exact owned file.
                    result = self.fake_command(argv, home, **kwargs)
                    if argv[1:] == ["list-keychains", "-d", "user"]:
                        reads += 1
                        if wrong == "search_list" and reads == 2:
                            return 0, b'"/private/wrong-list.keychain-db"\n'
                    return result
                with self.assertRaisesRegex(mac.PhaseError, "cleanup could not be verified"):
                    self.signer(incomplete)
                self.assertFalse((self.signed / "signed.json").exists())
                failure = json.loads((self.signed / "signing-failure.json").read_bytes())
                self.assertFalse(failure["signing_failed"])
                field = "search_list_matches" if wrong == "search_list" else "keychain_absent"
                self.assertFalse(failure["cleanup"][field])
                shutil.rmtree(self.signed)

    def test_signature_requires_developer_id_team_timestamp_runtime_and_no_entitlements(self):
        for data in (display().replace(b"Developer ID Application:", b"Apple Development:"),
                     display(team="OTHERTEAM0"), display().replace(b"0x10000(runtime)", b"0x0(none)"),
                     display().replace(b"Timestamp=", b"Signed Time=")):
            with self.assertRaises(ValueError):
                mac.signature_details(data, "flere", TEAM)
        with self.assertRaises(ValueError):
            mac.signature_requirement("flere", 'TEAM"bad', CERT)
        path = self.built / ("flere-" + mac.TARGET)
        with mock.patch.object(mac, "command", side_effect=self.fake_command), \
                mock.patch.object(mac, "capture", return_value=(0, plistlib.dumps({"com.apple.security.get-task-allow": True}), b"")), \
                self.assertRaisesRegex(mac.PhaseError, "entitlements"):
            mac.signature(path, "flere", TEAM, CERT, self.root)

    def test_notary_exact_acceptance_resumes_read_only_without_resubmit_or_resign(self):
        self.signer(); self.calls.clear()
        result = self.notary()
        self.assertEqual(result["status"], "Accepted")
        archive_sha = mac.sha((self.signed / mac.zip_name(SELECTED["version"])).read_bytes())
        receipt = mac.validate_notary(self.state, result["notary_receipt_sha256"], SELECTED, self.signed_pin, archive_sha)
        self.assertEqual(receipt["submission_id"], SUBMISSION)
        self.assertEqual([c[2] for c in self.calls], ["submit", "wait", "info", "log"])
        self.assertFalse(any("--force" in c for c in self.calls))
        self.calls.clear()
        again = self.notary(resume_pin=result["notary_receipt_sha256"])
        self.assertEqual(again, result); self.assertEqual(self.calls, [])

    def test_notary_timeout_resumes_same_id_and_zip_and_rejects_wrong_pin(self):
        self.signer(); self.calls.clear()
        def timeout(argv, home, **kwargs):
            if argv[2] == "wait":
                self.calls.append(list(map(str, argv)))
                raise mac.PhaseError("fixture wait timeout")
            return self.fake_command(argv, home, **kwargs)
        with self.assertRaisesRegex(mac.PhaseError, "timeout"):
            self.notary(timeout)
        raw = (self.state / "notary.json").read_bytes()
        self.assertEqual(json.loads(raw)["status"], "pending")
        self.assertEqual(json.loads(raw)["submission_id"], SUBMISSION)
        with self.assertRaises(ValueError):
            self.notary(resume_pin="0" * 64)
        self.calls.clear()
        result = self.notary(resume_pin=mac.sha(raw))
        self.assertEqual(result["status"], "Accepted")
        self.assertEqual([c[2] for c in self.calls], ["wait", "info", "log"])

    def test_ambiguous_upload_never_resubmits_and_rejected_log_never_accepts(self):
        self.signer()
        def lost_reply(argv, home, **kwargs):
            self.calls.append(list(map(str, argv)))
            raise mac.PhaseError("upload result lost")
        with self.assertRaises(ValueError):
            self.notary(lost_reply)
        pin = mac.sha((self.state / "notary.json").read_bytes())
        self.calls.clear()
        with self.assertRaisesRegex(mac.PhaseError, "ambiguous"):
            self.notary(resume_pin=pin)
        self.assertEqual(self.calls, [])
        for change in ({"status": "Invalid"}, {"statusCode": True}, {"sha256": "0" * 64},
                       {"jobId": "87654321-1234-1234-1234-123456789abc"}):
            log = dict(jobId=SUBMISSION, status="Accepted", statusCode=0, sha256="f" * 64)
            log.update(change)
            with self.assertRaises(ValueError):
                mac.accepted_log(log, SUBMISSION, "f" * 64)

    def test_signed_inventory_zip_or_stale_presign_manifest_rejects(self):
        self.signer()
        archive = self.signed / mac.zip_name(SELECTED["version"])
        raw = archive.read_bytes(); archive.write_bytes(raw + b"corrupt")
        with self.assertRaises(ValueError):
            mac.validate_signed(self.signed, self.signed_pin, SELECTED)
        archive.write_bytes(raw)
        extra = self.signed / "identity.p12"; extra.write_bytes(b"fixture extra")
        with self.assertRaisesRegex(mac.PhaseError, "inventory"):
            mac.validate_signed(self.signed, self.signed_pin, SELECTED)
        extra.unlink()
        path = self.signed / ("flere-" + mac.TARGET + ".manifest.json")
        path.write_bytes(mac.release.json_bytes(self.build["manifests"]["flere"]))
        with self.assertRaises(ValueError):
            mac.pair(self.signed, SELECTED, SOURCE)

    def test_runtime_cannot_execute_pending_notarization_or_changed_signed_bytes(self):
        self.signer()
        result = self.notary()
        raw = (self.state / "notary.json").read_bytes()
        pending = json.loads(raw); pending["status"] = "pending"
        pending_pin = mac.save(self.state / "notary.json", pending)
        output = self.root / "runtime"
        with mock.patch.object(mac, "native"), mock.patch.object(mac, "command") as execute:
            with self.assertRaisesRegex(mac.PhaseError, "not accepted"):
                mac.verify(self.signed, self.state, output, SELECTED, self.signed_pin, pending_pin)
            self.assertFalse(output.exists()); execute.assert_not_called()
            (self.state / "notary.json").write_bytes(raw)
            payload = self.signed / ("flere-" + mac.TARGET)
            payload.write_bytes(payload.read_bytes() + b"changed after acceptance")
            with self.assertRaisesRegex(mac.PhaseError, "asset set"):
                mac.verify(self.signed, self.state, output, SELECTED, self.signed_pin, result["notary_receipt_sha256"])
            self.assertFalse(output.exists()); execute.assert_not_called()

    def test_final_runtime_binds_both_components_six_flags_and_exact_install_copies(self):
        self.signer(); result = self.notary()
        before = {p.name: p.read_bytes() for p in self.signed.iterdir()}
        self.calls.clear()
        def runtime(argv, home, **kwargs):
            env = kwargs.get("env", mac.environment(home))
            self.assertFalse(set(mac.SIGN_KEYS + mac.NOTARY_KEYS) & set(env))
            if argv[0] == "/usr/sbin/spctl" and "--status" in argv:
                return 0, b"assessments enabled\n"
            if "install" in argv:
                package = Path(argv[argv.index("install") + 1]); component = package.name
                destination = home / ".local/bin" / component; destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(package / component, destination)
                return 0, b"{}\n"
            component = Path(argv[0]).name
            if component in mac.COMPONENTS and argv[-1] == "--build-info":
                return 0, mac.release.json_bytes(self.build["manifests"][component]["build"])
            if component in mac.COMPONENTS and argv[-1] == "--version":
                return 0, f"{component} {SELECTED['version']} fixture\n".encode()
            if component in mac.COMPONENTS and argv[-1] == "--help":
                return 0, b"fixture --build-info help\n"
            return self.fake_command(argv, home, **kwargs)
        with mock.patch.object(mac, "native"), mock.patch.object(mac, "command", side_effect=runtime), \
                mock.patch.object(mac.platform, "mac_ver", return_value=("15.7.9", ("", "", ""), "arm64")), \
                mock.patch.object(mac, "capture", return_value=(0, b"", b"")):
            checked = mac.verify(self.signed, self.state, self.root / "runtime", SELECTED, self.signed_pin,
                                 result["notary_receipt_sha256"])
        raw = (self.root / "runtime/accepted.json").read_bytes(); accepted = json.loads(raw)
        self.assertEqual(mac.sha(raw), checked["accepted_receipt_sha256"])
        self.assertEqual(accepted["status"], "accepted_not_published")
        self.assertEqual(len(accepted["checks"]), 10)
        self.assertEqual(accepted["submission_id"], SUBMISSION)
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.signed.iterdir()})
        self.assertEqual(len([c for c in self.calls if "--check-notarization" in c]), 2)
        accepted["checks"].pop()
        bad_pin = mac.save(self.root / "runtime/accepted.json", accepted)
        with self.assertRaisesRegex(mac.PhaseError, "incomplete final"):
            mac.validate_accepted(self.signed, self.state, self.root / "runtime/accepted.json", bad_pin,
                                  SELECTED, self.signed_pin, result["notary_receipt_sha256"])

    def fake_child(self, stdout=b"", stderr=b"", *, complete=False, stubborn=False):
        class Child:
            def __init__(child, argv, **kwargs):
                child.returncode = 0 if complete else None
                child.events = []
                kwargs["stdout"].write(stdout); kwargs["stdout"].flush()
                destination = kwargs["stdout"] if kwargs["stderr"] == mac.subprocess.STDOUT else kwargs["stderr"]
                destination.write(stderr); destination.flush()
                self.child = child
            def poll(child):
                return child.returncode
            def terminate(child):
                child.events.append("terminate")
                if not stubborn:
                    child.returncode = -15
            def kill(child):
                child.events.append("kill"); child.returncode = -9
            def wait(child, timeout=None):
                child.events.append(("wait", timeout))
                if child.returncode is None:
                    raise mac.subprocess.TimeoutExpired("fixture", timeout)
                return child.returncode
        return Child

    def test_entitlement_capture_bounds_both_live_streams_and_reaps_overflow(self):
        for stdout, stderr in ((b"x" * 17, b""), (b"", b"x" * 17), (b"x" * 9, b"x" * 8)):
            with self.subTest(stdout=len(stdout), stderr=len(stderr)), \
                    mock.patch.object(mac.subprocess, "Popen", self.fake_child(stdout, stderr)), \
                    self.assertRaisesRegex(mac.PhaseError, "output exceeded bound") as error:
                mac.capture(["/usr/bin/codesign", "fixture-secret-argument"], self.root, maximum=16)
            self.assertNotIn("fixture-secret-argument", str(error.exception))
            self.assertEqual(self.child.events, ["terminate", ("wait", 5), ("wait", None)])

    def test_entitlement_capture_timeout_kills_stubborn_owned_child_without_disclosure(self):
        with mock.patch.object(mac.subprocess, "Popen", self.fake_child(stderr=b"fixture-secret-output", stubborn=True)), \
                mock.patch.object(mac.time, "monotonic", side_effect=[0, 31]), \
                self.assertRaisesRegex(mac.PhaseError, "native tool timed out: codesign") as error:
            mac.capture(["/usr/bin/codesign", "fixture-secret-argument"], self.root, timeout=30, maximum=65536)
        self.assertNotIn("fixture-secret", str(error.exception))
        self.assertEqual(self.child.events, ["terminate", ("wait", 5), "kill", ("wait", None)])

    def test_entitlement_capture_keeps_streams_separate_and_checks_fast_exit_overflow(self):
        with mock.patch.object(mac.subprocess, "Popen", self.fake_child(b"<plist/>", b"diagnostic", complete=True)):
            self.assertEqual(mac.capture(["/usr/bin/codesign"], self.root, maximum=32), (0, b"<plist/>", b"diagnostic"))
        self.assertEqual(self.child.events, [("wait", None)])
        with mock.patch.object(mac.subprocess, "Popen", self.fake_child(b"stdout", b"stderr", complete=True)):
            self.assertEqual(mac.command(["/usr/bin/codesign"], self.root), (0, b"stdoutstderr"))
        with mock.patch.object(mac.subprocess, "Popen", self.fake_child(stderr=b"x" * 17, complete=True)), \
                self.assertRaisesRegex(mac.PhaseError, "output exceeded bound"):
            mac.capture(["/usr/bin/codesign"], self.root, maximum=16)
        self.assertEqual(self.child.events, [("wait", None)])


if __name__ == "__main__":
    unittest.main()
