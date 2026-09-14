#!/usr/bin/env python3
"""Generate both source formulas from one verified immutable release archive."""
import argparse
import hashlib
from pathlib import Path
import re
import sys
import tarfile

sys.dont_write_bytecode = True
from verify import verify_inputs

COMPONENTS = {
    "flere": ("Flere", "Terminal workbench with persistent project sessions", "Cargo.toml", "*std_cargo_args"),
    "flere-connect": ("FlereConnect", "SSH companion for the Flere terminal workbench",
                      "companion/Cargo.toml", '*std_cargo_args(path: "companion")'),
}


def recipes(archive, expected, template):
    match = re.fullmatch(r"flere-([0-9]+\.[0-9]+\.[0-9]+)-source\.tar\.gz", archive.name)
    if not match or not re.fullmatch(r"[a-f0-9]{64}", expected):
        raise ValueError("need a versioned full source archive and trusted SHA-256")
    if archive.is_symlink() or not archive.is_file():
        raise ValueError("source archive must be a regular file")
    checksum = hashlib.sha256()
    with archive.open("rb") as file:
        for chunk in iter(lambda: file.read(65536), b""):
            checksum.update(chunk)
    if checksum.hexdigest() != expected:
        raise ValueError("source archive differs from the trusted SHA-256")
    version = match[1]
    verify_inputs(archive, version)
    result = {}
    for component, (klass, description, manifest, cargo_args) in COMPONENTS.items():
        text = template
        for key, value in {"CLASS": klass, "DESCRIPTION": description, "MANIFEST": manifest,
                           "CARGO_ARGS": cargo_args, "COMPONENT": component, "VERSION": version,
                           "SHA256": expected}.items():
            text = text.replace(f"@{key}@", value)
        result[component] = text
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_archive", type=Path)
    parser.add_argument("--sha256", required=True, help="expected digest from the verified release checksum list")
    parser.add_argument("--output", type=Path, default=Path(__file__).parent)
    parser.add_argument("--check", action="store_true", help="check generated recipes without writing")
    args = parser.parse_args()
    template = Path(__file__).with_name("formula.rb.in").read_text()
    rendered = recipes(args.source_archive, args.sha256, template)
    if args.check:
        for component, text in rendered.items():
            if (args.output / "Formula" / f"{component}.rb").read_text() != text:
                raise ValueError(f"formula differs from verified source recipe: {component}")
        print("Both formulas match the generated verified source recipes.")
    else:
        (args.output / "Formula").mkdir(parents=True, exist_ok=True)
        for component, text in rendered.items():
            (args.output / "Formula" / f"{component}.rb").write_text(text)
        print("Prepared both source formulas; nothing installed, committed or published.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"Homebrew source preparation stopped: {error}", file=sys.stderr)
        sys.exit(1)
