#!/usr/bin/env python3
"""Measure owned UI input forwarding while an outer PTY reader pauses (Linux)."""
import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import termios
import time

from eval_support import Fixture, distribution, save_report

PRODUCER = r'''
import os, threading, time, tty
from pathlib import Path
tty.setraw(0)
received = open('received.bin', 'ab', buffering=0)
stopped = threading.Event()
def output():
    number = 0
    while not stopped.is_set():
        width, height = os.get_terminal_size(1)
        char = bytes([33 + number % 90])
        head = f'FRAME_{number:08d}'.encode()[:width].ljust(width, b' ')
        os.write(1, b'\x1b[H' + head + b'\r\n' +
                 (char * width + b'\r\n') * max(1, height - 2))
        number += 1
        time.sleep(.025)
threading.Thread(target=output, daemon=True).start()
Path('input-ready').touch()
seen = b''
while True:
    data = os.read(0, 4096)
    if not data:
        break
    received.write(data)
    seen = (seen + data)[-128:]
    if b'END_OUTPUT;' in seen:
        stopped.set()
'''


def run(binary, pause_ms):
    if os.uname().sysname != 'Linux':
        raise RuntimeError('the wait-state observation requires Linux /proc')
    with Fixture(binary) as fixture:
        wid = fixture.card('output-pressure fixture', shell=True)
        tab = fixture.tab(wid)
        (fixture.root / 'producer.py').write_text(PRODUCER)
        fixture.input(tab, b'exec /usr/bin/python3 producer.py\r')
        until = time.monotonic() + 5
        while not (fixture.root / 'input-ready').exists():
            if time.monotonic() > until:
                raise TimeoutError('owned producer did not start')
            time.sleep(.01)
        identities = fixture.json('list')
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 34, 150, 0, 0))
        os.set_blocking(master, False)
        ui = subprocess.Popen([fixture.binary, '--state', str(fixture.state), 'attach'],
                              stdin=slave, stdout=slave, stderr=slave, cwd=fixture.root,
                              env=fixture.env, start_new_session=True)
        os.close(slave)
        consumed = 0

        def drain():
            nonlocal consumed
            if select.select([master], [], [], .002)[0]:
                try:
                    consumed += len(os.read(master, 262144))
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise

        def received():
            file = fixture.root / 'received.bin'
            return file.read_bytes() if file.exists() else b''

        def wait_input(expected):
            deadline = time.monotonic() + 5
            while received() != expected:
                if ui.poll() is not None:
                    raise RuntimeError('owned frontend exited before input arrived')
                if time.monotonic() > deadline:
                    raise TimeoutError('input was lost or altered')
                drain()

        def output_count():
            text = fixture.json('capture', tab['id'], tab['run'], 100)['text']
            values = re.findall(r'FRAME_(\d+)', text)
            if not values:
                raise AssertionError('producer output marker is missing')
            return int(values[-1])

        def wait_state(proc):
            result = {}
            try:
                fields = proc.joinpath('syscall').read_text().split()
                result['syscall'] = fields[0]
                if fields[0] in ('0', '1', '20'):
                    result['fd'] = int(fields[1], 16)
                result['wait_channel'] = proc.joinpath('wchan').read_text().strip()
            except (OSError, ValueError, IndexError):
                result['unavailable'] = True
            return result

        try:
            expected = b'warmup;'
            os.write(master, expected)
            wait_input(expected)
            regular = []
            for i in range(5):
                marker = f'regular-{i};'.encode()
                expected += marker
                start = time.monotonic()
                os.write(master, marker)
                wait_input(expected)
                regular.append((time.monotonic() - start) * 1000)
            states = []
            proc = Path('/proc', str(ui.pid))
            writers = []
            for task in proc.joinpath('task').iterdir():
                try:
                    if task.joinpath('comm').read_text().strip() == 'flere-output':
                        writers.append(task)
                except OSError:
                    pass  # An unrelated short-lived helper may have just exited.
            def resources():
                status = proc.joinpath('status').read_text()
                rss = re.search(r'^VmRSS:\s+(\d+)', status, re.M)
                stat = proc.joinpath('stat').read_text().rsplit(')', 1)[1].split()
                return {'rss_kib': int(rss[1]) if rss else None,
                        'cpu_ticks': int(stat[11]) + int(stat[12])}
            before_resources = resources()
            # Stop reading only this owned UI's outer PTY. The supervisor and
            # producer keep running. The typed marker is never a shell command.
            first_output = output_count()
            paused = time.monotonic()
            send_at = paused + pause_ms / 1000 / 3
            resume_at = paused + pause_ms / 1000
            sent = arrived = None
            before_pause_bytes = consumed
            while time.monotonic() < resume_at:
                now = time.monotonic()
                if sent is None and now >= send_at:
                    expected += b'paused-reader;'
                    os.write(master, b'paused-reader;')
                    sent = time.monotonic()
                if sent is not None and arrived is None and received() == expected:
                    arrived = time.monotonic()
                states.append({'offset_ms': (now - paused) * 1000,
                               **wait_state(proc), **resources(),
                               'writers': [wait_state(task) for task in writers]})
                if ui.poll() is not None:
                    raise RuntimeError('frontend exited while the reader was paused')
                time.sleep(.005)
            assert sent is not None and consumed == before_pause_bytes
            output_advance = output_count() - first_output
            assert output_advance >= 5, 'producer stopped or failed to sustain the pressure workload'
            resumed = time.monotonic()
            arrived_while_paused = arrived is not None
            wait_input(expected)
            if arrived is None:
                arrived = time.monotonic()
            expected += b'END_OUTPUT;'
            os.write(master, b'END_OUTPUT;')
            wait_input(expected)
            os.write(master, b'\0q')
            deadline = time.monotonic() + 5
            while ui.poll() is None:
                if time.monotonic() > deadline:
                    raise TimeoutError('frontend did not detach after reader resumed')
                drain()
            assert ui.returncode == 0
            assert received() == expected, 'input was lost, altered, duplicated or leaked'
            assert fixture.json('list') == identities, 'fixture identity/layout changed'
            queue_stats = []
            for log in fixture.state.joinpath('diagnostics').glob('ui-*.log'):
                for match in re.finditer(r'terminal-output peak_bytes=(\d+) limit_bytes=(\d+)', log.read_text()):
                    queue_stats.append({'peak_bytes': int(match[1]), 'limit_bytes': int(match[2])})
            blocked = lambda sample: sample.get('syscall') in ('1', '20') and sample.get('fd') == 1
            return {'schema': 2, 'provenance': fixture.provenance(),
                    'regular_input': distribution(regular),
                    'reader_pause_ms': (resumed - paused) * 1000,
                    'input_scheduled_after_pause_ms': (sent - paused) * 1000,
                    'input_forwarding_ms': (arrived - sent) * 1000,
                    'input_arrived_while_reader_paused': arrived_while_paused,
                    'stdout_write_observed': any(blocked(s) or any(map(blocked, s['writers'])) for s in states),
                    'main_stdout_write_observed': any(map(blocked, states)),
                    'writer_stdout_write_observed': any(any(map(blocked, s['writers'])) for s in states),
                    'queue_stats': queue_stats,
                    'rss_before_kib': before_resources['rss_kib'],
                    'rss_peak_kib': max(s['rss_kib'] or 0 for s in states),
                    'cpu_ms_during_pause': (states[-1]['cpu_ticks'] - before_resources['cpu_ticks']) * 1000 / os.sysconf('SC_CLK_TCK'),
                    'wait_states': states, 'ui_bytes_drained': consumed,
                    'producer_updates_during_pause': output_advance,
                    'exact_input_verified': True, 'session_identity_verified': True,
                    'method': 'Controlled outer-reader pause with synthetic output and real local UI/PTYS. '
                              'Input timing ends at exact-byte receipt by the owned Python child, '
                              'not at rendered characters. No SSH, Windows, model or live user sessions.'}
        finally:
            if ui.poll() is None:
                os.killpg(ui.pid, signal.SIGTERM)
                try:
                    ui.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    os.killpg(ui.pid, signal.SIGKILL)
                    ui.wait()
            os.close(master)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary')
    parser.add_argument('output')
    parser.add_argument('--pause-ms', type=int, default=900)
    args = parser.parse_args()
    if not 300 <= args.pause_ms <= 10000:
        parser.error('pause must be 300..10000 ms')
    report = run(args.binary, args.pause_ms)
    save_report(args.output, report)
    print(json.dumps({key: value for key, value in report.items() if key not in ('wait_states', 'provenance')}))


if __name__ == '__main__':
    main()
