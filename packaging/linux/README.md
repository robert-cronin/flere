# Linux distribution packages

The checked-in definitions wrap the tested **v0.3.0 Linux x86-64 GNU release**. Both
`flere` and `flere-connect` travel together. The original binaries are preserved;
the package builder does not compile a changed checkout or execute either binary.

| Format | Current validation |
| --- | --- |
| Debian `.deb` | Native Ubuntu install, v0.3.0 → v0.3.2 upgrade, 12 CLI checks, ownership detection, removal and purge passed. The tested package passed independent inspection; v0.3.3 is published through the automated release. |
| AUR `flere-bin` | Native Arch `makepkg`, v0.3.0 → v0.3.3 pacman upgrade/removal and corrected licence metadata passed. A separate private v0.3.4 fixture passed core/companion ownership refusal. All 18 stateless checks passed; not submitted to AUR. |
| [RPM](rpm/README.md) | Native Fedora 44 container on Linux: v0.3.0 → v0.3.3 install/upgrade/remove, 12 CLI checks, core/companion ownership refusal and state/package preservation passed. The tested v0.3.3 wrapper is unsigned and unpublished; checked-in recipes remain v0.3.0. |
| Nix | [Source packaging draft](../nix/README.md). Native strict-sandbox builds, all 485 tests and exact inventories passed for v0.3.3 plus the declared parser correction. Separately, native Nix-on-Ubuntu core/companion ownership, UI refusal and normal profile removal passed for unpublished v0.3.4 source `2d52985`. Actual 0.3.3 → 0.3.4 profile upgrades and emulated NixOS core runtime also passed. Physical clipboard and external SSH remain unverified. |

Preparation is separate from publishing `.deb`/RPM assets or submitting the
AUR recipe. Official [AUR name search](https://aur.archlinux.org/rpc/v5/search/flere?by=name),
exact-info and Arch package queries returned no matching `flere-bin` package on
2026-09-14 at 19:00 UTC. This does not reserve a name; recheck before submission.
The intended package base is `flere-bin`, containing both commands. Maintainer
identity and account SSH publishing access remain pending.

## Published Debian download

[Flere v0.3.3](https://github.com/robert-cronin/flere/releases/tag/v0.3.3) includes
`flere_0.3.3-1_amd64.deb` (3,548,644 bytes), SHA-256
`dbc6bbfb450fcdf0e721660b52b5e4ca2022f7ba254127a6ceacb499585f3efe`. The Linux workflow passed all jobs, inspected the exact wrapper before
publication, and verified all eight public assets. Independent anonymous checks
matched the same bytes and all 333 source files/modes to `ce6bb62`.

Use the [download, checksum and APT commands](../../docs/distribution.md#install-the-debian-package).
This is a standalone package, not an APT repository. The native cross-version and
missing-dependency lifecycle receipts below belong to v0.3.2; v0.3.3 adds current
native build/test and wrapper-integrity evidence. The checked-in AUR/RPM lock
remains pinned to v0.3.0 until those channels are separately advanced.

## Compatibility and dependencies

The prebuilt binaries require **glibc 2.39 or newer**. Ubuntu 24.04 and the
[native Fedora 44 container](rpm/README.md#native-linux-version-upgrade-and-ownership-validation)
have passed the recorded x86-64 runtime checks. [Debian 12](https://packages.debian.org/bookworm/libc6) and
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

### Prepare a later sealed Linux release

Schema 1 releases contain seven files: both executables and manifests, the full
source archive, `release.json` and `SHA256SUMS`. Schema 2 support adds a verified
`.deb`; it is enabled for future releases after native lifecycle acceptance. With
Python 3.11 or newer, the package generator derives a lock from either format:

```sh
python3 scripts/linux-packages.py \
  --assets "$VERIFIED_LINUX_RELEASE" \
  --descriptor-sha256 "$REVIEWED_DESCRIPTOR_SHA256" \
  --output "$HOME/.cache/flere/tmp/linux-packages-new-release"
```

Use the descriptor digest recorded by the reviewed release workflow or retained
verification receipt. The tool reuses the release validator to check that pin,
the exact seven- or eight-file inventory, manifests, full source fingerprint, checksums and
recorded Linux acceptance before creating output. It reads the three license
files from that archive and takes the package timestamp from the normalized
source archive. It does not download files, install packages or execute the
release payloads.

The Release workflow also [prepares an allowlisted recipe artifact](../../docs/releasing.md#prepared-recipe-artifacts)
after anonymous public verification. It uses this generator without package-build
flags, adds the source Homebrew formulas and records exact source/descriptor/file
hashes. This preparation does not publish a catalogue or change checked-in pins.

The derived lock includes only the five base source inputs; a schema 2 Debian
wrapper is an output, never an input to another wrapper. Ordered input pins keep
the generated lock and recipes deterministic across processes.

This mode emits `release-lock.json` alongside the existing AUR/RPM recipes,
provenance and generated checksums. All three package formats use that same lock;
add `--deb` or `--rpm` only on a host with the corresponding build prerequisites.
`--revision N` changes the distribution package revision, starting at 1, without
changing the upstream version or payloads. The provenance records the trusted
descriptor digest. Review the generated lock and recipes before updating the
checked-in definitions; this command does not change them. `--check-recipes`
will correctly report drift until they match the selected release.

Without `--descriptor-sha256`, the existing v0.3.0 checked-in lock remains the
input. Generating a later package does not establish its package-manager
lifecycle or publication. The version-specific results below distinguish the
original v0.3.0 wrapper from later native v0.3.2 acceptance.

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
bytes and retained `--no-download`. Those emulated jobs did not test dependency
downloading, a public APT repository, other distributions or different-version
runtime upgrades; the subsequent native checks are recorded below.

### Native Ubuntu upgrade and ownership

A subsequent native x86-64 Ubuntu check installed the verified `0.3.0-1`
package, upgraded to `0.3.2-1`, then removed, reinstalled and purged it. All 12
installed CLI checks passed. Payloads/manifests, unrelated package records and
synthetic state were preserved as expected; no owned children survived and the
isolated container was removed. The tested `0.3.2-1` archive is 3,549,964 bytes,
SHA-256 `3695f1472f014987c4bb68eff85285b882e63e43feed701df1400c68801eb39f`.
Its exact bytes were copied back and passed the independent release inspector.

An empty test supervisor and attached frontend both identified `/usr/bin/flere`
as Debian-owned and returned the APT upgrade guidance. Coordinated update
preparation stopped before staging a replacement; the installed executable stayed
unchanged. This verifies the ownership protocol, not a visually inspected Update
modal. No shell or native chat was opened. The initial attempt stopped before
installation because a test parser mistook APT's `[amd64]` annotation for an old
version; the corrected parser checks the actual prior-version field.

This lifecycle used preinstalled dependencies and disabled networking. A separate
native Ubuntu check then removed Git, verified that both the installed command
and cached Git archive were absent, and installed the same exact `0.3.2-1` package
through normal APT resolution. APT downloaded Git `1:2.43.0-1ubuntu7.3` from the
signed Ubuntu repositories. The five installed Flere files, manifests, ownership,
permissions and control checksums matched the tested archive; Flere was removed
and Git remained installed. Unrelated package records, synthetic state and APT
source/keyring files stayed unchanged, the final dpkg audit was empty and the
isolated container was removed. No Flere runtime or native chat ran in this
additional dependency check.

This proves normal download/resolution of one deliberately missing mandatory
dependency. Other dependencies were preinstalled; neither check establishes a
pristine-machine all-dependency install, a Flere APT repository, live-session
migration or broader desktop behavior. The older emulated receipts remain separate.

### AUR validation

The original v0.3.0 archive has a package-metadata defect: its `license` entries
are `LICENSE-Nerd-Fonts` and `OFL-1.1`, despite all three correct licence files
being installed. The `package()` loop overwrote the first element of Bash's
`license` array. The generator and checked-in recipe now use `license_file`;
the declared MIT/OFL array, payload pins and `.SRCINFO` are unchanged. A real
Bash regression fails before the correction and passes afterward. Native
validation of newly generated v0.3.3 and private v0.3.4 packages passed below;
the original archive and receipt remain separate evidence with that limitation.

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

### Native Arch version upgrade and ownership

A subsequent run used that same pinned image on native x86_64 Linux. Normal
signed `pacman -Syu` provisioning completed before networking was disconnected.
The corrected recipe passed `makepkg --verifysource`, `.SRCINFO` comparison and
unprivileged packaging. Pacman then installed v0.3.0, upgraded to v0.3.3 and
removed it. The wrapper preserves the public v0.3.3 payloads from `ce6bb62`;
its explicit licence-loop correction comes from the later packaging helper.

A separate private fixture used both native offline release builds from exact
unpublished source `3401a77`. It passed installation, both actual pacman ownership
UIs, preparation refusal before staging and removal. All 18 stateless checks
across the three versions passed, alongside exact binary/manifest/licence hashes,
modes and corrected MIT/OFL metadata. The core and companion displayed pacman
guidance without assuming a package repository or AUR helper. The application
uses pacman's registration and metadata checks; independent acceptance hashing
verifies the tested payload bytes.

Synthetic state, unrelated package records and repository/public-keyring
configuration stayed unchanged during each lifecycle. Both frontends, their local
bridge and the supervisor exited normally; the exact container was removed and
its absence confirmed. The first attempt stopped before provisioning because the
helper rejected inherited repository signature settings. A read-only query of the
same image established the format, and the corrected helper resolves only an
empty override to the independently verified global policy. Explicit unsigned
repository policies remain rejected; no security setting was relaxed.

The tested v0.3.3 package has SHA-256
`984c09e9d12526f31ac429271c20a197630d0a923e0c736faaf3c923950a0027`.
The private v0.3.4 fixture has SHA-256
`8051f6cc75d226ae1b57f928ec7dd886989cdfee9a101c13f747a7d4beb07df4`.
Both are unsigned, unpublished validation artifacts. The checked-in recipe
retains its v0.3.0 payload pins. These checks do not establish physical desktop,
external SSH or AUR submission acceptance.

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
