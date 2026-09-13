"""Harmless split-pane PTY: record bytes/sizes, emit only fixture-controlled output."""
import json
import os
import pathlib
import select
import sys
import termios
import tty

root = pathlib.Path.cwd()
session = os.environ['FLERE_SESSION']
prefix = root / ('pane-' + session)
input_path = prefix.with_suffix('.input')
size_path = prefix.with_suffix('.size')
control_path = prefix.with_suffix('.control')
old = termios.tcgetattr(0)
last = 0
size = None
try:
    tty.setraw(0)
    input_path.write_bytes(b'')
    sys.stdout.write('\x1b[?2004h\x1b[2J\x1b[HPANE_READY_' + session)
    sys.stdout.flush()
    while True:
        current = os.get_terminal_size(0)
        if current != size:
            size_path.write_text(json.dumps([current.columns, current.lines]))
            size = current
        try:
            control = json.loads(control_path.read_text())
        except (OSError, ValueError):
            control = {}
        if control.get('seq', 0) > last:
            last = control['seq']
            sys.stdout.write(control.get('output', ''))
            sys.stdout.flush()
        if select.select([0], [], [], .01)[0]:
            data = os.read(0, 65536)
            if not data:
                break
            with input_path.open('ab') as stream:
                stream.write(data)
finally:
    termios.tcsetattr(0, termios.TCSANOW, old)
