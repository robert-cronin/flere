#!/usr/bin/env python3
"""Prepare a Windows companion ZIP and Scoop/WinGet manifests. Never publishes.

Inputs are the flat, verified files from release-assets.py. A generated manifest
does not establish native Windows acceptance or availability at its proposed URL.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import sys
import zipfile

REPOSITORY = "https://github.com/robert-cronin/flere"
IDENTIFIER = "RobertCronin.FlereConnect"
TARGETS = ("x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu")
MAX_PAYLOAD = 256 * 1024 * 1024
PROJECT = Path(__file__).resolve().parent.parent


def read_regular(path, maximum):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= maximum:
        raise ValueError(f"invalid regular input: {path.name}")
    data = path.read_bytes()
    if not 0 < len(data) <= maximum:
        raise ValueError(f"input changed or exceeds bound: {path.name}")
    return data


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def verify(assets, target):
    if target not in TARGETS:
        raise ValueError("only the Windows x86_64 companion is supported")
    name = f"flere-connect-{target}"
    raw = read_regular(assets / (name + ".manifest.json"), 65536)
    manifest = json.loads(raw)
    build, payload = manifest["build"], manifest["payload"]
    version = build["package_version"]
    source = manifest.get("source") or {}
    if (manifest["schema_version"] != 1 or build["component"] != "flere-connect"
            or build["target"] != target or payload["file_name"] != "flere-connect"
            or payload["download_file"] != name or build["profile"] != "release"
            or not isinstance(version, str)
            or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version)
            or not re.fullmatch(r"[0-9a-f]{64}", str(source.get("source_sha256", "")))
            or not re.fullmatch(r"[0-9a-f]{40}", str(source.get("git_commit", "")))
            or source.get("dirty") is not False):
        raise ValueError("invalid Windows release identity or source provenance")
    binary = read_regular(assets / name, MAX_PAYLOAD)
    if type(payload["bytes"]) is not int or len(binary) != payload["bytes"] or sha256(binary) != payload["sha256"]:
        raise ValueError("Windows payload size or SHA-256 differs from its manifest")
    if len(binary) < 64 or binary[:2] != b"MZ":
        raise ValueError("Windows payload is not a PE executable")
    pe = struct.unpack_from("<I", binary, 0x3c)[0]
    if pe > len(binary) - 6 or binary[pe:pe + 6] != b"PE\0\0\x64\x86":
        raise ValueError("Windows payload is not an x86_64 PE executable")
    return manifest, raw, binary


def write_zip(path, entries):
    # Fixed timestamps, names and modes; no source tree, host paths or metadata.
    with zipfile.ZipFile(path, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for name, data in entries.items():
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (0o100755 if name.endswith(".exe") else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, data)


def yaml_scalar(value):
    # JSON strings are quoted YAML scalars, including when upstream text has ':'
    # or YAML-reserved characters. All mapping keys below are fixed by this tool.
    return json.dumps(value, ensure_ascii=True)


def prepare(assets, output, target, license_path=PROJECT / "LICENSE"):
    if output.exists():
        raise ValueError("output directory already exists")
    manifest, raw, binary = verify(assets, target)
    license_bytes = read_regular(license_path, 65536)
    version = manifest["build"]["package_version"]
    name = f"flere-connect-{version}-{target}.zip"
    url = f"{REPOSITORY}/releases/download/v{version}/{name}"
    output.mkdir(parents=True, mode=0o700)
    write_zip(output / name, {
        "flere.exe": binary, "flere-connect.exe": binary,
        "manifest.json": raw, "LICENSE": license_bytes,
    })
    checksum = sha256((output / name).read_bytes())
    scoop = output / "scoop" / "bucket"
    scoop.mkdir(parents=True)
    (scoop / "flere.json").write_text(json.dumps({
        "version": version,
        "description": "OpenSSH and clipboard companion for a Linux or macOS Flere workbench",
        "homepage": REPOSITORY, "license": "MIT",
        "architecture": {"64bit": {"url": url, "hash": checksum}},
        "bin": ["flere.exe", "flere-connect.exe"],
        "notes": [
            "Requires an existing OpenSSH client and a Linux or macOS SSH host.",
            "Run flere ssh ALIAS. The Windows package is the companion, not a native Windows workbench.",
            "Use scoop update flere for this installation; the in-app updater manages a separate user installation.",
        ],
    }, indent=2) + "\n")
    winget = output / "winget" / "manifests" / "r" / "RobertCronin" / "FlereConnect" / version
    winget.mkdir(parents=True)
    common = f"PackageIdentifier: {IDENTIFIER}\nPackageVersion: {yaml_scalar(version)}\n"
    (winget / f"{IDENTIFIER}.yaml").write_text(common +
        "DefaultLocale: en-US\nManifestType: version\nManifestVersion: 1.10.0\n")
    (winget / f"{IDENTIFIER}.locale.en-US.yaml").write_text(common +
        "PackageLocale: en-US\nPublisher: robert-cronin\nPackageName: Flere Connect\n" +
        f"PackageUrl: {REPOSITORY}\nLicense: MIT\n" +
        f"LicenseUrl: {REPOSITORY}/blob/v{version}/LICENSE\n" +
        "ShortDescription: OpenSSH and clipboard companion for a Linux or macOS Flere workbench\n" +
        "Description: Requires an existing OpenSSH client and a remote Linux or macOS host. Run flere ssh ALIAS.\n" +
        "ManifestType: defaultLocale\nManifestVersion: 1.10.0\n")
    (winget / f"{IDENTIFIER}.installer.yaml").write_text(common +
        "InstallerType: zip\nNestedInstallerType: portable\nScope: user\n" +
        "Installers:\n- Architecture: x64\n  NestedInstallerFiles:\n" +
        "  - RelativeFilePath: flere.exe\n    PortableCommandAlias: flere\n" +
        "  - RelativeFilePath: flere-connect.exe\n    PortableCommandAlias: flere-connect\n" +
        f"  InstallerUrl: {yaml_scalar(url)}\n  InstallerSha256: {checksum.upper()}\n" +
        "ManifestType: installer\nManifestVersion: 1.10.0\n")
    result = {
        "schema_version": 1, "status": "prepared_not_published",
        "version": version, "target": target, "asset": name, "url": url,
        "sha256": checksum, "payload_sha256": manifest["payload"]["sha256"],
        "source_sha256": manifest["source"]["source_sha256"],
        "requires": ["native Windows acceptance at this payload SHA-256",
                     "winget validate and isolated Scoop install/upgrade/uninstall",
                     "publish ZIP and verify its anonymous download before publishing manifests"],
    }
    (output / "windows-distribution.json").write_text(json.dumps(result, indent=2) + "\n")
    (output / "SHA256SUMS.windows").write_text(f"{checksum}  {name}\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("assets", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--target", choices=TARGETS, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.assets, args.output, args.target), indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, struct.error) as error:
        print(f"Windows packaging stopped: {error}", file=sys.stderr)
        sys.exit(1)
