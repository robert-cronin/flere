#!/usr/bin/env python3
"""Measure unrelated supervisor requests during an owned durable state mutation."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import time
from unittest.mock import patch

from eval_support import Fixture, distribution, frame, read_frame, save_report

PRODUCER = '''import os, threading, time, tty
from pathlib import Path
tty.setraw(0)
received = open('received.bin', 'ab', buffering=0)
def output():
    count = 0
    while True:
        os.write(1, f'HEARTBEAT_{count:08d}\\r\\n'.encode())
        Path('progress').write_text(str(count))
        count += 1
        time.sleep(.04)
threading.Thread(target=output, daemon=True).start()
Path('input-ready').touch()
while True:
    data = os.read(0, 4096)
    if not data:
        break
    received.write(data)
'''


def request(fixture, fields, timeout):
    # Same framing/owner socket as wire::request. Input/ping keep a 2 s budget;
    # socket mutations use an explicit observation budget. Nothing is retried.
    with fixture.connect() as stream:
        stream.settimeout(timeout)
        stream.sendall(frame('\t'.join(map(str, fields)).encode()))
        data = read_frame(stream)
        if data.startswith(b'!'):
            raise RuntimeError(data[1:].decode())
        return data


def identities(value):
    return {'epoch': value['epoch'], 'active_workspace': value['active_workspace'],
            'workspaces': [{'id': w['id'], 'cwd': w['cwd'], 'selected_tab': w['selected_tab'],
                            'tabs': [(t['id'], t['pid'], t['run'], t['alive']) for t in w['tabs']]}
                           for w in value['workspaces']]}


def run(binary, delay_ms, samples, mutation_client='socket'):
    if os.uname().sysname != 'Linux':
        raise RuntimeError('the controlled syscall-delay probe requires Linux')
    if delay_ms and not shutil.which('strace'):
        raise RuntimeError('install strace or run with --delay-ms 0')
    binary = str(Path(binary).resolve(strict=True))
    original_popen = subprocess.Popen
    original_connect = Fixture.connect
    def setup_connect(fixture):
        stream = original_connect(fixture)
        # Fault injection also slows creation of the disposable fixture. Keep
        # setup/verification separate from measured requests' explicit budgets.
        stream.settimeout(20)
        return stream

    def launch(command, *args, **kwargs):
        if delay_ms and command[0] == binary and command[-1] == 'serve':
            root = Path(command[2]).parent
            command = ['strace', '-f', '-qq', '-e', 'trace=fsync', '-e',
                       f'inject=fsync:delay_enter={delay_ms}ms', '-o',
                       str(root / 'durability.trace'), *command]
        return original_popen(command, *args, **kwargs)

    with patch('subprocess.Popen', launch), patch.object(Fixture, 'connect', setup_connect), Fixture(binary) as fixture:
        workspace = fixture.card('durability fixture', shell=True)
        tab = fixture.tab(workspace)
        (fixture.root / 'producer.py').write_text(PRODUCER)
        fixture.input(tab, b'exec /usr/bin/python3 producer.py\r')

        def wait_for(check, label, timeout=5):
            end = time.monotonic() + timeout
            while not check():
                if fixture.server.poll() is not None:
                    raise RuntimeError('owned supervisor exited during ' + label)
                if time.monotonic() >= end:
                    raise TimeoutError(label)
                time.sleep(.005)

        def received():
            file = fixture.root / 'received.bin'
            return file.read_bytes() if file.exists() else b''

        last_count = -1
        def count():
            nonlocal last_count
            try:
                last_count = max(last_count, int((fixture.root / 'progress').read_text()))
            except (OSError, ValueError):
                pass  # Keep the last observed counter across a partial file write.
            return last_count

        wait_for(lambda: (fixture.root / 'input-ready').exists(), 'producer startup')
        wait_for(lambda: count() >= 5, 'producer failed to advance')
        original = identities(fixture.json('list'))
        expected = b'warmup;'
        fixture.input(tab, expected)
        wait_for(lambda: received() == expected, 'input warmup')
        rows = []

        def is_saved(marker):
            saved = json.loads((fixture.state / 'workspaces.v2.json').read_bytes())
            saved_workspace = next(w for w in saved['workspaces'] if w['id'] == workspace)
            return saved_workspace['meta']['notes'] == marker

        def measured(fields, timeout=2, origin=None, saved_marker=None):
            started = time.monotonic()
            try:
                request(fixture, fields, timeout)
                status = 'ok'
            except socket.timeout:
                status = 'timeout'
            except (OSError, EOFError, RuntimeError) as error:
                status = 'error'
                detail = str(error)[:1000]
            finished = time.monotonic()
            return {'elapsed_ms': (finished - started) * 1000, 'status': status,
                    'started_after_ms': (started - origin) * 1000 if origin is not None else None,
                    'error': detail if status == 'error' else '',
                    'saved_value_at_reply': is_saved(saved_marker) if saved_marker is not None else None}

        def measured_core(arguments, origin):
            started = time.monotonic()
            command = [binary, '--state', str(fixture.state), 'coordinate', str(workspace),
                       'update_workspace', json.dumps(arguments, separators=(',', ':'))]
            try:
                completed = subprocess.run(command, env=fixture.env, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, timeout=35)
                status = 'ok' if completed.returncode == 0 else 'error'
                error = completed.stderr.decode(errors='replace')[:1000]
            except subprocess.TimeoutExpired:
                status, error = 'timeout', 'outer CLI observation budget expired; outcome unknown'
            finished = time.monotonic()
            return {'elapsed_ms': (finished - started) * 1000, 'status': status,
                    'started_after_ms': (started - origin) * 1000, 'error': error,
                    'saved_value_at_reply': is_saved(arguments['notes'])}

        with ThreadPoolExecutor(max_workers=3) as pool:
            for index in range(samples):
                marker = f'durable-trial-{index}'
                notes = json.dumps({'notes': marker}, separators=(',', ':')).encode().hex()
                prior_files = set(fixture.state.glob('.*.new'))
                begin_count = count()
                inventory = fixture.json('list')
                current = next(w for w in inventory['workspaces'] if w['id'] == workspace)
                arguments = {'workspace': workspace, 'notes': marker, 'expected_epoch': inventory['epoch'],
                             'expected': {'name': current['name'], 'meta': current['meta']}}
                started = time.monotonic()
                if mutation_client == 'core':
                    mutation = pool.submit(measured_core, arguments, started)
                else:
                    mutation = pool.submit(measured, ['metadata', workspace, notes], 8, started, marker)
                # Observe the actual pending write, not an assumed sleep window.
                # The private atomic-write file exists through the file fsync.
                def pending_write():
                    for file in set(fixture.state.glob('.*.new')) - prior_files:
                        try:
                            if marker.encode() in file.read_bytes():
                                return True
                        except FileNotFoundError:
                            pass  # A zero-delay control may already have renamed.
                    return False

                pending = False
                if delay_ms:
                    until = started + 5
                    while not pending:
                        pending = pending_write()
                        if pending:
                            break
                        if mutation.done():
                            raise AssertionError('mutation completed before pending-write barrier')
                        if time.monotonic() >= until:
                            raise TimeoutError('mutation never reached pending-write barrier')
                        time.sleep(.001)
                requests_started_ms = (time.monotonic() - started) * 1000
                token = f'input-{index:04d};'.encode()
                expected += token
                before_input = received()
                wanted_input = before_input + token
                ping = pool.submit(measured, ['ping'], 2, started)
                typed = pool.submit(measured, ['input', tab['id'], tab['run'], token.hex()], 2, started)
                ping_result = ping.result(timeout=10)
                input_result = typed.result(timeout=10)
                first_input_observed = None
                while not mutation.done():
                    if first_input_observed is None and received() == wanted_input:
                        first_input_observed = (time.monotonic() - started) * 1000
                    if time.monotonic() - started > (38 if mutation_client == 'core' else 10):
                        raise TimeoutError('durable mutation did not finish')
                    time.sleep(.001)
                mutation_result = mutation.result(timeout=10)
                # A timeout does not establish whether the operation applied.
                # Observe eventual input once, never replay it. Retain misses in
                # the report rather than dropping failed samples from results.
                input_observed_at = time.monotonic()
                until = input_observed_at + 4
                while received() != wanted_input and time.monotonic() < until:
                    if fixture.server.poll() is not None:
                        raise RuntimeError('owned supervisor exited during input observation')
                    time.sleep(.005)
                observed_input = received()
                if observed_input == wanted_input and first_input_observed is None:
                    first_input_observed = (time.monotonic() - started) * 1000
                input_observation_ms = (time.monotonic() - input_observed_at) * 1000
                input_delivery = ('exact' if observed_input == wanted_input else
                                  'unobserved' if observed_input == before_input else 'unexpected')
                # A lost acknowledgement can precede the eventual commit. Observe
                # the original attempt without retrying or interpreting its error
                # as proof of rollback. A successful reply must already be saved.
                saved_at_reply = mutation_result['saved_value_at_reply']
                until = time.monotonic() + 4
                while not is_saved(marker) and time.monotonic() < until:
                    time.sleep(.005)
                durable_value_verified = is_saved(marker)
                assert identities(fixture.json('list')) == original, 'session identity/layout changed'
                rows.append({'pending_write_observed': pending,
                             'unrelated_requests_submitted_after_ms': requests_started_ms,
                             'mutation': mutation_result, 'ping': ping_result, 'input': input_result,
                             'producer_updates': count() - begin_count,
                             'input_delivery': input_delivery,
                             'first_input_observed_after_ms': first_input_observed,
                             'input_observed_before_mutation_reply': (first_input_observed is not None
                                 and first_input_observed < mutation_result['elapsed_ms']),
                             'post_mutation_input_observation_ms': input_observation_ms,
                             'saved_value_at_reply': saved_at_reply,
                             'durable_value_verified': durable_value_verified,
                             'exact_input_verified': input_delivery == 'exact'})
        before = count()
        wait_for(lambda: count() >= before + 5, 'producer failed to continue')
        trace = fixture.root / 'durability.trace'
        trace_text = trace.read_text() if trace.exists() else ''
        syncs = trace_text.count('fsync(')
        delayed = trace_text.count('DELAYED')
        if delay_ms:
            assert delayed >= 2 * samples, 'requested fsync injection was not observed'
        return {'schema': 3, 'provenance': fixture.provenance(),
                'mutation_client': mutation_client,
                'mutation_errors': sum(r['mutation']['status'] != 'ok' for r in rows),
                'durable_values_verified': all(r['durable_value_verified'] for r in rows),
                'successful_replies_already_saved': all(r['saved_value_at_reply'] for r in rows if r['mutation']['status'] == 'ok'),
                'injected_fsync_delay_ms': delay_ms, 'samples': rows, 'setup_observation_budget_s': 20,
                'durable_ack_latency': distribution([r['mutation']['elapsed_ms'] for r in rows]),
                'unrelated_ping_latency': distribution([r['ping']['elapsed_ms'] for r in rows]),
                'input_request_latency': distribution([r['input']['elapsed_ms'] for r in rows]),
                'input_observed_before_mutation_reply_samples': sum(r['input_observed_before_mutation_reply'] for r in rows),
                'ping_failures': sum(r['ping']['status'] != 'ok' for r in rows),
                'input_failures': sum(r['input']['status'] != 'ok' for r in rows),
                'ping_timeouts': sum(r['ping']['status'] == 'timeout' for r in rows),
                'input_timeouts': sum(r['input']['status'] == 'timeout' for r in rows),
                'input_unobserved_samples': sum(r['input_delivery'] == 'unobserved' for r in rows),
                'input_unexpected_samples': sum(r['input_delivery'] == 'unexpected' for r in rows),
                'observed_fsync_calls': syncs if delay_ms else None,
                'observed_delayed_fsync_calls': delayed if delay_ms else None,
                'exact_input_verified': received() == expected,
                'expected_input_bytes': len(expected), 'received_input_bytes': len(received()),
                'supervisor_diagnostics': [line for log in fixture.state.joinpath('diagnostics').glob('supervisor-*.log')
                                           for line in log.read_text().splitlines()
                                           if any(event in line for event in ('client-timeout', 'client-write-error',
                                                'supervisor-persist', 'supervisor-command', 'supervisor-drain',
                                                'store-file-sync', 'store-directory-sync'))],
                'session_identity_verified': identities(fixture.json('list')) == original,
                'method': 'Owned Linux supervisor and Python shell child. Process-scoped strace fsync '
                          'delay; unrelated requests start after a real pending-write file is observed. '
                          'Setup/verification allow 20 s. The socket mutation control has an 8 s budget; '
                          '--mutation-client core invokes the actual CLI update_workspace route with a 35 s outer observation limit, '
                          "using the binary's own request deadline. Ping/input use the "
                          'core 2 s budget. No retries. Reported request latency ends at reply/timeout; '
                          'First child-input observation uses 1 ms file polling while the mutation remains pending; '
                          'this is an upper bound including client reply handling. Missing '
                          'child input is observed for another 4 s after the mutation reply, with misses retained. No UI, SSH, Windows, '
                          'native models or user sessions.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--delay-ms', type=int, default=200)
    parser.add_argument('--samples', type=int, default=4)
    parser.add_argument('--mutation-client', choices=('socket', 'core'), default='socket')
    args = parser.parse_args()
    if not 0 <= args.delay_ms <= 1500 or not 1 <= args.samples <= 20:
        parser.error('delay must be 0..1500 ms and samples 1..20')
    report = run(args.binary, args.delay_ms, args.samples, args.mutation_client)
    save_report(args.output, report)
    print(json.dumps({k: v for k, v in report.items() if k not in ('provenance', 'samples', 'supervisor_diagnostics')}))
    if (not report['exact_input_verified'] or report['mutation_errors']
            or report['ping_failures'] or report['input_failures']
            or not report['durable_values_verified'] or not report['successful_replies_already_saved']):
        raise SystemExit(2)  # Retain failed completion and integrity measurements.


if __name__ == '__main__':
    main()
