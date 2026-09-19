"""Owned, disposable supervisor fixtures for offline evaluations; no native models."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import time


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def frame(data):
    return struct.pack(">I", len(data)) + data


def read_exact(stream, size):
    chunks = bytearray()
    while len(chunks) < size:
        data = stream.recv(size - len(chunks))
        if not data:
            raise EOFError("fixture socket closed mid-frame")
        chunks.extend(data)
    return bytes(chunks)


def read_frame(stream):
    size, = struct.unpack(">I", read_exact(stream, 4))
    if size > 12 * 1024 * 1024:
        raise ValueError("fixture response exceeds protocol bound")
    return read_exact(stream, size)


def distribution(values):
    import math
    ordered = sorted(values)
    return {"samples": len(ordered), **{
        name: ordered[max(0, math.ceil(len(ordered) * p) - 1)]
        for name, p in [("p50_ms", .5), ("p95_ms", .95), ("p99_ms", .99)]
    }, "max_ms": ordered[-1]}


def save_report(destination, report):
    path = Path(destination).expanduser().resolve()
    cache = (Path.home() / ".cache").resolve()
    if not path.is_relative_to(cache):
        raise ValueError("evaluation output must be under the user's home cache")
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    path.write_bytes(json.dumps(report, indent=2).encode() + b"\n")
    path.chmod(0o600)


class Fixture:
    def __init__(self, binary):
        self.binary = str(Path(binary).resolve(strict=True))
        parent = Path.home() / ".cache/flere/evals"
        parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.root = Path(tempfile.mkdtemp(prefix="probe-", dir=parent))
        self.state = self.root / "s"
        self.env = {"HOME": str(self.root), "PATH": "/usr/bin:/bin", "SHELL": "/bin/sh",
                    "ENV": "", "PS1": "$ ", "HISTFILE": "/dev/null", "TERM": "xterm-256color",
                    "LANG": "C.UTF-8", "TMPDIR": str(self.root)}
        self.log = (self.root / "supervisor.log").open("wb")
        self.server = None
        try:
            self.server = subprocess.Popen([self.binary, "--state", str(self.state), "serve"],
                                           env=self.env, stdin=subprocess.DEVNULL,
                                           stdout=self.log, stderr=self.log)
            deadline = time.monotonic() + 10
            while True:
                try:
                    self.request("ping")
                    break
                except (OSError, EOFError):
                    if self.server.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("owned supervisor did not become ready")
                    time.sleep(.01)
        except BaseException:
            self.close()
            raise

    def connect(self):
        stream = socket.socket(socket.AF_UNIX)
        stream.settimeout(5)
        try:
            stream.connect(str(self.state / "control.sock"))
        except BaseException:
            stream.close()
            raise
        return stream

    def request(self, *fields):
        with self.connect() as stream:
            stream.sendall(frame("\t".join(map(str, fields)).encode()))
            data = read_frame(stream)
            if data.startswith(b"!"):
                raise RuntimeError(data[1:].decode())
            return data

    def json(self, *fields):
        return json.loads(self.request(*fields))

    def card(self, name, shell=False):
        return self.json("new" if shell else "new-stopped", name.encode().hex(),
                         str(self.root).encode().hex())["workspace"]

    def tab(self, workspace):
        return next(w for w in self.json("list")["workspaces"] if w["id"] == workspace)["tabs"][-1]

    def input(self, tab, data):
        self.request("input", tab["id"], tab["run"], data.hex())

    def metadata(self, workspace, notes):
        self.request("metadata", workspace, encode({"notes": notes}).hex())

    def provenance(self):
        with open(self.binary, "rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
        return {"binary_sha256": digest,
                "build": json.loads(subprocess.check_output([self.binary, "--build-info"], env=self.env, timeout=5)),
                "platform": os.uname().sysname, "architecture": os.uname().machine}

    def close(self):
        if self.server is not None and self.server.poll() is None:
            try:
                self.request("stop")
                self.server.wait(timeout=5)
            except (OSError, EOFError, RuntimeError, subprocess.TimeoutExpired):
                self.server.terminate()
                try:
                    self.server.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.server.kill()
                    self.server.wait()
        self.log.close()
        shutil.rmtree(self.root)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
