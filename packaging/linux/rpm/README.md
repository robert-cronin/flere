# Upstream RPM package

The [generated spec](flere.spec) wraps the unchanged public **v0.3.0 Linux
x86_64** `flere` and `flere-connect` binaries. It is an upstream binary package;
no Fedora repository submission, signing or publication has occurred.

## Prepare

Use the flat, verified inputs described in the [Linux package guide](../README.md):
both binaries and manifests, the original source archive, and `SHA256SUMS`.
The builder reads only the three named license files from the verified archive.
With Python 3 and `rpmbuild` already installed:

```sh
python3 scripts/linux-packages.py \
  --assets "$HOME/.cache/flere/tmp/release-assets-0.3.0" \
  --output "$HOME/.cache/flere/tmp/rpm-package-0.3.0" \
  --rpm --check-recipes
```

The output directory must be new. RPM's generated shell-script paths do not
safely handle whitespace or shell/macro characters; the wrapper requires an
absolute output path using only ASCII letters, digits, `/`, `_`, `.` and `-`.
The command creates `flere-0.3.0-1.x86_64.rpm`, recipes, provenance and checksums.
It downloads nothing, installs no tools and executes neither payload. RPM build
staging, temporary files and an isolated build home are removed afterward.

The spec repeats the seven pinned SHA-256 checks before packaging and compares
all installed payload bytes without executing them. Stripping, debug extraction
and build-ID links are disabled to preserve the reviewed binaries. ELF dependency
discovery stays enabled. The package explicitly requires glibc 2.39+, libgcc,
`libz.so.1()(64bit)` and Git's `git-core` package. OpenSSH clients are recommended;
Vim and Wayland libraries are optional suggestions. It is not an ARM or musl
package, and compatibility with other RPM distributions has not been tested.

RPM owns the two `/usr/bin` commands, manifests under `/usr/share/flere`, and
three `%license` files under `/usr/share/licenses/flere`. The license expression
is `MIT AND OFL-1.1`. There are no runtime scriptlets, triggers or service files;
installation/removal starts no supervisor, shell or native chat. Use the system
package manager for upgrades, and check `command -v flere` if a user-installed
command earlier in `PATH` shadows `/usr/bin/flere`. User state is separate from
the package.

## Validation

On 2026-09-14, the package passed real RPM 6.0.2 build and lifecycle checks in a
disposable Fedora 44 amd64 container under Docker Desktop on macOS arm64. The
official Fedora image index was pinned to
`sha256:43b29f65a41eb9c35e1cd5323e3bdf3b655c2357a9f4f1ff2f9c2798e5045d80`;
the selected amd64 image was
`sha256:be9d65e2344d805cc11114319c685ecaa96b6d9b4350a0a6460cdb931babbd19`.
Build tools and dependencies were provisioned with Fedora's normal TLS and
package-signature checks. The build and lifecycle then ran with container
networking disconnected.

The private RPM contains exactly seven regular payload files and two owned
directories. SHA-256, file modes and root ownership matched the reviewed inputs;
RPM license flags, declared/automatic dependencies, empty scriptlet/trigger
queries and package digest checks passed. The 3,708,223-byte package SHA-256 is:

```text
0895f1b7428c4a5744eec44677405ec31cda31882fa4d4e9e8f7286ab4e0f509
```

An unprivileged build, `rpm --install --test`, installation, installed ownership
queries and `rpm -V` all passed. Both executables passed `--build-info`, `--help`
and `--version`; build metadata matched the v0.3.0 manifests. After removal,
all payload paths were absent. Synthetic user state and all 220 unrelated package
headers were unchanged. No Flere process remained, and the container was removed.
The host installation and real user state were never mounted.

The focused Fedora tests passed nine checks with two Debian-tool skips. They
also proved that a changed input fails the spec's checksum check and leaves no
staging tree. An initial test exposed RPM's whitespace-path failure; the wrapper
now rejects those paths before invoking RPM. This is emulated package lifecycle
and stateless CLI evidence, not native Fedora hardware, interactive terminal
acceptance, or an upgrade between runtime versions. No public RPM artifact or
repository is available yet.

The implementation follows the [RPM spec reference](https://rpm.org/docs/4.20.x/manual/spec.html)
and uses the [official Fedora container image](https://hub.docker.com/_/fedora).
