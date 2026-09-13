# Harmless native-terminal contract fixture; no model/harness is launched.
# Exact startup probe format: Codex rust-v0.154.0 tui/src/terminal_probe.rs.
import os
import re
import select
import termios
import time
import tty
from pathlib import Path

original = termios.tcgetattr(0)
try:
    tty.setraw(0)
    os.write(1, b"\x1b[6n\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[?u\x1b[c")
    reply = b""
    deadline = time.monotonic() + 0.100
    while time.monotonic() < deadline:
        if not select.select([0], [], [], max(0, deadline - time.monotonic()))[0]:
            break
        reply += os.read(0, 4096)
        if b"\x1b]10;rgb:" in reply and b"\x1b]11;rgb:" in reply and reply.endswith(b"c"):
            break
    Path("probe-replies.bin").write_bytes(reply)
    colors = re.findall(rb"\x1b\](10|11);rgb:([0-9a-f]{4})/([0-9a-f]{4})/([0-9a-f]{4})\x1b\\", reply)
    if len(colors) != 2:
        raise RuntimeError("Missing startup default-color responses")
    bg = next([int(c, 16) // 257 for c in value[1:]] for value in colors if value[0] == b"11")
    # Codex's dark-background message tint: blend 12% white over the queried bg.
    shade = [int(c * 0.88 + 255 * 0.12) for c in bg]
    sgr = ("\x1b[48;2;" + ";".join(map(str, shade)) + "m").encode()
    output = (b"\x1b[0m\x1b[2J\x1b[HDEFAULT_TEXT\r\n"
              b"\x1b[2mDIM_TEXT\x1b[22m\r\n"
              b"\x1b[3mITALIC_TEXT\x1b[23m\r\n"
              b"\x1b[1;4;9mSTYLED_TEXT\x1b[0m\r\n" + sgr +
              b"\x1b[2KSHADED_PROMPT\x1b[0m\r\n"
              b"\x1b[38:2::100:150:200mRGB_TEXT\x1b[0m\r\nCOLOR_QUERY_OK")
    Path("probe-output.bin").write_bytes(output)
    os.write(1, output)
    # Keep the scene stable for snapshot/refresh checks. Exit is fixture-controlled.
    os.read(0, 1)
finally:
    termios.tcsetattr(0, termios.TCSANOW, original)
    os.write(1, b"\r\nPROBE_RESTORED\r\n")
