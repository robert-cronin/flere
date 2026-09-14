#!/usr/bin/env python3
"""Prepare Linux packages from pinned public release assets, without installing.

The release lock is reviewed metadata, not a signature. All inputs must match it
and SHA256SUMS. Only named payloads, manifests and license text enter packages.
No executable is run, no network is accessed, and no source checkout is bundled.
"""
import argparse
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile

ROOT = Path(__file__).resolve().parent.parent
UPSTREAM = "https://github.com/robert-cronin/flere"
TARGET = "x86_64-unknown-linux-gnu"
COMPONENTS = ("flere", "flere-connect")
LICENSES = ("LICENSE", "src/assets/fonts/OFL.txt", "src/assets/fonts/LICENSE-Nerd-Fonts")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def load_lock():
    lock = json.loads((ROOT / "packaging/linux/release.json").read_bytes())
    if (lock["schema_version"] != 1 or lock["target"] != TARGET
            or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", lock["version"])
            or not isinstance(lock["revision"], int) or lock["revision"] < 1
            or not re.fullmatch(r"[0-9a-f]{40}", lock["source_commit"])
            or not re.fullmatch(r"[0-9a-f]{64}", lock["source_sha256"])
            or lock["minimum_glibc"] != "2.39"):
        raise ValueError("unsupported Linux release lock")
    expected = {f"{component}-{TARGET}{suffix}" for component in COMPONENTS
                for suffix in ("", ".manifest.json")}
    expected.add(f"flere-{lock['version']}-source.tar.gz")
    if set(lock["assets"]) != expected or set(lock["licenses"]) != set(LICENSES):
        raise ValueError("release lock must name exactly the supported public inputs")
    return lock


def read_file(path, maximum):
    if path.is_symlink() or not path.is_file() or not 0 < path.stat().st_size <= maximum:
        raise ValueError(f"invalid input file: {path.name}")
    data = path.read_bytes()
    if len(data) > maximum:
        raise ValueError(f"input grew beyond its size limit: {path.name}")
    return data


def verify(data, expected, name):
    if len(data) != expected["bytes"] or digest(data) != expected["sha256"]:
        raise ValueError(f"input differs from pinned release: {name}")


def lock_from_release(directory, descriptor_sha256, revision=1):
    """Derive package pins only after validating an independently pinned release."""
    if (not isinstance(descriptor_sha256, str)
            or not re.fullmatch(r"[0-9a-f]{64}", descriptor_sha256)
            or type(revision) is not int or revision < 1):
        raise ValueError("need a trusted release descriptor SHA-256 and positive package revision")
    raw = read_file(directory / "release.json", 65536)
    if digest(raw) != descriptor_sha256:
        raise ValueError("release descriptor differs from trusted SHA-256")
    descriptor = json.loads(raw)
    # Reuse the publication boundary: fixed inputs, source archive bounds,
    # payload/source identities, checksum list and recorded Linux acceptance.
    spec = importlib.util.spec_from_file_location("release_automation", ROOT / "scripts/release-automation.py")
    release = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(release)
    release.validate(directory, descriptor["version"], descriptor["commit"],
                     descriptor["run_id"], descriptor["workflow_sha"], descriptor_sha256)
    source_name = release.source_archive.name(descriptor["version"])
    archive_bytes = read_file(directory / source_name, release.source_archive.MAX_ARCHIVE)
    verify(archive_bytes, descriptor["assets"][source_name], source_name)
    licenses = {}
    with tarfile.open(fileobj=io.BytesIO(archive_bytes), mode="r:gz") as archive:
        for name in LICENSES:
            member = archive.getmember(f"flere-{descriptor['version']}/{name}")
            if not member.isfile() or not 0 < member.size <= 65536:
                raise ValueError(f"invalid source license member: {name}")
            data = archive.extractfile(member).read()
            licenses[name] = {"bytes": len(data), "sha256": digest(data)}
    return {"schema_version": 1, "version": descriptor["version"], "revision": revision,
            "target": TARGET, "source_commit": descriptor["commit"],
            "source_sha256": descriptor["source_sha256"],
            "source_date_epoch": descriptor["source_archive"]["mtime"],
            "minimum_glibc": descriptor["minimum_glibc"],
            "assets": descriptor["assets"], "licenses": licenses}


def load_inputs(directory, lock):
    """Validate everything before creating output; never extract an archive tree."""
    sums = {}
    for line in read_file(directory / "SHA256SUMS", 65536).decode().splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9_.-]+)", line)
        if not match or match[2] in sums:
            raise ValueError("malformed or duplicate SHA256SUMS entry")
        sums[match[2]] = match[1]
    inputs = {}
    for name, expected in lock["assets"].items():
        data = read_file(directory / name, 256 * 1024 * 1024)
        verify(data, expected, name)
        if sums.get(name) != expected["sha256"]:
            raise ValueError(f"SHA256SUMS does not match pinned asset: {name}")
        inputs[name] = data

    manifests = {}
    for component in COMPONENTS:
        name = f"{component}-{TARGET}"
        manifest = json.loads(inputs[name + ".manifest.json"])
        build, payload, source = manifest["build"], manifest["payload"], manifest["source"]
        if (manifest["schema_version"] != 1 or build["component"] != component
                or build["target"] != TARGET or build["package_version"] != lock["version"]
                or build["profile"] != "release" or source["dirty"] is not False
                or source["git_commit"] != lock["source_commit"]
                or source["source_sha256"] != lock["source_sha256"]
                or payload["file_name"] != component or payload["download_file"] != name):
            raise ValueError(f"manifest identity differs from pinned release: {component}")
        verify(inputs[name], payload, name)
        # Header check establishes an ELF64 little-endian x86-64 payload without
        # executing it. Native --build-info acceptance is a separate validation.
        if inputs[name][:6] != b"\x7fELF\x02\x01" or inputs[name][18:20] != b"\x3e\x00":
            raise ValueError(f"payload is not a Linux x86-64 ELF: {component}")
        manifests[component] = manifest
    protocol = manifests["flere"]["build"]["compatibility"]["remote_protocol"]["current"]
    accepts = manifests["flere-connect"]["build"]["compatibility"]["remote_protocol"]["accepts"]
    if not isinstance(protocol, str) or not protocol or not isinstance(accepts, list) or protocol not in accepts:
        raise ValueError("release companion does not accept the core protocol")

    source_name = f"flere-{lock['version']}-source.tar.gz"
    licenses = {}
    with tarfile.open(fileobj=io.BytesIO(inputs.pop(source_name)), mode="r:gz") as archive:
        for name in LICENSES:
            archive_name = f"flere-{lock['version']}/{name}"
            matches = [entry for entry in archive if entry.name == archive_name]
            if len(matches) != 1 or not matches[0].isfile() or matches[0].size > 65536:
                raise ValueError(f"invalid source license member: {name}")
            data = archive.extractfile(matches[0]).read()
            verify(data, lock["licenses"][name], name)
            licenses[Path(name).name] = data
    return inputs, licenses


def aur_files(lock):
    version = lock["version"]
    release_url = f"{UPSTREAM}/releases/download/v{version}"
    sources = [(name, f"{release_url}/{name}", item["sha256"])
               for name, item in lock["assets"].items() if not name.endswith(".tar.gz")]
    sources += [(Path(name).name,
                 f"https://raw.githubusercontent.com/robert-cronin/flere/{lock['source_commit']}/{name}",
                 lock["licenses"][name]["sha256"]) for name in LICENSES]
    fields = [
        ("pkgdesc", "Terminal workbench with a custom UI and SSH companion (prebuilt)"),
        ("pkgver", version), ("pkgrel", str(lock["revision"])), ("url", UPSTREAM),
        ("arch", "x86_64"), ("license", "MIT"), ("license", "OFL-1.1"),
        ("depends", "glibc>=2.39"), ("depends", "gcc-libs"), ("depends", "zlib"),
        ("depends", "git"),
        ("optdepends", "openssh: SSH connections and linked remote workspaces"),
        ("optdepends", "vim: external editor"),
        ("optdepends", "wayland: clipboard on Wayland sessions"),
        ("provides", f"flere={version}"), ("provides", f"flere-connect={version}"),
        ("conflicts", "flere"), ("conflicts", "flere-connect"),
        ("options", "!strip"), ("options", "!debug"),
    ]
    arrays = {"arch", "license", "depends", "optdepends", "provides", "conflicts", "options"}
    pkgbuild = "# Generated by scripts/linux-packages.py from the reviewed release lock.\n"
    pkgbuild += "# Upstream release bytes are preserved; package-manager installs start no sessions.\n"
    pkgbuild += "pkgname=flere-bin\n"
    for key in dict(fields):
        values = [value for name, value in fields if name == key]
        pkgbuild += f"{key}=" + ("(" + " ".join(repr(value) for value in values) + ")" if key in arrays else repr(values[0])) + "\n"
    pkgbuild += "source=(\n" + "".join(f"  '{name}::{url}'\n" for name, url, _ in sources) + ")\n"
    pkgbuild += "sha256sums=(\n" + "".join(f"  '{checksum}'\n" for _, _, checksum in sources) + ")\n\n"
    pkgbuild += '''package() {
  install -Dm755 "$srcdir/flere-x86_64-unknown-linux-gnu" "$pkgdir/usr/bin/flere"
  install -Dm755 "$srcdir/flere-connect-x86_64-unknown-linux-gnu" "$pkgdir/usr/bin/flere-connect"
  for component in flere flere-connect; do
    install -Dm644 "$srcdir/$component-x86_64-unknown-linux-gnu.manifest.json" "$pkgdir/usr/share/flere/$component.manifest.json"
  done
  for license in LICENSE OFL.txt LICENSE-Nerd-Fonts; do
    install -Dm644 "$srcdir/$license" "$pkgdir/usr/share/licenses/flere-bin/$license"
  done
}
'''
    srcinfo = "pkgbase = flere-bin\n"
    srcinfo += "".join(f"\t{key} = {value}\n" for key, value in fields)
    srcinfo += "".join(f"\tsource = {name}::{url}\n" for name, url, _ in sources)
    srcinfo += "".join(f"\tsha256sums = {checksum}\n" for _, _, checksum in sources)
    srcinfo += "\npkgname = flere-bin\n"
    return {"PKGBUILD": pkgbuild.encode(), ".SRCINFO": srcinfo.encode()}



def rpm_files(lock):
    """Upstream binary wrapper; not a Fedora source-package submission."""
    sources = [(name, f"{UPSTREAM}/releases/download/v{lock['version']}/{name}", item["sha256"])
               for name, item in lock["assets"].items() if not name.endswith(".tar.gz")]
    sources += [(Path(name).name,
                 f"https://raw.githubusercontent.com/robert-cronin/flere/{lock['source_commit']}/{name}",
                 lock["licenses"][name]["sha256"]) for name in LICENSES]
    spec = f"""# Generated by scripts/linux-packages.py from the reviewed release lock.
# This upstream prebuilt wrapper preserves the published payload bytes.
%global debug_package %{{nil}}
%global __brp_strip %{{nil}}
%global __brp_strip_comment_note %{{nil}}
%global _build_id_links none

Name:           flere
Version:        {lock['version']}
Release:        {lock['revision']}
Summary:        Terminal workbench with a custom UI and SSH companion
License:        MIT AND OFL-1.1
URL:            {UPSTREAM}
ExclusiveArch:  x86_64
BuildRequires:  coreutils
Requires:       glibc%{{?_isa}} >= {lock['minimum_glibc']}
Requires:       libgcc%{{?_isa}}
Requires:       libz.so.1()(64bit)
Requires:       git-core
Recommends:     openssh-clients
Suggests:       vim-enhanced
Suggests:       wayland-libs
Provides:       flere-connect = %{{version}}-%{{release}}
Conflicts:      flere-connect
"""
    spec += "".join(f"Source{i}:        {url}\n" for i, (_, url, _) in enumerate(sources))
    spec += """
%description
Flere provides a custom terminal UI for project workspaces and peer agents.
Includes the SSH companion. Native shells, editors and agents remain external.
This prebuilt x86-64 release requires glibc 2.39 or newer. Installation and
removal start no supervisor, shell or native chat.

%prep
cd '%{_sourcedir}'
sha256sum --check <<'FLERE_SHA256'
"""
    spec += "".join(f"{checksum}  {name}\n" for name, _, checksum in sources)
    spec += "FLERE_SHA256\n\n%build\n# The reviewed executables are packaged without recompilation.\n\n%install\n"
    destinations = []
    for name, _, _ in sources:
        if name in {f"{component}-{TARGET}" for component in COMPONENTS}:
            component = name.removesuffix("-" + TARGET)
            destinations.append((f"%{{_bindir}}/{component}", 0o755))
        elif name.endswith(".manifest.json"):
            component = name.removesuffix("-" + TARGET + ".manifest.json")
            destinations.append((f"%{{_datadir}}/flere/{component}.manifest.json", 0o644))
        else:
            destinations.append((f"%{{_licensedir}}/%{{name}}/{name}", 0o644))
    for i, (destination, mode) in enumerate(destinations):
        spec += f"install -Dm{mode:o} '%{{SOURCE{i}}}' '%{{buildroot}}{destination}'\n"
    spec += "\n%check\n# Check bytes without executing either prebuilt program.\n"
    for i, (destination, _) in enumerate(destinations):
        spec += f"cmp '%{{SOURCE{i}}}' '%{{buildroot}}{destination}'\n"
    spec += "\n%files\n%defattr(-,root,root,-)\n%dir %{_datadir}/flere\n%dir %{_licensedir}/%{name}\n"
    for destination, _ in destinations:
        prefix = "%license " if destination.startswith("%{_licensedir}") else ""
        spec += prefix + destination + "\n"
    return {"flere.spec": spec.encode()}


def build_rpm(output, inputs, licenses, lock):
    # RPM invokes generated shell scripts and does not safely handle whitespace
    # in its build paths. Restrict the new RPM path before writing any staging.
    output = output.resolve()
    if not re.fullmatch(r"[/A-Za-z0-9_.-]+", str(output)):
        raise ValueError("RPM output path must use only ASCII letters, digits, /, _, . or -")
    staging = output / ".rpm-staging"
    staging.mkdir(mode=0o700)
    try:
        for name in ("SOURCES", "SPECS", "BUILD", "BUILDROOT", "RPMS", "SRPMS", "home", "tmp"):
            (staging / name).mkdir(mode=0o700)
        for name, data in dict(inputs, **licenses).items():
            path = staging / "SOURCES" / name
            path.write_bytes(data)
            path.chmod(0o644)
        spec = staging / "SPECS/flere.spec"
        spec.write_bytes(rpm_files(lock)["flere.spec"])
        environment = dict(os.environ, HOME=str(staging / "home"),
                           XDG_CONFIG_HOME=str(staging / "home/.config"),
                           XDG_CACHE_HOME=str(staging / "home/.cache"),
                           SOURCE_DATE_EPOCH=str(lock["source_date_epoch"]),
                           TMPDIR=str(staging / "tmp"))
        subprocess.run(["rpmbuild", "-bb", "--target", "x86_64",
                        "--define", f"_topdir {staging}",
                        "--define", f"_tmppath {staging / 'tmp'}",
                        "--define", "_buildhost localhost",
                        "--define", "use_source_date_epoch_as_buildtime 1",
                        "--define", "clamp_mtime_to_source_date_epoch 1", str(spec)],
                       env=environment, check=True)
        name = f"flere-{lock['version']}-{lock['revision']}.x86_64.rpm"
        artifacts = list((staging / "RPMS").rglob("*.rpm"))
        if len(artifacts) != 1 or artifacts[0].name != name or artifacts[0].is_symlink():
            raise ValueError("rpmbuild did not produce exactly the expected binary package")
        artifact = output / name
        shutil.copyfile(artifacts[0], artifact)
        artifact.chmod(0o644)
        return artifact
    finally:
        shutil.rmtree(staging)


def deb_files(inputs, licenses):
    files = {}
    for component in COMPONENTS:
        name = f"{component}-{TARGET}"
        files[f"usr/bin/{component}"] = (inputs[name], 0o755)
        files[f"usr/share/flere/{component}.manifest.json"] = (inputs[name + ".manifest.json"], 0o644)
    copyright_text = f"Flere: {UPSTREAM}\n\n".encode()
    for name, data in licenses.items():
        copyright_text += f"===== {name} =====\n\n".encode() + data + b"\n"
    files["usr/share/doc/flere/copyright"] = (copyright_text, 0o644)
    return files


def build_deb(output, inputs, licenses, lock):
    staging = output / ".deb-staging"
    scratch = output / ".deb-tmp"
    staging.mkdir(mode=0o755)
    scratch.mkdir(mode=0o700)
    try:
        files = deb_files(inputs, licenses)
        for name, (data, mode) in files.items():
            path = staging / name
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
            path.write_bytes(data)
            path.chmod(mode)
        control = staging / "DEBIAN"
        control.mkdir(mode=0o755)
        size = sum((len(data) + 1023) // 1024 for data, _ in files.values())
        (control / "control").write_text(f"""Package: flere
Version: {lock['version']}-{lock['revision']}
Section: devel
Priority: optional
Architecture: amd64
Maintainer: robert-cronin <8540764+robert-cronin@users.noreply.github.com>
Installed-Size: {size}
Depends: libc6 (>= 2.39), libgcc-s1, zlib1g, git
Recommends: openssh-client
Suggests: vim | neovim, libwayland-client0
Provides: flere-connect (= {lock['version']}-{lock['revision']})
Conflicts: flere-connect
Homepage: {UPSTREAM}
Description: terminal workbench and SSH companion
 Flere provides a custom terminal UI for project workspaces and peer agents.
 Includes the SSH companion. Native shells, editors and agents remain external.
 This prebuilt x86-64 release requires glibc 2.39 or newer.
""")
        (control / "md5sums").write_text("".join(
            f"{hashlib.md5(data, usedforsecurity=False).hexdigest()}  {name}\n"
            for name, (data, _) in sorted(files.items())))
        for path in control.iterdir():
            path.chmod(0o644)
        epoch = lock["source_date_epoch"]
        # mkdir(parents=True) creates intermediate directories with the process
        # umask, regardless of the requested leaf mode. Normalize this new,
        # exclusively owned staging tree so archive directories are always 0755.
        staging.chmod(0o755)
        for path in sorted(staging.rglob("*"), reverse=True):
            if path.is_dir():
                path.chmod(0o755)
            os.utime(path, (epoch, epoch))
        os.utime(staging, (epoch, epoch))
        artifact = output / f"flere_{lock['version']}-{lock['revision']}_amd64.deb"
        environment = dict(os.environ, SOURCE_DATE_EPOCH=str(epoch), TMPDIR=str(scratch.resolve()))
        subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "--build", str(staging), str(artifact)],
                       env=environment, check=True)
        artifact.chmod(0o644)
        return artifact
    finally:
        shutil.rmtree(staging)
        shutil.rmtree(scratch)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--assets", required=True, type=Path, help="audited flat release-assets directory")
    parser.add_argument("--output", required=True, type=Path, help="new private home-cache output directory")
    parser.add_argument("--deb", action="store_true", help="also build .deb with installed dpkg-deb >= 1.19")
    parser.add_argument("--rpm", action="store_true", help="also build .rpm with installed rpmbuild")
    parser.add_argument("--check-recipes", action="store_true", help="verify checked-in AUR/RPM recipes match the lock")
    parser.add_argument("--descriptor-sha256", help="derive pins from this trusted SHA-256 of the assets' release.json")
    parser.add_argument("--revision", type=int, help="package revision for a descriptor-derived release (default: 1)")
    args = parser.parse_args(argv)
    if args.output.exists() or args.output.is_symlink():
        parser.error("output already exists")
    if args.deb and shutil.which("dpkg-deb") is None:
        parser.error("--deb requires installed dpkg-deb; no system tools are installed automatically")
    if args.rpm and shutil.which("rpmbuild") is None:
        parser.error("--rpm requires installed rpmbuild; no system tools are installed automatically")
    if args.revision is not None and args.descriptor_sha256 is None:
        parser.error("--revision requires --descriptor-sha256")
    derived = args.descriptor_sha256 is not None
    lock = (lock_from_release(args.assets, args.descriptor_sha256,
                              1 if args.revision is None else args.revision) if derived else load_lock())
    inputs, licenses = load_inputs(args.assets, lock)
    recipes = aur_files(lock)
    rpm_recipes = rpm_files(lock)
    if args.check_recipes:
        for name, data in recipes.items():
            if (ROOT / "packaging/linux/aur/flere-bin" / name).read_bytes() != data:
                raise ValueError(f"checked-in AUR recipe is stale: {name}")
        for name, data in rpm_recipes.items():
            if (ROOT / "packaging/linux/rpm" / name).read_bytes() != data:
                raise ValueError(f"checked-in RPM recipe is stale: {name}")
    args.output.mkdir(parents=True, mode=0o700)
    aur = args.output / "aur/flere-bin"
    aur.mkdir(parents=True, mode=0o755)
    for name, data in recipes.items():
        (aur / name).write_bytes(data)
        (aur / name).chmod(0o644)
    rpm = args.output / "rpm"
    rpm.mkdir(mode=0o755)
    for name, data in rpm_recipes.items():
        (rpm / name).write_bytes(data)
        (rpm / name).chmod(0o644)
    if args.deb:
        build_deb(args.output, inputs, licenses, lock)
    if args.rpm:
        build_rpm(args.output, inputs, licenses, lock)
    provenance = {"schema_version": 1, "status": "prepared_not_published", "release": lock,
                  "upstream_release": f"{UPSTREAM}/releases/tag/v{lock['version']}",
                  "trust": "Pinned SHA-256 verifies reviewed bytes; not an independent signature"}
    if derived:
        provenance["release_descriptor_sha256"] = args.descriptor_sha256
        (args.output / "release-lock.json").write_text(json.dumps(lock, indent=2) + "\n")
    (args.output / "package-provenance.json").write_text(json.dumps(provenance, indent=2) + "\n")
    artifacts = sorted(path for path in args.output.rglob("*") if path.is_file())
    (args.output / "SHA256SUMS").write_text("".join(
        f"{digest(path.read_bytes())}  {path.relative_to(args.output).as_posix()}\n" for path in artifacts))
    print(json.dumps({"status": "prepared_not_published", "artifacts": [str(p.relative_to(args.output)) for p in artifacts]}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, tarfile.TarError, subprocess.CalledProcessError) as error:
        print(f"Linux package preparation stopped: {error}", file=sys.stderr)
        sys.exit(1)
