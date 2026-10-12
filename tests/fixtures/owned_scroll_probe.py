"""Disposable full-screen transcript: owns history, requests SGR mouse, keeps a draft.
This is a native-input stand-in, never a model client. Log bytes before handling them.
"""
import os
import pathlib
import re
import select
import sys
import termios
import tty

root = pathlib.Path('.')
old = termios.tcgetattr(0)
position = 50
draft = 'DRAFT_UNSUBMITTED'
pending = b''
mouse_off = primary = False

def emit(text):
    sys.stdout.write(text)
    sys.stdout.flush()

def draw():
    rows = os.get_terminal_size(0).lines
    emit(f'\x1b[2J\x1b[HOWNED_TOP={position:03d}')
    if mouse_off:
        emit('\r\nMOUSE_OFF')
    emit(f'\x1b[{rows - 1};1H{draft}')

try:
    tty.setraw(0)
    (root / 'received').write_bytes(b'')
    emit('\x1b[?1049h\x1b[?1007l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h')
    draw()
    while True:
        if not mouse_off and (root / 'disable-mouse').exists():
            emit('\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l')
            mouse_off = True
            draw()
        if not primary and (root / 'primary').exists():
            emit('\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1049l')
            emit('\x1b[2J\x1b[H')
            for n in range(90):
                emit(f'INLINE-{n:03d}\r\n')
            emit(draft)
            primary = True
        if not select.select([0], [], [], 0.01)[0]:
            continue
        data = os.read(0, 4096)
        if not data:
            break
        with (root / 'received').open('ab') as log:
            log.write(data)
        pending += data
        while pending:
            wheel = re.match(rb'\x1b\[<(64|65);(\d+);(\d+)M', pending)
            if wheel:
                # The child receives coordinates relative to its own viewport.
                size = os.get_terminal_size(0)
                assert 1 <= int(wheel[2]) <= size.columns
                assert 1 <= int(wheel[3]) <= size.lines
                position += -3 if wheel[1] == b'64' else 3
                pending = pending[wheel.end():]
            elif pending.startswith((b'\x1b[5~', b'\x1b[6~')):
                page = max(1, os.get_terminal_size(0).lines - 1)
                position += -page if pending[2:3] == b'5' else page
                pending = pending[4:]
            elif pending.startswith(b'\x1b'):
                # Input may split within a report. Unknown complete keys are a test failure.
                if re.match(rb'\x1b\[[^A-Za-z~]*[A-Za-z~]', pending):
                    raise AssertionError(f'unexpected native input: {pending!r}')
                break
            else:
                assert 32 <= pending[0] < 127, 'control key reached composer'
                draft += chr(pending[0])
                pending = pending[1:]
            position = min(100, max(0, position))
            draw()
finally:
    termios.tcsetattr(0, termios.TCSANOW, old)
