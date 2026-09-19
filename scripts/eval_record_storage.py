#!/usr/bin/env python3
"""Measure indexed storage with growing synthetic history; never launch a model.

Offline growth edits only stopped disposable databases. Measured requests use
real supervisor sockets. RSS/write_bytes are Linux process measurements, not
Windows/SSH typing latency or model token/compaction measurements.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sqlite3
import time

from eval_storage import STORE_LIMIT, human, saved, seed_history, start, stop
from eval_support import Fixture, distribution, encode, save_report


def memory(pid):
    values = {}
    for line in Path(f'/proc/{pid}/status').read_text().splitlines():
        if line.startswith(('VmRSS:', 'VmHWM:')):
            key, value = line.split(':', 1)
            values[key] = int(value.split()[0]) * 1024
    return values


def write_bytes(pid):
    return int(dict(line.split(': ', 1) for line in
                    Path(f'/proc/{pid}/io').read_text().splitlines())['write_bytes'])


def database(fixture):
    marker = saved(fixture)
    assert marker['version'] == 11
    name = marker['records']
    assert len(name) == 40 and name.startswith('records-') and all(
        c in '0123456789abcdef' for c in name[8:])
    return fixture.state / name / 'state.sqlite'


def grow(db, workspace, count):
    # This oracle deliberately bypasses the API only while the owned supervisor
    # is stopped; API/native lifecycle behavior has separate integration tests.
    with sqlite3.connect(db) as connection:
        existing, = connection.execute('SELECT count(*) FROM messages').fetchone()
        for number in range(existing, count):
            identity = f'{(1 << 124) + number:032x}'
            original = {'id': identity, 'from': 0, 'to': workspace,
                        'body': f'Synthetic retained record {number}: ' + 's' * 16000,
                        'intent': 'quiet', 'saved': 1}
            lifecycle = {'surfaced': 1, 'native_surfaced': 1, 'acknowledged': 1,
                         'delivery': None}
            connection.execute('INSERT INTO messages(id,recipient,sender,original) VALUES(?,?,?,?)',
                               (identity, f'{workspace:016x}', '0' * 16, encode(original).decode()))
            connection.execute('INSERT INTO lifecycle(id,value) VALUES(?,?)',
                               (identity, encode(lifecycle).decode()))
        connection.commit()
        connection.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchall()
        total, size = connection.execute('SELECT count(*),sum(length(cast(original AS blob))) FROM messages').fetchone()
    return total, size


def run(binary, counts):
    cases = []
    with Fixture(binary) as fixture:
        workspace = fixture.card('synthetic indexed storage')
        record = human(fixture, workspace, 'send_message', {'to': workspace, 'body': 'seed'})['message']
        record.update(id=f'{1 << 120:032x}', acknowledged=1, surfaced=1, native_surfaced=1)
        stop(fixture)
        source = seed_history(saved(fixture), record, STORE_LIMIT - 4096)
        (fixture.state / 'workspaces.v2.json').write_bytes(encode(source))
        start(fixture)
        sent = human(fixture, workspace, 'send_message', {'to': workspace, 'body': 'activation ' + 'x' * 16000})
        human(fixture, workspace, 'inbox', {'ack_ids': [sent['message']['id']]})
        db = database(fixture)
        program = fixture.root / 'standin.py'
        program.write_text("import time\nprint('FIXTURE_READY',flush=True)\ntime.sleep(300)\n")
        (fixture.state / 'harnesses.json').write_bytes(encode([
            {'name': 'codex', 'command': ['/usr/bin/python3', str(program)]}]))
        for count in counts:
            stop(fixture)
            actual, body_bytes = grow(db, workspace, count)
            begin = time.monotonic()
            start(fixture)
            startup = (time.monotonic() - begin) * 1000
            cold = memory(fixture.server.pid)
            assert all(not w['tabs'] for w in fixture.json('list')['workspaces'])
            fixture.request('native', workspace, 'codex', '')
            tab = fixture.tab(workspace)
            def native(op, args=None):
                return fixture.json('agent-operation', tab['id'], tab['run'], op, encode(args or {}).hex())
            native('set_focus', {'seconds': 1800, 'reason': 'synthetic storage measurement'})
            times = {key: [] for key in ('context', 'send', 'inbox', 'ack', 'ack_retry', 'history', 'metadata')}
            sizes = {key: [] for key in times}
            writes = {key: [] for key in times}
            def measure(label, operation):
                before = write_bytes(fixture.server.pid)
                begin = time.monotonic()
                result = operation()
                times[label].append((time.monotonic() - begin) * 1000)
                writes[label].append(write_bytes(fixture.server.pid) - before)
                sizes[label].append(len(encode(result)))
                return result
            for sample in range(7):
                context = measure('context', lambda: native('context'))
                assert context['pending_messages'] == 0
                body = f'Synthetic exact pending correction {sample}: teal-κ7'
                sent = measure('send', lambda: human(fixture, workspace, 'send_message', {'to': workspace, 'body': body}))
                identity = sent['message']['id']
                inbox = measure('inbox', lambda: native('inbox'))
                assert [(m['id'], m['body']) for m in inbox['messages']] == [(identity, body)]
                ack = measure('ack', lambda: native('inbox', {'ack_ids': [identity]}))
                assert ack['acknowledged'] == [identity] and ack['pending'] == 0
                retry = measure('ack_retry', lambda: native('inbox', {'ack_ids': [identity]}))
                assert retry == ack
                history = measure('history', lambda: native('inbox', {'include_acknowledged': True, 'limit': 1}))
                assert len(history['messages']) == 1 and history['messages'][0]['acknowledged'] is not None
                measure('metadata', lambda: (fixture.metadata(workspace, f'synthetic sample {sample}'), {'saved': True})[1])
            cases.append({'records_before_samples': actual, 'retained_original_bytes': body_bytes,
                          'database_bytes': db.stat().st_size, 'cold_start_ms': startup,
                          'cold_memory': cold, 'after_samples_memory': memory(fixture.server.pid),
                          'calls': {key: {**distribution(times[key]), 'response_bytes_max': max(sizes[key]),
                                         'process_write_bytes_total': sum(writes[key])} for key in times}})
            print(json.dumps(cases[-1]), flush=True)
        provenance = fixture.provenance()
    return {'schema': 1, 'cases': cases, 'provenance': provenance,
            'method': 'Seven sequential samples per size, one run; stopped synthetic database growth; '
                      'real supervisor/native socket calls with sleeping Python stand-in. Exact body and ACK checks. '
                      'Cold restart asserts no tab launch. Counts traverse scoped metadata; body reads and working memory are bounded. '
                      'RSS and process write_bytes include incidental maintenance and filesystem accounting; '
                      'single-machine results, no power-loss, Windows/SSH or model performance claim.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--records', nargs='+', type=int, default=[600, 2400, 7200])
    args = parser.parse_args()
    if not args.records or any(n < 550 or n > 16000 for n in args.records) or args.records != sorted(set(args.records)):
        parser.error('record counts must increase uniquely within 550..16000')
    save_report(args.output, run(args.binary, args.records))
