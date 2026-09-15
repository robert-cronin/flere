#!/usr/bin/env python3
"""Combine pinned Linux/Windows candidates as data; never build, execute or publish."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
TARGET = "x86_64-pc-windows-msvc"
PAYLOAD = "flere-connect-" + TARGET
CHECKS = ["01-toolchain", "02-rustc", "03-cargo", "04-fetch", "05-fmt", "06-clippy",
          "07-tests", "08-release", "09-package", "10-flat-assets"] + [
              f"portable-{alias}-{flag}" for alias in ("flere", "flere-connect")
              for flag in ("build-info", "version", "help")]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


windows = load("aggregate_windows", ROOT / "scripts/windows-manifests.py")
candidate = load("aggregate_candidate", ROOT / "packaging/windows/hosted-candidate.py")


def require(value, message):
    if not value:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def read(path, maximum, *, empty=False):
    require(not path.is_symlink() and path.is_file() and path.stat().st_size <= maximum,
            "invalid bounded aggregate input: " + path.name)
    raw = path.read_bytes()
    require(len(raw) <= maximum and (empty or raw), "aggregate input size differs: " + path.name)
    return raw


def zip_name(version):
    require(windows.release_version(version), "invalid Windows release version")
    return f"flere-connect-{version}-{TARGET}.zip"


def names(version):
    return {PAYLOAD, PAYLOAD + ".manifest.json", zip_name(version)}


def verify_windows(directory, version, commit, source, license_bytes):
    manifest, _, _ = windows.verify(directory, TARGET)
    require(manifest["build"]["package_version"] == version
            and manifest["source"]["git_commit"] == commit
            and manifest["source"]["source_sha256"] == source,
            "Windows source/version differs from the Linux release")
    path = directory / zip_name(version)
    read(path, windows.MAX_PAYLOAD)
    candidate.verify_zip(path, manifest, license_bytes)
    return manifest


def acceptance(evidence, pinned, version, manifest):
    require(set(evidence) == {"checks", "payload_sha256", "zip_sha256", "runner"}
            and evidence["checks"] == CHECKS
            and evidence["payload_sha256"] == pinned[PAYLOAD]["sha256"]
            and evidence["zip_sha256"] == pinned[zip_name(version)]["sha256"],
            "Windows acceptance differs from final payload/ZIP")
    runner = evidence["runner"]
    require(set(runner) == {"os", "architecture", "image", "image_version", "rustc"}
            and runner["os"] == "windows-2025" and runner["architecture"] == "x86_64"
            and isinstance(runner["image"], str) and re.fullmatch(r"win25(?:-vs[0-9]{4})?", runner["image"])
            and isinstance(runner["image_version"], str)
            and re.fullmatch(r"[A-Za-z0-9_.+-]{1,100}", runner["image_version"])
            and isinstance(runner["rustc"], str)
            and re.fullmatch(r"rustc 1\.98\.0 [A-Za-z0-9 ().+-]{1,100}", runner["rustc"])
            and runner["rustc"] == manifest["build"]["rustc"].splitlines()[0],
            "Windows native runner/compiler evidence differs")


def normalize_candidate(directory, receipt, version, commit, run_id, workflow_sha, manifest, zip_bytes):
    require(receipt["schema"] == "flere-windows-candidate-v1"
            and receipt["status"] == "prepared_not_published"
            and receipt["commit"] == commit and receipt["version"] == version
            and receipt["target"] == TARGET and receipt["workflow_sha"] == workflow_sha
            and receipt["run_id"] == run_id
            and isinstance(receipt["run_attempt"], str) and re.fullmatch(r"[1-9][0-9]*", receipt["run_attempt"])
            and receipt["source_unchanged"] is True and receipt["stateless_state_unchanged"] is True
            and type(receipt["stateless_checks"]) is int and receipt["stateless_checks"] == 6
            and receipt["source_sha256"] == manifest["source"]["source_sha256"]
            and receipt["build"] == manifest["build"]
            and receipt["payload_sha256"] == manifest["payload"]["sha256"],
            "Windows candidate source/native acceptance differs")
    archive = {"name": zip_name(version), "bytes": len(zip_bytes), "sha256": sha(zip_bytes)}
    require(receipt["zip"] == archive
            and receipt["generated_files"][archive["name"]] == {k: archive[k] for k in ("bytes", "sha256")},
            "Windows ZIP differs from pinned native candidate")
    expected = list(CHECKS)
    for tool, extra in (("winget", ["winget-version", "winget-validate"]),
                        ("chocolatey", ["chocolatey-version", "chocolatey-pack"])):
        # Catalogue tooling remains separate from portable ZIP acceptance. A
        # recorded failure never becomes a pass; an unavailable tool stays so.
        require(receipt[tool]["status"] in ("passed", "blocked"), "candidate tool result is incomplete")
        if receipt[tool]["status"] == "passed":
            expected.extend(extra)
    require([row["name"] for row in receipt["checks"]] == expected, "candidate native check inventory differs")
    for row in receipt["checks"]:
        require(row["status"] == "passed" and type(row["exit"]) is int and row["exit"] == 0
                and type(row["log_bytes"]) is int and 0 <= row["log_bytes"] <= 16 * 1024 * 1024,
                "candidate native check failed or exceeded bounds")
        data = read(directory / "logs" / (row["name"] + ".log"), 16 * 1024 * 1024, empty=True)
        require(len(data) == row["log_bytes"] and sha(data) == row["log_sha256"], "candidate check log differs")
        if row["name"] == "02-rustc":
            require(data.decode().replace("\r\n", "\n").strip() == manifest["build"]["rustc"].strip()
                    and "host: " + TARGET in data.decode(), "native compiler log differs")
        if row["name"].endswith("-build-info"):
            require(json.loads(data) == manifest["build"], "portable alias build identity differs")
    machine = receipt["machine"]
    require(machine["machine"] in ("AMD64", "x86_64")
            and isinstance(machine["system"], str) and machine["system"].startswith("Windows-"),
            "candidate is not native Windows x86-64")
    # Never copy private command argv, paths, logs or the entire candidate into
    # release.json. Only fixed check names, hashes and restricted runner data.
    evidence = {"checks": list(CHECKS), "payload_sha256": manifest["payload"]["sha256"],
                "zip_sha256": archive["sha256"], "runner": {
                    "os": "windows-2025", "architecture": "x86_64", "image": machine["image_os"],
                    "image_version": machine["image_version"], "rustc": manifest["build"]["rustc"].splitlines()[0]}}
    acceptance(evidence, {PAYLOAD: {"sha256": evidence["payload_sha256"]},
                          archive["name"]: {"sha256": archive["sha256"]}}, version, manifest)
    return evidence


def prepare(linux_directory, windows_directory, output, version, commit, run_id, workflow_sha,
            linux_descriptor_sha256, windows_receipt_sha256):
    release = load("aggregate_release", ROOT / "scripts/release-automation.py")
    release.identity(version, commit, run_id)
    release.identity(version, workflow_sha, run_id)
    require(all(isinstance(pin, str) and re.fullmatch(r"[0-9a-f]{64}", pin)
                for pin in (linux_descriptor_sha256, windows_receipt_sha256)), "trusted input digests required")
    cache = Path.home() / ".cache/flere/tmp"
    output = output.absolute()
    require(output.is_relative_to(cache) and output != cache and output.resolve() == output
            and not output.exists() and not output.is_symlink()
            and not any(output.is_relative_to(path.absolute()) for path in (linux_directory, windows_directory)),
            "aggregate output must be a fresh separate private HOME-cache path")
    require(not linux_directory.is_symlink() and not windows_directory.is_symlink(), "candidate directory is a symlink")
    linux = release.validate(linux_directory, version, commit, run_id, workflow_sha, linux_descriptor_sha256)
    descriptor = json.loads(release.read(linux_directory / "release.json", 65536))
    require(descriptor["schema_version"] == 2, "aggregate requires the complete verified Linux/Debian candidate")
    raw = read(windows_directory / "candidate.json", 1024 * 1024)
    require(sha(raw) == windows_receipt_sha256, "Windows receipt differs from trusted digest")
    receipt = json.loads(raw)
    zip_path = windows_directory / "windows" / zip_name(version)
    zip_bytes = read(zip_path, windows.MAX_PAYLOAD)
    _, _, licenses = release.debian_inputs(linux_directory, version, commit, descriptor["source_sha256"],
                                           descriptor["source_archive"])
    with zipfile.ZipFile(zip_path) as archive:
        # Check the member bound before reading any ZIP-controlled manifest.
        items = archive.infolist()
        require(len(items) == 4 and {x.filename for x in items} == {"flere.exe", "flere-connect.exe", "manifest.json", "LICENSE"},
                "Windows ZIP inventory differs")
        require(archive.getinfo("manifest.json").file_size <= 65536, "Windows manifest bound exceeded")
        raw_manifest = archive.read("manifest.json")
        manifest = json.loads(raw_manifest)
    candidate.verify_zip(zip_path, manifest, licenses["LICENSE"])
    # Stage beneath the same private parent; only a fully validated eleven-file
    # set is renamed to output. An invalid input never leaves a success artifact.
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="aggregate-", dir=output.parent) as temporary:
        stage = Path(temporary)
        for name in sorted(release.candidate_names(version, 2)):
            data = release.read(linux_directory / name)
            require({"bytes": len(data), "sha256": sha(data)} == linux["files"][name], "Linux input changed after validation")
            (stage / name).write_bytes(data)
        (stage / zip_name(version)).write_bytes(zip_bytes)
        with zipfile.ZipFile(stage / zip_name(version)) as archive:
            (stage / PAYLOAD).write_bytes(archive.read("flere-connect.exe"))
        (stage / (PAYLOAD + ".manifest.json")).write_bytes(raw_manifest)
        verify_windows(stage, version, commit, descriptor["source_sha256"], licenses["LICENSE"])
        evidence = {"linux": descriptor["evidence"],
                    "windows": normalize_candidate(windows_directory, receipt, version, commit, run_id,
                                                    workflow_sha, manifest, zip_bytes),
                    "inputs": {"linux_descriptor_sha256": linux_descriptor_sha256,
                               "windows_receipt_sha256": windows_receipt_sha256}}
        sealed = release.seal(stage, version, commit, run_id, workflow_sha, evidence, schema=3)
        require(not output.exists() and not output.is_symlink(), "aggregate output appeared during verification")
        stage.rename(output)
    return sealed


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("linux-directory", "windows-directory", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    for name in ("version", "commit", "run-id", "workflow-sha", "linux-descriptor-sha256", "windows-receipt-sha256"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    result = prepare(**vars(args))
    if destination := os.environ.get("GITHUB_OUTPUT"):
        with open(destination, "a") as stream:
            stream.write(f"asset_directory={args.output.absolute()}\ndescriptor_sha256={result['descriptor_sha256']}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
        print(f"Release aggregation stopped: {error}", file=sys.stderr)
        sys.exit(1)
