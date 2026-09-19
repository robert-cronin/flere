#!/usr/bin/env python3
"""Measure ordinary input cost with retained, stopped tabs on a synthetic board."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import time

from eval_runtime import resources
from eval_support import Fixture, distribution, frame, read_frame, save_report

PROBE = '''import os, sys, tty
from pathlib import Path
tty.setraw(0)
received = open('received.bin', 'ab', buffering=0)
Path('input-ready').touch()
while True:
    data = os.read(0, 4096)
    if not data:
        break
    received.write(data)
    if sys.argv[1] == 'echo':
        sys.stdout.buffer.write(data)
        sys.stdout.buffer.flush()
'''


def wait_for(fixture, predicate, label, seconds=5):
    until = time.monotonic() + seconds
    while not predicate():
        if fixture.server.poll() is not None:
            raise RuntimeError('owned supervisor exited during ' + label)
        if time.monotonic() >= until:
            raise TimeoutError(label)
        time.sleep(.005)


def background_layout(saved, owner):
    return [(w['id'], w['name'], w['cwd'], w['meta'], w.get('selected', 0),
             w.get('split'), w.get('tabs', [])) for w in saved['workspaces'] if w['id'] != owner]


def seed_stopped_board(fixture, cards, tabs, title_bytes):
    owner = fixture.card('input consumer', shell=True)
    fixture.request('stop')
    fixture.server.wait(timeout=5)
    path = fixture.state / 'workspaces.v2.json'
    saved = json.loads(path.read_bytes())
    active = next(w for w in saved['workspaces'] if w['id'] == owner)
    assert len(active['tabs']) == 1, 'fixture shell was not saved'
    active['tabs'][0]['owner'] = None  # The owned previous process has stopped.
    for index in range(cards - 1):
        card = copy.deepcopy(active)
        card.update(id=index + 3, name=f'stopped fixture {index}', selected=0, split=None, tabs=[])
        for tab in range(tabs):
            order = 10000 + index * tabs + tab
            card['tabs'].append({'order': order, 'kind': 'shell', 'cwd': str(fixture.root),
                                 'harness': '', 'conversation': '', 'path': '',
                                 'title': 's' * title_bytes, 'owner': None})
        if card['tabs']:
            card['selected'] = card['tabs'][0]['order']
        saved['workspaces'].append(card)
    # Only this stopped disposable supervisor's synthetic store is seeded. The
    # cold-start route must retain other tabs without starting them in background.
    expected_background = background_layout(saved, owner)
    path.write_text(json.dumps(saved, separators=(',', ':')))
    fixture.server = subprocess.Popen([fixture.binary, '--state', str(fixture.state), 'serve'],
                                      env=fixture.env, stdin=subprocess.DEVNULL,
                                      stdout=fixture.log, stderr=fixture.log)

    def ready():
        try:
            fixture.request('ping')
            return True
        except (OSError, EOFError):
            return False
    wait_for(fixture, ready, 'cold supervisor startup')
    inventory = fixture.json('list')
    assert all(not w['tabs'] for w in inventory['workspaces']), 'cold start launched a tab'
    fixture.json('restore-workspace-next', inventory['epoch'], owner)
    inventory = fixture.json('list')
    assert sum(len(w['tabs']) for w in inventory['workspaces']) == 1, 'unrequested tab restored'
    return owner, fixture.tab(owner), expected_background


def audit_records_match(before, after, epoch, tab, lengths):
    if not after.startswith(before):
        return False
    appended = after[len(before):]
    lines = appended.splitlines(keepends=True)
    if len(lines) != len(lengths):
        return False
    for line, length in zip(lines, lengths):
        fields = line.removesuffix(b'\n').split(b'\t')
        expected = [epoch.encode(), b'input', str(tab['id']).encode(),
                    f"run={tab['run']};bytes={length}".encode().hex().encode()]
        if not line.endswith(b'\n') or len(fields) != 5 or not fields[0].isdigit() or fields[1:] != expected:
            return False
    return True


def run(binary, cards, tabs, title_bytes, samples, repeats, echo=False):
    if os.uname().sysname != 'Linux':
        raise RuntimeError('CPU/RSS sampling requires Linux /proc')
    with Fixture(binary) as fixture:
        owner, tab, expected_background = seed_stopped_board(fixture, cards, tabs, title_bytes)
        (fixture.root / 'input-probe.py').write_text(PROBE)
        mode = 'echo' if echo else 'quiet'
        fixture.input(tab, f'exec /usr/bin/python3 input-probe.py {mode}\r'.encode())
        wait_for(fixture, lambda: (fixture.root / 'input-ready').exists(), 'raw consumer startup')
        received_path = fixture.root / 'received.bin'
        expected = bytearray(b'warmup;')
        fixture.input(tab, expected)
        wait_for(fixture, lambda: received_path.read_bytes() == expected, 'input warmup')
        fixture.request('save-tabs')
        time.sleep(1.1)  # Settle one ordinary background-observation round.
        inventory = fixture.json('list')
        path = fixture.state / 'workspaces.v2.json'
        saved = path.read_bytes()
        inode = path.stat().st_ino
        assert background_layout(json.loads(saved), owner) == expected_background, 'cold restore changed retained background metadata'
        pending = [(w['id'], w['tabs']) for w in json.loads(saved)['workspaces'] if w['id'] != owner]
        assert sum(len(t) for _, t in pending) == (cards - 1) * tabs
        ping_reply = fixture.request('ping')
        cases = []
        for repeat in range(repeats):
            operations = ['ping', 'input', 'text']
            if repeat % 2:
                operations.reverse()
            for op in operations:
                results = []
                audit_path = fixture.state / 'actions.log'
                audit_before = audit_path.read_bytes()
                audit_lengths = []
                before_cpu, before_rss = resources(fixture.server.pid)
                begin = time.monotonic()
                for index in range(samples):
                    token = f'{repeat}:{op}:{index:05d};'.encode()
                    if op == 'ping':
                        fields, wanted = ['ping'], ping_reply
                    else:
                        fields, wanted = [op, tab['id'], tab['run'], token.hex()], b'ok'
                        expected.extend(token)
                        audit_lengths.append(len(token))
                    packet = frame('\t'.join(map(str, fields)).encode())
                    started = time.monotonic()
                    try:
                        with fixture.connect() as stream:
                            stream.settimeout(2)
                            stream.sendall(packet)
                            reply = read_frame(stream)
                        status, error = ('ok', '') if reply == wanted else ('error', 'unexpected reply')
                    except socket.timeout:
                        status, error = 'timeout', 'reply not confirmed; no retry'
                    except (OSError, EOFError, ValueError) as failure:
                        status, error = 'error', str(failure)[:500]
                    results.append({'ms': (time.monotonic() - started) * 1000,
                                    'status': status, 'error': error})
                end = time.monotonic()
                after_cpu, after_rss = resources(fixture.server.pid)
                # Verify all attempted bytes, including failed/unknown requests;
                # never replay a command because its acknowledgement was lost.
                until = time.monotonic() + 4
                while received_path.read_bytes() != expected and time.monotonic() < until:
                    if fixture.server.poll() is not None:
                        raise RuntimeError('owned supervisor exited during byte verification')
                    time.sleep(.005)
                actual = received_path.read_bytes()
                echo_observed = None
                if echo and op != 'ping':
                    until = time.monotonic() + 4
                    while True:
                        capture = fixture.json('capture', tab['id'], tab['run'], 20)['text']
                        echo_observed = token.decode() in ''.join(capture.split())
                        if echo_observed or time.monotonic() >= until:
                            break
                        time.sleep(.005)
                errors = sum(r['status'] != 'ok' for r in results)
                audit_exact = audit_records_match(audit_before, audit_path.read_bytes(),
                                                  inventory['epoch'], tab, audit_lengths)
                unchanged = fixture.json('list') == inventory
                store_unchanged = path.read_bytes() == saved and path.stat().st_ino == inode
                good = [r['ms'] for r in results if r['status'] == 'ok']
                cases.append({'repeat': repeat, 'operation': op, 'attempted': samples,
                              'errors': errors, 'samples': results,
                              'successful_request_latency': distribution(good) if good else None,
                              'elapsed_s': end - begin,
                              'supervisor_cpu_s': after_cpu - before_cpu,
                              'supervisor_cpu_us_per_attempt': (after_cpu - before_cpu) * 1e6 / samples,
                              'sampled_supervisor_rss_bytes': max(before_rss, after_rss),
                              'exact_child_input': actual == expected, 'echo_observed': echo_observed,
                              'audit_records_exact': audit_exact,
                              'expected_bytes': len(expected), 'observed_bytes': len(actual),
                              'session_identities_unchanged': unchanged,
                              'saved_layout_and_store_unchanged': store_unchanged})
                if errors or actual != expected or echo_observed is False or not audit_exact or not unchanged or not store_unchanged:
                    break
            else:
                continue
            break
        valid = (len(cases) == repeats * 3 and all(not c['errors'] and c['exact_child_input']
                 and c['echo_observed'] is not False and c['session_identities_unchanged']
                 and c['saved_layout_and_store_unchanged'] and c['audit_records_exact'] for c in cases))
        return {'schema': 1, 'provenance': fixture.provenance(), 'cards': cards, 'echo': echo,
                'stopped_tabs_per_background_card': tabs, 'stopped_tabs': (cards - 1) * tabs,
                'title_bytes': title_bytes, 'requests_per_case': samples, 'repeats': repeats,
                'cases': cases, 'all_checks_passed': valid,
                'saved_layout_bytes': len(saved), 'saved_layout_sha256': hashlib.sha256(saved).hexdigest(),
                'method': 'One owned raw-byte Python consumer and synthetic cold-started stopped cards. '
                          'Only the consumer card is explicitly restored. Sequential socket requests '
                          'measure ordinary command cost; they are not independent arrivals or visible '
                          'typing latency. Errors are retained, never retried. CPU is the supervisor only '
                          '(Linux tick granularity); RSS is two endpoint samples per case. Child, client, '
                          'UI, network and model resources are excluded. Optional echo verifies the final '
                          'token in the supervisor terminal capture and exercises output-driven maintenance; '
                          'it does not time physical display output. All child bytes, complete saved '
                          'layout/store, session identities and metadata-only audit records are checked. Reverse operation order '
                          'on alternate repeats. No native models, live user state or network.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--cards', type=int, default=64)
    parser.add_argument('--tabs', type=int, default=4)
    parser.add_argument('--title-bytes', type=int, default=256)
    parser.add_argument('--samples', type=int, default=400)
    parser.add_argument('--repeats', type=int, default=2)
    parser.add_argument('--echo', action='store_true')
    args = parser.parse_args()
    if not (1 <= args.cards <= 128 and 0 <= args.tabs <= 32 and 1 <= args.title_bytes <= 1024
            and 10 <= args.samples <= 2000 and 1 <= args.repeats <= 8
            and args.samples * args.repeats * 3 <= 20000):
        parser.error('configuration exceeds bounded fixture limits')
    report = run(args.binary, args.cards, args.tabs, args.title_bytes, args.samples, args.repeats, args.echo)
    save_report(args.output, report)
    print(json.dumps({k: v for k, v in report.items() if k not in ('provenance', 'cases')}))
    for case in report['cases']:
        print(json.dumps({k: v for k, v in case.items() if k != 'samples'}))
    if not report['all_checks_passed']:
        raise SystemExit(2)


if __name__ == '__main__':
    main()
