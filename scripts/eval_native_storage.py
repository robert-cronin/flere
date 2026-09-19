#!/usr/bin/env python3
"""Native-scoped capacity evidence using owned Python stand-ins, never models."""
import argparse
import copy
import json
from pathlib import Path
import shutil
import time
import uuid

from eval_activity import STANDIN, snapshot_tabs
from eval_coordination import Agent
from eval_storage import CAPACITY_ERROR, STORE_LIMIT, human, saved, seed_history, start, stop
from eval_support import Fixture, encode, save_report


def run_case(binary, headroom, notified):
    with Fixture(binary) as fixture:
        history_card = fixture.card('retained storage fixture')
        legacy = human(fixture, history_card, 'send_message', {
            'to': 'user', 'body': 'Synthetic retained evidence.'})['message']
        legacy.update(id=f'{1 << 120:032x}', acknowledged=1700000000, surfaced=1700000000)
        stop(fixture)
        document = seed_history(saved(fixture), legacy, STORE_LIMIT - 60000)
        store = fixture.state / 'workspaces.v2.json'
        store.write_bytes(encode(document))
        start(fixture)
        fixture.request('save-tabs')
        executable = fixture.root / 'codex'
        shutil.copyfile('/usr/bin/python3', executable)
        executable.chmod(0o700)
        program = fixture.root / 'standin.py'
        program.write_text(STANDIN)
        (fixture.state / 'harnesses.json').write_bytes(encode([{'name': 'codex', 'command': [
            str(executable), str(program), '0', 'busy', '2']}]))
        sender = Agent(fixture, 'capacity sender')
        actor = Agent(fixture, 'capacity recipient')
        actor.call('set_focus', {'seconds': 1800, 'reason': 'Synthetic capacity probe.'})
        def send(number):
            args = {'workspace': actor.workspace, 'session': actor.tab['id'], 'run': actor.tab['run'],
                    'request_id': f'synthetic-capacity-{number}',
                    'body': f'Synthetic correction {number}: preserve exact evidence and local-only scope. 界'}
            reply = sender.call('send_chat_message', args)
            return args, reply['message']['id']
        handled_args, handled = send(0)
        first_message = actor.call('inbox')['messages'][0]
        assert first_message['id'] == handled and first_message['body'] == handled_args['body']
        actor.call('inbox', {'ack_ids': [handled]})
        retry_args, pending_id = send(1)
        if notified:
            actor.call('set_focus', {'seconds': 0})
            notice = actor.call('checkpoints')
            assert pending_id in json.dumps(notice.get('mailbox_notice')), 'fixture did not receive its notice'
            actor.call('set_focus', {'seconds': 1800, 'reason': 'Synthetic capacity probe.'})
        # Let directory/native observation and any one-time deferred receipt settle.
        time.sleep(1.2)
        epoch, tabs = snapshot_tabs(fixture)
        for agent in (sender, actor):
            tab = tabs[agent.tab['id']]
            assert (tab['workspace'], tab['run'], tab['pid'], tab['alive']) == (
                agent.workspace, agent.tab['run'], agent.tab['pid'], True)
            assert any(c['uuid'] == str(uuid.UUID(hex=agent.tab['run'])) for c in tab['conversations'])
        needed = STORE_LIMIT - headroom - store.stat().st_size
        assert 0 < needed < 60000, 'fixture padding exceeds bounded metadata request'
        fixture.metadata(history_card, 'p' * needed)
        assert store.stat().st_size == STORE_LIMIT - headroom, 'fixture did not reach requested capacity'
        expected = saved(fixture)['coordination']
        records = {m['id']: m for m in expected['messages']}
        pending = records[pending_id]
        assert pending['body'] == retry_args['body'] and pending['acknowledged'] is None
        assert (pending['native_surfaced'] is not None) == notified
        if notified:
            assert pending['delivery']['outcome'] == 'mcp-returned'
        assert pending['chat']['conversation'] == str(uuid.UUID(hex=actor.tab['run']))
        assert pending['chat']['sender']['conversation'] == str(uuid.UUID(hex=sender.tab['run']))
        assert pending['chat']['initial_session'] == actor.tab['id'] and pending['chat']['initial_run'] == actor.tab['run']
        inventory = fixture.json('list')
        cases = []

        def verify():
            # Human inspection is an independent read-only oracle for both exact
            # bodies, even when recording native surfacing cannot fit in storage.
            page = human(fixture, actor.workspace, 'inbox', {
                'include_acknowledged': True, 'after': legacy['id'], 'limit': 8})
            assert page['messages'] == [records[handled], records[pending_id]], 'native records differ from exact oracle'
            assert saved(fixture)['coordination'] == expected, 'capacity mutation changed retained evidence'
            assert fixture.json('list') == inventory, 'capacity operation changed native identities or metadata'
            current_epoch, current_tabs = snapshot_tabs(fixture)
            assert current_epoch == epoch
            for agent in (sender, actor):
                tab = current_tabs[agent.tab['id']]
                assert (tab['run'], tab['pid'], tab['alive']) == (agent.tab['run'], agent.tab['pid'], True)

        for label, agent, operation, args in [
            ('context', actor, 'get_context', {}),
            ('deduplicated_retry', sender, 'send_chat_message', retry_args),
            ('first_body', actor, 'inbox', {}),
            ('body_retry', actor, 'inbox', {}),
            ('ack', actor, 'inbox', {'ack_ids': [pending_id]}),
            ('ack_retry', actor, 'inbox', {'ack_ids': [pending_id]}),
            ('handled_history', actor, 'inbox', {'include_acknowledged': True, 'limit': 1}),
        ]:
            if label == 'handled_history':
                agent.forget_context()  # Recovery supplies no remembered message ID.
            before, inode = store.read_bytes(), store.stat().st_ino
            begin = time.monotonic()
            try:
                reply = agent.call(operation, args)
            except RuntimeError as error:
                assert str(error) == CAPACITY_ERROR, f'unexpected native capacity failure: {error}'
                accepted = False
                assert (store.read_bytes(), store.stat().st_ino) == (before, inode), 'failed native mutation changed store'
            else:
                accepted = True
                if label == 'context':
                    assert reply['pending_messages'] == 1 and [m['id'] for m in reply['messages']] == [pending_id]
                    assert 'body' not in reply['messages'][0], 'compact context injected the body'
                elif label == 'deduplicated_retry':
                    assert reply['message']['id'] == pending_id, 'request retry created a different message'
                elif label in ('first_body', 'body_retry'):
                    message, = reply['messages']
                    for field in ('surfaced', 'native_surfaced'):
                        assert isinstance(message[field], int)
                        if pending[field] is None:
                            pending[field] = message[field]
                    assert message == pending, 'native body/provenance differs from exact oracle'
                elif label in ('ack', 'ack_retry'):
                    assert reply['acknowledged'] == [pending_id] and reply['pending'] == 0
                    stamp = next(m for m in saved(fixture)['coordination']['messages'] if m['id'] == pending_id)['acknowledged']
                    assert isinstance(stamp, int), 'native ACK receipt lacks durable acknowledgement'
                    if pending['acknowledged'] is None:
                        pending['acknowledged'] = stamp
                    assert pending['acknowledged'] == stamp, 'ACK retry changed the handled timestamp'
                else:
                    assert reply['messages'] == [records[handled]], 'lost-ID history did not recover handled record'
                    assert reply['next_after'] == handled and reply['remaining'] == 1
            elapsed = (time.monotonic() - begin) * 1000
            verify()
            growth = store.stat().st_size - len(before)
            if label in ('context', 'deduplicated_retry', 'body_retry', 'ack_retry', 'handled_history'):
                assert (store.read_bytes(), store.stat().st_ino) == (before, inode), 'read/retry rewrote unchanged state'
            cases.append({'operation': label, 'accepted': accepted, 'ms': elapsed,
                          'error': None if accepted else CAPACITY_ERROR, 'store_growth_bytes': growth})
        original = store.read_bytes()
        stop(fixture)
        start(fixture)
        assert all(not w['tabs'] for w in fixture.json('list')['workspaces']), 'cold restart launched a native chat'
        assert saved(fixture)['coordination'] == expected and store.read_bytes() == original
        cold_page = human(fixture, actor.workspace, 'inbox', {
            'include_acknowledged': True, 'after': legacy['id'], 'limit': 8})
        assert cold_page['messages'] == [records[handled], records[pending_id]]
        return {'configured_headroom_bytes': headroom, 'notice_before_capacity': notified,
                'initial_store_bytes': STORE_LIMIT - headroom, 'native_standins': 2,
                'retained_messages': len(expected['messages']), 'cases': cases,
                'exact_records_identities_and_retry_ids_preserved': True,
                'cold_restart_preserved_without_launch': True, 'provenance': fixture.provenance()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    args = parser.parse_args()
    results = []
    for headroom in (4096, 0):
        for notified in (False, True):
            case = run_case(args.binary, headroom, notified)
            results.append(case)
            print(f'headroom={headroom} notice={notified}: ' +
                  str([(c['operation'], c['accepted']) for c in case['cases']]), flush=True)
    save_report(args.output, {'schema': 1, 'cases': results,
        'method': 'Owned copied-Python stand-ins with exact native conversation provenance. '
                  'DND stays on during measured calls; a separate case receives an MCP notice before filling. '
                  'Existing current fields record that notice as native_surfaced; this is not proof of a body read. '
                  'Checks compact context, idempotent send, body retrieval/retry, durable ACK/retry, lost-ID '
                  'handled-history discovery, exact original records, identity and cold restart without launching tabs. '
                  'No native model, user chat, network or live migration; timings are single samples.'})
