#!/usr/bin/env python3
"""Install a verified Flere core and companion from a public HTTPS release.

Use --core-only for a remote server that does not need the local SSH companion.

Requires Python 3 with system HTTPS trust and the platform SHA-256 utility; no
Rust, source checkout or GitHub credentials. This script never publishes releases.
The selected HTTPS channel is the trust root; adjacent hashes are not signatures.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import selectors
import time
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request

MAX_PAYLOAD = 256 * 1024 * 1024
MAX_CHANNEL_BYTES = 8192
CHANNEL_URL = "https://raw.githubusercontent.com/robert-cronin/flere/main/packaging/channels/stable.json"
CHANNEL_TARGETS = {"x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc",
                   "aarch64-apple-darwin", "x86_64-apple-darwin"}
LEGACY_NOTICE = ("The macOS arm64 default is the pinned unsigned v0.3.0 release. "
                 "It does not include later Linux/Windows changes; normal macOS security checks still apply.")


def https(url):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("release URL must be HTTPS without credentials")
    if parsed.query or parsed.fragment or "\\" in url or any(ord(c) < 33 for c in url):
        raise ValueError("release URL must not contain queries, fragments or control characters")
    return url


class Redirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        # GitHub asset redirects can have signed query strings. These are generated
        # by the selected HTTPS service, never persisted as the release source.
        parsed = urllib.parse.urlsplit(new_url)
        if parsed.scheme != "https" or parsed.username or parsed.password:
            raise ValueError("release redirected outside authenticated HTTPS")
        return super().redirect_request(request, fp, code, message, headers, new_url)


def fetch(url, limit, output=None):
    opener = urllib.request.build_opener(Redirect)
    digest = hashlib.sha256()
    data = bytearray()
    size = 0
    with opener.open(url, timeout=30) as response:
        while True:
            chunk = response.read(min(65536, limit + 1 - size))
            if not chunk:
                break
            size += len(chunk)
            if size > limit:
                raise ValueError("release download exceeds its declared bound")
            digest.update(chunk)
            if output:
                output.write(chunk)
            else:
                data.extend(chunk)
    return bytes(data), size, digest.hexdigest()


def platform_target():
    targets = {("Linux", "x86_64"): "x86_64-unknown-linux-gnu",
               ("Darwin", "arm64"): "aarch64-apple-darwin",
               ("Darwin", "x86_64"): "x86_64-apple-darwin"}
    target = targets.get((platform.system(), platform.machine()))
    if target is None:
        raise ValueError("this installer needs Linux/macOS; Windows uses its separately accepted companion package")
    return target


def version_tuple(value):
    if not isinstance(value, str):
        raise ValueError("channel version must be canonical X.Y.Z")
    parts = value.split(".")
    if len(parts) != 3 or any(not part or len(part) > 10 or
                             any(c not in "0123456789" for c in part) or
                             (len(part) > 1 and part[0] == "0") for part in parts):
        raise ValueError("channel version must be canonical X.Y.Z")
    result = tuple(int(part) for part in parts)
    if any(part > 2147483647 for part in result):
        raise ValueError("channel version component exceeds its bound")
    return result


def parse_channel(raw_bytes):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate channel field")
            result[key] = value
        return result

    if not isinstance(raw_bytes, bytes) or len(raw_bytes) > MAX_CHANNEL_BYTES:
        raise ValueError("channel document exceeds its byte bound or is not bytes")
    try:
        channel = json.loads(raw_bytes.decode("utf-8"), object_pairs_hook=unique)
    except (UnicodeError, RecursionError, ValueError) as error:
        raise ValueError("channel document must be bounded UTF-8 JSON without duplicate fields") from error
    if (not isinstance(channel, dict) or set(channel) != {"schema_version", "targets"} or
            type(channel["schema_version"]) is not int or channel["schema_version"] != 1 or
            not isinstance(channel["targets"], dict) or set(channel["targets"]) - CHANNEL_TARGETS):
        raise ValueError("invalid channel schema or target inventory")
    for target, record in channel["targets"].items():
        if not isinstance(record, dict):
            raise ValueError("invalid channel target record")
        if record == {"policy": "unavailable"}:
            continue
        if set(record) != {"policy", "version", "manifests"} or record["policy"] not in ("current", "legacy_unsigned"):
            raise ValueError("invalid channel target fields or policy")
        version_tuple(record["version"])
        if record["policy"] == "legacy_unsigned" and (target != "aarch64-apple-darwin" or record["version"] != "0.3.0"):
            raise ValueError("legacy unsigned policy is only valid for macOS arm64 v0.3.0")
        components = {"flere-connect"} if target == "x86_64-pc-windows-msvc" else {"flere", "flere-connect"}
        manifests = record["manifests"]
        if not isinstance(manifests, dict) or set(manifests) != components:
            raise ValueError("invalid channel component inventory")
        for pin in manifests.values():
            if (not isinstance(pin, dict) or set(pin) != {"bytes", "sha256"} or
                    type(pin["bytes"]) is not int or not 0 < pin["bytes"] <= 65536 or
                    not isinstance(pin["sha256"], str) or len(pin["sha256"]) != 64 or
                    any(c not in "0123456789abcdef" for c in pin["sha256"])):
                raise ValueError("invalid channel manifest size/SHA-256 pin")
    return channel


def default_selection(target):
    try:
        raw, _, _ = fetch(CHANNEL_URL, MAX_CHANNEL_BYTES)
    except OSError as error:
        raise ValueError("cannot fetch the default release channel; no fallback release was selected") from error
    channel = parse_channel(raw)
    selected = channel["targets"].get(target)
    if selected is None or selected["policy"] == "unavailable":
        raise ValueError("the default release channel has no available package for this target")
    if selected["policy"] == "legacy_unsigned":
        print(LEGACY_NOTICE, file=sys.stderr)
    return selected, {"url": CHANNEL_URL, "sha256": hashlib.sha256(raw).hexdigest(),
                      "target": target, "policy": selected["policy"], "version": selected["version"]}


def release_manifest(version, target, component):
    return f"https://github.com/robert-cronin/flere/releases/download/v{version}/{component}-{target}.manifest.json"


def companion_manifest(core_url, target):
    parsed = urllib.parse.urlsplit(core_url)
    prefix = "/robert-cronin/flere/releases/"
    tail = parsed.path.removeprefix(prefix).split("/")
    release = (len(tail) == 3 and tail[:2] == ["latest", "download"] or
               len(tail) == 3 and tail[0] == "download" and bool(tail[1]))
    if (parsed.netloc != "github.com" or not parsed.path.startswith(prefix) or not release or
            tail[-1] != f"flere-{target}.manifest.json"):
        raise ValueError("a custom core manifest requires --companion-url, or choose --core-only")
    return core_url.rsplit("/", 1)[0] + f"/flere-connect-{target}.manifest.json"


def prepare(url, component, target, directory, selection=None):
    https(url)
    if not url.endswith("manifest.json"):
        raise ValueError("choose a manifest.json URL")
    try:
        raw, _, _ = fetch(url, 65536)
    except OSError as error:
        raise ValueError(f"cannot fetch {url}: {error}; the release may be unpublished or this machine offline") from error
    if selection is not None:
        pin = selection["manifests"][component]
        if len(raw) != pin["bytes"] or hashlib.sha256(raw).hexdigest() != pin["sha256"]:
            raise ValueError("release manifest differs from the channel size/SHA-256 pin")
    manifest = json.loads(raw)
    payload = manifest["payload"]
    if manifest["schema_version"] != 1 or manifest["build"]["component"] != component or manifest["build"]["target"] != target:
        raise ValueError("release manifest component/schema/platform differs")
    if selection is not None and manifest["build"]["package_version"] != selection["version"]:
        raise ValueError("release manifest version differs from the channel selection")
    if payload["file_name"] != component or type(payload["bytes"]) is not int or not 0 < payload["bytes"] <= MAX_PAYLOAD:
        raise ValueError("invalid fixed package payload or size")
    expected = payload["sha256"]
    if not isinstance(expected, str) or len(expected) != 64 or any(c not in "0123456789abcdef" for c in expected):
        raise ValueError("invalid SHA-256 in release manifest")
    asset = payload.get("download_file", component)
    if not isinstance(asset, str) or not asset or len(asset) > 256 or asset.startswith(".") or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_." for c in asset):
        raise ValueError("invalid release asset basename")
    package = directory / component
    package.mkdir(mode=0o700)
    binary = package / component
    with binary.open("xb") as output:
        _, size, digest = fetch(url.rsplit("/", 1)[0] + "/" + asset, payload["bytes"], output)
        output.flush()
        os.fsync(output.fileno())
    if size != payload["bytes"] or digest != expected:
        raise ValueError(f"downloaded {component} does not match pinned size/SHA-256")
    binary.chmod(0o700)
    (package / "manifest.json").write_bytes(raw)
    return {"component": component, "url": url, "directory": package, "binary": binary, "manifest": manifest}


def run_json(command):
    # Fixed bounds apply to both trusted executable diagnostics and installer
    # receipts; a malformed helper cannot fill memory while we wait for exit.
    process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    output = [bytearray(), bytearray()]
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ, 0)
            selector.register(process.stderr, selectors.EVENT_READ, 1)
            deadline = time.monotonic() + 30
            while selector.get_map():
                if time.monotonic() >= deadline:
                    raise ValueError("verified installer helper timed out; inspect per-component install receipts before retrying")
                for key, _ in selector.select(0.1):
                    data = os.read(key.fileobj.fileno(), 65536)
                    if not data:
                        selector.unregister(key.fileobj)
                    else:
                        output[key.data].extend(data)
                        if len(output[key.data]) > (131072 if key.data == 0 else 16384):
                            raise ValueError("verified installer helper output exceeds its bound")
            status = process.wait(timeout=max(0.01, deadline - time.monotonic()))
        if status:
            detail = output[1].decode("utf-8", errors="replace")
            raise ValueError(f"{Path(command[0]).name} failed ({status}): {detail[:2048]}")
        return json.loads(output[0])
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdout.close()
        process.stderr.close()


def verify_pair(packages):
    core = packages[0]
    for package in packages:
        if run_json([str(package["binary"]), "--build-info"]) != package["manifest"]["build"]:
            raise ValueError(f"{package['component']} embedded metadata differs from its verified manifest")
        verified = run_json([str(core["binary"]), "verify-package", str(package["directory"])])
        if verified.get("status") != "verified" or verified.get("manifest") != package["manifest"]:
            raise ValueError("core package verifier returned a different manifest")
    if len(packages) == 2:
        first, second = (package["manifest"] for package in packages)
        if first["build"]["package_version"] != second["build"]["package_version"]:
            raise ValueError("core and companion must come from the same release version")
        if first.get("source") and second.get("source") and first["source"]["source_sha256"] != second["source"]["source_sha256"]:
            raise ValueError("core and companion were packaged from different source revisions")
        current = first["build"]["compatibility"]["remote_protocol"]["current"]
        if current not in second["build"]["compatibility"]["remote_protocol"]["accepts"]:
            raise ValueError("companion cannot read the selected core protocol")


def write_report(path, report):
    with path.open("w") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
        output.flush()
        os.fsync(output.fileno())


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest_url", nargs="?", type=https, help="core manifest; defaults to Flere's pinned per-target release channel")
    parser.add_argument("--companion-url", type=https, help="explicit paired companion manifest for a custom channel")
    parser.add_argument("--core-only", action="store_true", help="install only the server core")
    parser.add_argument("--adopt", action="store_true", help="explicitly retain and adopt existing manual user installations")
    args = parser.parse_args(argv)
    if args.core_only and args.companion_url:
        parser.error("--core-only cannot be combined with --companion-url")
    target = platform_target()
    selection = discovery = None
    if args.manifest_url is None:
        selection, discovery = default_selection(target)
        core_url = release_manifest(selection["version"], target, "flere")
    else:
        core_url = args.manifest_url
    companion_url = None if args.core_only else args.companion_url or companion_manifest(core_url, target)
    cache = Path(os.environ.get("XDG_CACHE_HOME", str(Path.home() / ".cache")))
    if not cache.is_absolute():
        raise ValueError("XDG_CACHE_HOME must be absolute")
    directory = cache / "flere" / "downloads"
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    if directory.is_symlink() or directory.stat().st_uid != os.getuid() or directory.stat().st_mode & 0o077:
        raise ValueError("download cache must be private and user-owned")
    temporary = Path(tempfile.mkdtemp(prefix="bootstrap-", dir=directory))
    try:
        packages = [prepare(core_url, "flere", target, temporary, selection)]
        if companion_url:
            packages.append(prepare(companion_url, "flere-connect", target, temporary,
                                    None if args.companion_url else selection))
        # Download, hash, inspect and cross-check BOTH before the first install.
        verify_pair(packages)
        descriptor, report_name = tempfile.mkstemp(prefix="installation-result-", suffix=".json", dir=directory)
        os.close(descriptor)
        report_path = Path(report_name)
        report = {"schema_version": 1, "status": "verified", "installed": [], "attempted": None,
                  "sources": {package["component"]: package["url"] for package in packages},
                  "atomic_pair": False}
        if discovery is not None:
            report["discovery"] = discovery
        write_report(report_path, report)
        try:
            for package in packages:
                report["attempted"] = package["component"]
                write_report(report_path, report)
                command = [str(package["binary"]), "install", str(package["directory"]), "--source-url", package["url"]]
                if args.adopt:
                    command.append("--adopt")
                receipt = run_json(command)
                if receipt.get("component") != package["component"] or receipt.get("current", {}).get("manifest") != package["manifest"]:
                    raise ValueError("installer returned an unexpected receipt; inspect component install-status")
                report["installed"].append({"component": package["component"], "receipt": receipt})
                write_report(report_path, report)
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            report["status"] = "partial" if report["installed"] else "unverified"
            report["detail"] = str(error)
            write_report(report_path, report)
            raise ValueError(f"{report['status']} installation; {report['attempted']} failed or is unverified. Existing per-component receipts were retained; details: {report_path}. {error}") from error
        report["status"] = "installed"
        report["attempted"] = None
        write_report(report_path, report)
        print(json.dumps({"status": "installed", "components": [p["component"] for p in packages], "report": str(report_path)}))
        installed = Path.home() / ".local/bin/flere"
        resolved = shutil.which("flere")
        if resolved is None:
            print(f"Installed {installed}. Add {installed.parent} to PATH when ready; no shell config was changed.", file=sys.stderr)
        elif Path(resolved) != installed:
            print(f"Installed {installed}, but PATH currently finds {resolved}. Use the installed path or adjust PATH.", file=sys.stderr)
        if not args.core_only:
            print("Core and local SSH companion installed; use flere ssh EXISTING_ALIAS.", file=sys.stderr)
    finally:
        shutil.rmtree(temporary)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"Flere installation stopped: {error}", file=sys.stderr)
        sys.exit(1)
