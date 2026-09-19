#!/usr/bin/env python3
"""Offline negative controls for the model evaluator; never launches a model."""
import copy
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

import eval_agent_context as probe


def call(name, args, value):
    return {'request': {'params': {'name': name, 'arguments': args}},
        'response': {'result': {'content': [{'type': 'text', 'text': json.dumps(value)}]}}}


class ReceiptControls(unittest.TestCase):
    def setUp(self):
        self.old = {'pending': {'id': 'pending', 'from': 1, 'to': 2,
            'body': 'Exact synthetic correction: limit = 5.', 'intent': 'quiet', 'saved': 1,
            'acknowledged': None},
            'history': {'id': 'history', 'from': 1, 'to': 2, 'body': 'anchor-41',
                'intent': 'quiet', 'saved': 0, 'acknowledged': 1}}
        self.new = copy.deepcopy(self.old)
        self.new['pending']['acknowledged'] = 2
        self.calls = [call('inbox', {'include_acknowledged': True}, {'messages': list(self.old.values())}),
            call('inbox', {'ack_ids': ['pending']}, {'acknowledged': ['pending']})]

    def check(self):
        return probe.receipt_checks(self.calls, self.old, self.new, ['pending'], 'history')

    def test_valid_evidence(self):
        self.assertTrue(all(self.check().values()))

    def test_false_ack_without_write(self):
        self.new['pending']['acknowledged'] = None
        self.assertFalse(self.check()['durable_acknowledgements'])

    def test_ack_before_body(self):
        self.calls.reverse()
        self.assertFalse(self.check()['bodies_read_before_ack'])

    def test_summary_is_not_a_body(self):
        self.calls[0] = call('get_context', {}, {'messages': [{'id': mid, 'summary': True} for mid in self.old]})
        self.assertFalse(self.check()['bodies_read_before_ack'])
        self.assertFalse(self.check()['history_untouched'])

    def test_altered_body_or_provenance(self):
        for key, replacement in [('body', 'incorrect'), ('from', 999)]:
            with self.subTest(key=key):
                messages = copy.deepcopy(list(self.old.values()))
                messages[0][key] = replacement
                self.calls[0] = call('inbox', {}, {'messages': messages})
                self.assertFalse(self.check()['exact_read_bodies'])

    def test_duplicate_or_false_receipts(self):
        self.calls.append(copy.deepcopy(self.calls[-1]))
        self.assertFalse(self.check()['exact_acknowledgements'])
        self.calls.pop()
        self.calls[-1] = call('inbox', {'ack_ids': ['pending']}, {'acknowledged': []})
        self.assertFalse(self.check()['ack_receipts_match'])

    def test_retained_history_cannot_be_reacknowledged(self):
        self.calls.append(call('inbox', {'ack_ids': ['history']}, {'acknowledged': ['history']}))
        self.assertFalse(self.check()['exact_acknowledgements'])
        self.new['history']['acknowledged'] = 3
        self.assertFalse(self.check()['history_untouched'])

    def test_added_or_changed_originals(self):
        self.new['pending']['body'] = 'replacement'
        self.assertFalse(self.check()['exact_originals'])
        self.new = copy.deepcopy(self.old)
        self.new['pending']['extra_original'] = 'unexpected'
        self.assertFalse(self.check()['exact_originals'])


class ProcessControls(unittest.TestCase):
    def setUp(self):
        parent = Path.home() / '.cache/flere/evals'
        parent.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='model-controls-', dir=parent)
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def run_child(self, program, timeout=1, budget=24):
        return probe.run_native([sys.executable, '-c', program], dict(os.environ), 'synthetic',
            self.root / 'events.jsonl', self.root / 'stderr.log', timeout, budget)

    def test_normal_exit(self):
        result = self.run_child("print('{\"type\":\"turn.completed\"}')")
        self.assertEqual(result[:3], (0, False, False))

    def test_timeout(self):
        started = time.monotonic()
        _, timed_out, _, _ = self.run_child('import time; time.sleep(30)', timeout=.1)
        self.assertTrue(timed_out)
        self.assertLess(time.monotonic() - started, 4)

    def test_scope_violation_stops_child(self):
        event = {'type': 'item.started', 'item': {'id': 'bad', 'type': 'command_execution'}}
        _, timed_out, violation, _ = self.run_child(
            f'import time; print({json.dumps(event)!r}, flush=True); time.sleep(30)')
        self.assertTrue(violation)
        self.assertFalse(timed_out)

    def test_completed_and_started_are_one_call(self):
        item = {'id': 'owned', 'type': 'mcp_tool_call', 'server': 'flere_eval', 'tool': 'inbox'}
        events = [{'type': 'item.started', 'item': item}, {'type': 'item.completed', 'item': item}]
        self.assertTrue(probe.allowed_native_tools(events, 1))
        events.append({'type': 'item.started', 'item': dict(item, id='extra')})
        self.assertFalse(probe.allowed_native_tools(events, 1))
        self.assertFalse(probe.allowed_native_tools([{'item': dict(item, server='unrelated')}], 24))

    def test_unknown_native_tool_is_rejected(self):
        self.assertFalse(probe.allowed_native_tools([{'item': {'id': 'unknown', 'type': 'new_tool'}}], 24))

    def test_excess_native_calls_stop_child(self):
        events = [{'type': 'item.started', 'item': {'id': str(n), 'type': 'mcp_tool_call',
            'server': 'flere_eval', 'tool': 'inbox'}} for n in range(25)]
        output = ''.join(json.dumps(e) + '\n' for e in events)
        _, timed_out, violation, _ = self.run_child(
            f'import time; print({output!r}, flush=True); time.sleep(30)')
        self.assertTrue(violation)
        self.assertFalse(timed_out)

    def test_owned_descendant_stops_with_parent(self):
        marker = self.root / 'should-not-exist'
        child = f'import time; from pathlib import Path; time.sleep(.5); Path({str(marker)!r}).touch()'
        program = f'import subprocess, sys; subprocess.Popen([sys.executable, "-c", {child!r}])'
        self.assertEqual(self.run_child(program)[:3], (0, False, False))
        time.sleep(.6)
        self.assertFalse(marker.exists())

    def test_proxy_budget_survives_restart(self):
        trace = self.root / 'trace.jsonl'
        trace.write_text('{}\n' * 24)
        settings = self.root / 'settings.json'
        settings.write_text(json.dumps({'trace': str(trace), 'tool_budget': 24}))
        request = {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
            'params': {'name': 'inbox', 'arguments': {}}}
        result = subprocess.run([sys.executable, str(Path(probe.__file__)), '--proxy', str(settings)],
            input=json.dumps(request), text=True, capture_output=True, check=True, timeout=5)
        self.assertTrue(json.loads(result.stdout)['result']['isError'])
        self.assertEqual(len(trace.read_text().splitlines()), 25)


if __name__ == '__main__':
    unittest.main()
