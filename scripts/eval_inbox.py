#!/usr/bin/env python3
"""Measure native-scoped inbox reads using copied Python and exact retained mail."""
import argparse
import json
from pathlib import Path
import shutil
import time
import uuid

from eval_activity import STANDIN, snapshot_tabs
from eval_coordination import Agent
from eval_runtime import resources
from eval_support import Fixture, distribution, encode, save_report


def run(binary, descriptors, calls, repeats):
    if not Path('/proc/self/stat').exists():
        raise RuntimeError('this native-ownership probe requires Linux /proc')
    with Fixture(binary) as fixture:
        executable = fixture.root / 'codex'
        shutil.copyfile('/usr/bin/python3', executable)
        executable.chmod(0o700)
        program = fixture.root / 'standin.py'
        program.write_text(STANDIN)
        (fixture.state / 'harnesses.json').write_bytes(encode([{'name': 'codex', 'command': [
            str(executable), str(program), str(descriptors), 'busy', '.5']}]))
        sender = Agent(fixture, 'inbox sender')
        actor = Agent(fixture, 'inbox recipient')
        actor.call('set_focus', {'seconds': 1800, 'reason': 'Synthetic scoped-inbox evaluation.'})
        expected = []
        for number in range(2):
            body = f'Exact synthetic evidence {number}: preserve conversation scope. 界'
            sent = sender.call('send_chat_message', {'workspace': actor.workspace,
                'session': actor.tab['id'], 'run': actor.tab['run'],
                'request_id': f'inbox-evaluation-{number}', 'body': body})['message']
            expected.append(sent['id'])
            assert actor.call('inbox')['messages'][-1]['body'] == body
            if number == 0:
                actor.call('inbox', {'ack_ids': [sent['id']]})
        # Exercise notice checks with DND off: surfaced mail is still unhandled,
        # but must not produce another notice or require proof just to send none.
        actor.call('set_focus', {'seconds': 0})
        # Both bodies are surfaced; one remains unhandled. Native observation must
        # settle before timed reads so store changes cannot hide in setup noise.
        def verify_identity():
            epoch, tabs = snapshot_tabs(fixture)
            assert set(tabs) == {sender.tab['id'], actor.tab['id']}
            for agent in (sender, actor):
                tab = tabs[agent.tab['id']]
                assert tab['alive'] and tab['run'] == agent.tab['run'] and tab['pid'] == agent.tab['pid']
                assert tab['workspace'] == agent.workspace
                assert any(c['uuid'] == str(uuid.UUID(hex=agent.tab['run']))
                           for c in tab['conversations']), 'native conversation not recognized'
            return epoch
        deadline = time.monotonic() + 8
        while True:
            try:
                epoch = verify_identity()
                break
            except AssertionError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.05)
        time.sleep(1.1)
        store = fixture.state / 'workspaces.v2.json'
        original = store.read_bytes()
        records = json.loads(original)['coordination']['messages']
        assert [m['id'] for m in records] == expected
        assert records[0]['acknowledged'] is not None and records[1]['acknowledged'] is None
        assert all(m['chat']['conversation'] == str(uuid.UUID(hex=actor.tab['run']))
                   and m['chat']['sender']['conversation'] == str(uuid.UUID(hex=sender.tab['run']))
                   for m in records), 'fixture did not establish bound-message provenance'
        inventory = actor.call('list_workspaces', {'workspace': actor.workspace})
        assert len(inventory['workspaces']) == 1
        card = inventory['workspaces'][0]
        assert card['id'] == actor.workspace and len(card['tabs']) == 1
        assert (card['tabs'][0]['id'], card['tabs'][0]['run'], card['tabs'][0]['pid']) == (
            actor.tab['id'], actor.tab['run'], actor.tab['pid'])
        cases = []
        for repeat in range(repeats):
            operations = [('pending', 'inbox', {}),
                          ('history', 'inbox', {'include_acknowledged': True}),
                          ('status_control', 'message_status', {'id': expected[0]}),
                          ('checkpoints_no_notice', 'checkpoints', {}),
                          ('cards_no_notice', 'list_workspaces', {'workspace': actor.workspace})]
            if repeat % 2:
                operations.reverse()
            for label, operation, args in operations:
                actor.metrics.clear()
                before = store.stat()
                cpu_before, _ = resources(fixture.server.pid)
                begin = time.monotonic()
                for _ in range(calls):
                    value = actor.call(operation, args)
                    if operation == 'inbox':
                        wanted = records if label == 'history' else records[1:]
                        assert value['messages'] == wanted, 'inbox changed exact records or their scope'
                        assert value['pending'] == 1 and value['acknowledged'] == []
                        assert value['next_after'] is None
                    elif operation == 'message_status':
                        message = value['message']
                        assert message['id'] == expected[0] and 'body' not in message
                        assert message['acknowledged'] == records[0]['acknowledged']
                    elif operation == 'checkpoints':
                        assert 'mailbox_notice' not in value, 'repeated notice for already surfaced mail'
                        assert value == {'checkpoints': [], 'total': 0, 'remaining': 0, 'next_after': None}
                    else:
                        assert 'mailbox_notice' not in value, 'repeated notice for already surfaced mail'
                        assert value == inventory, 'card inventory changed during read'
                elapsed = time.monotonic() - begin
                cpu_after, rss = resources(fixture.server.pid)
                after = store.stat()
                assert (before.st_ino, before.st_mtime_ns) == (after.st_ino, after.st_mtime_ns), 'unchanged reads rewrote history'
                assert verify_identity() == epoch, 'read changed native identity'
                cases.append({'operation': label, 'repeat': repeat, 'calls': calls,
                    'seconds': elapsed, 'latency_including_mcp_startup': distribution([m['ms'] for m in actor.metrics]),
                    'supervisor_cpu_seconds': cpu_after - cpu_before,
                    'sampled_supervisor_rss_bytes': rss,
                    'body_bytes': sum(m['body_bytes'] for m in actor.metrics),
                    'mcp_response_bytes': sum(m['mcp_response_bytes'] for m in actor.metrics),
                    'store_replacements': 0})
        assert store.read_bytes() == original, 'durable evidence changed'
        return {'schema': 1, 'provenance': fixture.provenance(), 'fixture': {
            'native_standins': 2, 'extra_descriptors_each': descriptors,
            'acknowledged_messages': 1, 'pending_messages': 1, 'notice_candidates': 0, 'focus': 'off'}, 'cases': cases,
            'method': 'Sequential real stdio MCP calls against owned copied-Python native stand-ins. '
                      'Native sender/recipient provenance, complete returned records and no duplicate notices verified. '
                      'Each repeat reverses operation order; compare identical configurations across binaries. '
                      'No cache of authorization identities, native models, live chats, network or visible-input measurement. '
                      'Timing includes MCP process startup; CPU/RSS describe only the supervisor.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--descriptors', type=int, default=128)
    parser.add_argument('--calls', type=int, default=40)
    parser.add_argument('--repeats', type=int, default=3)
    args = parser.parse_args()
    if not (0 <= args.descriptors <= 192 and 1 <= args.calls <= 500 and 1 <= args.repeats <= 10):
        parser.error('use descriptors 0..192, calls 1..500, repeats 1..10')
    result = run(args.binary, args.descriptors, args.calls, args.repeats)
    save_report(args.output, result)
    for case in result['cases']:
        print(f"{case['operation']} #{case['repeat']}: "
              f"p50={case['latency_including_mcp_startup']['p50_ms']:.3f}ms "
              f"p95={case['latency_including_mcp_startup']['p95_ms']:.3f}ms", flush=True)
