"""Owned shell child for output-pressure correctness tests; no native models."""
import os
from pathlib import Path
import threading
import time
import tty

tty.setraw(0)
received = open('output-input', 'ab', buffering=0)
stopped = threading.Event()


def paint():
    number = 0
    while not stopped.is_set():
        width, height = os.get_terminal_size(1)
        char = bytes([33 + number % 90])
        out = bytearray(b'\x1b[?7l')
        for row in range(height):
            text = f'OUTPUT_{number:08d}'.encode().ljust(width, char) if row == 0 else char * width
            out.extend(f'\x1b[{row + 1};1H'.encode() + text)
        os.write(1, out)
        number += 1
        time.sleep(.02)


producer = threading.Thread(target=paint, daemon=True)
producer.start()
Path('output-ready').touch()
seen = b''
while True:
    data = os.read(0, 4096)
    if not data:
        break
    received.write(data)
    seen = (seen + data)[-128:]
    if b'END_OUTPUT;' in seen and not stopped.is_set():
        stopped.set()
        producer.join()
        width, height = os.get_terminal_size(1)
        final = bytearray(b'\x1b[?7l\x1b[?25l\x1b[2J')
        for row in range(height):
            text = ('FINAL_OUTPUT' if row == 0 else f'row-{row:03d}').ljust(width, chr(65 + row % 26))
            final.extend(f'\x1b[{row + 1};1H\x1b[38;2;{40 + row};100;200m\x1b[48;2;15;20;25m{text}'.encode())
        os.write(1, final)
