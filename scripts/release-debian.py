#!/usr/bin/env python3
"""Bounded, data-only inspection of the maintained Debian wrapper; never installs."""
import importlib.util
import io
import lzma
from pathlib import Path
import re
import tarfile

spec = importlib.util.spec_from_file_location("linux_packages", Path(__file__).with_name("linux-packages.py"))
packages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packages)
MAX_DEB = 64 * 1024 * 1024
MAX_TAR = 128 * 1024 * 1024


def name(version):
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("Debian wrapper requires exact X.Y.Z")
    return f"flere_{version}-1_amd64.deb"


def ar_members(data, epoch):
    if not 8 < len(data) <= MAX_DEB or data[:8] != b"!<arch>\n":
        raise ValueError("invalid or oversized Debian ar archive")
    members, offset = {}, 8
    for expected in ("debian-binary", "control.tar.xz", "data.tar.xz"):
        header = data[offset:offset + 60]
        if len(header) != 60 or header[58:] != b"`\n" or header[:16].decode("ascii").strip() != expected:
            raise ValueError("Debian ar inventory/order differs")
        fields = [header[a:b].decode("ascii").strip() for a, b in ((16, 28), (28, 34), (34, 40), (40, 48), (48, 58))]
        if any(not re.fullmatch(r"[0-9]+", field) for field in fields):
            raise ValueError("invalid Debian ar metadata")
        timestamp, uid, gid, mode, size = fields
        if (int(timestamp) != epoch or int(uid) != 0 or int(gid) != 0
                or mode != "100644" or not 0 < int(size) <= MAX_DEB):
            raise ValueError("Debian ar owner/mode/time differs")
        size = int(size)
        offset += 60
        members[expected] = data[offset:offset + size]
        if len(members[expected]) != size:
            raise ValueError("truncated Debian ar member")
        offset += size
        if size % 2:
            if data[offset:offset + 1] != b"\n":
                raise ValueError("invalid Debian ar padding")
            offset += 1
    if offset != len(data) or members["debian-binary"] != b"2.0\n":
        raise ValueError("extra Debian ar content or unsupported format")
    return members


def inspect_tar(compressed, files, epoch):
    maximum = min(MAX_TAR, sum(len(data) for data, _ in files.values()) + 65536)
    decoder = lzma.LZMADecompressor(format=lzma.FORMAT_XZ, memlimit=64 * 1024 * 1024)
    try:
        raw = decoder.decompress(compressed, max_length=maximum + 1)
    except lzma.LZMAError as error:
        raise ValueError("invalid or excessive Debian XZ content") from error
    if len(raw) > maximum or not decoder.eof or decoder.unused_data or len(raw) % 512:
        raise ValueError("Debian tar size/stream bound differs")
    directories = {""}
    for path in files:
        parts = path.split("/")
        directories.update("/".join(parts[:count]) for count in range(1, len(parts)))
    seen, end = set(), 0
    try:
        with tarfile.open(fileobj=io.BytesIO(raw), mode="r:") as archive:
            for member in archive:
                path = member.name.removeprefix("./")
                if path == ".":
                    path = ""
                if (path in seen or len(seen) >= 64 or member.pax_headers or member.sparse is not None
                        or member.offset_data - member.offset != 512
                        or (member.uid, member.gid, member.uname, member.gname, member.mtime) != (0, 0, "root", "root", epoch)):
                    raise ValueError("Debian tar duplicate/metadata/ownership differs")
                seen.add(path)
                if member.isdir():
                    if path not in directories or member.mode != 0o755 or member.size:
                        raise ValueError("Debian directory inventory/mode differs")
                elif member.type == tarfile.REGTYPE and path in files:
                    expected, mode = files[path]
                    if member.mode != mode or member.size != len(expected) or archive.extractfile(member).read() != expected:
                        raise ValueError("Debian file bytes/mode differs from verified inputs")
                else:
                    raise ValueError("unexpected Debian file, hook or link")
                padded = (member.size + 511) // 512 * 512
                if any(raw[member.offset_data + member.size:member.offset_data + padded]):
                    raise ValueError("nonzero Debian tar padding")
                end = member.offset_data + padded
    except tarfile.TarError as error:
        raise ValueError("malformed Debian tar content") from error
    if seen != directories | set(files) or len(raw) - end < 1024 or any(raw[end:]):
        raise ValueError("Debian inventory/trailing content differs")
    return {path: {"bytes": len(data), "sha256": packages.digest(data), "mode": mode}
            for path, (data, mode) in sorted(files.items())}


def inputs(directory, version, commit, source_sha256, source_archive, pins):
    lock = packages.candidate_lock(directory, version, commit, source_sha256, source_archive, pins)
    binaries, licenses = packages.pinned_inputs(directory, lock)
    return lock, binaries, licenses


def inspect(path, lock, binaries, licenses):
    if path.name != name(lock["version"]) or lock["revision"] != 1:
        raise ValueError("Debian wrapper identity/revision differs")
    raw = packages.read_file(path, MAX_DEB)
    members = ar_members(raw, lock["source_date_epoch"])
    files = packages.deb_files(binaries, licenses)
    control = packages.deb_control_files(files, lock)
    controls = inspect_tar(members["control.tar.xz"], control, lock["source_date_epoch"])
    inventory = inspect_tar(members["data.tar.xz"], files, lock["source_date_epoch"])
    return {"file_name": path.name, "package": "flere", "version": lock["version"] + "-1",
            "architecture": "amd64", "source_commit": lock["source_commit"],
            "source_sha256": lock["source_sha256"], "control": controls, "files": inventory}
