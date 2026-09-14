[Documentation](README.md) · [Quick start](getting-started.md)

# Install and distribute Flere

Flere's core workbench runs on Linux and macOS. The Windows application is the
OpenSSH/clipboard companion for a workbench on a Linux or macOS host.

## Available downloads

[Flere v0.3.0](https://github.com/robert-cronin/flere/releases/tag/v0.3.0) provides
tested Linux x86_64 GNU and macOS arm64 executables for the core and companion.
The release includes installation scripts, manifests, source and checksums.
Linux prebuilt executables require **glibc 2.39 or newer**. macOS downloads are
not Developer ID notarized; older macOS and Intel runtime acceptance are not
established by the current Apple Silicon release.

The [Unix installer](../scripts/install.py) installs a verified core/companion
pair under `~/.local/bin` and records the chosen update source. It needs Python 3,
but no Rust toolchain or source checkout. Follow the
[installation guide](getting-started.md#install-a-published-package).

[Flere v0.3.3](https://github.com/robert-cronin/flere/releases/tag/v0.3.3) provides
Linux x86_64 core/companion, a Debian package, complete source, manifests and
checksums. All eight anonymous downloads match the sealed immutable release.
This Linux release keeps v0.3.0 as the default installer release and does not
advance Mac or Windows prebuilt channels.

## Package-manager channels

Homebrew, the core Cargo crate and the Debian download are published. Other
channels retain their individual validation and publication requirements.

| Channel | Scope | Status |
| --- | --- | --- |
| [Homebrew tap](https://github.com/robert-cronin/homebrew-flere) | Source builds for macOS arm64 and Linux x86_64 | Published v0.3.2; native Linux fresh dependencies and version upgrade passed. Hosted macOS 15/26 arm64 default-prefix installs, fresh direct Rust and literal version upgrade passed |
| [Cargo / crates.io](https://crates.io/crates/flere/0.3.3) | Core source package for Linux/macOS | Published v0.3.3 through Trusted Publishing; public archive and all 124 source files verified. Companion excluded |
| [Debian `.deb`](#install-the-debian-package) | Linux x86_64 with glibc 2.39+ | v0.3.3 download published and independently inspected; native Ubuntu lifecycle and missing-Git download checks passed for v0.3.2 |
| [AUR](../packaging/linux/README.md) | Linux x86_64 with glibc 2.39+ | Arch build/metadata and emulated pacman install/remove/CLI checks passed. Account setup and submission pending |
| [Scoop / WinGet](../packaging/windows/README.md) | Windows x86_64 companion | Generator prepared; physical Windows acceptance and publication pending |
| [Nix draft](../packaging/nix/README.md) | Proposed source builds for Linux x86_64 | Both native strict-sandbox builds, 485 tests and exact inventories passed for v0.3.3 plus the declared parser patch. Profile ownership and interactive NixOS acceptance remain pending; not a supported installation method |
| [RPM](../packaging/linux/rpm/README.md) | Prebuilt Linux x86_64 with glibc 2.39+ | Build, payload/ownership checks, offline install/remove and six stateless CLI checks passed in emulated Fedora 44. Unsigned and unpublished |
| [Chocolatey](../packaging/windows/README.md#chocolatey-recipe) | Windows x86_64 companion | Recipe generation and nine offline checks pass; native packing, install/upgrade/remove and publication pending |

### Install with Homebrew

With a current Homebrew installation:

Homebrew 7 requires trusting the formula names before installation:

```sh
brew trust --formula robert-cronin/flere/flere robert-cronin/flere/flere-connect
brew install robert-cronin/flere/flere
flere --version
brew install robert-cronin/flere/flere-connect  # optional local SSH/clipboard companion
```

Homebrew builds locally from the pinned v0.3.2 source archive and supplies Rust
1.98 or newer as a build dependency. Ensure Homebrew's `bin` directory is on PATH.
Use `brew upgrade robert-cronin/flere/flere` after `brew update`; upgrade the
companion through Homebrew too if installed. See the
[tap guide](../packaging/homebrew/README.md) for removal and maintenance.

Native Linux and hosted macOS 15/26 arm64 checks passed source installation,
formula tests, stateless CLI and license checks, strict linkage, upgrade from core
v0.3.0_1/companion v0.3.0 to v0.3.2, and removal for both components. The macOS jobs
used the default `/opt/homebrew` prefix, empty private caches, fresh direct
Homebrew Rust and literal `brew upgrade`, with the normal sandbox enabled.
Preinstalled transitive dependencies were recorded; scoped trust preceded a
verified tap checkout, so automatic tap cloning was not tested. Disposable state
stayed unchanged. See the
[tap validation record](../packaging/homebrew/README.md#maintain-and-validate).

The source build uses the host's runtime; the prebuilt Linux glibc requirement
does not apply to these formulas. Intel Macs and Linux ARM remain excluded.
The macOS source-formula checks cover 15.7.9 and 26.6.2, not other OS versions or
physical terminal behavior. They do not extend acceptance of the separate
v0.3.0 macOS prebuilt downloads.

The [prepared binary casks](../packaging/homebrew-prebuilt/README.md) remain
unpublished: normal macOS Gatekeeper blocked execution of the unnotarized download
during an isolated install test. The source formulas keep macOS security checks
intact and require no Apple Developer Program membership from users.

### Install with Cargo

```sh
cargo install flere --locked --version 0.3.3
```

This builds the core with Rust 1.98+ and a system C linker; macOS needs Xcode
Command Line Tools. Cargo normally installs into `~/.cargo/bin`. The companion
is separate. See [Cargo setup](getting-started.md#install-with-cargo).

The published archive records source commit `ce6bb62` from the matching v0.3.3
release. Its registry checksum and anonymous download match the exact upload:
`81b184a93ae81cf524a090d17acf7cd58878d1761aee6de64f7ac4e42da11452`.

### Install the Debian package

On Ubuntu 24.04 x86_64, or another compatible Debian-based x86_64 system with
glibc 2.39 or newer:

```sh
wget -O flere_0.3.3-1_amd64.deb https://github.com/robert-cronin/flere/releases/download/v0.3.3/flere_0.3.3-1_amd64.deb && \
  echo 'dbc6bbfb450fcdf0e721660b52b5e4ca2022f7ba254127a6ceacb499585f3efe  flere_0.3.3-1_amd64.deb' | sha256sum --check && \
  sudo apt install ./flere_0.3.3-1_amd64.deb
```

The checksum must pass before APT runs. The package
includes both commands. APT installs missing declared dependencies; there is no
Flere APT repository or automatic package-feed update. Download a later reviewed
`.deb` and install it with APT to upgrade. Remove with `sudo apt remove flere`;
saved workspaces stay separate. Debian 12 and Ubuntu 22.04 have older glibc and
cannot run these prebuilt binaries. See the [package validation and limits](../packaging/linux/README.md).

## Upgrade through the installation owner

Use the package manager that installed Flere to upgrade or remove that copy.
Package recipes do not invoke Flere's per-user installer, create application
state, start sessions, alter SSH settings or approve native agent prompts.

| Installation method | Owner of upgrades and removal |
| --- | --- |
| Unix installer / Flere managed installation | Flere's verified install/update flow and its managed receipt; inspect the exact selected state before applying a running update |
| Homebrew source formulas | Homebrew, separately for `flere` and `flere-connect`; use `brew upgrade` and `brew uninstall` |
| Cargo registry or `cargo install --path` | Cargo; install the chosen verified version/source with Cargo and remove with `cargo uninstall flere`. The core crate excludes the companion |
| Debian package | APT/dpkg, using the reviewed `.deb` or configured package repository |
| RPM package | The owning RPM package manager, using the reviewed RPM; Flere does not replace its `/usr/bin` files |
| AUR package | Rebuild the reviewed AUR recipe and upgrade/remove the resulting package through pacman |
| Scoop / WinGet / Chocolatey companion | The chosen Windows manager owns both command aliases; use that same manager for upgrade/removal once its channel is available |
| Manual source/binary copy | The owner explicitly rebuilds/replaces that copy, or reviews adoption into Flere's managed installation; unknown copies are not adopted automatically |
| Nix draft | Update the pinned expression and rebuild through the owning Nix configuration. Draft validation does not establish a supported installation or in-app upgrade path |

This table describes ownership, not additional published channels. Avoid mixing
managers that provide the same command aliases. Detach the Windows companion
before manager upgrades/removal; the recipes do not kill a process to unlock its
executable. Existing user state and SSH configuration remain outside the packages.

The published v0.3.0 **Update Flere** action manages a separate per-user
installation and does not automatically identify a package-manager installation.
Use the original manager for those installations to avoid creating an additional
`~/.local/bin/flere` that takes precedence on PATH.

Version 0.3.1 checks the running executable's installation receipts or
system-package ownership before offering a local update. Recognized Homebrew,
Cargo, Debian and RPM installations show instructions for their manager; Flere
does not run those commands. Coordinated updates also inspect the selected remote
frontend and supervisor and the local companion. Manager-owned or unknown copies
require a manual upgrade through their owner. Verified manual per-user copies
show an explicit adoption choice in the local review; ownership is checked again
before replacement. Older endpoints and candidates without this capability require
one manual upgrade, including older builds with the same package version.
The unreleased v0.3.4 development source also recognizes verified local Nix
store outputs for both components. It shows guidance to update the owning Nix
configuration or profile and reopen, with in-app Apply disabled. The verified
scope is the default root-controlled local daemon/store; unproved `/nix/store`
paths remain unknown. This does not extend acceptance of the separate v0.3.3
Nix packaging draft or establish interactive NixOS/profile behavior.

Use `command -v flere` and `flere --build-info` to inspect the command your shell selects.

Updating package files does not prove a running supervisor or every attached UI
has loaded them. Inspect `flere build-status` for the selected state; see
[build identity and refresh](getting-started.md#check-which-build-is-running).
Package installation and removal do not kill running sessions or delete saved
workspaces. Avoid automatic cleanup of application data in package uninstallers.

## Preparing a release for multiple channels

Keep one immutable, tested executable per component/target on GitHub Releases.
Generate catalogue recipes from verified manifests and payload hashes, using
versioned `/releases/download/vVERSION/` URLs. Do not use a moving `latest` URL in
a version-pinned package or manually edit a checksum to silence a mismatch.

The existing generators cover different release inputs:

| Output | Verified input and generator |
| --- | --- |
| Flat executable/manifests and `SHA256SUMS` | `scripts/release-assets.py` checks prepared component packages; the manual Linux workflow seals eight files including the Debian wrapper (historical schema 1 uses seven) with `scripts/release-automation.py` |
| Debian, RPM and AUR | `scripts/linux-packages.py --descriptor-sha256 …` derives one lock from the sealed Linux release, including source/license pins; optional native builders preserve the same payloads. See the [Linux generator](../packaging/linux/README.md#prepare-a-later-sealed-linux-release) |
| Homebrew source formulas | `packaging/homebrew/render.py` checks the full source archive against its reviewed SHA-256 and generates both formulas. See the [tap maintenance instructions](../packaging/homebrew/README.md#maintain-and-validate) |
| Scoop, WinGet and Chocolatey | `scripts/windows-manifests.py` verifies the Windows companion, builds one deterministic ZIP and shares its exact version/URL/hash among all three recipes. See [Windows preparation](../packaging/windows/README.md) |

A Linux release cannot supply an absent Windows payload, and Windows recipes do
not package the core. The unnotarized Homebrew binary casks remain separate from
the source tap. These generators do not update Nix pins or publish a channel.

The packaging tools prepare local output only. Review the exact allowlisted files,
test installation/upgrade/removal on their real platform, and publish downloads
before any manifest that references them. Verify each anonymous download after
publication. A checksum detects changed bytes; it is not an independent publisher
signature or proof of platform acceptance.

Never publish a development cache directory wholesale. It may include unrelated
state, logs, credentials or private Git history. Each generator emits a small,
explicit set of distributable files; retain operational receipts privately.

See [active work](../TODO-ACTIVE.md) for remaining channels and branding/link
follow-up. The [release process](releasing.md) describes the implemented manual
Linux workflow, its completed hosted publication, and remaining signing/channel setup.
