#!/usr/bin/env python3
"""Native Linux candidate checks; run without publication or signing credentials."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_automation", Path(__file__).with_name("release-automation.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def run(args, checkout, *, env=None):
    return subprocess.check_output(list(map(str, args)), cwd=checkout, env=env, text=True)


def validation_tail(base, previous, environment):
    """Read only this invocation's new private candidate log, never a reported path."""
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    root = os.open(base, flags)
    try:
        metadata = os.fstat(root)
        if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
            raise ValueError("validation cache is not private and owned")
        created = {n for n in os.listdir(root) if re.fullmatch(r"candidate-[A-Za-z0-9_-]{8}", n)} - previous
        if len(created) != 1:
            raise ValueError("new validation candidate is ambiguous or missing")
        candidate = os.open(created.pop(), flags, dir_fd=root)
        try:
            metadata = os.fstat(candidate)
            if metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
                raise ValueError("validation candidate is not private and owned")
            descriptor = os.open("validation.log", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                                 dir_fd=candidate)
        finally:
            os.close(candidate)
    finally:
        os.close(root)
    with os.fdopen(descriptor, "rb") as log:
        metadata = os.fstat(log.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.getuid():
            raise ValueError("validation log is not an owned regular file")
        offset = max(0, metadata.st_size - 32768)
        log.seek(offset)
        raw = log.read(32768)
    if offset:
        # Drop a partial first line before redaction; never expose a clipped token.
        raw = raw.partition(b"\n")[2]
    text = raw.decode("utf-8", errors="replace")
    for key, value in environment.items():
        if value and re.search(r"TOKEN|SECRET|PASSWORD|PASSWD|CREDENTIAL|PRIVATE.?KEY|API.?KEY|ACCESS.?KEY|AUTH", key, re.I):
            text = text.replace(value, "[redacted]")
    text = re.sub(r"\b(?:gh[pousr]_|github_pat_)[A-Za-z0-9_]+", "[redacted]", text)
    text = re.sub(r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}", "[redacted]", text)
    text = re.sub(r"https?://[^/\s]+@", "https://[redacted]@", text)
    text = re.sub(r"-----BEGIN [^-]*PRIVATE KEY-----.*", "[redacted private key]", text, flags=re.S)
    lines = []
    for line in text.splitlines()[-60:]:
        if re.search(r"(?:authorization|password|passwd|token|secret|credential|api[_-]?key)\s*[:=]|\b(?:Bearer|Basic)\s+", line, re.I):
            line = "[redacted credential-bearing line]"
        # Literal prefixes prevent terminal escapes and GitHub workflow commands.
        line = "".join(c if c.isprintable() or c == "\t" else "?" for c in line)
        lines.append("validation | " + line[:500])
    while len("\n".join(lines)) > 16384:
        lines.pop(0)
    return "\n".join(lines)


def package_candidate(checkout, environment=None):
    environment = dict(os.environ if environment is None else environment)
    cache = Path(environment.get("XDG_CACHE_HOME", str(Path(environment["HOME"]) / ".cache")))
    base = cache / "flere/dev-updates"
    try:
        previous = set(os.listdir(base)) if base.exists() else set()
    except OSError:
        previous = None
    try:
        return json.loads(run([sys.executable, checkout / "scripts/dev", "package", "--with-companion"],
                              checkout, env=environment))
    except subprocess.CalledProcessError:
        try:
            if previous is None:
                raise ValueError("validation cache could not be observed")
            tail = validation_tail(base, previous, environment)
            print("Bounded validation.log tail (credential values redacted):", file=sys.stderr)
            print(tail or "validation | (no complete log lines available)", file=sys.stderr)
        except (OSError, ValueError):
            print("Validation failed; a unique safe log tail was unavailable.", file=sys.stderr)
        raise


def inspect_elf(text):
    versions = {tuple(map(int, version.split(".")))
                for version in re.findall(r"\bGLIBC_([0-9]+\.[0-9]+(?:\.[0-9]+)?)\b", text)}
    if not versions or max(versions) > (2, 39):
        raise ValueError("ELF GLIBC symbol requirements exceed the declared 2.39 minimum")


def needed_libraries(text):
    libraries = sorted(set(re.findall(r"\(NEEDED\).*Shared library: \[([^\]]+)\]", text)))
    if "libc.so.6" not in libraries or not set(libraries) <= release.LINUX_LIBRARIES:
        raise ValueError("ELF requires libraries outside the declared Linux baseline")
    return libraries


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--workflow-sha", required=True)
    parser.add_argument("--with-debian", action="store_true", help="activate only after native Debian lifecycle acceptance")
    args = parser.parse_args()
    checkout = args.checkout.resolve()
    release.identity(args.version, args.commit, args.run_id)
    if (sys.platform != "linux" or platform.machine() != "x86_64"
            or platform.freedesktop_os_release().get("ID") != "ubuntu"
            or platform.freedesktop_os_release().get("VERSION_ID") != "24.04"
            or platform.libc_ver() != ("glibc", "2.39")):
        raise ValueError("release acceptance requires native Ubuntu 24.04 x86-64 / glibc 2.39")
    if release.git(checkout, "rev-parse", "HEAD") != args.commit or release.git(checkout, "status", "--porcelain"):
        raise ValueError("release source must be the clean selected commit")
    rustc = run(["rustc", "--version"], checkout).strip()
    if not rustc.startswith("rustc 1.98.0 "):
        raise ValueError("release validation requires provisioned Rust 1.98.0")
    cache = Path.home() / ".cache/flere/tmp"
    cache.mkdir(parents=True, exist_ok=True, mode=0o700)
    work = Path(tempfile.mkdtemp(prefix="release-ci-", dir=cache))
    # scripts/dev writes its log and build receipts under private home cache. It
    # runs both components' fmt, offline strict Clippy, tests and release builds.
    result = package_candidate(checkout)
    package_root = Path(result["artifact"])
    subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"], cwd=checkout, check=True)
    assets = work / "assets"
    assets.mkdir(mode=0o700)
    evidence = {"checks": release.CHECKS, "payloads": {}, "elf_needed": {}, "runner": {
        "os": "ubuntu-24.04", "architecture": platform.machine(),
        "image": os.environ["ImageOS"], "image_version": os.environ["ImageVersion"],
        "rustc": rustc, "linker": run(["ld", "--version"], checkout).splitlines()[0], "glibc": "2.39"}}
    # Only the format's seven/eight final public files are uploaded. Build logs, local paths and
    # package receipts outside the two public manifests remain private to the job.
    with tempfile.TemporaryDirectory(prefix="release-install-", dir=cache) as isolated:
        acceptance_home = Path(isolated)
        environment = dict(os.environ, HOME=str(acceptance_home),
                           XDG_DATA_HOME=str(acceptance_home / "data"),
                           XDG_CACHE_HOME=str(acceptance_home / "cache"),
                           XDG_CONFIG_HOME=str(acceptance_home / "config"))
        core = package_root / "flere/flere"
        for component in release.COMPONENTS:
            package = package_root / component
            manifest = json.loads((package / "manifest.json").read_bytes())
            name = f"{component}-{release.TARGET}"
            payload = package / component
            inspect_elf(run(["readelf", "--version-info", payload], checkout))
            evidence["elf_needed"][component] = needed_libraries(run(["readelf", "--dynamic", payload], checkout))
            before = release.sha(release.read(payload))
            if json.loads(run([payload, "--build-info"], checkout, env=environment)) != manifest["build"]:
                raise ValueError("native executable identity differs from its final manifest")
            command = [core, "--state", acceptance_home / "state", "install", package]
            if component == "flere-connect":
                command += ["--component", component]
            # An exact-payload reinstall exercises managed installation without
            # claiming an upgrade from a different version or a running session.
            for _ in range(2):
                run(command, checkout, env=environment)
                installed = acceptance_home / ".local/bin" / component
                if release.sha(release.read(installed)) != before:
                    raise ValueError("managed installation changed release bytes")
                if json.loads(run([installed, "--build-info"], checkout, env=environment)) != manifest["build"]:
                    raise ValueError("installed executable identity differs")
            if (acceptance_home / "state").exists():
                raise ValueError("installation unexpectedly started application state")
            if release.sha(release.read(payload)) != before:
                raise ValueError("payload changed during native acceptance")
            shutil.copyfile(payload, assets / name)
            shutil.copyfile(package / "manifest.json", assets / (name + ".manifest.json"))
            evidence["payloads"][component] = before
    if release.git(checkout, "status", "--porcelain"):
        raise ValueError("validation changed selected source or a lockfile")
    release.source_archive.create(checkout, assets / release.source_archive.name(args.version),
                                  args.version, args.commit, result["source"]["source_sha256"])
    schema = 1
    if args.with_debian:
        source = release.manifests(assets, args.version, args.commit)
        archive = release.source_archive.inspect(assets / release.source_archive.name(args.version), args.version, source)
        lock, binaries, licenses = release.debian_inputs(assets, args.version, args.commit, source, archive)
        package = release.debian.packages.build_deb(assets, binaries, licenses, lock)
        release.debian.inspect(package, lock, binaries, licenses)
        schema = 2
    # Sealing repeats Debian inspection independently before binding its digest.
    sealed = release.seal(assets, args.version, args.commit, args.run_id, args.workflow_sha, evidence, schema)

    if os.environ.get("GITHUB_OUTPUT"):
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
            output.write(f"asset_directory={assets}\ndescriptor_sha256={sealed['descriptor_sha256']}\n")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
            summary.write(f"Linux candidate `{args.version}` from `{args.commit}`\n\n")
            summary.write(f"Release descriptor SHA-256: `{sealed['descriptor_sha256']}`\n\n")
            summary.write("Native CI covers build identity and managed install/reinstall. It does not establish interactive desktop or cross-version upgrade acceptance.\n\n")
            summary.write("| Target/channel | Status |\n| --- | --- |\n")
            channels = release.DEBIAN_CHANNELS if schema == 2 else release.CHANNELS
            for name, status in {**release.BLOCKED, **channels}.items():
                summary.write(f"| {name} | {status} |\n")
    print(json.dumps(sealed, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"Linux release checks stopped: {error}", file=sys.stderr)
        sys.exit(1)
