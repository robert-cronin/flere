#!/usr/bin/env python3
"""Owned Linux PTY/save-queue probe, optionally delaying fsync with strace."""
import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import shutil
import struct
import subprocess
import termios
import time
from unittest.mock import patch

from eval_support import Fixture, distribution, save_report


PROBE = '''import os, tty
from pathlib import Path
tty.setraw(0)
received = open('received.bin', 'ab', buffering=0)
Path('input-ready').touch()
while True:
    data = os.read(0, 4096)
    if not data:
        break
    received.write(data)
'''


def run(binary, delay_ms, samples):
    binary = str(Path(binary).resolve(strict=True))
    if os.uname().sysname != 'Linux':
        raise RuntimeError('the controlled disk-delay probe requires Linux')
    if delay_ms and not shutil.which('strace'):
        raise RuntimeError('install strace or run with --delay-ms 0')
    original_popen = subprocess.Popen

    def trace(command, output):
        if not delay_ms:
            return command
        return ['strace', '-f', '-qq', '-e', 'trace=fsync', '-e',
                f'inject=fsync:delay_enter={delay_ms}ms', '-o', str(output), *command]

    def launch(command, *args, **kwargs):
        if command[0] == binary and command[-1] == 'serve':
            root = Path(command[2]).parent
            command = trace(command, root / 'supervisor-sync.trace')
        return original_popen(command, *args, **kwargs)

    with patch('subprocess.Popen', launch), Fixture(binary) as fixture:
        workspace = fixture.card('settings fixture', shell=True)
        tab = fixture.tab(workspace)
        (fixture.root / 'input-probe.py').write_text(PROBE)
        fixture.input(tab, b'exec /usr/bin/python3 input-probe.py\r')
        deadline = time.monotonic() + 8
        while not (fixture.root / 'input-ready').exists():
            if time.monotonic() >= deadline:
                raise TimeoutError('synthetic input receiver did not start')
            time.sleep(.01)
        (fixture.state / 'ui.json').write_text('{"reduced_motion":false,"pet":false}')
        expected_identity = fixture.json('list')
        epoch = expected_identity['epoch']
        try:
            fixture.json('ui-preferences', 'get', epoch)
            queued = True
        except RuntimeError as error:
            if str(error) != 'unknown command':
                raise
            queued = False
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 34, 150, 0, 0))
        os.set_blocking(master, False)
        ui = original_popen(trace([binary, '--state', str(fixture.state), 'attach'],
                                 fixture.root / 'ui-sync.trace'),
                            stdin=slave, stdout=slave, stderr=slave, cwd=fixture.root,
                            env=fixture.env, start_new_session=True)
        os.close(slave)
        output_bytes = 0

        def drain():
            nonlocal output_bytes
            if select.select([master], [], [], .002)[0]:
                try:
                    output_bytes += len(os.read(master, 262144))
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise

        def wait_for(condition, label, seconds=12):
            until = time.monotonic() + seconds
            while not condition():
                if ui.poll() is not None:
                    raise RuntimeError(f'frontend exited during {label}: {ui.returncode}')
                if time.monotonic() >= until:
                    raise TimeoutError(label)
                drain()

        def received():
            path = fixture.root / 'received.bin'
            return path.read_bytes() if path.exists() else b''

        try:
            # First plain byte proves the entire UI->socket->PTY path is ready.
            os.write(master, b'WARMUP')
            wait_for(lambda: received() == b'WARMUP', 'initial input')
            expected = b'WARMUP'
            latencies = []
            for i in range(samples):
                token = f'input-{i:04d};'.encode()
                expected += token
                started = time.monotonic()
                # Toggle reduced motion, leave navigation, then type. No command
                # is submitted; the child is our raw-byte Python receiver.
                os.write(master, b'\0M\0' + token)
                wait_for(lambda: received() == expected, 'input following save')
                latencies.append((time.monotonic() - started) * 1000)
                observed = (fixture.json('ui-preferences', 'get', epoch)['preferences']
                            if queued else json.loads((fixture.state / 'ui.json').read_bytes()))
                assert observed['reduced_motion'] == bool((i + 1) % 2), 'setting toggle was lost'
            started = time.monotonic()
            # A final change followed by immediate detach exercises the pending
            # save lifetime; redundant final history writes are also observable.
            os.write(master, b'\0Mq')
            until = time.monotonic() + 12
            while ui.poll() is None:
                if time.monotonic() >= until:
                    raise TimeoutError('frontend detach')
                drain()
            detach_ms = (time.monotonic() - started) * 1000
            assert ui.returncode == 0, 'frontend did not detach successfully'
            desired = bool((samples + 1) % 2)
            pending_at_detach = False
            if queued:
                current = fixture.json('ui-preferences', 'get', epoch)
                assert current['preferences']['reduced_motion'] == desired
                assert current['error'] is None
                pending_at_detach = current['pending']
                until = time.monotonic() + 12
                while fixture.json('ui-preferences', 'status', epoch)['pending']:
                    assert time.monotonic() < until, 'save did not finish after detach'
                    time.sleep(.01)
                assert fixture.json('ui-preferences', 'status', epoch)['error'] is None
            saved = json.loads((fixture.state / 'ui.json').read_bytes())
            assert saved['reduced_motion'] == desired, 'latest setting was lost'
            assert received() == expected, 'navigation leaked input or typed input was lost'
            assert fixture.json('list') == expected_identity, 'session identity/layout changed'
            sync_calls = sum(p.read_text().count('fsync(') for p in fixture.root.glob('*-sync.trace'))
            return {'schema': 1, 'provenance': fixture.provenance(),
                    'injected_fsync_delay_ms': delay_ms, 'queued_saves_supported': queued,
                    'input_after_save': distribution(latencies), 'samples_ms': latencies,
                    'detach_ms': detach_ms, 'pending_at_detach': pending_at_detach,
                    'verified_latest_saved': True, 'verified_exact_input': True,
                    'verified_session_identity': True, 'drained_ui_bytes': output_bytes,
                    'observed_fsync_calls': sync_calls if delay_ms else None,
                    'method': 'Controlled slow-disk fault probe; times include tracing and observer overhead. '
                              'Input timing ends at the owned Python child receiving exact bytes, not at '
                              'rendered output or a physical display. No SSH, Windows, native models or live sessions.'}
        finally:
            if ui.poll() is None:
                os.killpg(ui.pid, signal.SIGTERM)
                try:
                    ui.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(ui.pid, signal.SIGKILL)
                    ui.wait()
            os.close(master)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--delay-ms', type=int, default=100)
    parser.add_argument('--samples', type=int, default=4)
    args = parser.parse_args()
    if not 0 <= args.delay_ms <= 200 or not 1 <= args.samples <= 100:
        parser.error('delay must be 0..200 ms and samples 1..100')
    report = run(args.binary, args.delay_ms, args.samples)
    save_report(args.output, report)
    print(json.dumps(report))


if __name__ == '__main__':
    main()
