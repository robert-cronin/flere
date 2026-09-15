#!/usr/bin/env python3
"""Staged native arm64 release producer. No publication and no unsigned fallback."""
import argparse
import base64
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import secrets
import shlex
import struct
import subprocess
import sys
import tempfile
import time
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
TARGET = "aarch64-apple-darwin"
COMPONENTS = ("flere", "flere-connect")
LICENSES = {"LICENSE": "LICENSE", "OFL.txt": "src/assets/fonts/OFL.txt",
            "LICENSE-Nerd-Fonts": "src/assets/fonts/LICENSE-Nerd-Fonts"}
LIMIT = 256 * 1024 * 1024
CHECKS = ["cargo fmt --check", "cargo clippy --offline --all-targets -- -D warnings",
          "cargo test --offline", "cargo build --offline --release",
          "cargo fmt --manifest-path companion/Cargo.toml --check",
          "cargo clippy --offline --manifest-path companion/Cargo.toml --all-targets -- -D warnings",
          "cargo test --offline --manifest-path companion/Cargo.toml",
          "cargo build --offline --release --manifest-path companion/Cargo.toml"]
SIGN_KEYS = ("MACOS_DEVELOPER_ID_SHA1", "MACOS_TEAM_ID", "MACOS_CERTIFICATE_P12_BASE64", "MACOS_CERTIFICATE_PASSWORD")
NOTARY_KEYS = ("MACOS_NOTARY_KEY_ID", "MACOS_NOTARY_ISSUER_ID", "MACOS_NOTARY_PRIVATE_KEY")
FINAL_CHECKS = [c + ":signature_online_ticket_gatekeeper" for c in COMPONENTS] + [
    c + ":" + flag for c in COMPONENTS for flag in ("build-info", "version", "help")] + [
    c + ":managed_install_reinstall" for c in COMPONENTS]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


release = load("macos_release", ROOT / "scripts/release-automation.py")
linux = load("macos_package", ROOT / "scripts/release-linux-ci.py")
windows = load("macos_source", ROOT / "packaging/windows/hosted-candidate.py")


class PhaseError(ValueError):
    """Only fixed, credential-free diagnostics may be exposed to workflow logs."""


def require(value, message):
    if not value:
        raise PhaseError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path, maximum=LIMIT):
    return release.read(path, maximum)


def save(path, value):
    data = release.json_bytes(value)
    temporary = path.with_name(path.name + ".new")
    with temporary.open("xb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)
    return sha(data)


def identity(version, commit, run_id, workflow_sha):
    release.identity(version, commit, run_id)
    release.identity(version, workflow_sha, run_id)
    return dict(version=version, commit=commit, run_id=run_id, workflow_sha=workflow_sha, target=TARGET)


def native(selected, *, credentials=False):
    require(sys.platform == "darwin" and platform.machine() == "arm64", "native macOS arm64 is required")
    for key, value in {"GITHUB_ACTIONS": "true", "FLERE_RUNNER_ENVIRONMENT": "github-hosted",
                       "GITHUB_REPOSITORY": release.REPOSITORY, "GITHUB_EVENT_NAME": "workflow_dispatch",
                       "GITHUB_REF": "refs/heads/main", "GITHUB_RUN_ID": selected["run_id"]}.items():
        require(os.environ.get(key) == value, "requires the manual hosted Release job: " + key)
    require(release.git(ROOT, "rev-parse", "HEAD") == selected["workflow_sha"], "automation checkout differs")
    if not credentials:
        require(not any(os.environ.get(key) for key in SIGN_KEYS + NOTARY_KEYS),
                "build/runtime phases must not receive Apple signing credentials")


def configuration(env, phase):
    if phase in ("sign", "notarize"):
        other = NOTARY_KEYS if phase == "sign" else SIGN_KEYS
        require(not any(env.get(key) for key in other), "Apple credentials must be scoped to this release phase")
    keys = SIGN_KEYS + NOTARY_KEYS if phase == "preflight" else SIGN_KEYS if phase == "sign" else NOTARY_KEYS
    missing = [key for key in keys if not env.get(key)]
    require(not missing, "macOS release settings missing: " + ", ".join(missing))
    if phase in ("preflight", "sign"):
        require(re.fullmatch(r"[A-Fa-f0-9]{40}", env["MACOS_DEVELOPER_ID_SHA1"]), "invalid Developer ID certificate fingerprint")
        require(re.fullmatch(r"[A-Z0-9]{10}", env["MACOS_TEAM_ID"]), "invalid Apple team ID")
        require(len(env["MACOS_CERTIFICATE_PASSWORD"]) <= 4096, "certificate password exceeds bound")
        try:
            raw = base64.b64decode(env["MACOS_CERTIFICATE_P12_BASE64"], validate=True)
        except ValueError as error:
            raise ValueError("invalid base64 signing certificate") from error
        require(0 < len(raw) <= 1024 * 1024, "signing certificate exceeds bound")
    if phase in ("preflight", "notarize"):
        require(re.fullmatch(r"[A-Z0-9]{10}", env["MACOS_NOTARY_KEY_ID"]), "invalid notary API key ID")
        uuid(env["MACOS_NOTARY_ISSUER_ID"])
        key = env["MACOS_NOTARY_PRIVATE_KEY"]
        require(len(key) <= 16384 and key.startswith("-----BEGIN PRIVATE KEY-----\n")
                and key.rstrip().endswith("-----END PRIVATE KEY-----"), "invalid notary private key envelope")


def private_output(path, *, existing=False):
    path = path.absolute()
    cache = Path.home() / ".cache/flere/tmp"
    require(path.is_relative_to(cache) and path != cache and path.resolve() == path and not path.is_symlink(),
            "output must be a separate private HOME-cache path")
    require(path.is_dir() if existing else not path.exists(), "output existence differs from requested phase")
    if not existing:
        path.mkdir(mode=0o700, parents=True)
    return path


def environment(home, *, build=False):
    env = {"HOME": str(home), "TMPDIR": str(home), "LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8",
           "PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "PYTHONDONTWRITEBYTECODE": "1"}
    if build:
        env.update(PATH=os.environ["PATH"], CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                   RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                   RUSTUP_TOOLCHAIN="1.98.0", CARGO_NET_OFFLINE="true", CARGO_BUILD_JOBS="2", RUST_TEST_THREADS="1")
    for key, child in (("XDG_CACHE_HOME", ".cache"), ("XDG_CONFIG_HOME", ".config"), ("XDG_DATA_HOME", ".local/share")):
        env[key] = str(home / child)
    return env


def capture(argv, home, *, env=None, timeout=120, check=True, maximum=1024 * 1024, merge=False):
    """Bound both streams while the owned child runs; never expose secret output."""
    require(type(maximum) is int and 0 < maximum <= 1024 * 1024
            and type(timeout) is int and 0 < timeout <= 1800, "invalid native tool capture bounds")
    with tempfile.TemporaryFile(dir=home) as output, tempfile.TemporaryFile(dir=home) as error:
        process = subprocess.Popen(list(map(str, argv)), stdin=subprocess.DEVNULL, stdout=output,
                                   stderr=subprocess.STDOUT if merge else error, env=env or environment(home))
        try:
            deadline = time.monotonic() + timeout
            while process.poll() is None:
                require(time.monotonic() < deadline, "native tool timed out: " + Path(argv[0]).name)
                require(os.fstat(output.fileno()).st_size + os.fstat(error.fileno()).st_size <= maximum,
                        "native tool output exceeded bound")
                time.sleep(0.1)
            require(os.fstat(output.fileno()).st_size + os.fstat(error.fileno()).st_size <= maximum,
                    "native tool output exceeded bound")
            output.seek(0)
            error.seek(0)
            data, diagnostic = output.read(maximum + 1), error.read(maximum + 1)
            require(len(data) + len(diagnostic) <= maximum, "native tool output exceeded bound")
            require(not check or process.returncode == 0, "native tool failed: " + Path(argv[0]).name)
            return process.returncode, data, diagnostic
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
            process.wait()


def command(argv, home, *, env=None, timeout=120, check=True):
    # Preserve merged stdout/stderr ordering for the existing native-tool calls.
    status, data, _ = capture(argv, home, env=env, timeout=timeout, check=check, merge=True)
    return status, data


def macho(data):
    require(len(data) >= 32, "truncated Mach-O")
    magic, cpu, subtype, kind, count, length, flags, reserved = struct.unpack_from("<8I", data)
    require((magic, cpu, kind) == (0xFEEDFACF, 0x0100000C, 2) and subtype & 0xFFFFFF == 0,
            "requires thin macOS arm64 executable, not Intel/universal/arm64e")
    require(0 < count <= 4096 and 0 < length <= len(data) - 32, "Mach-O load command bound differs")
    offset, libraries, minimum = 32, [], []
    for _ in range(count):
        require(offset + 8 <= 32 + length, "truncated Mach-O load command")
        cmd, size = struct.unpack_from("<2I", data, offset)
        require(size >= 8 and size % 8 == 0 and offset + size <= 32 + length, "invalid Mach-O load command")
        if cmd in (0xC, 0x80000018, 0x8000001F, 0x20, 0x80000023):
            require(size >= 24, "truncated dylib command")
            start = struct.unpack_from("<I", data, offset + 8)[0]
            require(24 <= start < size and b"\0" in data[offset + start:offset + size], "invalid dylib path")
            name = data[offset + start:offset + size].split(b"\0", 1)[0].decode("utf-8")
            require(name.startswith(("/usr/lib/", "/System/Library/Frameworks/"))
                    and "/../" not in name and all(c.isprintable() for c in name), "non-system Mach-O dependency")
            libraries.append(name)
        elif cmd == 0x8000001C:
            raise ValueError("unexpected Mach-O runtime search path")
        elif cmd == 0x32:
            require(size >= 24, "truncated build-version command")
            system, version = struct.unpack_from("<2I", data, offset + 8)
            require(system == 1 and version >> 16 >= 11, "invalid macOS deployment platform/version")
            minimum.append("%d.%d.%d" % (version >> 16, (version >> 8) & 255, version & 255))
        offset += size
    require(offset == 32 + length and len(minimum) == 1 and libraries, "incomplete Mach-O deployment/linkage metadata")
    return {"architecture": "arm64", "minimum_macos": minimum[0], "libraries": sorted(set(libraries))}


def payload_names():
    return {c + "-" + TARGET + suffix for c in COMPONENTS for suffix in ("", ".manifest.json")}


def inventory(directory, names):
    require(not directory.is_symlink() and {p.name for p in directory.iterdir()} == set(names), "macOS artifact inventory differs")
    return {name: {"bytes": len(raw := read(directory / name)), "sha256": sha(raw)} for name in sorted(names)}


def pair(directory, selected, source):
    manifests = {}
    for component in COMPONENTS:
        name = component + "-" + TARGET
        data = read(directory / name)
        item = json.loads(read(directory / (name + ".manifest.json"), 65536))
        b, s, p = item["build"], item["source"], item["payload"]
        require(type(item["schema_version"]) is int and item["schema_version"] == 1
                and b["component"] == component and b["target"] == TARGET and b["profile"] == "release"
                and b["package_version"] == selected["version"] and s["git_commit"] == selected["commit"]
                and s["source_sha256"] == source and s["dirty"] is False
                and p["file_name"] == component and p["download_file"] == name
                and type(p["bytes"]) is int and p["bytes"] == len(data) and p["sha256"] == sha(data),
                "macOS manifest source/version/final payload differs")
        macho(data)
        manifests[component] = item
    protocol = manifests["flere"]["build"]["compatibility"]["remote_protocol"]["current"]
    accepts = manifests["flere-connect"]["build"]["compatibility"]["remote_protocol"]["accepts"]
    require(isinstance(protocol, str) and protocol and isinstance(accepts, list) and protocol in accepts,
            "macOS companion cannot read the paired core protocol")
    return manifests


def built_document(value, selected):
    require(value["schema"] == "flere-macos-build-v1" and value["status"] == "built_not_signed"
            and value["identity"] == selected and value["checks"] == CHECKS
            and value["source_unchanged"] is True and re.fullmatch(r"[0-9a-f]{64}", value["source_sha256"])
            and set(value["files"]) == payload_names() | set(LICENSES), "incomplete native macOS build receipt")
    return value


def validate_built(directory, pin, selected):
    raw = read(directory / "build.json", 65536)
    require(sha(raw) == pin, "native build receipt differs from trusted digest")
    value = built_document(json.loads(raw), selected)
    files = inventory(directory, payload_names() | set(LICENSES) | {"build.json", "validation.log"})
    require({k: v for k, v in files.items() if k not in ("build.json", "validation.log")} == value["files"]
            and files["validation.log"]["sha256"] == value["validation_log_sha256"], "native build bytes/log changed")
    require(pair(directory, selected, value["source_sha256"]) == value["manifests"], "native build manifests changed")
    return value


def build(checkout, output, selected):
    native(selected)
    snapshot = windows.source_snapshot(checkout, commit=selected["commit"], version=selected["version"])
    output = private_output(output)
    with tempfile.TemporaryDirectory(prefix="m", dir=Path.home() / ".cache/flere/tmp") as temporary:
        home = Path(temporary)
        env = environment(home, build=True)
        _, compiler = command(["rustc", "-vV"], home, env=env)
        require(compiler.startswith(b"rustc 1.98.0 ") and b"host: aarch64-apple-darwin" in compiler,
                "provisioned native Rust 1.98.0 required")
        result = linux.package_candidate(checkout, env)
        require(result["status"] == "packaged" and result["source"]["source_sha256"] == snapshot["source_sha256"]
                and result["source"]["git_commit"] == selected["commit"] and result["source"]["dirty"] is False
                and result["source"]["checks"] == CHECKS, "native package/source checks differ")
        packages = Path(result["artifact"])
        require(packages.resolve() == packages and packages.is_relative_to(home / ".cache/flere/dev-updates"),
                "package result escaped its private home")
        manifests = {}
        for component in COMPONENTS:
            name = component + "-" + TARGET
            data = read(packages / component / component)
            item = json.loads(read(packages / component / "manifest.json", 65536))
            (output / name).write_bytes(data); (output / name).chmod(0o755)
            (output / (name + ".manifest.json")).write_bytes(release.json_bytes(item))
            _, observed = command([output / name, "--build-info"], home, env=env)
            require(json.loads(observed) == item["build"], "native payload build identity differs")
            manifests[component] = item
        for name, relative in LICENSES.items():
            data = read(checkout / relative, 65536)
            require(sha(data) == snapshot["entries"][relative]["sha256"], "source license changed")
            (output / name).write_bytes(data)
        require(windows.source_snapshot(checkout, commit=selected["commit"], version=selected["version"]) == snapshot,
                "source changed during native build")
        pair(output, selected, snapshot["source_sha256"])
        _, xcode = command(["/usr/bin/xcodebuild", "-version"], home)
        receipt = dict(schema="flere-macos-build-v1", status="built_not_signed", identity=selected,
                       source_sha256=snapshot["source_sha256"], source_unchanged=True, checks=CHECKS,
                       manifests=manifests, files=inventory(output, payload_names() | set(LICENSES)),
                       validation_log_sha256=sha(read(packages / "validation.log", 64 * 1024 * 1024)),
                       runner={"os": platform.mac_ver()[0], "architecture": "arm64",
                               "image": os.environ.get("ImageOS"), "image_version": os.environ.get("ImageVersion"),
                               "rustc": compiler.decode().strip(), "xcode": xcode.decode().strip()},
                       macho={c: macho(read(output / (c + "-" + TARGET))) for c in COMPONENTS})
        (output / "validation.log").write_bytes(read(packages / "validation.log", 64 * 1024 * 1024))
        pin = save(output / "build.json", receipt)
        validate_built(output, pin, selected)
        return {"directory": str(output), "build_receipt_sha256": pin}


def signature_requirement(component, team, fingerprint):
    require(component in COMPONENTS and re.fullmatch(r"[A-Z0-9]{10}", team)
            and re.fullmatch(r"[A-Fa-f0-9]{40}", fingerprint), "invalid signature identity")
    return ('anchor apple generic and certificate leaf = H"' + fingerprint.upper() + '"'
            ' and certificate leaf[subject.OU] = "' + team + '"'
            ' and identifier "io.github.robert-cronin.' + component + '"')


def signature_details(data, component, team):
    text = data.decode("utf-8")
    def field(name):
        rows = re.findall(r"^" + re.escape(name) + r"=(.+)$", text, re.M)
        require(len(rows) == 1, "signature field missing/ambiguous: " + name)
        return rows[0]
    require(field("Identifier") == "io.github.robert-cronin." + component and field("TeamIdentifier") == team,
            "signature component/team differs")
    require(re.search(r"^Authority=Developer ID Application: .+$", text, re.M)
            and "Signature=adhoc" not in text, "requires Developer ID Application signature")
    flags = re.findall(r"\bflags=0x([0-9a-fA-F]+)\(", text)
    require(len(flags) == 1 and int(flags[0], 16) & 0x10000, "hardened runtime is missing")
    timestamp = field("Timestamp")
    require(timestamp.strip() and timestamp.lower() != "none", "secure timestamp is missing")
    return {"identifier": field("Identifier"), "team_id": team, "hardened_runtime": True, "secure_timestamp": True}


def signature(path, component, team, fingerprint, home, *, online=False):
    args = ["/usr/bin/codesign", "--verify", "--strict", "-R", signature_requirement(component, team, fingerprint)]
    command(args + (["--check-notarization"] if online else []) + [path], home)
    _, display = command(["/usr/bin/codesign", "--display", "--verbose=4", path], home)
    result = signature_details(display, component, team)
    # No entitlement exceptions are required by these Rust command-line tools.
    # Keep stdout separate: codesign's informational stderr is not a plist.
    _, data, _ = capture(["/usr/bin/codesign", "--display", "--entitlements", "-", "--xml", path],
                         home, timeout=30, maximum=65536)
    require(len(data) <= 65536 and (not data.strip() or plistlib.loads(data) == {}), "unexpected signing entitlements")
    return result


def zip_name(version):
    return "flere-" + version + "-" + TARGET + ".zip"


def zip_entries(directory):
    return {**{c: read(directory / (c + "-" + TARGET)) for c in COMPONENTS},
            **{c + ".manifest.json": read(directory / (c + "-" + TARGET + ".manifest.json"), 65536) for c in COMPONENTS},
            **{name: read(directory / name, 65536) for name in LICENSES}}


def verify_zip(path, entries):
    read(path)
    with zipfile.ZipFile(path) as archive:
        items = archive.infolist()
        require(len(items) == len(entries) and {i.filename for i in items} == set(entries), "macOS ZIP inventory differs")
        for item in items:
            expected = entries[item.filename]
            mode = 0o100755 if item.filename in COMPONENTS else 0o100644
            require(item.file_size == len(expected) and item.file_size <= LIMIT and item.external_attr >> 16 == mode
                    and item.create_system == 3 and item.date_time == (1980, 1, 1, 0, 0, 0)
                    and not item.extra and not item.comment and not item.flag_bits & 1
                    and archive.read(item) == expected, "macOS ZIP final bytes or metadata differ")


def rebind(manifest, payload):
    result = copy.deepcopy(manifest)
    result["payload"].update(bytes=len(payload), sha256=sha(payload))
    return result


def keychain_list(raw):
    paths = shlex.split(raw.decode())
    require(len(paths) <= 128 and all(Path(path).is_absolute() for path in paths), "invalid keychain search list")
    return paths


def sign(directory, output, selected, build_pin):
    native(selected, credentials=True)
    configuration(os.environ, "sign")
    built = validate_built(directory, build_pin, selected)
    require(not output.absolute().is_relative_to(directory.absolute()), "signing output overlaps input")
    output = private_output(output)
    for name in payload_names() | set(LICENSES) | {"build.json"}:
        (output / name).write_bytes(read(directory / name))
    team, fingerprint = os.environ["MACOS_TEAM_ID"], os.environ["MACOS_DEVELOPER_ID_SHA1"].upper()
    signatures = {}
    # Key material is outside the artifact directory. codesign --keychain limits
    # identity lookup, but its certificate-chain lookup uses the search list.
    # Restore that exact list even on failure; never change the default keychain.
    with tempfile.TemporaryDirectory(prefix="sign-key-", dir=output.parent) as temporary:
        home = Path(temporary)
        keychain = home / "release.keychain-db"
        certificate = home / "identity.p12"
        certificate.write_bytes(base64.b64decode(os.environ["MACOS_CERTIFICATE_P12_BASE64"], validate=True))
        certificate.chmod(0o600)
        password = secrets.token_hex(32)
        created = False
        search_changed = False
        signing_complete = False
        _, raw_search = command(["/usr/bin/security", "list-keychains", "-d", "user"], home)
        previous_search = keychain_list(raw_search)
        try:
            search_changed = True  # creation can itself register the new keychain
            command(["/usr/bin/security", "create-keychain", "-p", password, keychain], home)
            created = True
            command(["/usr/bin/security", "set-keychain-settings", "-lut", "21600", keychain], home)
            command(["/usr/bin/security", "unlock-keychain", "-p", password, keychain], home)
            command(["/usr/bin/security", "import", certificate, "-k", keychain, "-P",
                     os.environ["MACOS_CERTIFICATE_PASSWORD"], "-T", "/usr/bin/codesign"], home)
            command(["/usr/bin/security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", password, keychain], home)
            # Mark intent before mutation so even a lost tool response restores.
            search_changed = True
            command(["/usr/bin/security", "list-keychains", "-d", "user", "-s", keychain, *previous_search], home)
            for component in COMPONENTS:
                path = output / (component + "-" + TARGET)
                path.chmod(0o755)
                command(["/usr/bin/codesign", "--force", "--sign", fingerprint, "--keychain", keychain,
                         "--timestamp", "--options", "runtime", "--identifier", "io.github.robert-cronin." + component, path], home)
                signatures[component] = signature(path, component, team, fingerprint, home)
                data = read(path)
                require(sha(data) != built["manifests"][component]["payload"]["sha256"], "signing did not replace the original signature")
                (output / (path.name + ".manifest.json")).write_bytes(release.json_bytes(rebind(built["manifests"][component], data)))
            signing_complete = True
        finally:
            cleanup = {"restore_command": False, "delete_command": False,
                       "search_list_matches": False, "keychain_absent": False}
            try:
                if search_changed:
                    command(["/usr/bin/security", "list-keychains", "-d", "user", "-s", *previous_search], home)
                cleanup["restore_command"] = True
            except (OSError, ValueError, subprocess.SubprocessError):
                pass  # Continue only owned cleanup; never expose secret argv/output.
            try:
                if created or keychain.exists():
                    command(["/usr/bin/security", "delete-keychain", keychain], home)
                cleanup["delete_command"] = True
            except (OSError, ValueError, subprocess.SubprocessError):
                pass
            try:
                _, observed = command(["/usr/bin/security", "list-keychains", "-d", "user"], home)
                cleanup["search_list_matches"] = keychain_list(observed) == previous_search
                cleanup["keychain_absent"] = not keychain.exists() and not keychain.is_symlink()
            except (OSError, ValueError, subprocess.SubprocessError):
                pass
            if not signing_complete or not all(cleanup.values()):
                save(output / "signing-failure.json", {"schema": "flere-macos-signing-failure-v1", "status": "failed",
                     "identity": selected, "signing_failed": not signing_complete, "cleanup": cleanup})
            require(all(cleanup.values()), "temporary signing keychain cleanup could not be verified; no signed artifact")
    entries = zip_entries(output)
    with zipfile.ZipFile(output / zip_name(selected["version"]), "x", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, data in entries.items():
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0)); info.create_system = 3
            info.external_attr = (0o100755 if name in COMPONENTS else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)
    names = payload_names() | set(LICENSES) | {zip_name(selected["version"]), "build.json"}
    receipt = {"schema": "flere-macos-signing-v1", "status": "signed_not_submitted", "identity": selected,
               "source_sha256": built["source_sha256"], "build_receipt_sha256": build_pin,
               "team_id": team, "certificate_sha1": fingerprint, "signatures": signatures,
               "keychain_removed": True, "search_list_restored": True, "files": inventory(output, names)}
    pin = save(output / "signed.json", receipt)
    validate_signed(output, pin, selected)
    return {"directory": str(output), "signed_receipt_sha256": pin}


def validate_signed(directory, pin, selected):
    raw = read(directory / "signed.json", 65536)
    require(sha(raw) == pin, "signed receipt differs from trusted digest")
    value = json.loads(raw)
    names = payload_names() | set(LICENSES) | {zip_name(selected["version"]), "build.json"}
    files = inventory(directory, names | {"signed.json"})
    require(value["schema"] == "flere-macos-signing-v1" and value["status"] == "signed_not_submitted"
            and value["identity"] == selected and value["keychain_removed"] is True and value["search_list_restored"] is True
            and value["files"] == {k: v for k, v in files.items() if k != "signed.json"}, "signed final asset set differs")
    require(files["build.json"]["sha256"] == value["build_receipt_sha256"], "original build receipt differs")
    built = built_document(json.loads(read(directory / "build.json", 65536)), selected)
    require(value["source_sha256"] == built["source_sha256"], "signed source differs")
    actual = pair(directory, selected, built["source_sha256"])
    for component in COMPONENTS:
        require(actual[component] == rebind(built["manifests"][component], read(directory / (component + "-" + TARGET))),
                "signing changed build/source identity")
        signature_requirement(component, value["team_id"], value["certificate_sha1"])
        require(value["signatures"][component] == {"identifier": "io.github.robert-cronin." + component,
                "team_id": value["team_id"], "hardened_runtime": True, "secure_timestamp": True}, "signature evidence differs")
    for name in LICENSES:
        require(files[name] == built["files"][name], "signing changed a source license")
    verify_zip(directory / zip_name(selected["version"]), zip_entries(directory))
    return value


def uuid(value):
    require(isinstance(value, str) and re.fullmatch(r"[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}", value), "invalid notary UUID")
    return value.lower()


def accepted_log(log, submission, archive_sha):
    require(uuid(log["jobId"]) == submission and log["status"] == "Accepted" and type(log["statusCode"]) is int
            and log["statusCode"] == 0 and log["sha256"] == archive_sha, "notary log is not acceptance of this exact ZIP")


def notarize(directory, state, selected, signed_pin, *, resume_pin=None):
    native(selected, credentials=True)
    configuration(os.environ, "notarize")
    signed = validate_signed(directory, signed_pin, selected)
    require(not state.absolute().is_relative_to(directory.absolute()), "notary state overlaps immutable signed input")
    archive = directory / zip_name(selected["version"])
    archive_sha = signed["files"][archive.name]["sha256"]
    if resume_pin is None:
        state = private_output(state)
        receipt = dict(schema="flere-macos-notary-v1", status="submitting", identity=selected,
                       signed_receipt_sha256=signed_pin, zip_sha256=archive_sha, submission_id=None)
        save(state / "notary.json", receipt)  # Intent precedes any upload; an ambiguous result never resubmits.
    else:
        state = private_output(state, existing=True)
        raw = read(state / "notary.json", 65536)
        require(sha(raw) == resume_pin, "notary resume receipt differs from trusted digest")
        receipt = json.loads(raw)
        require(receipt["schema"] == "flere-macos-notary-v1" and receipt["identity"] == selected
                and receipt["signed_receipt_sha256"] == signed_pin and receipt["zip_sha256"] == archive_sha,
                "notary resume source/ZIP differs")
        require(receipt["submission_id"] is not None, "submission result is ambiguous; recover the exact Apple submission ID, do not resubmit")
        uuid(receipt["submission_id"])
        if receipt["status"] == "Accepted":
            validate_notary(state, resume_pin, selected, signed_pin, archive_sha)
            return {"state": str(state), "notary_receipt_sha256": resume_pin, "status": "Accepted"}
        require(receipt["status"] in ("submitted", "pending"), "notary submission was rejected; never automatically resubmit")
    with tempfile.TemporaryDirectory(prefix="notary-key-", dir=state.parent) as temporary:
        home = Path(temporary)
        key = home / "AuthKey.p8"
        key.write_text(os.environ["MACOS_NOTARY_PRIVATE_KEY"]); key.chmod(0o600)
        auth = ["--key", key, "--key-id", os.environ["MACOS_NOTARY_KEY_ID"], "--issuer", os.environ["MACOS_NOTARY_ISSUER_ID"]]
        if receipt["submission_id"] is None:
            _, raw = command(["/usr/bin/xcrun", "notarytool", "submit", archive, "--no-wait",
                              "--output-format", "json", "--no-progress", *auth], home, timeout=300)
            receipt.update(submission_id=uuid(json.loads(raw)["id"]), status="submitted")
            save(state / "notary.json", receipt)
        submission = receipt["submission_id"]
        receipt["status"] = "pending"
        save(state / "notary.json", receipt)
        # A timed-out wait does not reject or duplicate the submission. Query the
        # same ID once afterward; preserve pending state if that request fails.
        command(["/usr/bin/xcrun", "notarytool", "wait", submission, "--timeout", "15m",
                 "--output-format", "json", "--no-progress", *auth], home, timeout=930, check=False)
        _, raw = command(["/usr/bin/xcrun", "notarytool", "info", submission, "--output-format", "json", *auth], home)
        observed = json.loads(raw)
        require(uuid(observed["id"]) == submission and observed["status"] in ("Accepted", "In Progress", "Invalid", "Rejected"),
                "notary status/ID differs")
        if observed["status"] != "In Progress":
            _, raw = command(["/usr/bin/xcrun", "notarytool", "log", submission, *auth], home)
            require(len(raw) <= 1024 * 1024, "notary log exceeds bound")
            log = json.loads(raw)
            if observed["status"] == "Accepted":
                accepted_log(log, submission, archive_sha)
            (state / "notary-log.json").write_bytes(raw)
            receipt.update(status=observed["status"], log_sha256=sha(raw))
    # No key data is in the state directory or receipt. Recheck all immutable
    # signed bytes after the network operations before recording acceptance.
    validate_signed(directory, signed_pin, selected)
    pin = save(state / "notary.json", receipt)
    return {"state": str(state), "notary_receipt_sha256": pin, "status": receipt["status"]}


def validate_notary(state, pin, selected, signed_pin, archive_sha):
    raw = read(state / "notary.json", 65536)
    require(sha(raw) == pin, "notary receipt differs from trusted digest")
    value = json.loads(raw)
    require(value["schema"] == "flere-macos-notary-v1" and value["status"] == "Accepted"
            and value["identity"] == selected and value["signed_receipt_sha256"] == signed_pin
            and value["zip_sha256"] == archive_sha, "notary receipt is not accepted for these signed bytes")
    log = read(state / "notary-log.json", 1024 * 1024)
    require(sha(log) == value["log_sha256"], "notary log digest differs")
    accepted_log(json.loads(log), uuid(value["submission_id"]), archive_sha)
    return value


def validate_accepted(directory, state, receipt_path, pin, selected, signed_pin, notary_pin):
    """Data-only aggregate seam; native verification is attested by its exact pin."""
    signed = validate_signed(directory, signed_pin, selected)
    notary = validate_notary(state, notary_pin, selected, signed_pin, signed["files"][zip_name(selected["version"])]["sha256"])
    raw = read(receipt_path, 65536)
    require(sha(raw) == pin, "final macOS acceptance receipt differs from trusted digest")
    value = json.loads(raw)
    require(value["schema"] == "flere-macos-accepted-v1" and value["status"] == "accepted_not_published"
            and value["identity"] == selected and value["source_sha256"] == signed["source_sha256"]
            and value["signed_receipt_sha256"] == signed_pin and value["notary_receipt_sha256"] == notary_pin
            and value["submission_id"] == notary["submission_id"] and value["checks"] == FINAL_CHECKS
            and value["files"] == signed["files"] and re.fullmatch(r"[0-9]+(?:\.[0-9]+){1,2}", value["runner_macos"]),
            "incomplete final macOS signed-byte acceptance")
    return value


def verify(directory, state, output, selected, signed_pin, notary_pin):
    native(selected)
    signed = validate_signed(directory, signed_pin, selected)
    notary = validate_notary(state, notary_pin, selected, signed_pin, signed["files"][zip_name(selected["version"])]["sha256"])
    require(not any(output.absolute().is_relative_to(path.absolute()) for path in (directory, state)), "runtime output overlaps immutable input")
    output = private_output(output)
    manifests = pair(directory, selected, signed["source_sha256"])
    checks = []
    with tempfile.TemporaryDirectory(prefix="v", dir=output.parent) as temporary:
        home = Path(temporary)
        env = environment(home)
        _, status = command(["/usr/sbin/spctl", "--status"], home)
        require(status.strip() == b"assessments enabled", "Gatekeeper assessments must remain enabled")
        packages = output / "packages"; packages.mkdir()
        for component in COMPONENTS:
            package = packages / component; package.mkdir()
            path = package / component
            path.write_bytes(read(directory / (component + "-" + TARGET))); path.chmod(0o755)
            (package / "manifest.json").write_bytes(release.json_bytes(manifests[component]))
            signature(path, component, signed["team_id"], signed["certificate_sha1"], home, online=True)
            quarantine = f"0083;{int(time.time()):x};FlereReleaseCI;" + notary["submission_id"]
            command(["/usr/bin/xattr", "-w", "com.apple.quarantine", quarantine, path], home)
            command(["/usr/sbin/spctl", "--assess", "--type", "execute", "--verbose=2", path], home)
            checks.append(component + ":signature_online_ticket_gatekeeper")
        baseline = windows.tree(home)
        for component in COMPONENTS:
            path = packages / component / component
            for flag in ("--build-info", "--version", "--help"):
                _, data = command([path, flag], home, env=env)
                if flag == "--build-info":
                    require(json.loads(data) == manifests[component]["build"], "signed native build identity differs")
                elif flag == "--version":
                    require(data.decode().startswith(component + " " + selected["version"] + " ")
                            and manifests[component]["build"]["build_id"] in data.decode(), "signed version/build differs")
                else:
                    require(b"--build-info" in data, "signed CLI help differs")
                require(windows.tree(home) == baseline, "stateless signed CLI wrote state")
                checks.append(component + ":" + flag[2:])
        core = packages / "flere/flere"
        for component in COMPONENTS:
            args = [core, "--state", home / "state", "install", packages / component]
            if component != "flere":
                args += ["--component", component]
            for _ in range(2):
                command(args, home, env=env)
                installed = home / ".local/bin" / component
                require(sha(read(installed)) == manifests[component]["payload"]["sha256"], "installation changed signed bytes")
                _, observed = command([installed, "--build-info"], home, env=env)
                require(json.loads(observed) == manifests[component]["build"], "installed signed identity differs")
            require(not (home / "state").exists(), "managed install started application state")
            checks.append(component + ":managed_install_reinstall")
    validate_signed(directory, signed_pin, selected)
    receipt = {"schema": "flere-macos-accepted-v1", "status": "accepted_not_published", "identity": selected,
               "source_sha256": signed["source_sha256"], "signed_receipt_sha256": signed_pin,
               "notary_receipt_sha256": notary_pin, "submission_id": notary["submission_id"],
               "checks": checks, "files": signed["files"], "runner_macos": platform.mac_ver()[0],
               "limits": ["Hosted native arm64 CI; no Intel/universal or physical desktop claim.",
                          "Quarantine/online Gatekeeper assessment is not an ordinary browser download acceptance.",
                          "Bare executables and ZIP are not stapled; online Apple ticket availability is required."]}
    pin = save(output / "accepted.json", receipt)
    validate_accepted(directory, state, output / "accepted.json", pin, selected, signed_pin, notary_pin)
    return {"directory": str(output), "accepted_receipt_sha256": pin}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("preflight", "build", "sign", "notarize", "verify"))
    for name in ("version", "commit", "run-id", "workflow-sha"):
        parser.add_argument("--" + name, required=True)
    for name in ("checkout", "directory", "output", "notary-state"):
        parser.add_argument("--" + name, type=Path)
    for name in ("build-receipt-sha256", "signed-receipt-sha256", "notary-receipt-sha256", "resume-receipt-sha256"):
        parser.add_argument("--" + name)
    args = parser.parse_args()
    selected = identity(args.version, args.commit, args.run_id, args.workflow_sha)
    required = {"preflight": [], "build": ["checkout", "output"], "sign": ["directory", "output", "build_receipt_sha256"],
                "notarize": ["directory", "notary_state", "signed_receipt_sha256"],
                "verify": ["directory", "notary_state", "output", "signed_receipt_sha256", "notary_receipt_sha256"]}
    require(all(getattr(args, key) is not None for key in required[args.phase]), "required phase inputs missing")
    if args.phase == "preflight":
        native(selected, credentials=True); configuration(os.environ, "preflight")
        result = {"status": "configured", "identity": selected}
    elif args.phase == "build":
        result = build(args.checkout.resolve(), args.output, selected)
    elif args.phase == "sign":
        result = sign(args.directory, args.output, selected, args.build_receipt_sha256)
    elif args.phase == "notarize":
        result = notarize(args.directory, args.notary_state, selected, args.signed_receipt_sha256,
                          resume_pin=args.resume_receipt_sha256)
    else:
        result = verify(args.directory, args.notary_state, args.output, selected,
                        args.signed_receipt_sha256, args.notary_receipt_sha256)
    print(json.dumps(result, indent=2))
    if args.phase == "notarize" and result["status"] != "Accepted":
        return 75 if result["status"] == "pending" else 1
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except PhaseError as error:
        print("macOS release phase stopped: " + str(error), file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError, zipfile.BadZipFile):
        # Never print native-tool exceptions/argv: an authentication failure can
        # include the certificate password or credential material in its context.
        print("macOS release phase failed; inspect the retained phase state. No publication was attempted.", file=sys.stderr)
        sys.exit(1)
