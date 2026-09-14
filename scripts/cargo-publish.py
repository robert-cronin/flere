#!/usr/bin/env python3
"""Reconcile core crates.io publication with exact Git source; never uploads."""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import time
import tomllib
import urllib.error
import urllib.request
import zlib

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("cargo_verify", ROOT / "packaging/cargo/verify.py")
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)
MAX_CRATE = 10_000_000
INCLUDES = ["/src/**", "/build-support/**", "/tests/fixtures/local-image.png",
            "/tests/fixtures/local-image.jpg", "/packaging/cargo/README.md", "/LICENSE", "/Cargo.lock"]


def sha(data):
    return hashlib.sha256(data).hexdigest()


def identity(version, commit):
    if (not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version)
            or not re.fullmatch(r"[0-9a-f]{40}", commit)):
        raise ValueError("need the selected X.Y.Z version and full Git commit")


def source(checkout, version, commit):
    identity(version, commit)
    if (Path(verify.git(checkout, "rev-parse", "--show-toplevel").decode().strip()).resolve() != checkout.resolve()
            or verify.git(checkout, "rev-parse", "HEAD").decode().strip() != commit
            or verify.git(checkout, "status", "--porcelain", "--untracked-files=normal")):
        raise ValueError("Cargo publication requires the clean selected source")
    original = tomllib.loads(verify.git(checkout, "show", f"{commit}:Cargo.toml").decode())
    companion = tomllib.loads(verify.git(checkout, "show", f"{commit}:companion/Cargo.toml").decode())
    if (original["package"]["name"] != "flere" or original["package"]["version"] != version
            or companion["package"]["version"] != version
            or original["package"].get("include") != INCLUDES):
        raise ValueError("selected source is not the maintained core package layout/version")
    return original


def normalized_manifest(original):
    """Check Cargo's current core normalization without rebuilding on a retry."""
    expected = copy.deepcopy(original)
    if any(key in expected for key in ("workspace", "lib", "bin", "example", "test", "bench")):
        raise ValueError("explicit Cargo targets/workspace need an updated source-package check")
    expected["package"].update({key: False for key in
                                ("autolib", "autobins", "autoexamples", "autotests", "autobenches")})
    expected["lib"] = {"name": "flere", "path": "src/lib.rs"}
    expected["bin"] = [{"name": "flere", "path": "src/main.rs"}]
    for table in [expected, *expected.get("target", {}).values()]:
        for section in ("dependencies", "build-dependencies", "dev-dependencies"):
            for name, dependency in table.get(section, {}).items():
                if isinstance(dependency, str):
                    table[section][name] = {"version": dependency}
                elif any(key in dependency for key in ("path", "git", "workspace", "registry")):
                    raise ValueError("non-crates.io dependencies need an updated source-package check")
    return expected


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def public_read(url, maximum, missing=False):
    # Callers construct only fixed crates.io API/index/static URLs. No credential
    # provider, token, API response URL or redirect is used for these reads.
    request = urllib.request.Request(url, headers={"User-Agent": "flere-cargo-release",
                                                  "Cache-Control": "no-cache"})
    try:
        with urllib.request.build_opener(NoRedirect()).open(request, timeout=30) as response:
            data = response.read(maximum + 1)
    except urllib.error.HTTPError as error:
        code = error.code
        error.close()
        if code == 404 and missing:
            return None
        raise ValueError(f"public registry read returned HTTP {code}") from None
    if not 0 < len(data) <= maximum:
        raise ValueError("public registry response exceeds its size bound")
    return data


class PendingRegistry(ValueError):
    pass


def registry_version(version, read=public_read):
    raw = read(f"https://crates.io/api/v1/crates/flere/{version}", 1_000_000, missing=True)
    index = read("https://index.crates.io/fl/er/flere", 2_000_000, missing=True)
    matches = []
    for line in (index or b"").splitlines():
        record = json.loads(line)
        if record.get("vers") == version:
            matches.append(record)
    if len(matches) > 1:
        raise ValueError("duplicate core version in registry index")
    if raw is None and not matches:
        return None
    if raw is None or not matches:
        raise PendingRegistry("registry API and index visibility differ; retry without publishing")
    api, entry = json.loads(raw)["version"], matches[0]
    if (api["crate"] != "flere" or api["num"] != version or entry["name"] != "flere"
            or not re.fullmatch(r"[0-9a-f]{64}", api["checksum"])
            or entry["cksum"] != api["checksum"]
            or api["yanked"] is not False or entry["yanked"] is not False
            or type(api["crate_size"]) is not int or not 0 < api["crate_size"] <= MAX_CRATE):
        raise ValueError("registry version/checksum/availability differs")
    return {"sha256": api["checksum"], "bytes": api["crate_size"]}


def visible_version(version, required=False, read=public_read, sleep=time.sleep):
    for attempt in range(6):
        try:
            result = registry_version(version, read)
            if result is not None or not required:
                return result
        except PendingRegistry:
            pass
        if attempt < 5:
            sleep(3)
    raise ValueError("registry visibility did not settle; retry this job without changing the release")


def inspect_crate(data, expected, checkout, version, commit, output, original):
    if len(data) != expected["bytes"] or sha(data) != expected["sha256"]:
        raise ValueError("downloaded crate differs from the registry checksum/size")
    decoder = zlib.decompressobj(31)
    unpacked = decoder.decompress(data, 40_000_001)
    if (len(unpacked) > 40_000_000 or decoder.unconsumed_tail
            or not decoder.eof or decoder.unused_data):
        raise ValueError("crate gzip content exceeds bounds or contains trailing data")
    crate = output / f"flere-{version}.crate"
    crate.write_bytes(data)
    extraction = output / "extracted"
    contents = verify.inspect(crate, checkout, commit, version, extraction)
    tracked = verify.git(checkout, "ls-tree", "-rz", "--name-only", commit).decode().split("\0")
    wanted = {name for name in tracked if name.startswith(("src/", "build-support/"))
              or name in {"LICENSE", "packaging/cargo/README.md", "tests/fixtures/local-image.png",
                          "tests/fixtures/local-image.jpg", "Cargo.lock"}}
    wanted |= {"Cargo.toml", "Cargo.toml.orig", ".cargo_vcs_info.json"}
    if set(contents["files"]) != wanted or contents["vcs"].get("path_in_vcs") != "":
        raise ValueError("published crate inventory differs from the complete core source")
    if tomllib.loads((extraction / "Cargo.toml").read_text()) != normalized_manifest(original):
        raise ValueError("normalized Cargo manifest differs from selected source")
    lock = tomllib.loads(verify.git(checkout, "show", f"{commit}:Cargo.lock").decode())
    if tomllib.loads((extraction / "Cargo.lock").read_text()) != lock:
        raise ValueError("packaged Cargo lock differs from selected source")
    return {"crate_sha256": expected["sha256"], "crate_bytes": len(data),
            "source_files_verified": len(wanted), "source_commit": commit}


def reconcile(checkout, version, commit, output, *, required=False, local_crate=None,
              read=public_read, sleep=time.sleep):
    original = source(checkout, version, commit)
    normalized_manifest(original)  # Reject unsupported layout before a new upload.
    expected = visible_version(version, required, read, sleep)
    if expected is None:
        return {"status": "not_published", "version": version, "source_commit": commit}
    if local_crate is not None:
        if local_crate.is_symlink() or not local_crate.is_file() or local_crate.stat().st_size > MAX_CRATE:
            raise ValueError("normal Cargo publish did not retain a regular bounded archive")
        local = local_crate.read_bytes()
        if len(local) != expected["bytes"] or sha(local) != expected["sha256"]:
            raise ValueError("registry checksum differs from the archive produced by Cargo publish")
    data = read(f"https://static.crates.io/crates/flere/flere-{version}.crate", MAX_CRATE)
    proof = inspect_crate(data, expected, checkout, version, commit, output, original)
    source(checkout, version, commit)
    return {"status": "published_verified", "version": version, **proof}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("check", "verify"))
    parser.add_argument("--checkout", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--local-crate", type=Path)
    args = parser.parse_args()
    checkout, output = args.checkout.resolve(), args.output.resolve()
    if not output.is_relative_to(Path.home().resolve()) or output.is_relative_to(checkout):
        parser.error("proof output must be outside the source and under private home cache")
    if args.phase == "check":
        output.mkdir(mode=0o700, parents=True, exist_ok=False)
    elif args.local_crate is None or not output.is_dir():
        parser.error("verify requires the existing proof directory and normal Cargo output archive")
    result = reconcile(checkout, args.version, args.commit, output,
                       required=args.phase == "verify", local_crate=args.local_crate)
    (output / "receipt.json").write_text(json.dumps(result, indent=2) + "\n")
    if os.environ.get("GITHUB_OUTPUT"):
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as stream:
            stream.write(f"needs_publish={'true' if result['status'] == 'not_published' else 'false'}\n")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(os.environ["GITHUB_STEP_SUMMARY"]).open("a") as stream:
            stream.write("Core crates.io result:\n\n```json\n" + json.dumps(result, indent=2) + "\n```\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, zlib.error) as error:
        print(f"Core registry reconciliation stopped: {json.dumps(str(error))}", file=sys.stderr)
        sys.exit(1)
