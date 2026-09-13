#!/usr/bin/env python3
"""Prepare flat, verified public release assets locally. Never uploads or publishes.

Run on package directories produced by scripts/dev package. The output contains
target-specific executables/manifests, SHA256SUMS and an explicit publication plan.
Signing/notarization and actual platform acceptance remain separate prerequisites.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import sys


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def validate_release(manifests):
    """One publication uses one version/source; only actual target pairs must negotiate."""
    identities = set()
    targets = {}
    for _, manifest, _ in manifests:
        build = manifest["build"]
        version = build["package_version"]
        source = (manifest.get("source") or {}).get("source_sha256")
        if (not isinstance(version, str) or not version or
                not isinstance(source, str) or len(source) != 64 or
                any(c not in "0123456789abcdef" for c in source)):
            raise ValueError("release packages need a version and a recorded source SHA-256")
        identities.add((version, source))
        targets.setdefault(build["target"], {})[build["component"]] = build
    if len(identities) != 1:
        raise ValueError("all release packages must have the same version and source SHA-256")
    for target, components in targets.items():
        if "flere" not in components or "flere-connect" not in components:
            continue  # Core-only servers and companion-only Windows are valid.
        protocol = components["flere"]["compatibility"]["remote_protocol"]["current"]
        accepts = components["flere-connect"]["compatibility"]["remote_protocol"]["accepts"]
        if not isinstance(protocol, str) or not protocol or not isinstance(accepts, list) or protocol not in accepts:
            raise ValueError(f"release companion cannot read the paired core protocol for {target}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("packages", nargs="+", type=Path)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output directory already exists")
    # Validate every package before preparing any release output.
    manifests = []
    names = set()
    for directory in args.packages:
        manifest_path = directory / "manifest.json"
        if manifest_path.is_symlink() or not manifest_path.is_file() or manifest_path.stat().st_size > 65536:
            raise ValueError("invalid package manifest file")
        manifest = json.loads(manifest_path.read_bytes())
        payload = manifest["payload"]
        build = manifest["build"]
        if manifest["schema_version"] != 1 or build["component"] not in ("flere", "flere-connect") or payload["file_name"] != build["component"]:
            raise ValueError("unsupported package component/schema")
        name = payload.get("download_file")
        if name != f"{build['component']}-{build['target']}" or name in names or any(c not in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_." for c in name):
            raise ValueError("release needs unique fixed component-target asset names")
        names.add(name)
        binary = directory / payload["file_name"]
        if binary.is_symlink() or not binary.is_file() or not 0 < payload["bytes"] <= 256 * 1024 * 1024 or binary.stat().st_size != payload["bytes"] or digest(binary) != payload["sha256"]:
            raise ValueError("package payload differs from its manifest")
        manifests.append((directory, manifest, name))
    validate_release(manifests)
    args.output.mkdir(parents=True, mode=0o700)
    checksums = []
    entries = []
    for directory, manifest, name in manifests:
        binary = args.output / name
        shutil.copyfile(directory / manifest["payload"]["file_name"], binary)
        binary.chmod(0o755)
        if digest(binary) != manifest["payload"]["sha256"]:
            raise ValueError("package changed during release preparation")
        manifest_name = name + ".manifest.json"
        manifest_file = args.output / manifest_name
        manifest_file.write_text(json.dumps(manifest, indent=2) + "\n")
        checksums += [f"{digest(binary)}  {name}", f"{digest(manifest_file)}  {manifest_name}"]
        entries.append({"component": manifest["build"]["component"], "target": manifest["build"]["target"],
                        "package_version": manifest["build"]["package_version"],
                        "manifest_asset": manifest_name, "payload_asset": name})
    (args.output / "SHA256SUMS").write_text("\n".join(checksums) + "\n")
    plan = {"schema_version": 1, "status": "prepared_not_published", "assets": entries,
            "publication_requires": ["explicit public distribution repository/channel", "publisher provenance/signing policy",
                                      "platform runtime acceptance", "macOS signing/notarization decision for macOS assets"],
            "trust": "HTTPS distribution channel; SHA-256 detects corruption and is not an independent signature"}
    (args.output / "publication-plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    print(json.dumps(plan, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"Release preparation stopped: {error}", file=sys.stderr)
        sys.exit(1)
