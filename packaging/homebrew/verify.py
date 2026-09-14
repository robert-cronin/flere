#!/usr/bin/env python3
"""Verify the source archive pinned by both unpublished Homebrew formulas."""
import argparse
import hashlib
from pathlib import Path
import re
import sys
import tarfile

COMPONENTS = ("flere", "flere-connect")
REQUIRED = (
    "Cargo.toml", "Cargo.lock", "LICENSE", "src/main.rs", "build-support/build.rs",
    "companion/Cargo.toml", "companion/Cargo.lock", "companion/src/main.rs",
    "src/assets/fonts/OFL.txt", "src/assets/fonts/LICENSE-Nerd-Fonts",
    "src/assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf",
)


def verify(archive, tap):
    identities = set()
    for component in COMPONENTS:
        text = (tap / "Formula" / f"{component}.rb").read_text()
        version = re.search(r'^  version "([0-9]+\.[0-9]+\.[0-9]+)"$', text, re.MULTILINE)
        checksum = re.search(r'^  sha256 "([a-f0-9]{64})"$', text, re.MULTILINE)
        if not version or not checksum:
            raise ValueError("formula needs an explicit version and SHA-256")
        version, checksum = version[1], checksum[1]
        url = f"https://github.com/robert-cronin/flere/releases/download/v{version}/flere-{version}-source.tar.gz"
        if f'  url "{url}"\n' not in text:
            raise ValueError("formula must use the versioned public full source archive")
        identities.add((version, checksum))
    if len(identities) != 1:
        raise ValueError("both formulas must use the same source release")
    version, expected = identities.pop()
    checksum = hashlib.sha256()
    if archive.is_symlink() or not archive.is_file():
        raise ValueError("source archive must be a regular file")
    with archive.open("rb") as source:
        for chunk in iter(lambda: source.read(65536), b""):
            checksum.update(chunk)
    if checksum.hexdigest() != expected:
        raise ValueError("source archive SHA-256 does not match both formulas")
    verify_inputs(archive, version)
    return version


def verify_inputs(archive, version):
    with tarfile.open(archive, "r:gz") as source:
        for required in REQUIRED:
            try:
                entry = source.getmember(f"flere-{version}/{required}")
            except KeyError as error:
                raise ValueError(f"full source archive is missing {required}") from error
            if not entry.isfile():
                raise ValueError(f"source input must be a regular file: {required}")
        for manifest in ("Cargo.toml", "companion/Cargo.toml"):
            with source.extractfile(f"flere-{version}/{manifest}") as file:
                text = file.read().decode()
            if f'version = "{version}"' not in text or 'rust-version = "1.98"' not in text:
                raise ValueError("source package version or Rust requirement differs")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source_archive", type=Path)
    parser.add_argument("--tap", type=Path, default=Path(__file__).parent)
    args = parser.parse_args()
    version = verify(args.source_archive, args.tap)
    print(f"Both formulas match the verified full Flere {version} source archive.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, tarfile.TarError) as error:
        print(f"Homebrew source verification stopped: {error}", file=sys.stderr)
        sys.exit(1)
