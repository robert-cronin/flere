#!/usr/bin/env python3
"""Offline orchestration fixtures; Apple tools and network calls are mocked."""
import importlib.util
import json
import os
from pathlib import Path
import unittest
from unittest import mock


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


workflow = load("release_workflow", "release-workflow.py")
fixtures = load("workflow_mac_fixtures", "test_release_macos.py")
SELECTED = fixtures.SELECTED
EMIT = workflow.outputs


def history(conclusion="skipped"):
    return {"total_count": 1, "jobs": [{"name": "macos-submit", "conclusion": conclusion,
            "steps": [{"name": workflow.SUBMIT_STEP, "conclusion": conclusion}]}]}


class Workflow(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.MacRelease()
        self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        patcher = mock.patch.object(workflow, "mac", fixtures.mac)
        patcher.start(); self.addCleanup(patcher.stop)
        self.mac = workflow.mac
        self.output = []
        patcher = mock.patch.object(workflow, "outputs", side_effect=lambda value: self.output.append(dict(value)))
        self.outputs = patcher.start(); self.addCleanup(patcher.stop)

    def arguments(self, phase):
        return [phase, "--version", SELECTED["version"], "--commit", SELECTED["commit"],
                "--run-id", SELECTED["run_id"], "--workflow-sha", SELECTED["workflow_sha"]]

    def invoke_notary(self, phase, command, checkpoint=None, attempt=1):
        settings = {key: fixtures.SETTINGS[key] if key in self.mac.NOTARY_KEYS else "" for key in fixtures.SETTINGS}
        settings.update(HOME=str(self.fixture.root), GITHUB_RUN_ATTEMPT=str(attempt))
        args = self.arguments(phase) + ["--directory", str(self.fixture.signed),
                                       "--signed-receipt-sha256", self.fixture.signed_pin]
        if checkpoint:
            args += ["--notary-state", checkpoint["state"], "--notary-receipt-sha256", checkpoint["notary_receipt_sha256"]]
        with mock.patch.dict(os.environ, settings), mock.patch.object(self.mac, "native"), \
                mock.patch.object(self.mac, "command", side_effect=command):
            return workflow.main(args)

    def test_explicit_profile_selects_one_schema_and_no_implicit_mac_fallback(self):
        self.assertEqual(workflow.PROFILES, {"linux-windows": 3, "complete": 4})
        for profile, schema in workflow.PROFILES.items():
            workflow.main(["profile", "--profile", profile])
            self.assertEqual(self.output[-1], {"profile": profile, "schema": schema})
        with self.assertRaises(self.mac.PhaseError):
            workflow.main(["profile"])

    def test_workflow_outputs_reject_control_injection_and_invalid_pins(self):
        destination = self.fixture.root / "job-output"
        with mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(destination)}):
            EMIT({"notary_receipt_sha256": "a" * 64})
            baseline = destination.read_bytes()
            for value in ({"directory": "path\nother=output"}, {"directory": "path\0"},
                          {"notary_receipt_sha256": "wrong"}, {"secret": "unapproved"}):
                with self.subTest(value=value), self.assertRaises(self.mac.PhaseError): EMIT(value)
                self.assertEqual(destination.read_bytes(), baseline)

    def test_prior_started_submission_never_automatically_resubmits(self):
        workflow.prior_submission_guard([{"total_count": 0, "jobs": []}, history()])
        for status in (None, "success", "failure", "cancelled", "timed_out", "in_progress"):
            with self.subTest(status=status), self.assertRaisesRegex(self.mac.PhaseError, "previously started"):
                workflow.prior_submission_guard([history(status)])

    def test_missing_truncated_or_ambiguous_prior_history_fails_closed(self):
        bad = [dict(history(), total_count=2), {"total_count": 1, "jobs": [{"name": "macos-submit"}]},
               {"total_count": 2, "jobs": history()["jobs"] * 2},
               {"total_count": 1, "jobs": [{"name": "macos-submit", "conclusion": "failure", "steps": []}]}]
        for item in bad:
            with self.subTest(item=item), self.assertRaises(self.mac.PhaseError):
                workflow.prior_submission_guard([item])

    def test_guard_binds_run_identity_and_reads_only_bounded_previous_attempts(self):
        api = mock.Mock()
        api.get.side_effect = [{"id": int(SELECTED["run_id"]), "head_sha": SELECTED["workflow_sha"],
                               "event": "workflow_dispatch", "head_branch": "main"}, history()]
        with mock.patch.object(self.mac, "native"), mock.patch.dict(os.environ, {"GITHUB_RUN_ATTEMPT": "2", "GH_TOKEN": "fixture"}), \
                mock.patch.object(self.mac.release, "GitHub", return_value=api):
            workflow.guard_submission(SELECTED)
        self.assertEqual([c.args[0] for c in api.get.call_args_list],
                         ["actions/runs/1234", "actions/runs/1234/attempts/1/jobs?per_page=100"])
        for attempt in ("0", "21", "2\n"):
            with mock.patch.object(self.mac, "native"), mock.patch.dict(os.environ, {"GITHUB_RUN_ATTEMPT": attempt}), \
                    self.assertRaises(self.mac.PhaseError):
                workflow.guard_submission(SELECTED)
        api.get.reset_mock(side_effect=True)
        api.get.return_value = {"id": 999, "head_sha": SELECTED["workflow_sha"], "event": "workflow_dispatch", "head_branch": "main"}
        with mock.patch.object(self.mac, "native"), mock.patch.dict(os.environ, {"GITHUB_RUN_ATTEMPT": "2", "GH_TOKEN": "fixture"}), \
                mock.patch.object(self.mac.release, "GitHub", return_value=api), self.assertRaises(self.mac.PhaseError):
            workflow.guard_submission(SELECTED)
        self.assertEqual(api.get.call_count, 1)

    def test_windows_receipt_is_independently_pinned_and_source_run_bound(self):
        directory = self.fixture.root / "windows"; directory.mkdir()
        receipt = {"status": "prepared_not_published", **{k: SELECTED[k] for k in ("version", "commit", "run_id", "workflow_sha")}}
        env = {"GITHUB_ACTIONS": "true", "FLERE_RUNNER_ENVIRONMENT": "github-hosted", "GITHUB_EVENT_NAME": "workflow_dispatch",
               "GITHUB_REPOSITORY": "robert-cronin/flere", "GITHUB_REF": "refs/heads/main", "RUNNER_OS": "Windows",
               "RUNNER_ARCH": "X64", "FLERE_WORKFLOW_SHA": SELECTED["workflow_sha"], "GITHUB_RUN_ID": "1234", "GITHUB_RUN_ATTEMPT": "1"}
        args = self.arguments("windows-pin") + ["--directory", str(directory)]
        with mock.patch.dict(os.environ, env):
            for key in (None, "status", "commit", "version", "run_id", "workflow_sha"):
                value = dict(receipt)
                if key: value[key] = "wrong"
                raw = json.dumps(value).encode(); (directory / "candidate.json").write_bytes(raw)
                if key:
                    with self.subTest(key=key), self.assertRaises(self.mac.PhaseError): workflow.main(args)
                else:
                    workflow.main(args)
                    self.assertEqual(self.output[-1], {"windows_receipt_sha256": self.mac.sha(raw)})

    def test_pending_checkpoint_then_accepted_resume_uses_one_submission_and_original_bytes(self):
        self.fixture.signer()
        files = self.mac.inventory(self.fixture.signed, {p.name for p in self.fixture.signed.iterdir()})
        def pending(argv, home, **kwargs):
            if "notarytool" in argv and argv[2] == "info":
                return 0, json.dumps({"id": fixtures.SUBMISSION, "status": "In Progress"}).encode()
            return self.fixture.fake_command(argv, home, **kwargs)
        self.assertEqual(self.invoke_notary("notary-submit", pending), 0)
        checkpoint = self.output[-1]
        self.assertEqual(checkpoint["checkpoint_status"], "pending")
        original = (Path(checkpoint["state"]) / "notary.json").read_bytes()
        self.assertEqual(self.invoke_notary("notary-resume", self.fixture.fake_command, checkpoint, 2), 0)
        accepted = self.output[-1]
        self.mac.validate_notary(Path(accepted["state"]), accepted["notary_receipt_sha256"], SELECTED,
                                 self.fixture.signed_pin, files[self.mac.zip_name(SELECTED["version"])]["sha256"])
        self.assertEqual(sum("notarytool" in call and call[2] == "submit" for call in self.fixture.calls), 1)
        self.assertEqual((Path(checkpoint["state"]) / "notary.json").read_bytes(), original)
        self.assertEqual(self.mac.inventory(self.fixture.signed, set(files)), files)

    def test_pending_resume_fails_acceptance_but_retains_new_pin_without_resubmission(self):
        self.fixture.signer()
        def pending(argv, home, **kwargs):
            if "notarytool" in argv and argv[2] == "info":
                return 0, json.dumps({"id": fixtures.SUBMISSION, "status": "In Progress"}).encode()
            return self.fixture.fake_command(argv, home, **kwargs)
        self.invoke_notary("notary-submit", pending)
        checkpoint = self.output[-1]
        with self.assertRaisesRegex(self.mac.PhaseError, "not accepted"):
            self.invoke_notary("notary-resume", pending, checkpoint, 2)
        self.assertTrue((Path(self.output[-1]["state"]) / "notary.json").is_file())
        self.assertEqual(sum("notarytool" in call and call[2] == "submit" for call in self.fixture.calls), 1)

    def test_lost_submission_response_preserves_ambiguous_intent_and_resume_cannot_upload(self):
        self.fixture.signer()
        def lost(argv, home, **kwargs):
            if "notarytool" in argv and argv[2] == "submit":
                self.fixture.calls.append(list(map(str, argv)))
                raise OSError("private simulated secret response must not be logged")
            return self.fixture.fake_command(argv, home, **kwargs)
        self.invoke_notary("notary-submit", lost)
        checkpoint = self.output[-1]
        self.assertEqual(checkpoint["checkpoint_status"], "submitting")
        count = len(self.fixture.calls)
        with self.assertRaisesRegex(self.mac.PhaseError, "ambiguous"):
            self.invoke_notary("notary-resume", self.fixture.fake_command, checkpoint, 2)
        self.assertEqual(len(self.fixture.calls), count)

    def test_rejected_checkpoint_is_not_acceptance_and_never_resubmits(self):
        self.fixture.signer()
        def invalid(argv, home, **kwargs):
            if "notarytool" in argv and argv[2] == "info":
                return 0, json.dumps({"id": fixtures.SUBMISSION, "status": "Invalid"}).encode()
            return self.fixture.fake_command(argv, home, **kwargs)
        self.invoke_notary("notary-submit", invalid)
        checkpoint = self.output[-1]
        self.assertEqual(checkpoint["checkpoint_status"], "Invalid")
        count = len(self.fixture.calls)
        with self.assertRaisesRegex(self.mac.PhaseError, "rejected"):
            self.invoke_notary("notary-resume", self.fixture.fake_command, checkpoint, 2)
        self.assertEqual(len(self.fixture.calls), count)

    def test_checkpoint_rejects_rebound_zip_identity_and_unexpected_file(self):
        self.fixture.signer(); self.fixture.notary()
        raw = (self.fixture.state / "notary.json").read_bytes()
        for key in ("zip_sha256", "signed_receipt_sha256", "identity"):
            value = json.loads(raw); value[key] = "wrong"
            (self.fixture.state / "notary.json").write_bytes(json.dumps(value).encode())
            with self.subTest(key=key), self.assertRaises(self.mac.PhaseError):
                workflow.checkpoint(self.fixture.signed, self.fixture.state, SELECTED, self.fixture.signed_pin)
        (self.fixture.state / "notary.json").write_bytes(raw)
        (self.fixture.state / "not-a-checkpoint").write_bytes(b"unapproved")
        with self.assertRaisesRegex(self.mac.PhaseError, "unexpected"):
            workflow.checkpoint(self.fixture.signed, self.fixture.state, SELECTED, self.fixture.signed_pin)

    def test_changed_or_symlink_checkpoint_cannot_be_promoted(self):
        self.fixture.signer(); result = self.fixture.notary()
        destination = self.fixture.root / "resume"
        with self.assertRaisesRegex(self.mac.PhaseError, "differs"):
            workflow.copy_checkpoint(self.fixture.state, destination, "0" * 64)
        self.assertFalse(destination.exists())
        if os.name == "posix":
            receipt = self.fixture.state / "notary.json"
            original = self.fixture.root / "original.json"; receipt.rename(original); receipt.symlink_to(original)
            with self.assertRaises(ValueError):
                workflow.copy_checkpoint(self.fixture.state, destination, result["notary_receipt_sha256"])
            self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
