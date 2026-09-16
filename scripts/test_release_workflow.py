#!/usr/bin/env python3
"""Offline orchestration fixtures; Apple tools and network calls are mocked."""
import importlib.util
import io
import json
import os
from pathlib import Path
import tempfile
import textwrap
from types import SimpleNamespace
import unittest
import urllib.request
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



class ChannelWorkflow(unittest.TestCase):
    """Execute the actual inline job adapter with bounded public-data fixtures."""
    def setUp(self):
        self.text = (Path(__file__).resolve().parents[1] / '.github/workflows/release.yml').read_text()
        self.job = self.text.split('  prepare-channel:\n', 1)[1].split('\n  prepare-recipes:', 1)[0]
        inline = self.job.split("          python3 - <<'PY'\n", 1)[1].split('\n          PY', 1)[0]
        self.code = compile(textwrap.dedent(inline), '<prepare-channel workflow>', 'exec')
        parent = Path.home() / '.cache/flere/tmp'
        parent.mkdir(parents=True, exist_ok=True)
        temp = tempfile.TemporaryDirectory(prefix='channel-workflow-', dir=parent)
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        (self.root / 'candidate').mkdir()
        self.release = fixtures.mac.release
        self.head = 'd' * 40
        self.api = 'https://api.github.com/repos/robert-cronin/flere/git/ref/heads/main'
        self.url = f'https://raw.githubusercontent.com/robert-cronin/flere/{self.head}/packaging/channels/stable.json'
        self.previous = b'{"schema_version":1,"targets":{}}\n'
        self.responses = {self.api: json.dumps({'ref': 'refs/heads/main', 'object': {'type': 'commit', 'sha': self.head}}).encode(),
                          self.url: self.previous}
        self.calls = []
        self.prepared = []
        self.adapter = SimpleNamespace(release=self.release, prepare=mock.Mock(side_effect=self.prepare))
        self.env = dict(HOME=str(self.root), GITHUB_RUN_ID='1234', GITHUB_RUN_ATTEMPT='2',
                        RELEASE_VERSION='0.4.0', RELEASE_COMMIT='a' * 40, RELEASE_WORKFLOW_SHA='b' * 40,
                        GITHUB_OUTPUT=str(self.root / 'outputs'), GITHUB_STEP_SUMMARY=str(self.root / 'summary'))

    def prepare(self, directory, previous, previous_sha, output, version, commit, run, workflow_sha, descriptor_sha):
        self.prepared.append((directory, previous.read_bytes(), previous_sha, version, commit, run, workflow_sha, descriptor_sha))
        output.mkdir()
        receipt = {'status': 'prepared_not_published', 'previous_sha256': previous_sha}
        (output / 'stable.json').write_bytes(b'prepared channel')
        (output / 'promotion.json').write_bytes(self.release.json_bytes(receipt))
        return receipt

    def execute(self, schema=3, *, wrong_pin=False):
        raw = self.release.json_bytes({'schema_version': schema})
        (self.root / 'candidate/release.json').write_bytes(raw)
        env = dict(self.env, RELEASE_DESCRIPTOR_SHA='0' * 64 if wrong_pin else self.release.sha(raw))
        def fetch(url, *, timeout):
            self.calls.append((url, timeout))
            return io.BytesIO(self.responses[url])
        opener = mock.Mock(open=mock.Mock(side_effect=fetch))
        def make_opener(handler):
            self.assertIsInstance(handler, self.release.NoRedirect)
            return opener
        before = Path.cwd()
        try:
            os.chdir(self.root)
            with mock.patch.dict(os.environ, env), \
                    mock.patch('importlib.util.spec_from_file_location', return_value=mock.Mock()), \
                    mock.patch('importlib.util.module_from_spec', return_value=self.adapter), \
                    mock.patch.object(urllib.request, 'build_opener', side_effect=make_opener):
                exec(self.code, {})
        finally:
            os.chdir(before)

    def test_current_main_is_data_only_and_exact_preimage_reaches_generator_and_artifact(self):
        self.execute()
        self.assertEqual(self.calls, [(self.api, 30), (self.url, 30)])
        self.assertEqual(self.prepared, [(Path('candidate'), self.previous, self.release.sha(self.previous),
            '0.4.0', 'a' * 40, '1234', 'b' * 40, self.release.sha(self.release.json_bytes({'schema_version': 3})))])
        output = self.root / '.cache/flere/tmp/release-channel-1234-2/promotion'
        self.assertEqual({p.name for p in output.iterdir()}, {'stable.json', 'promotion.json', 'main-preimage.json'})
        self.assertEqual(json.loads((output / 'main-preimage.json').read_bytes()), {
            'schema_version': 1, 'repository': 'robert-cronin/flere', 'ref': 'refs/heads/main',
            'commit': self.head, 'path': 'packaging/channels/stable.json', 'url': self.url,
            'bytes': len(self.previous), 'sha256': self.release.sha(self.previous)})
        self.assertEqual((self.root / 'outputs').read_text(), f'directory={output}\n')

    def test_historical_linux_profiles_skip_explicitly_without_channel_reads_or_artifact(self):
        for schema in (1, 2):
            with self.subTest(schema=schema): self.execute(schema)
        self.assertEqual(self.calls, [])
        self.adapter.prepare.assert_not_called()
        self.assertIn('historical Linux-only', (self.root / 'summary').read_text())
        self.assertFalse((self.root / 'outputs').exists())

    def test_unknown_schema_and_descriptor_mismatch_fail_before_public_data_read(self):
        for schema, wrong in ((5, False), (True, False), (3, True)):
            with self.subTest(schema=schema, wrong_pin=wrong), self.assertRaises(ValueError):
                self.execute(schema, wrong_pin=wrong)
        self.assertEqual(self.calls, [])
        self.adapter.prepare.assert_not_called()

    def test_invalid_main_identity_or_oversized_responses_never_reach_generator(self):
        valid = self.responses[self.api]
        for raw in (b'x' * 16385,
                    json.dumps({'ref': 'refs/heads/other', 'object': {'type': 'commit', 'sha': self.head}}).encode(),
                    json.dumps({'ref': 'refs/heads/main', 'object': {'type': 'tag', 'sha': self.head}}).encode(),
                    json.dumps({'ref': 'refs/heads/main', 'object': {'type': 'commit', 'sha': '../other'}}).encode()):
            self.responses[self.api] = raw
            with self.subTest(raw=raw[:100]), self.assertRaises(ValueError): self.execute()
        self.responses[self.api] = valid
        for raw in (b'', b'x' * 8193):
            self.responses[self.url] = raw
            with self.subTest(size=len(raw)), self.assertRaises(ValueError): self.execute()
        self.adapter.prepare.assert_not_called()
        self.assertFalse((self.root / 'outputs').exists())

    def test_generator_refusal_cannot_advertise_a_publishable_artifact(self):
        self.adapter.prepare.side_effect = ValueError('channel cannot downgrade')
        with self.assertRaisesRegex(ValueError, 'cannot downgrade'): self.execute(4)
        self.assertFalse((self.root / 'outputs').exists())
        self.assertFalse((self.root / 'summary').exists())

    def test_job_has_exact_read_only_dependencies_and_three_file_upload(self):
        self.assertIn('needs: [select, aggregate, verify-public]', self.job)
        self.assertIn('ref: ${{ github.workflow_sha }}', self.job)
        self.assertIn('artifact-ids: ${{ needs.aggregate.outputs.artifact_id }}', self.job)
        self.assertIn("if: steps.channel.outputs.directory != ''", self.job)
        self.assertIn('persist-credentials: false', self.job)
        for forbidden in ('contents: write', 'id-token:', 'environment: release', 'secrets.', 'GH_TOKEN', 'git push', 'git commit'):
            self.assertNotIn(forbidden, self.job)
        prefix = '${{ steps.channel.outputs.directory }}/'
        self.assertEqual({line.strip().removeprefix(prefix) for line in self.job.splitlines() if line.strip().startswith(prefix)},
                         {'stable.json', 'promotion.json', 'main-preimage.json'})


if __name__ == "__main__":
    unittest.main()
