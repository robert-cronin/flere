"""Harmless PTY probe: Codex-style inline history, then explicit alt-scroll mode.
Control files trigger output, not terminal input; every received byte is retained.
"""
import os
import pathlib
import select
import sys
import termios
import tty

old = termios.tcgetattr(0)
root = pathlib.Path('.')

def emit(text):
    sys.stdout.write(text)
    sys.stdout.flush()

def transcript(first, count):
    rows = os.get_terminal_size(0).lines
    bottom = max(2, rows - 4)
    emit(f'\x1b[1;{bottom}r\x1b[{bottom};1H')
    for i in range(first, first + count):
        emit(f'\r\n\x1b[32mCHAT-{i:03d} 界\x1b[0m')
    emit(f'\x1b[r\x1b[{rows - 1};1HDRAFT_UNSUBMITTED')

try:
    tty.setraw(0)
    (root / 'received').write_bytes(b'')
    emit('\x1b[3J\x1b[2J\x1b[H')
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 90
    transcript(0, count)
    more = alt = off = False
    while True:
        if not more and (root / 'more').exists():
            transcript(90, 5)
            more = True
        if not alt and (root / 'alt').exists():
            emit('\x1b[?1049h\x1b[?1007h\x1b[?1h\x1b[2J\x1b[HALT_SCROLL_READY')
            alt = True
        if not off and (root / 'off').exists():
            emit('\x1b[?1007l\x1b[?1049l\x1b[?1l')
            off = True
        if select.select([0], [], [], 0.01)[0]:
            data = os.read(0, 4096)
            if not data:
                break
            with (root / 'received').open('ab') as f:
                f.write(data)
finally:
    termios.tcsetattr(0, termios.TCSANOW, old)
