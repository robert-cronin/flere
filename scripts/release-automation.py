#!/usr/bin/env python3
"""Select, seal and promote exact release bytes. Never executes release payloads.

Only the explicit publish subcommand writes GitHub. Use the workflow's trusted
copy of this script, not a script downloaded with a candidate artifact.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.parse
import urllib.request

sys.dont_write_bytecode = True
source_spec = importlib.util.spec_from_file_location("release_source", Path(__file__).with_name("release-source.py"))
source_archive = importlib.util.module_from_spec(source_spec)
source_spec.loader.exec_module(source_archive)

REPOSITORY = "robert-cronin/flere"
TARGET = "x86_64-unknown-linux-gnu"
COMPONENTS = ("flere", "flere-connect")
LIMIT = 256 * 1024 * 1024
CHECKS = ["core_offline_checks", "companion_offline_checks", "distribution_fixtures",
          "native_build_identity", "managed_install_and_reinstall", "elf_glibc_2_39"]
LINUX_LIBRARIES = {"libgcc_s.so.1", "libc.so.6", "libm.so.6", "libz.so.1",
                   "ld-linux-x86-64.so.2", "libpthread.so.0", "libdl.so.2", "librt.so.1"}
BLOCKED = {
    "aarch64-apple-darwin": "blocked: Developer ID, accepted notarization and quarantined download acceptance required",
    "x86_64-apple-darwin": "blocked: native Intel acceptance, Developer ID and accepted notarization required",
    "x86_64-pc-windows-gnu": "blocked: physical Windows companion acceptance required",
}
# Exact frozen channel snapshot used before the separate Cargo job.
LEGACY_CHANNELS = {
    "managed_latest": "blocked: latest-release pointer is not advanced by this workflow",
    "homebrew": "prepared: source formula validation and tap writer required",
    "crates_io": "blocked: first publication and trusted publisher enrollment required",
    "debian_aur": "blocked: reviewed new release lock and native package-manager acceptance required",
    "scoop_winget": "blocked: physical Windows acceptance, catalogue validation and publisher setup required",
}

CHANNELS = {**LEGACY_CHANNELS,
    "crates_io": "pending: separate core Cargo job follows verified GitHub publication"}


def identity(version, commit, run_id):
    if not isinstance(version, str) or not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", version):
        raise ValueError("version must be an unused X.Y.Z without a v prefix")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("commit must be a full lowercase 40-character Git SHA")
    if not isinstance(run_id, str) or not re.fullmatch(r"[1-9][0-9]*", run_id):
        raise ValueError("run ID must be a positive decimal GitHub run ID")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path, maximum=LIMIT):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= maximum:
        raise ValueError(f"invalid regular release file: {path.name}")
    data = path.read_bytes()
    if not 0 < len(data) <= maximum:
        raise ValueError(f"release file exceeds limit: {path.name}")
    return data


def json_bytes(value):
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode()


def payload_names():
    return {f"{component}-{TARGET}{suffix}" for component in COMPONENTS
            for suffix in ("", ".manifest.json")}


def candidate_names(version):
    return payload_names() | {source_archive.name(version)}


def allowlist(version):
    return candidate_names(version) | {"release.json", "SHA256SUMS"}


def git(checkout, *args):
    return subprocess.check_output(["git", "-C", str(checkout), *args], text=True).strip()


def select(checkout, version, commit, run_id, workflow_sha, api):
    identity(version, commit, run_id)
    identity(version, workflow_sha, run_id)
    if os.environ.get("GITHUB_REF") != "refs/heads/main" or os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch":
        raise ValueError("Release must be dispatched from the trusted main workflow")
    if os.environ.get("GITHUB_REPOSITORY") != REPOSITORY:
        raise ValueError("release workflow is restricted to the public upstream repository")
    if git(checkout, "rev-parse", "HEAD") != workflow_sha:
        raise ValueError("automation checkout differs from the workflow commit")
    for selected in (commit, workflow_sha):
        if git(checkout, "cat-file", "-t", selected) != "commit":
            raise ValueError("selected SHA is not a commit")
        subprocess.run(["git", "-C", str(checkout), "merge-base", "--is-ancestor", selected, "origin/main"], check=True)
    for manifest in ("Cargo.toml", "companion/Cargo.toml"):
        package = tomllib.loads(git(checkout, "show", f"{commit}:{manifest}"))["package"]
        if package["version"] != version:
            raise ValueError("both Cargo package versions must equal the requested unused version")
    # Reject even a draft or a tag without a release. Rerun failed jobs to resume
    # the same candidate; starting the whole workflow again must not rebuild it.
    if api.get(f"git/ref/tags/v{version}", missing=True) is not None or api.get(f"releases/tags/v{version}", missing=True) is not None:
        raise ValueError("version already has a tag or release; do not overwrite or relabel it")
    return {"version": version, "commit": commit, "run_id": run_id, "workflow_sha": workflow_sha}


def manifests(directory, version, commit):
    builds = {}
    sources = set()
    for component in COMPONENTS:
        name = f"{component}-{TARGET}"
        data = read(directory / name)
        item = json.loads(read(directory / (name + ".manifest.json"), 65536))
        build, source, payload = item["build"], item["source"], item["payload"]
        if (item["schema_version"] != 1 or build["component"] != component
                or build["package_version"] != version or build["target"] != TARGET
                or build["profile"] != "release" or source["git_commit"] != commit
                or source["dirty"] is not False
                or not re.fullmatch(r"[0-9a-f]{64}", source["source_sha256"])
                or payload["file_name"] != component or payload["download_file"] != name
                or payload["bytes"] != len(data) or payload["sha256"] != sha(data)):
            raise ValueError("wrong-source, stale checksum or mismatched release manifest")
        if data[:6] != b"\x7fELF\x02\x01" or data[18:20] != b"\x3e\x00":
            raise ValueError("only native x86-64 Linux ELF payloads are eligible")
        builds[component] = build
        sources.add(source["source_sha256"])
    protocol = builds["flere"]["compatibility"]["remote_protocol"]["current"]
    accepts = builds["flere-connect"]["compatibility"]["remote_protocol"]["accepts"]
    if len(sources) != 1 or not isinstance(protocol, str) or not protocol or not isinstance(accepts, list) or protocol not in accepts:
        raise ValueError("core/companion source or remote protocol mismatch")
    return sources.pop()


def acceptance(evidence, pinned):
    if (set(evidence) != {"checks", "payloads", "runner", "elf_needed"}
            or evidence["checks"] != CHECKS
            or evidence["payloads"] != {c: pinned[f"{c}-{TARGET}"]["sha256"] for c in COMPONENTS}
            or set(evidence["runner"]) != {"os", "architecture", "image", "image_version", "rustc", "linker", "glibc"}
            or evidence["runner"]["os"] != "ubuntu-24.04"
            or evidence["runner"]["architecture"] != "x86_64"
            or evidence["runner"]["glibc"] != "2.39"
            or not evidence["runner"]["rustc"].startswith("rustc 1.98.0 ")
            or set(evidence["elf_needed"]) != set(COMPONENTS)):
        raise ValueError("missing native acceptance or evidence differs from final payload digests")
    for libraries in evidence["elf_needed"].values():
        if (not isinstance(libraries, list) or not libraries or libraries != sorted(set(libraries))
                or not set(libraries) <= LINUX_LIBRARIES or "libc.so.6" not in libraries):
            raise ValueError("native acceptance has unsupported required ELF libraries")


def seal(directory, version, commit, run_id, workflow_sha, evidence):
    identity(version, commit, run_id)
    identity(version, workflow_sha, run_id)
    if {p.name for p in directory.iterdir()} != candidate_names(version):
        raise ValueError("candidate must contain exactly the Linux pair, manifests and full source archive")
    source = manifests(directory, version, commit)
    archive = source_archive.inspect(directory / source_archive.name(version), version, source)
    pinned = {name: {"bytes": len(read(directory / name)), "sha256": sha(read(directory / name))}
              for name in sorted(candidate_names(version))}
    acceptance(evidence, pinned)
    descriptor = {"schema_version": 1, "repository": REPOSITORY, "version": version,
                  "commit": commit, "workflow_sha": workflow_sha, "run_id": run_id,
                  "source_sha256": source, "source_archive": archive, "assets": pinned, "evidence": evidence,
                  "targets": {TARGET: "native CI accepted; interactive desktop acceptance is not claimed", **BLOCKED},
                  "channels": CHANNELS, "minimum_glibc": "2.39"}
    (directory / "release.json").write_bytes(json_bytes(descriptor))
    (directory / "SHA256SUMS").write_text("".join(
        f"{sha(read(directory / name))}  {name}\n" for name in sorted(candidate_names(version) | {"release.json"})))
    return validate(directory, version, commit, run_id, workflow_sha)


def validate(directory, version, commit, run_id, workflow_sha, expected_digest=None):
    identity(version, commit, run_id)
    identity(version, workflow_sha, run_id)
    if directory.is_symlink() or {p.name for p in directory.iterdir()} != allowlist(version):
        raise ValueError("release asset allowlist differs: no extra files, paths or targets are allowed")
    raw = read(directory / "release.json", 65536)
    if expected_digest is not None and sha(raw) != expected_digest:
        raise ValueError("release descriptor digest differs from this run's sealed candidate")
    descriptor = json.loads(raw)
    for key, value in {"schema_version": 1, "repository": REPOSITORY, "version": version,
                       "commit": commit, "run_id": run_id, "workflow_sha": workflow_sha}.items():
        if descriptor.get(key) != value:
            raise ValueError("release provenance differs from selected commit/workflow/run/version")
    if set(descriptor["assets"]) != candidate_names(version) or descriptor["minimum_glibc"] != "2.39":
        raise ValueError("descriptor asset allowlist or Linux support contract differs")
    if descriptor["source_sha256"] != manifests(directory, version, commit):
        raise ValueError("descriptor source differs from manifests")
    for name, expected in descriptor["assets"].items():
        data = read(directory / name)
        if expected != {"bytes": len(data), "sha256": sha(data)}:
            raise ValueError("final bytes differ from sealed release assets")
    archive = source_archive.inspect(directory / source_archive.name(version), version, descriptor["source_sha256"])
    if descriptor.get("source_archive") != archive:
        raise ValueError("source archive provenance differs from sealed descriptor")
    expected_sums = "".join(f"{sha(read(directory / name))}  {name}\n"
                            for name in sorted(candidate_names(version) | {"release.json"}))
    if read(directory / "SHA256SUMS", 65536).decode() != expected_sums:
        raise ValueError("SHA256SUMS differs from final release bytes")
    acceptance(descriptor["evidence"], descriptor["assets"])
    # Historical snapshots are accepted only with the caller's exact digest.
    channels = (CHANNELS, LEGACY_CHANNELS) if expected_digest is not None else (CHANNELS,)
    if (descriptor["targets"] != {TARGET: "native CI accepted; interactive desktop acceptance is not claimed", **BLOCKED}
            or descriptor["channels"] not in channels):
        raise ValueError("missing target acceptance or unexpected channel readiness")
    return {"descriptor_sha256": sha(raw), "files": {
        name: {"bytes": len(read(directory / name)), "sha256": sha(read(directory / name))}
        for name in sorted(allowlist(version))}}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class PublicRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        check_public_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def check_public_url(url):
    parts = urllib.parse.urlsplit(url)
    if (parts.scheme != "https" or parts.username or parts.password or parts.port
            or parts.hostname not in {"github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"}):
        raise ValueError("release download left the allowlisted HTTPS hosts")


def public_download(url):
    check_public_url(url)
    # New unauthenticated request: never forward the API token to a signed CDN URL.
    with urllib.request.build_opener(PublicRedirect()).open(url, timeout=120) as response:
        data = response.read(LIMIT + 1)
    if not 0 < len(data) <= LIMIT:
        raise ValueError("download exceeds release size limit")
    return data


class GitHub:
    def __init__(self, token):
        if not token:
            raise ValueError("GitHub API token is required")
        self.token = token

    def request(self, method, path, data=None, *, missing=False, binary=False, upload=False):
        host = "uploads.github.com" if upload else "api.github.com"
        url = f"https://{host}/repos/{REPOSITORY}/{path}"
        headers = {"Authorization": f"Bearer {self.token}", "Accept": "application/octet-stream" if binary else "application/vnd.github+json",
                   "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "flere-release"}
        if data is not None:
            headers["Content-Type"] = "application/octet-stream" if upload else "application/json"
            if not upload:
                data = json_bytes(data)
        request = urllib.request.Request(url, data=data, headers=headers, method=method)
        try:
            with urllib.request.build_opener(NoRedirect()).open(request, timeout=120) as response:
                result = response.read(LIMIT + 1)
        except urllib.error.HTTPError as error:
            if error.code == 404 and missing:
                return None
            if error.code in (301, 302, 303, 307, 308) and binary and method == "GET":
                return public_download(error.headers["Location"])
            raise ValueError(f"GitHub {method} {path} returned HTTP {error.code}; reconcile before retry") from None
        if len(result) > LIMIT:
            raise ValueError("GitHub response exceeds limit")
        return result if binary else json.loads(result)

    def get(self, path, **kwargs):
        return self.request("GET", path, **kwargs)


def release_body(version, commit, run_id, descriptor_sha):
    return (f"Flere {version}\n\nSource commit: `{commit}`\n"
            f"Release descriptor SHA-256: `{descriptor_sha}`\n"
            f"Build run: https://github.com/{REPOSITORY}/actions/runs/{run_id}\n\n"
            "Linux x86-64 core and companion plus the complete reviewed source archive; glibc 2.39 or newer for these binaries. Native CI checks are recorded in release.json. "
            "Interactive desktop acceptance is not claimed. macOS signed binaries and Windows companion distribution remain blocked. "
            "Linux asset publication does not advance package-manager channels or the latest-release pointer. "
            "The separate core Cargo job follows public verification.\n")


def verify_tag(api, version, commit, *, missing=False, wait_for_visibility=False):
    attempts = 6 if wait_for_visibility else 1
    for attempt in range(attempts):
        # Only a 404 becomes None. Other API failures and mismatched visible
        # tags still fail immediately; this loop never repeats a mutation.
        ref = api.get(f"git/ref/tags/v{version}", missing=missing or wait_for_visibility)
        if ref is not None:
            break
        if not wait_for_visibility:
            return
        if attempt == attempts - 1:
            raise ValueError("published version tag remained unavailable after 6 checks; reconcile before retry")
        time.sleep(2)
    # This publisher creates lightweight tags only; never reinterpret a foreign
    # annotated tag or rewrite a tag to make an existing draft fit this run.
    if ref["object"] != {"type": "commit", "sha": commit, "url": f"https://api.github.com/repos/{REPOSITORY}/git/commits/{commit}"}:
        raise ValueError("version tag differs from the exact selected source commit")


def remote_assets(api, release_id, expected, *, complete):
    assets = api.get(f"releases/{release_id}/assets?per_page=100")
    names = [asset["name"] for asset in assets]
    if len(names) != len(set(names)) or not set(names) <= set(expected) or (complete and set(names) != set(expected)):
        raise ValueError("draft/public asset allowlist differs; never delete or overwrite to repair it")
    for asset in assets:
        wanted = expected[asset["name"]]
        if asset["state"] != "uploaded" or asset["size"] != wanted["bytes"]:
            raise ValueError("existing remote asset is incomplete or has the wrong size")
        data = api.get(f"releases/assets/{asset['id']}", binary=True)
        if {"bytes": len(data), "sha256": sha(data)} != wanted:
            raise ValueError("existing remote asset bytes differ; never overwrite a name")
    return set(names)


def publish(directory, version, commit, run_id, workflow_sha, expected_digest, api):
    # Validate every local file before the first external mutation. No payload is
    # executed in this job, and every retry consumes the same retained artifact.
    candidate = validate(directory, version, commit, run_id, workflow_sha, expected_digest)
    expected = candidate["files"]
    body = release_body(version, commit, run_id, expected_digest)
    release = api.get(f"releases/tags/v{version}", missing=True)
    if release is None:
        if api.get(f"git/ref/tags/v{version}", missing=True) is not None:
            raise ValueError("version tag already exists without this run's draft")
        release = api.request("POST", "releases", {
            "tag_name": f"v{version}", "target_commitish": commit, "name": f"Flere {version}",
            "body": body, "draft": True, "prerelease": False, "make_latest": "false"})
    if (release["tag_name"] != f"v{version}" or release["target_commitish"] != commit
            or release["body"] != body or release["prerelease"] is not False):
        raise ValueError("existing release belongs to another candidate; refuse to adopt it")
    verify_tag(api, version, commit, missing=release["draft"])
    if release["draft"]:
        present = remote_assets(api, release["id"], expected, complete=False)
        for name in sorted(set(expected) - present):
            data = read(directory / name)
            if {"bytes": len(data), "sha256": sha(data)} != expected[name]:
                raise ValueError("local artifact changed after validation")
            api.request("POST", f"releases/{release['id']}/assets?name={urllib.parse.quote(name)}", data, upload=True)
        remote_assets(api, release["id"], expected, complete=True)
        verify_tag(api, version, commit, missing=True)
        release = api.request("PATCH", f"releases/{release['id']}", {"draft": False, "make_latest": "false"})
    # A retry after an ambiguous publish response verifies the same immutable
    # release; it never rebuilds, replaces assets, or moves the tag/latest pointer.
    if release["draft"] or release.get("immutable") is not True:
        raise ValueError("publication did not return an immutable release; stop and inspect repository settings")
    verify_tag(api, version, commit, wait_for_visibility=True)
    remote_assets(api, release["id"], expected, complete=True)
    return {"release_id": release["id"], "descriptor_sha256": expected_digest, "status": "published_immutable"}


def verify_public(directory, version, commit, run_id, workflow_sha, expected_digest):
    candidate = validate(directory, version, commit, run_id, workflow_sha, expected_digest)
    for name, expected in candidate["files"].items():
        data = public_download(f"https://github.com/{REPOSITORY}/releases/download/v{version}/{name}")
        if {"bytes": len(data), "sha256": sha(data)} != expected:
            raise ValueError("anonymous public download differs from the accepted bytes")
    return {"status": "public_bytes_verified", "descriptor_sha256": expected_digest, "channels": CHANNELS}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("select", "seal", "validate", "publish", "verify-public"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--run-id", required=True)
    parser.add_argument("--workflow-sha", required=True)
    parser.add_argument("--checkout", type=Path)
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--evidence", type=Path)
    parser.add_argument("--descriptor-sha256")
    args = parser.parse_args(argv)
    values = (args.version, args.commit, args.run_id, args.workflow_sha)
    if args.action == "select":
        if args.checkout is None:
            parser.error("select requires --checkout")
        result = select(args.checkout, *values, GitHub(os.environ.get("GH_TOKEN")))
    else:
        if args.directory is None:
            parser.error("this action requires --directory")
        if args.action == "seal":
            if args.evidence is None:
                parser.error("seal requires --evidence")
            result = seal(args.directory, *values, json.loads(read(args.evidence, 65536)))
        elif args.action == "validate":
            result = validate(args.directory, *values, args.descriptor_sha256)
        else:
            if not args.descriptor_sha256 or not re.fullmatch(r"[0-9a-f]{64}", args.descriptor_sha256):
                parser.error("promotion requires this run's --descriptor-sha256")
            result = (publish(args.directory, *values, args.descriptor_sha256, GitHub(os.environ.get("GH_TOKEN")))
                      if args.action == "publish" else verify_public(args.directory, *values, args.descriptor_sha256))
    print(json.dumps(result, indent=2))
    if os.environ.get("GITHUB_OUTPUT"):
        with Path(os.environ["GITHUB_OUTPUT"]).open("a") as output:
            for key in ("version", "commit", "descriptor_sha256"):
                if key in result:
                    output.write(f"{key}={result[key]}\n")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"Release stopped: {error}", file=sys.stderr)
        sys.exit(1)
