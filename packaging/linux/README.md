# Linux distribution packages

These definitions wrap the tested **v0.3.0 Linux x86-64 GNU release**. Both
`flere` and `flere-connect` travel together. The original binaries are preserved;
the package builder does not compile a changed checkout or execute either binary.

| Format | Current validation |
| --- | --- |
| Debian `.deb` | Corrected package built with `dpkg-deb` on Ubuntu 24.04 x86-64; directory-mode and cross-umask reproducibility regressions pass. Final corrected-artifact review and extraction receipt remain pending. No system installation performed. |
| AUR `flere-bin` | Recipe and `.SRCINFO` prepared; source checksum arrays and `package()` checked using Bash/coreutils on Linux. Full Arch `makepkg` build and pacman installation remain pending. Not submitted to AUR. |
| RPM | Queued until `rpmbuild` and an RPM-based validation environment are available. No untested RPM artifact is offered. |
| Nix | Queued until a Nix environment can validate loader/library handling and runtime behavior. No untested expression is offered. |

Preparation is separate from publishing the new `.deb` asset or submitting the
AUR recipe. AUR's read-only package-info check returned no `flere` or `flere-bin`
entry on 2026-09-14; availability must be checked again before submission.

## Compatibility and dependencies

The prebuilt binaries require **glibc 2.39 or newer**. Ubuntu 24.04 x86-64 is the
validated runtime. [Debian 12](https://packages.debian.org/bookworm/libc6) and
[Ubuntu 22.04](https://packages.ubuntu.com/jammy/libc6) have older glibc and cannot
run these prebuilt assets. This is not an ARM, musl, or all-distributions package.
Building from source for an older system is a separate compatibility exercise.

ELF inspection found `libc.so.6`, `libm.so.6`, `libgcc_s.so.1`, and, for the core,
`libz.so.1`. The packages declare their distro equivalents plus Git for workspace
operations. OpenSSH is recommended/optional for remote features; an installed
Vim/Neovim editor and a Wayland client library are optional feature dependencies.
Native chat harnesses remain independently installed child programs.

The package manager owns `/usr/bin/flere` and `/usr/bin/flere-connect`; update
these packages through that manager. Existing commands under `~/.local/bin` may
take precedence in `PATH`; inspect `command -v flere` and `command -v
flere-connect` when changing installation method. Packaging contains no install,
remove or service hooks and starts no supervisor, shell or native chat. User
state is separate from the package contents.

## Prepare from verified release assets

Download the flat assets named by [release.json](release.json) plus the release's
`SHA256SUMS` into a private directory. The reviewed lock pins the original
release source commit, implementation digest, executable/manifests and source
archive checksums. No `latest` URL is used. The AUR recipe uses fixed version
release URLs and exact-commit license URLs, all protected by SHA-256.

```sh
python3 scripts/linux-packages.py \
  --assets "$HOME/.cache/flere/tmp/release-assets-0.3.0" \
  --output "$HOME/.cache/flere/tmp/linux-packages-0.3.0" \
  --deb --check-recipes
```

Use an output directory that does not yet exist. `--deb` requires an installed
`dpkg-deb` supporting `--root-owner-group` (1.19 or newer). Omitting `--deb`
prepares the AUR files and provenance on other hosts. The builder does not fetch
files, install tools, upload artifacts, or change installed Flere.

The result contains `flere_0.3.0-1_amd64.deb` when requested, the AUR files,
`package-provenance.json`, and checksums for these outputs. Debian archive paths
are limited to the two binaries, their original manifests under
`/usr/share/flere`, and combined project/font license notices at
`/usr/share/doc/flere/copyright`. AUR installs the same binaries/manifests and
separate license notices under `/usr/share/licenses/flere-bin`.

Only the three named license files are read from the verified source archive.
No current checkout files, Git history, runtime state, or unrelated files from
the input directory enter the binary packages. Corrupt assets, substituted
checksums, mixed release identities, incompatible core/companion protocols and
symlinked inputs are rejected before output is created. Checksums establish
byte identity with the reviewed release; they are not a publisher signature.

## Validation and publication follow-up

```sh
python3 scripts/test_linux_packages.py
```

Tests use disposable directories under `~/.cache/flere/tmp` (override with
`FLERE_TEST_CACHE`). They exercise corruption/provenance rejection and actual
archive contents with `dpkg-deb`; Linux Bash/coreutils checks the AUR source
arrays and install function. Platform-specific checks report skips when tools
are absent. They never execute the inert test payloads.

Before an AUR submission, run `makepkg --verifysource`, `makepkg`, and
`makepkg --printsrcinfo` in an Arch environment, comparing the resulting metadata
with `.SRCINFO`; inspect the package and perform an isolated installation check.
An AUR maintainer identity must be supplied by the actual submitter. Publishing
the `.deb` likewise follows review of its checksum and provenance, with package
installation/removal validation tracked separately from extraction checks.

The formats follow the official [PKGBUILD reference](https://wiki.archlinux.org/title/PKGBUILD),
[makepkg manual](https://man.archlinux.org/man/makepkg.8.en),
[Debian binary control policy](https://www.debian.org/doc/debian-policy/ch-controlfields.html),
and [dpkg-deb manual](https://manpages.debian.org/bookworm/dpkg/dpkg-deb.1.en.html).
