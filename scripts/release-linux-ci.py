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
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("release_automation", Path(__file__).with_name("release-automation.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def run(args, checkout, *, env=None):
    return subprocess.check_output(list(map(str, args)), cwd=checkout, env=env, text=True)


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
    result = json.loads(run([sys.executable, checkout / "scripts/dev", "package", "--with-companion"], checkout))
    package_root = Path(result["artifact"])
    subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"], cwd=checkout, check=True)
    assets = work / "assets"
    assets.mkdir(mode=0o700)
    evidence = {"checks": release.CHECKS, "payloads": {}, "elf_needed": {}, "runner": {
        "os": "ubuntu-24.04", "architecture": platform.machine(),
        "image": os.environ["ImageOS"], "image_version": os.environ["ImageVersion"],
        "rustc": rustc, "linker": run(["ld", "--version"], checkout).splitlines()[0], "glibc": "2.39"}}
    # Only these six final public files are uploaded. Build logs, local paths and
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
    sealed = release.seal(assets, args.version, args.commit, args.run_id, args.workflow_sha, evidence)
    if os.environ.get("GITHUB_OUTPUT"):
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
            output.write(f"asset_directory={assets}\ndescriptor_sha256={sealed['descriptor_sha256']}\n")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as summary:
            summary.write(f"Linux candidate `{args.version}` from `{args.commit}`\n\n")
            summary.write(f"Release descriptor SHA-256: `{sealed['descriptor_sha256']}`\n\n")
            summary.write("Native CI covers build identity and managed install/reinstall. It does not establish interactive desktop or cross-version upgrade acceptance.\n\n")
            summary.write("| Target/channel | Status |\n| --- | --- |\n")
            for name, status in {**release.BLOCKED, **release.CHANNELS}.items():
                summary.write(f"| {name} | {status} |\n")
    print(json.dumps(sealed, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"Linux release checks stopped: {error}", file=sys.stderr)
        sys.exit(1)
