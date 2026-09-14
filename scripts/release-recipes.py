#!/usr/bin/env python3
"""Prepare allowlisted package recipes from a pinned release; never publish or build."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import stat
import sys
import tarfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


release = load("recipe_release", ROOT / "scripts/release-automation.py")
packages = release.debian.packages
sys.path.insert(0, str(ROOT / "packaging/homebrew"))
try:
    homebrew = load("recipe_homebrew", ROOT / "packaging/homebrew/render.py")
finally:
    sys.path.pop(0)

RECIPE_FILES = frozenset({
    "linux/aur/flere-bin/PKGBUILD", "linux/aur/flere-bin/.SRCINFO",
    "linux/rpm/flere.spec", "linux/release-lock.json",
    "linux/package-provenance.json", "linux/SHA256SUMS",
    "homebrew/Formula/flere.rb", "homebrew/Formula/flere-connect.rb",
})
ARTIFACT_FILES = RECIPE_FILES | {"recipe-receipt.json", "SHA256SUMS"}


def inventory(directory, expected):
    paths = {}
    for path in directory.rglob("*"):
        mode = path.lstat().st_mode
        if stat.S_ISDIR(mode):
            continue
        if not stat.S_ISREG(mode):
            raise ValueError("recipe output must contain regular files only")
        paths[path.relative_to(directory).as_posix()] = path
    if set(paths) != expected:
        raise ValueError("recipe output allowlist differs")
    result = {}
    for name, path in sorted(paths.items()):
        data = release.read(path, 256 * 1024)
        result[name] = {"bytes": len(data), "sha256": release.sha(data)}
    return result


def prepare(directory, output, version, commit, run_id, workflow_sha, descriptor_sha256):
    if not isinstance(descriptor_sha256, str) or not re.fullmatch(r"[0-9a-f]{64}", descriptor_sha256):
        raise ValueError("recipe preparation requires a trusted descriptor SHA-256")
    cache = Path.home() / ".cache/flere/tmp"
    output = output.absolute()
    if (not output.is_relative_to(cache) or output == cache or output.resolve() != output
            or output.is_relative_to(directory.absolute())
            or any(ord(c) < 32 or ord(c) == 127 for c in str(output))):
        raise ValueError("recipe output must be a separate private home-cache path")
    if output.exists() or output.is_symlink():
        raise ValueError("recipe output already exists")
    release.validate(directory, version, commit, run_id, workflow_sha, descriptor_sha256)
    descriptor = json.loads(release.read(directory / "release.json", 65536))
    source_name = release.source_archive.name(version)
    source = descriptor["assets"][source_name]
    # Finish source/formula validation before creating any output.
    formulas = homebrew.recipes(directory / source_name, source["sha256"],
                                (ROOT / "packaging/homebrew/formula.rb.in").read_text())
    output.mkdir(mode=0o700, parents=True)
    with contextlib.redirect_stdout(io.StringIO()):
        packages.main(["--assets", str(directory), "--output", str(output / "linux"),
                       "--descriptor-sha256", descriptor_sha256])
    formula_directory = output / "homebrew/Formula"
    formula_directory.mkdir(parents=True)
    for component, text in formulas.items():
        path = formula_directory / f"{component}.rb"
        path.write_text(text)
        path.chmod(0o644)
    files = inventory(output, RECIPE_FILES)
    # Recheck the sealed inputs after both generators; no candidate is executed.
    release.validate(directory, version, commit, run_id, workflow_sha, descriptor_sha256)
    receipt = {"schema_version": 1, "status": "prepared_not_published",
               "repository": release.REPOSITORY, "version": version, "commit": commit,
               "run_id": run_id, "workflow_sha": workflow_sha,
               "release_descriptor_sha256": descriptor_sha256,
               "source_sha256": descriptor["source_sha256"],
               "source_archive": {"name": source_name, **source}, "files": files}
    (output / "recipe-receipt.json").write_bytes(release.json_bytes(receipt))
    checksummed = inventory(output, RECIPE_FILES | {"recipe-receipt.json"})
    (output / "SHA256SUMS").write_text("".join(
        f"{item['sha256']}  {name}\n" for name, item in checksummed.items()))
    inventory(output, ARTIFACT_FILES)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    for field in ("version", "commit", "run-id", "workflow-sha", "descriptor-sha256"):
        parser.add_argument("--" + field, required=True)
    args = parser.parse_args()
    receipt = prepare(args.directory, args.output, args.version, args.commit, args.run_id,
                      args.workflow_sha, args.descriptor_sha256)
    if path := os.environ.get("GITHUB_OUTPUT"):
        with open(path, "a") as output:
            output.write(f"directory={args.output.absolute()}\n")
    print(json.dumps({"status": receipt["status"], "version": receipt["version"],
                      "commit": receipt["commit"], "files": sorted(ARTIFACT_FILES)}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
        print(f"Release recipe preparation stopped: {error}", file=sys.stderr)
        sys.exit(1)
