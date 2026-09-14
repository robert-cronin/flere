# Linux distribution packages

These definitions wrap the tested **v0.3.0 Linux x86-64 GNU release**. Both
`flere` and `flere-connect` travel together. The original binaries are preserved;
the package builder does not compile a changed checkout or execute either binary.

| Format | Current validation |
| --- | --- |
| Debian `.deb` | Archive and independent reproduction verified; six native extracted CLI checks passed. Offline APT install, same-payload revision upgrade, removal and purge passed in emulated amd64 Ubuntu containers; unpublished. |
| AUR `flere-bin` | Real Arch `makepkg` verification/build and exact `.SRCINFO` comparison passed. Offline pacman install/remove and six installed stateless CLI checks passed in an emulated amd64 Arch container. Not submitted to AUR. |
| [RPM](rpm/README.md) | Verified payload build, archive/ownership checks, offline RPM install/remove and six installed stateless CLI checks passed in an emulated amd64 Fedora 44 container. Unsigned and unpublished. |
| Nix | [Source packaging draft](../nix/README.md). Native Docker parsing, evaluation, both builds and declared install checks passed. Final inventory and broader runtime/update-ownership acceptance remain pending; not a supported installation method. |

Preparation is separate from publishing `.deb`/RPM assets or submitting the
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
prepares the AUR files, RPM spec and provenance on other hosts. On an RPM build
host, use `--rpm` instead of `--deb`; see the [RPM guide](rpm/README.md). The builder does not fetch
files, install tools, upload artifacts, or change installed Flere.

The result contains `flere_0.3.0-1_amd64.deb` when requested, the AUR files,
RPM spec, `package-provenance.json`, and checksums for these outputs. Debian archive paths
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

On 2026-09-14, the corrected `flere_0.3.0-1_amd64.deb` passed archive and
extraction review. Its SHA-256 is:

```text
2cbfdce3edc269fd92ab3a9ef951cf3f3615639878885270c83c3d257dbbd997
```

The archive was byte-identical across umasks `0002` and `0077` and to an
independent Docker reproduction. Its exact five data files and two control
files were verified: directories and executables use mode `0755`; documentation,
manifests and control files use `0644`; all archive entries belong to root.
No links or package hooks were present. Payload SHA-256, the output checksum
manifest and Debian control `md5sums` checks passed.

A fresh native Linux check ran `--build-info`, `--help` and `--version` on each
extracted executable: all six commands succeeded, and embedded build metadata
matched the original release manifests. No application state or native chat was
created. This did not install the package, exercise APT install/upgrade/removal,
or rebuild the release binaries. The package remains unpublished.

Separate offline APT/dpkg lifecycle checks subsequently passed in two disposable
amd64 Ubuntu 24.04 containers under Docker Desktop emulation. The fresh-install
job exercised installation, exact file/manifest verification, removal, reinstall
and purge. The upgrade job first installed a synthetic `0.3.0-0` package with
identical payload bytes, then upgraded it to the untouched `0.3.0-1` candidate.
Only the fixture's `Version` and versioned `Provides` fields differed. This proves
a package-revision upgrade; it does not prove migration between runtime versions
or preservation of running sessions.

Both jobs passed all six installed `--build-info`, `--help` and `--version`
checks. All 222 unrelated package records and the synthetic user-state fixtures
remained unchanged. No child process survived; removed files and purged package
metadata were absent, and the final dpkg audit was empty. Both containers were
removed. The host installation and real user state were never mounted.

These jobs used cached dependencies, empty APT source lists and disabled container
networking. The first test attempt stopped before installation because
`--no-download` could not acquire the local input into APT's archive cache. The
successful harness prepopulated that disposable cache with hash-verified candidate
bytes and retained `--no-download`. Dependency downloading, a public APT repository,
other distributions and different-version runtime upgrades remain untested.

The AUR recipe also passed real Arch validation on 2026-09-14 in a disposable
amd64 container under Docker Desktop on macOS arm64. The official
`archlinux:base-devel-20260906.0.587075` image was pinned to:

```text
sha256:61f7de2dd88cc4ba1fe36c24cfe1a503c3936984492d6405eeab013ce6ac68c5
```

All seven fixed-URL inputs matched their reviewed hashes. `makepkg
--verifysource`, an unprivileged `makepkg` build, and `makepkg --printsrcinfo`
passed; generated `.SRCINFO` matched the checked-in file byte for byte. Archive
inspection verified the two original binaries, two manifests and three license
files, their modes and root ownership, with only makepkg's three metadata files
in addition. No links or install hooks were present. The private package's
SHA-256 is:

```text
97da8be5020503b4ad5f1bcf3244bff0042219b511c4cc4b6ceac6bc4a79126e
```

With container networking disabled, pacman installation, installed-file checks
and removal passed. All six installed `--build-info`, `--help` and `--version`
checks passed; embedded build metadata matched the v0.3.0 manifests. Synthetic
user state and all 175 unrelated package records remained unchanged, and removed
payloads were absent. The container was removed; no real user state or host
installation was mounted.

Dependency provisioning used Arch's documented HTTPS `XferCommand` after the
emulator rejected the built-in downloader's seccomp syscall. Package signatures
remained enforced, and the original pacman configuration was restored before
the offline lifecycle checks. An initial harness attempt stopped before install
when Rosetta created an empty cache directory; the completed run used separate
builder and stateless homes with that emulator directory present beforehand.
This validates an emulated Arch package lifecycle, not native Arch hardware,
interactive terminal features, or upgrades between Flere runtime versions. AUR
submission remains pending.

```sh
python3 scripts/test_linux_packages.py
```

Tests use disposable directories under `~/.cache/flere/tmp` (override with
`FLERE_TEST_CACHE`). They exercise corruption/provenance rejection and actual
archive contents with `dpkg-deb`; Linux Bash/coreutils checks the AUR source
arrays and install function. Where installed, real `rpmbuild`/RPM also checks
archive metadata, dependencies, payload digests/modes, license flags and
corruption rejection without executing the inert fixture payloads. Platform-specific checks report skips when tools
are absent. They never execute the inert test payloads.

Repeat the Arch checks when changing the recipe or release payload. Before an
AUR submission, refresh package-name availability and review the final recipe.
An AUR maintainer identity must be supplied by the actual submitter. Publishing
the `.deb` likewise follows review of its checksum and provenance, with package
installation/removal validation tracked separately from extraction checks.

The formats follow the official [PKGBUILD reference](https://wiki.archlinux.org/title/PKGBUILD),
[makepkg manual](https://man.archlinux.org/man/makepkg.8.en),
[pacman configuration manual](https://man.archlinux.org/man/pacman.conf.5.en),
[Debian binary control policy](https://www.debian.org/doc/debian-policy/ch-controlfields.html),
and [dpkg-deb manual](https://manpages.debian.org/bookworm/dpkg/dpkg-deb.1.en.html).
