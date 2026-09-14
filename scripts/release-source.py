#!/usr/bin/env python3
"""Create/inspect a full public source archive. Inspection never extracts or executes it."""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib
import zlib

MAX_ARCHIVE = 64 * 1024 * 1024
MAX_CONTENT = 128 * 1024 * 1024
MAX_FILE = 32 * 1024 * 1024
MAX_FILES = 4096
MAX_TAR = MAX_CONTENT + MAX_FILES * 1024 + 10240
FINGERPRINT = "flere-source-v1"
REQUIRED = {"Cargo.toml", "Cargo.lock", "companion/Cargo.toml", "companion/Cargo.lock",
            "src/main.rs", "src/lib.rs", "companion/src/main.rs", "build-support/build.rs",
            "LICENSE", "src/assets/fonts/OFL.txt", "src/assets/fonts/LICENSE-Nerd-Fonts",
            "src/assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf"}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def name(version):
    if not isinstance(version, str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("source archive needs an exact X.Y.Z version")
    return f"flere-{version}-source.tar.gz"


def relative_path(path):
    if (not path or "\\" in path or any(ord(c) < 32 or ord(c) == 127 for c in path)
            or any(part in ("", ".", "..") or part.lower() == ".git" for part in path.split("/"))):
        raise ValueError("unsafe source archive path")
    return path.encode("utf-8")


def fingerprint(entries):
    """Match src/install/package.rs::source_digest, including regular-file type bits.

    Each record is u64 BE path byte length, UTF-8 path, u32 BE Unix st_mode,
    lowercase ASCII SHA-256 of contents, then NUL. This is not the gzip digest.
    """
    digest = hashlib.sha256()
    for path in sorted(entries, key=lambda p: p.encode("utf-8")):
        entry = entries[path]
        encoded = relative_path(path)
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        digest.update((0o100000 | entry["mode"]).to_bytes(4, "big"))
        digest.update(entry["sha256"].encode("ascii"))
        digest.update(b"\0")
    return digest.hexdigest()


def package_inputs(entries, documents, version):
    if not REQUIRED <= entries.keys():
        raise ValueError("source archive is missing shared inputs or licenses")
    for prefix, component, build in (("", "flere", "build-support/build.rs"),
                                      ("companion/", "flere-connect", "../build-support/build.rs")):
        try:
            manifest = tomllib.loads(documents[prefix + "Cargo.toml"].decode())["package"]
            packages = tomllib.loads(documents[prefix + "Cargo.lock"].decode())["package"]
            own = [p for p in packages if p["name"] == component and "source" not in p]
            valid = (manifest["name"] == component and manifest["version"] == version
                     and manifest["rust-version"] == "1.98" and manifest["build"] == build
                     and len(own) == 1 and own[0]["version"] == version)
        except (KeyError, TypeError, UnicodeError, tomllib.TOMLDecodeError) as error:
            raise ValueError("invalid source package identity") from error
        if not valid:
            raise ValueError("source package version, lock or shared build path differs")


def inspect(path, version, expected_source):
    """Bound all decompression and parse only ordinary USTAR members, in memory.

    Parsing headers directly rejects hidden PAX/GNU extensions, concatenated
    archives and trailing material; no candidate paths reach the filesystem.
    """
    if path.name != name(version) or path.is_symlink() or not path.is_file():
        raise ValueError("invalid regular source archive file")
    with path.open("rb") as stream:
        compressed = stream.read(MAX_ARCHIVE + 1)
    if not 0 < len(compressed) <= MAX_ARCHIVE:
        raise ValueError("source archive exceeds compressed size limit")
    if compressed[:10] != b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x02\xff":
        raise ValueError("source gzip metadata is not normalized")
    try:
        decoder = zlib.decompressobj(31)
        raw = decoder.decompress(compressed, MAX_TAR + 1)
    except zlib.error as error:
        raise ValueError("invalid source gzip stream") from error
    if len(raw) > MAX_TAR or decoder.unconsumed_tail:
        raise ValueError("source archive exceeds expanded size limit")
    if not decoder.eof or decoder.unused_data:
        raise ValueError("truncated or trailing source gzip stream")
    entries, documents = {}, {}
    position, total, stamp = 0, 0, None
    prefix = f"flere-{version}/"
    previous = b""
    while position + 512 <= len(raw):
        header = raw[position:position + 512]
        position += 512
        if header == bytes(512):
            if len(raw) - position < 512 or any(raw[position:]) or len(raw) % 10240:
                raise ValueError("invalid source tar end or trailing data")
            break
        try:
            member = tarfile.TarInfo.frombuf(header, "utf-8", "strict")
        except (tarfile.TarError, UnicodeError, ValueError) as error:
            raise ValueError("invalid source tar header") from error
        if (header[257:265] != b"ustar\x0000" or member.type != tarfile.REGTYPE
                or member.mode not in (0o644, 0o755) or member.uid or member.gid
                or member.uname or member.gname or member.linkname
                or member.devmajor or member.devminor or not member.name.startswith(prefix)):
            raise ValueError("unsafe source archive member or metadata")
        if member.tobuf(format=tarfile.USTAR_FORMAT, encoding="utf-8", errors="strict") != header:
            raise ValueError("source tar header is not normalized")
        relative = member.name[len(prefix):]
        encoded = relative_path(relative)
        if encoded <= previous or len(entries) >= MAX_FILES:
            raise ValueError("duplicate, unsorted or excessive source archive members")
        previous = encoded
        if stamp is None:
            stamp = member.mtime
        if not 0 <= member.mtime < 8 ** 11 or member.mtime != stamp:
            raise ValueError("source archive timestamps differ")
        total += member.size
        if not 0 <= member.size <= MAX_FILE or total > MAX_CONTENT:
            raise ValueError("source archive exceeds member or content size limit")
        end = position + member.size
        padded = position + (member.size + 511) // 512 * 512
        if padded > len(raw) or any(raw[end:padded]):
            raise ValueError("truncated source archive member or nonzero padding")
        data = raw[position:end]
        entries[relative] = {"bytes": len(data), "mode": member.mode, "sha256": sha(data)}
        if relative in ("Cargo.toml", "Cargo.lock", "companion/Cargo.toml", "companion/Cargo.lock"):
            if len(data) > 65536:
                raise ValueError("source package document exceeds limit")
            documents[relative] = data
        position = padded
    else:
        raise ValueError("source tar end is missing")
    package_inputs(entries, documents, version)
    source = fingerprint(entries)
    if source != expected_source:
        raise ValueError("source archive fingerprint differs from release manifests")
    return {"file": path.name, "bytes": len(compressed), "sha256": sha(compressed),
            "source_sha256": source, "fingerprint_algorithm": FINGERPRINT,
            "files": len(entries), "content_bytes": total, "mtime": stamp}


def git(checkout, *args):
    # Read exact objects from this checkout, independent of inherited selectors,
    # replacement refs, global aliases or filesystem-monitor hooks. No source runs.
    environment = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    environment.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    return subprocess.check_output(["git", "--no-replace-objects", "-c", "core.fsmonitor=false", "-C", str(checkout), *args],
                                   env=environment, timeout=30)


def clean(checkout, commit):
    if (Path(os.fsdecode(git(checkout, "rev-parse", "--show-toplevel")).strip()).resolve() != checkout
            or git(checkout, "rev-parse", "HEAD").decode().strip() != commit
            or git(checkout, "status", "--porcelain", "--untracked-files=normal")):
        raise ValueError("source archive requires the clean selected checkout")


def create(checkout, output, version, commit, expected_source):
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("source archive needs a full selected commit")
    checkout = checkout.resolve()
    if output.name != name(version) or output.resolve().is_relative_to(checkout):
        raise ValueError("source archive output must be versioned and outside the checkout")
    clean(checkout, commit)
    entries, content, total = {}, {}, 0
    for record in git(checkout, "ls-tree", "-rlz", commit).split(b"\0"):
        if not record:
            continue
        metadata, encoded = record.split(b"\t", 1)
        mode, kind, oid, size = metadata.split()
        relative = encoded.decode("utf-8")
        relative_path(relative)
        if kind != b"blob" or mode not in (b"100644", b"100755"):
            raise ValueError("source Git tree must contain only regular files")
        total += int(size)
        if int(size) > MAX_FILE or total > MAX_CONTENT or len(entries) >= MAX_FILES:
            raise ValueError("source Git tree exceeds archive limits")
        data = git(checkout, "cat-file", "blob", oid.decode("ascii"))
        if len(data) != int(size):
            raise ValueError("source Git blob size differs")
        entries[relative] = {"bytes": len(data), "mode": int(mode, 8) & 0o777, "sha256": sha(data)}
        content[relative] = data
    package_inputs(entries, content, version)
    if fingerprint(entries) != expected_source:
        raise ValueError("selected Git tree fingerprint differs from release manifests")
    stamp = int(git(checkout, "show", "-s", "--format=%ct", commit))
    if not 0 <= stamp < 8 ** 11:
        raise ValueError("selected commit timestamp exceeds archive limits")
    # Exclusive creation never replaces an earlier candidate. Only this newly
    # created file is removed if subsequent validation detects a failure.
    with output.open("xb") as stream:
        try:
            with gzip.GzipFile(filename="", mode="wb", fileobj=stream, compresslevel=9, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w|", format=tarfile.USTAR_FORMAT) as archive:
                    for relative in sorted(entries, key=lambda p: p.encode("utf-8")):
                        member = tarfile.TarInfo(f"flere-{version}/{relative}")
                        member.mode = entries[relative]["mode"]
                        member.size = entries[relative]["bytes"]
                        member.mtime = stamp
                        archive.addfile(member, io.BytesIO(content[relative]))
            stream.flush()
            checked = inspect(output, version, expected_source)
            clean(checkout, commit)
        except BaseException:
            output.unlink()
            raise
    return checked


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("create", "inspect"))
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--source-sha256", required=True)
    parser.add_argument("--checkout", type=Path)
    parser.add_argument("--commit")
    args = parser.parse_args()
    if args.action == "create":
        if args.checkout is None or args.commit is None:
            parser.error("create requires --checkout and --commit")
        result = create(args.checkout, args.archive, args.version, args.commit, args.source_sha256)
    else:
        result = inspect(args.archive, args.version, args.source_sha256)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
