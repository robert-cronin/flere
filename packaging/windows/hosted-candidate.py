#!/usr/bin/env python3
"""One nonpublishing Windows/MSVC candidate from the reviewed public commit."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import traceback
import xml.etree.ElementTree as ET
import zipfile

sys.dont_write_bytecode = True
PROJECT = Path(__file__).resolve().parents[2]
COMMIT = "3401a77193821b2c33127d2001239fb983a3712b"
VERSION = "0.3.4"
TARGET = "x86_64-pc-windows-msvc"
TOOLCHAIN = "1.98.0"
MAX_LOG = 16 * 1024 * 1024


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), PROJECT / "scripts" / (name + ".py"))
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def hosted(env):
    for key, expected in {
        "GITHUB_ACTIONS": "true", "FLERE_RUNNER_ENVIRONMENT": "github-hosted",
        "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REPOSITORY": "robert-cronin/flere",
        "GITHUB_REF": "refs/heads/main", "RUNNER_OS": "Windows", "RUNNER_ARCH": "X64",
    }.items():
        require(env.get(key) == expected, "requires the manual hosted Windows job: " + key)
    require(re.fullmatch("[0-9a-f]{40}", env.get("FLERE_WORKFLOW_SHA", "")), "workflow SHA missing")
    require(all(re.fullmatch("[1-9][0-9]*", env.get(key, ""))
                for key in ("GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT")), "run identity missing")


def source_snapshot(checkout):
    """Check actual checkout bytes against Git blobs, not only clean status/CRLF filters."""
    source = module("release-source")
    source.clean(checkout, COMMIT)
    subprocess.run(["git", "-C", str(checkout), "merge-base", "--is-ancestor", COMMIT,
                    "refs/remotes/origin/main"], check=True, stdin=subprocess.DEVNULL,
                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=30)
    entries, documents, total = {}, {}, 0
    for record in source.git(checkout, "ls-tree", "-rlz", COMMIT).split(b"\0"):
        if not record:
            continue
        metadata, raw_name = record.split(b"\t", 1)
        mode, kind, oid, size = metadata.split()
        name = raw_name.decode("utf-8")
        source.relative_path(name)
        require(kind == b"blob" and mode in (b"100644", b"100755"), "nonregular Git source")
        total += int(size)
        require(int(size) <= source.MAX_FILE and total <= source.MAX_CONTENT
                and len(entries) < source.MAX_FILES, "source inventory exceeds bound")
        path = checkout / name
        require(not path.is_symlink() and path.is_file() and path.stat().st_size == int(size),
                "checkout file type/size differs: " + name)
        data = path.read_bytes()
        require(data == source.git(checkout, "cat-file", "blob", oid.decode()),
                "checkout bytes differ from selected Git blob: " + name)
        entries[name] = {"bytes": len(data), "mode": int(mode, 8) & 0o777, "sha256": sha(data)}
        if name in ("Cargo.toml", "Cargo.lock", "companion/Cargo.toml", "companion/Cargo.lock"):
            documents[name] = data
    source.package_inputs(entries, documents, VERSION)
    return {"commit": COMMIT, "version": VERSION, "source_sha256": source.fingerprint(entries),
            "fingerprint_algorithm": source.FINGERPRINT, "entries": entries}


def verify_zip(path, manifest, license_bytes):
    expected = {"flere.exe", "flere-connect.exe", "manifest.json", "LICENSE"}
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        require(len(names) == len(expected) and set(names) == expected, "ZIP inventory differs")
        for item in archive.infolist():
            require(item.file_size <= 256 * 1024 * 1024 and not item.is_dir()
                    and item.date_time == (1980, 1, 1, 0, 0, 0), "ZIP metadata differs")
            mode = item.external_attr >> 16
            require(mode == (0o100755 if item.filename.endswith(".exe") else 0o100644), "ZIP mode differs")
        for name in ("flere.exe", "flere-connect.exe"):
            data = archive.read(name)
            require(len(data) == manifest["payload"]["bytes"]
                    and sha(data) == manifest["payload"]["sha256"], "ZIP executable differs")
        require(json.loads(archive.read("manifest.json")) == manifest
                and archive.read("LICENSE") == license_bytes, "ZIP manifest/license differs")


def tree(path):
    result = {}
    for item in sorted(path.rglob("*")):
        require(not item.is_symlink(), "unexpected state symlink")
        result[item.relative_to(path).as_posix()] = sha(item.read_bytes()) if item.is_file() else "directory"
    return result


def verify_nupkg(path, install_script):
    require(path.name == "flere-connect." + VERSION + ".nupkg" and not path.is_symlink()
            and path.stat().st_size <= 1024 * 1024, "Chocolatey package identity/bound differs")
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        properties = [name for name in names if re.fullmatch(
            r"package/services/metadata/core-properties/[0-9a-fA-F]{32}\.psmdcp", name)]
        expected = {"_rels/.rels", "[Content_Types].xml", "flere-connect.nuspec", "tools/chocolateyInstall.ps1"}
        require(len(properties) == 1 and len(names) == 5 and set(names) == expected | set(properties),
                "Chocolatey inventory includes unexpected files")
        for item in archive.infolist():
            require(not item.is_dir() and item.file_size <= 256 * 1024
                    and ((item.external_attr >> 16) & 0o170000) in (0, 0o100000), "Chocolatey member type/bound differs")
        require(archive.read("tools/chocolateyInstall.ps1") == install_script, "Chocolatey install script differs")
        spec = ET.fromstring(archive.read("flere-connect.nuspec"))
        fields = {node.tag.rsplit("}", 1)[-1]: node.text for node in spec.findall("{*}metadata/{*}*")}
        require(fields.get("id") == "flere-connect" and fields.get("version") == VERSION,
                "Chocolatey package metadata differs")
        return {name: {"bytes": len(data := archive.read(name)), "sha256": sha(data)} for name in sorted(names)}


def run(checkout, output):
    hosted(os.environ)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64"), "native Windows AMD64 required")
    require(subprocess.check_output(["git", "-C", str(PROJECT), "rev-parse", "HEAD"], text=True).strip()
            == os.environ["FLERE_WORKFLOW_SHA"], "automation checkout differs")
    home = Path.home().resolve()
    require(output.parent.resolve() == home / ".cache/flere/tmp" and not output.exists(), "fresh HOME-cache output required")
    output.mkdir(mode=0o700)
    proof, work = output / "proof", output / "work"
    proof.mkdir(); work.mkdir(); (proof / "logs").mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    receipt = {"schema": "flere-windows-candidate-v1", "status": "running", "commit": COMMIT,
               "version": VERSION, "target": TARGET, "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"],
               "run_id": os.environ["GITHUB_RUN_ID"], "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"],
               "machine": {"system": platform.platform(), "machine": platform.machine(),
                           "image_os": os.environ.get("ImageOS"), "image_version": os.environ.get("ImageVersion")},
               "checks": [], "winget": {"status": "not_run"}, "chocolatey": {"status": "not_run"},
               "limits": ["Unpublished companion candidate; no native Windows core.",
                          "No physical clipboard, terminal graphics/keyboard, existing-chat draft or external SSH acceptance.",
                          "No package-manager install/upgrade/remove or verified manager ownership acceptance.",
                          "Generated release URLs are proposed, not downloaded or published."]}

    def save():
        (proof / "candidate.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def command(name, argv, env, seconds=300, cwd=checkout):
        log = proof / "logs" / (name + ".log")
        started = time.monotonic()
        item = {"name": name, "argv": list(map(str, argv)), "status": "running"}
        receipt["checks"].append(item); save()
        print("CHECK " + name, flush=True)
        failure = None
        with log.open("xb") as stream:
            proc = subprocess.Popen(list(map(str, argv)), cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT)
            try:
                while proc.poll() is None:
                    require(time.monotonic() - started <= seconds, name + " timed out")
                    require(log.stat().st_size <= MAX_LOG, name + " log exceeds bound")
                    time.sleep(0.1)
            except BaseException as error:
                failure = error
            finally:
                if proc.poll() is None:
                    # Popen retains the owned Windows process handle; only its
                    # process tree is terminated. The disposable VM is the final
                    # containment boundary on a failed build/tool invocation.
                    subprocess.run(["taskkill.exe", "/PID", str(proc.pid), "/T", "/F"],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=15, check=True)
                    proc.wait(timeout=15)
                item.update(exit=proc.returncode, seconds=round(time.monotonic() - started, 3))
        size = log.stat().st_size
        if size > MAX_LOG:
            with log.open("r+b") as stream:
                stream.truncate(MAX_LOG)
            item["log_dropped_bytes"] = size - MAX_LOG
            failure = failure or ValueError(name + " log exceeds bound")
        data = log.read_bytes()
        item.update(log_bytes=len(data), log_sha256=sha(data),
                    status="passed" if proc.returncode == 0 and failure is None else "failed")
        save()
        if failure is not None:
            raise failure
        require(proc.returncode == 0, name + " failed; see retained log")
        return data

    try:
        before = source_snapshot(checkout)
        (proof / "source.json").write_text(json.dumps(before, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        env = os.environ.copy()
        for name in ("GH_TOKEN", "GITHUB_TOKEN", "CARGO_REGISTRY_TOKEN", "CARGO_REGISTRIES_CRATES_IO_TOKEN",
                     "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
            env.pop(name, None)
        env.update(RUSTUP_TOOLCHAIN=TOOLCHAIN, CARGO_BUILD_JOBS="2", PYTHONDONTWRITEBYTECODE="1")
        # Keep compiler caches explicit before giving product tests private state.
        env["CARGO_HOME"] = str(Path(env.get("CARGO_HOME", home / ".cargo")))
        env["RUSTUP_HOME"] = str(Path(env.get("RUSTUP_HOME", home / ".rustup")))
        command("01-toolchain", ["rustup", "toolchain", "install", TOOLCHAIN, "--profile", "minimal",
                "--component", "rustfmt", "--component", "clippy", "--target", TARGET], env, 600)
        compiler = command("02-rustc", ["rustc", "-vV"], env).decode()
        require("release: " + TOOLCHAIN + "\n" in compiler.replace("\r\n", "\n")
                and "host: " + TARGET in compiler, "native pinned compiler differs")
        command("03-cargo", ["cargo", "--version"], env)
        command("04-fetch", ["cargo", "fetch", "--locked", "--manifest-path", "companion/Cargo.toml",
                             "--target", TARGET], env, 600)
        state = work / "state"; state.mkdir()
        for name, relative in {"HOME": "home", "USERPROFILE": "home", "LOCALAPPDATA": "local", "APPDATA": "roaming",
                               "XDG_CACHE_HOME": "cache", "XDG_CONFIG_HOME": "config", "XDG_DATA_HOME": "data",
                               "XDG_STATE_HOME": "state", "TEMP": "temp", "TMP": "temp"}.items():
            path = state / relative; path.mkdir(exist_ok=True)
            env[name] = str(path)
        env.update(CARGO_NET_OFFLINE="true", RUST_TEST_THREADS="4")
        common = ["--offline", "--locked", "--manifest-path", "companion/Cargo.toml", "--target", TARGET,
                  "--target-dir", str(work / "target")]
        command("05-fmt", ["cargo", "fmt", "--manifest-path", "companion/Cargo.toml", "--check"], env)
        command("06-clippy", ["cargo", "clippy", *common, "--all-targets", "--", "-D", "warnings"], env, 1200)
        command("07-tests", ["cargo", "test", *common, "--all-targets", "--", "--test-threads=4", "--nocapture"], env, 1200)
        command("08-release", ["cargo", "build", *common, "--release"], env, 1200)
        require(source_snapshot(checkout) == before, "source changed during checks")
        source_receipt = {"git_commit": COMMIT, "dirty": False, "source_sha256": before["source_sha256"],
                          "checks": [row["name"] for row in receipt["checks"] if row["status"] == "passed"]}
        source_path = proof / "source-receipt.json"
        source_path.write_text(json.dumps(source_receipt, indent=2) + "\n", encoding="utf-8")
        binary = work / "target" / TARGET / "release/flere-connect.exe"
        package = work / "package"
        command("09-package", [binary, "package", binary, package, source_path], env)
        manifest = json.loads((package / "manifest.json").read_bytes())
        require(manifest["source"] == source_receipt and manifest["build"]["component"] == "flere-connect"
                and manifest["build"]["package_version"] == VERSION and manifest["build"]["target"] == TARGET
                and manifest["build"]["profile"] == "release", "actual package identity differs")
        assets = work / "assets"
        command("10-flat-assets", [sys.executable, "-B", PROJECT / "scripts/release-assets.py", assets, package], env)
        windows = proof / "windows"
        generated = module("windows-manifests").prepare(assets, windows, TARGET, checkout / "LICENSE")
        archive_path = windows / generated["asset"]
        verify_zip(archive_path, manifest, (checkout / "LICENSE").read_bytes())
        extracted = work / "portable"; extracted.mkdir()
        with zipfile.ZipFile(archive_path) as archive:
            # Names and types were checked above; no general archive extraction.
            for name in ("flere.exe", "flere-connect.exe"):
                (extracted / name).write_bytes(archive.read(name))
        baseline = tree(state)
        for alias in ("flere", "flere-connect"):
            for flag in ("--build-info", "--version", "--help"):
                data = command("portable-" + alias + "-" + flag[2:], [extracted / (alias + ".exe"), flag], env)
                if flag == "--build-info":
                    require(json.loads(data) == manifest["build"], "portable build identity differs")
                elif flag == "--version":
                    require(data.decode().strip().startswith("flere-connect " + VERSION + " ")
                            and manifest["build"]["build_id"] in data.decode(), "portable version/build differs")
                else:
                    require(b"--build-info" in data and b"ssh" in data, "portable help differs")
                require(tree(state) == baseline, "stateless alias wrote application state")
        winget = shutil.which("winget.exe")
        if winget:
            version = command("winget-version", [winget, "--version"], os.environ.copy()).decode().strip()
            folder = windows / "winget/manifests/r/RobertCronin/FlereConnect" / VERSION
            command("winget-validate", [winget, "validate", str(folder), "--disable-interactivity"], os.environ.copy())
            receipt["winget"] = {"status": "passed", "version": version}
        else:
            receipt["winget"] = {"status": "blocked", "reason": "WinGet executable unavailable on this hosted image; no installation attempted."}
        choco = shutil.which("choco.exe")
        if choco:
            version = command("chocolatey-version", [choco, "--version"], os.environ.copy()).decode().strip()
            recipe = windows / "chocolatey/flere-connect"
            packed = windows / "chocolatey-package"; packed.mkdir()
            command("chocolatey-pack", [choco, "pack", str(recipe / "flere-connect.nuspec"),
                    "--outputdirectory", str(packed)], os.environ.copy(), cwd=recipe)
            package_path = packed / ("flere-connect." + VERSION + ".nupkg")
            require(list(packed.iterdir()) == [package_path], "Chocolatey output inventory differs")
            inventory = verify_nupkg(package_path, (recipe / "tools/chocolateyInstall.ps1").read_bytes())
            receipt["chocolatey"] = {"status": "passed", "version": version,
                                     "sha256": sha(package_path.read_bytes()), "inventory": inventory}
        else:
            receipt["chocolatey"] = {"status": "blocked", "reason": "Chocolatey executable unavailable on this hosted image; no installation attempted."}
        require(source_snapshot(checkout) == before, "source changed during packaging")
        receipt.update(status="prepared_not_published", source_sha256=before["source_sha256"],
                       source_unchanged=True, stateless_checks=6, stateless_state_unchanged=True,
                       build=manifest["build"], payload_sha256=manifest["payload"]["sha256"],
                       zip={"name": generated["asset"], "bytes": archive_path.stat().st_size,
                            "sha256": sha(archive_path.read_bytes())},
                       generated_files={p.relative_to(windows).as_posix(): {"bytes": p.stat().st_size, "sha256": sha(p.read_bytes())}
                                        for p in sorted(windows.rglob("*")) if p.is_file()})
    except BaseException as error:
        receipt.update(status="failed", error=str(error), traceback=traceback.format_exc())
        raise
    finally:
        save()
        print(json.dumps({key: receipt[key] for key in ("status", "commit", "version", "winget", "chocolatey")}), flush=True)
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkout", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    run(args.checkout.resolve(), args.output.resolve())
