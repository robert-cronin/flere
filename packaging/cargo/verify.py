#!/usr/bin/env python3
"""Verify a clean, core-only Cargo source package without publishing/installing."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import tomllib


def sha(data):
    return hashlib.sha256(data).hexdigest()


def run(argv, cwd, env, log):
    with log.open("wb") as stream:
        result = subprocess.run(argv, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"{argv[0]} failed ({result.returncode}); see {log}")
    return log.read_bytes()


def git(root, *args):
    return subprocess.check_output(["git", "-C", str(root), *args])


def inspect(crate, root, commit, version, extraction):
    if not 0 < crate.stat().st_size <= 10_000_000:
        raise RuntimeError("crate exceeds the conservative 10 MB limit")
    prefix = f"flere-{version}"
    files = {}
    total = 0
    modes = {}
    for entry in git(root, "ls-tree", "-rz", commit).split(b"\0"):
        if entry:
            metadata, name = entry.split(b"\t", 1)
            modes[name.decode()] = int(metadata.split()[0], 8) & 0o777
    generated = {"Cargo.toml", "Cargo.lock", ".cargo_vcs_info.json"}
    required = {
        *generated, "Cargo.toml.orig", "LICENSE", "src/main.rs", "src/lib.rs",
        "src/sixel.rs", "build-support/build.rs", "packaging/cargo/README.md",
        "src/assets/fonts/OFL.txt", "src/assets/fonts/LICENSE-Nerd-Fonts",
        "tests/fixtures/local-image.png", "tests/fixtures/local-image.jpg",
    }
    with tarfile.open(crate, "r:gz") as archive:
        for member in archive:
            path = PurePosixPath(member.name)
            if (not member.isfile() or path.is_absolute() or ".." in path.parts
                    or len(path.parts) < 2 or path.parts[0] != prefix
                    or member.mode & 0o7022):
                raise RuntimeError(f"unsafe archive member: {member.name}")
            relative = str(PurePosixPath(*path.parts[1:]))
            if relative in files or len(files) >= 2048:
                raise RuntimeError("duplicate member or excessive archive entry count")
            total += member.size
            if total > 32_000_000:
                raise RuntimeError("excessive uncompressed archive size")
            allowed = (relative.startswith(("src/", "build-support/"))
                       or relative in required)
            if not allowed:
                raise RuntimeError(f"unexpected packaged file: {relative}")
            data = archive.extractfile(member).read()
            if len(data) != member.size:
                raise RuntimeError("truncated archive member")
            if relative not in generated:
                source = "Cargo.toml" if relative == "Cargo.toml.orig" else relative
                if data != git(root, "show", f"{commit}:{source}"):
                    raise RuntimeError(f"archive differs from recorded Git source: {relative}")
                if member.mode != modes.get(source):
                    raise RuntimeError(f"archive mode differs from Git source: {relative}")
            destination = extraction / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(data)
            destination.chmod(member.mode & 0o777)
            files[relative] = {"bytes": len(data), "sha256": sha(data), "mode": oct(member.mode)}
    if not required.issubset(files):
        raise RuntimeError(f"missing required files: {sorted(required - files.keys())}")
    vcs = json.loads((extraction / ".cargo_vcs_info.json").read_bytes())
    if vcs.get("git", {}).get("sha1") != commit or vcs["git"].get("dirty", False):
        raise RuntimeError("Cargo source receipt is missing, dirty, or names another commit")
    manifest = tomllib.loads((extraction / "Cargo.toml").read_text())
    if manifest["package"]["name"] != "flere" or manifest["package"]["version"] != version:
        raise RuntimeError("normalized package identity differs")
    return {"files": files, "uncompressed_bytes": total, "vcs": vcs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="new private proof directory under your home")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    output = args.output.expanduser().resolve()
    if not output.is_relative_to(Path.home().resolve()) or output.is_relative_to(root):
        parser.error("output must be outside the checkout and under your home")
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    commit = git(root, "rev-parse", "HEAD").decode().strip()
    if Path(git(root, "rev-parse", "--show-toplevel").decode().strip()).resolve() != root:
        raise RuntimeError("run from a real Flere Git checkout")
    version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(output / "target")
    # Stateless smoke commands cannot observe an inherited live Flere instance.
    for key, name in [("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"),
                      ("XDG_DATA_HOME", "data")]:
        env[key] = str(output / name)
    env.pop("FLERE", None)
    run(["cargo", "package", "--offline", "--locked"], root, env, output / "package.log")
    crate = output / f"flere-{version}.crate"
    shutil.copyfile(output / "target" / "package" / crate.name, crate)
    extraction = output / "extracted"
    contents = inspect(crate, root, commit, version, extraction)
    env["CARGO_TARGET_DIR"] = str(output / "extracted-target")
    run(["cargo", "test", "--offline", "--locked", "--lib", "--", "--test-threads=1"],
        extraction, env, output / "unit.log")
    build = ["cargo", "build", "--offline", "--locked", "--release"]
    run(build, extraction, env, output / "build.log")
    binary = output / "extracted-target" / "release" / "flere"
    reported = run([str(binary), "--version"], output, env, output / "version.log")
    if not reported.decode().startswith(f"flere {version} ("):
        raise RuntimeError("version command does not match the crate")
    help_text = run([str(binary), "--help"], output, env, output / "help.log")
    if b"Flere" not in help_text or b"--build-info" not in help_text:
        raise RuntimeError("help command is incomplete")
    before = json.loads(run([str(binary), "--build-info"], output, env, output / "build-info.json"))
    if (before.get("component") != "flere" or before.get("package_version") != version
            or before.get("identity_kind") != "cargo_generation_stamp"):
        raise RuntimeError("build metadata does not identify the packaged core")
    run(build, extraction, env, output / "cached-build.log")
    after = json.loads(run([str(binary), "--build-info"], output, env, output / "cached-build-info.json"))
    if before != after:
        raise RuntimeError("unchanged extracted build rotated its generation stamp")
    if any((output / part).exists() for part in ("state/flere/control.sock", "state/flere/supervisor.log")):
        raise RuntimeError("stateless validation unexpectedly created a runtime")
    receipt = {"schema_version": 1, "status": "passed", "source_commit": commit,
               "crate": {"name": crate.name, "bytes": crate.stat().st_size,
                         "sha256": sha(crate.read_bytes())}, "archive": contents,
               "build_info": before, "cached_generation_unchanged": True,
               "published": False, "installed": False}
    receipt_path = output / "receipt.json"
    receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n")
    receipt_path.chmod(0o600)
    print(receipt_path)


if __name__ == "__main__":
    main()
