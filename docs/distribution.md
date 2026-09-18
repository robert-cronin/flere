[Documentation](README.md) · [Quick start](getting-started.md)

# Install and distribute Flere

Flere's core workbench runs on Linux and macOS. The Windows application is the
OpenSSH/clipboard companion for a workbench on a Linux or macOS host.

## Available downloads

[Flere v0.3.12](https://github.com/robert-cronin/flere/releases/tag/v0.3.12) provides
Linux x86_64 core/companion, a Debian package, the Windows x86_64 MSVC companion
and its portable ZIP, complete source, manifests and checksums. All 11 anonymous
downloads match the sealed immutable release from source `5782540`, published by
[run 35300454496](https://github.com/robert-cronin/flere/actions/runs/35300454496).
This Linux/Windows release does not publish new macOS prebuilts.

The earlier v0.3.7 release remains tied to source `48e209ba` and
[run 34958704741](https://github.com/robert-cronin/flere/actions/runs/34958704741):
all nine required jobs passed; six macOS jobs were excluded. Windows passed 93
native tests with one physical clipboard ignore, plus six alias checks and native
recipe validation. Linux completed its core/companion checks and 191 Python tests
with one skip. The earlier v0.3.6 release retains its separate `8729733` evidence.
Linux prebuilt executables require **glibc 2.39 or newer**. See the
[verified Windows ZIP quickstart](../packaging/windows/README.md#install-the-public-windows-zip).

macOS arm64 prebuilt core/companion downloads remain at
[v0.3.0](https://github.com/robert-cronin/flere/releases/tag/v0.3.0), without
Developer ID notarization. Older macOS and Intel runtime acceptance are not
established by that release. Homebrew source formulas now track v0.3.8.

The Unix shell bootstrap pins the verified installer at commit `48e1cf9`. It
selects Linux v0.3.12 or the unsigned macOS arm64 v0.3.0 prebuilt from the
[per-platform release index](../packaging/channels/stable.json), independently of
GitHub's unchanged latest pointer. The v0.3.8 companion uses that same index for
default remote-core bootstrap. Intel Mac prebuilt selection is unavailable.
The [installation guide](getting-started.md#install-from-your-terminal) also
shows explicit version selection. The installer verifies a core/companion pair
under `~/.local/bin` and records each source; it needs Python 3, but no Rust
toolchain or source checkout.

Default v0.3.8 installs retain `default_channel` in their managed receipt. A later
explicit Update resolves the index during preparation; Apply uses the already
verified package. Explicit CLI and Advanced sources retain their saved pins;
the normal Update screen checks for the latest compatible release even with an
old Public receipt. Managed official-release windows check metadata periodically
and show availability, but never install in the background. Local packages keep
their source until an explicit update, and package-manager installs stay with
their manager. The unsigned macOS v0.3.0 route remains a legacy pin. See the
[receipt and reader limits](getting-started.md#install-a-published-package).

## Package-manager channels

Homebrew, the core Cargo crate, the Debian download and the owned Scoop bucket
are published. Other
channels retain their individual validation and publication requirements.

| Channel | Scope | Status |
| --- | --- | --- |
| [Homebrew tap](https://github.com/robert-cronin/homebrew-flere) | Source builds for macOS arm64 and Linux x86_64 | Published v0.3.8 from verified release source; earlier v0.3.2 native Linux and hosted macOS 15/26 arm64 lifecycle checks passed |
| [Cargo / crates.io](https://crates.io/crates/flere/0.3.12) | Core source package for Linux/macOS | Published v0.3.12 through Trusted Publishing from the matching GitHub source. Companion excluded |
| [Debian `.deb`](#install-the-debian-package) | Linux x86_64 with glibc 2.39+ | v0.3.12 download published and independently inspected; native Ubuntu lifecycle and missing-Git download checks passed for v0.3.2 |
| [AUR](../packaging/linux/README.md) | Linux x86_64 with glibc 2.39+ | Native Arch v0.3.0 → v0.3.3 lifecycle and corrected licence metadata passed; separate private v0.3.4 core/companion ownership refusal passed. Account setup and submission pending |
| [Scoop bucket](https://github.com/robert-cronin/scoop-flere) | Windows x86_64 companion | Published v0.3.8 recipe uses the verified public ZIP. Earlier owned-Git-bucket upgrade, ownership, local UI refusal and removal/preservation passed on candidate `4f3b693`; the new public bucket route and physical acceptance remain untested |
| [WinGet](../packaging/windows/README.md) | Windows x86_64 companion | Public v0.3.8 ZIP and native manifest validation passed. Earlier local-recipe upgrades, both installed-owner reports and removal/preservation passed. Community submission is deferred; physical acceptance remains pending |
| [Nix draft](../packaging/nix/README.md) | Proposed source builds for Linux x86_64 | The draft remains pinned to public v0.3.5 source `8744d35`, which passed 543 tests, strict-sandbox builds and exact inventories in [run 34950280814](https://github.com/robert-cronin/flere/actions/runs/34950280814). Historical v0.3.3 plus the declared parser patch passed 485 tests and strict-sandbox builds. Separate development ownership/UI, real 0.3.3 → 0.3.4 profile upgrade and emulated NixOS core runtime checks passed. Real SSH from the installed macOS companion to the Nix-built core passed; physical clipboard remains open. Not a supported installation method |
| [RPM](../packaging/linux/rpm/README.md) | Prebuilt Linux x86_64 with glibc 2.39+ | Native Fedora 44 container on Linux: v0.3.0 → v0.3.3 upgrade, 12 CLI checks, core/companion ownership refusal, preservation and removal passed. The v0.3.3 wrapper is unsigned and unpublished; checked-in recipes remain v0.3.0 |
| [Chocolatey](../packaging/windows/README.md#chocolatey-recipe) | Windows x86_64 companion | Native packing and public v0.3.4 → candidate v0.3.5 upgrade, both installed-owner reports and removal/preservation passed. Physical acceptance and catalogue publication remain pending |

The historical unpublished Windows candidate from exact source `422058c` passed 59 native
MSVC tests (one physical clipboard ignore), release packaging and six portable
alias checks in [run 34911230794](https://github.com/robert-cronin/flere/actions/runs/34911230794).
That run then failed on WinGet warnings. The corrected recipes passed
[run 34912671194](https://github.com/robert-cronin/flere/actions/runs/34912671194)
with WinGet 1.11.510 and Chocolatey 2.7.4, reusing the same ZIP. Both artifacts
were independently inspected; no installation, manager ownership or physical
acceptance is claimed by those runs.

The earlier public v0.3.5 Windows package at `8744d35` passed 81 native MSVC tests,
six portable alias checks, WinGet validation and Chocolatey packing; one physical
clipboard test remains explicitly ignored. Separate WinGet run `34929525551`
passed normal local-manifest install/remove and both aliases on the retained
422 payload. Chocolatey run `34930361884` passed installed ownership diagnostics,
normal removal and preservation of an explicitly initialized synthetic profile
on the retained `21cf68c` candidate. These are separate payload-specific records.
The corrected WinGet detector at `9d53f96` passed both installed-owner reports,
normal removal and preservation in
[run 34936386953](https://github.com/robert-cronin/flere/actions/runs/34936386953).
It is not included in public v0.3.4. Scoop public-ZIP installation, both aliases,
metadata preservation and normal Flere removal passed in
[run 34935706908](https://github.com/robert-cronin/flere/actions/runs/34935706908);
the overall run remains failed because extra Scoop self-removal timed out.
The fixture now leaves Scoop for disposable runner teardown. Later public
v0.3.4 → candidate v0.3.5 (`7e43fa1`) upgrades, both installed-owner reports and
normal removal/preservation passed for
[Scoop](https://github.com/robert-cronin/flere/actions/runs/34939641214),
[Chocolatey](https://github.com/robert-cronin/flere/actions/runs/34939646345) and
[WinGet](https://github.com/robert-cronin/flere/actions/runs/34943275275).
Those checks used local recipes and checksummed runner-local target downloads;
the rebuilt public v0.3.5 ZIP has different hashes. WinGet used an explicit
local manifest carrying the observed baseline ProductCode. Its disposable runner
removed the Store source before the preservation baseline, retaining the Microsoft
community source until VM teardown. Published v0.3.5 retains local-manifest
ownership detection. Later source `4f3b693` adds ordinary Scoop bucket and official
WinGet community ownership recognition; its 549 macOS tests and all eight checks
passed. The native Windows candidate passed 85 tests with one physical clipboard
ignore. Registered Scoop bucket [run 34947765377](https://github.com/robert-cronin/flere/actions/runs/34947765377)
passed installation, upgrade, both owner reports and normal removal with state
and PATH preserved. Its owned local Git bucket does not establish public
catalogue or desktop acceptance; WinGet catalogue checks remain pending. These
source changes are included in public v0.3.6; the earlier v0.3.5 ZIP lacks them.
The manager checks remain tied to their candidate bytes. The same `4f3b693` candidate then
passed local coordinated-update refusal through both installed Scoop aliases in
[run 34953623319](https://github.com/robert-cronin/flere/actions/runs/34953623319)
(workflow `65fc06c`). Actual owned-console cells showed complete Scoop guidance;
exact cancellation, restored console modes/code pages, preserved files/state and
normal process/package/bucket/listener cleanup passed. No update RPC or installer
staging was observed. The peer was local and passive; this does not prove
remote-core refusal, actual SSH or physical-terminal behavior. The prior
[run 34952666337](https://github.com/robert-cronin/flere/actions/runs/34952666337)
remains failed before UI startup because of a Python console-wrapper error,
with normal package/bucket cleanup. The owned Scoop bucket is now published at
v0.3.8. WinGet/Chocolatey UI refusal, their catalogue publication/upgrades and
physical acceptance remain pending; the historical bucket fixtures do not prove
the newly published bucket's download route.

### Install with Homebrew

With a current Homebrew installation:

Homebrew 7 requires trusting the formula names before installation:

```sh
brew trust --formula robert-cronin/flere/flere robert-cronin/flere/flere-connect
brew install robert-cronin/flere/flere
flere --version
brew install robert-cronin/flere/flere-connect  # optional local SSH/clipboard companion
```

Homebrew builds locally from the pinned v0.3.8 source archive and supplies Rust
1.98 or newer as a build dependency. Ensure Homebrew's `bin` directory is on PATH.
Use `brew upgrade robert-cronin/flere/flere` after `brew update`; upgrade the
companion through Homebrew too if installed. See the
[tap guide](../packaging/homebrew/README.md) for removal and maintenance.
Both published formulas at [tap commit `118dc16`](https://github.com/robert-cronin/homebrew-flere/commit/118dc161632343cee49fd48df914161804844c43)
match the verified v0.3.8 release recipes. This pin update did not repeat the
earlier full Homebrew lifecycle checks below.

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
cargo install flere --locked --version 0.3.8
```

This builds the core with Rust 1.98+ and a system C linker; macOS needs Xcode
Command Line Tools. Cargo normally installs into `~/.cargo/bin`. The companion
is separate. See [Cargo setup](getting-started.md#install-with-cargo).

The v0.3.8 archive records the matching release source `48e1cf9`. Its registry
API/index checksum and anonymous download were independently verified against
all 134 packaged source/metadata files. The 1,820,774-byte crate has SHA-256
`9e26bfa8356cfeedeed89c951ca1c14a7b11a6b3d61dad3bc25542de8a6ea4f8`.
The historical v0.3.7 crate retains its separate source `48e209ba` and SHA-256
`0f83abcb1ed7b13e61d5a7cd11c31d0f6b692240f1dc26922f4434f2f81eeef9`.

### Install the Debian package

On Ubuntu 24.04 x86_64, or another compatible Debian-based x86_64 system with
glibc 2.39 or newer:

```sh
wget -O flere_0.3.8-1_amd64.deb https://github.com/robert-cronin/flere/releases/download/v0.3.8/flere_0.3.8-1_amd64.deb && \
  echo 'b93d7d550af3aaca381579615b6283fe7ad7ade03034814ea2f209d971156dde  flere_0.3.8-1_amd64.deb' | sha256sum --check && \
  sudo apt install ./flere_0.3.8-1_amd64.deb
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
| Scoop / WinGet / Chocolatey companion | The chosen Windows manager owns both command aliases; use that same manager for upgrade/removal. The owned Scoop bucket is available; WinGet/Chocolatey catalogues remain pending |
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

The Homebrew companion parser correction included in v0.3.4 passed native Linux
acceptance at exact source `2d52985`: both normal source installs, formula tests,
six stateless checks and actual core/companion update UIs passed. Both refused
preparation before staging, kept their installed binaries and manager receipts
unchanged, then exited and uninstalled normally with synthetic state preserved.
The private formulas used a verified local source URL. The current v0.3.8 tap
retains this correction; the earlier v0.3.2 tap did not. No active sessions
or external SSH were exercised by that lifecycle check.

Version 0.3.4 also recognizes verified local Nix
store outputs for both components. It shows guidance to update the owning Nix
configuration or profile and reopen, with in-app Apply disabled. The verified
scope is the default root-controlled local daemon/store; unproved `/nix/store`
paths remain unknown. Exact source `2d52985` passed native Nix-on-Ubuntu
[installed-owner acceptance](https://github.com/robert-cronin/flere/actions/runs/34874904110):
both real update UIs refused preparation before staging, six stateless flags
passed, and normal private profile install/remove preserved synthetic user state
with clean runtime exits. This historical run does not replace the separate
v0.3.5 packaging proof above.
The later [actual profile upgrade](https://github.com/robert-cronin/flere/actions/runs/34883927251)
passed the literal 0.3.3 → 0.3.4 transition for both packages, all 12 stateless
checks and upgraded ownership/refusal UIs, preserving synthetic state through
normal removal. The separate [emulated NixOS run](https://github.com/robert-cronin/flere/actions/runs/34883876197)
passed core shell/editor/Git/detach and draft checks with normal application/VM
exits. It used TCG with KVM disabled. A separate native Nix-core check at
`2d52985` passed real OpenSSH from the installed macOS companion, two normal
attachments, exact session/draft retention and normal cleanup. Physical clipboard
remains unverified, and that SSH check did not use the Nix-packaged companion.
These runs do not establish NixOS/Home Manager upgrades.

Pacman ownership support, included in v0.3.4, was validated at source
`3401a77` after successful package registration and metadata checks. Both real
update UIs passed native Arch acceptance and refused preparation before staging.
Flere shows pacman guidance with Apply disabled; it does not assume a repository
or AUR helper. These metadata checks are distinct from the acceptance harness
comparing the installed payloads against their SHA-256 hashes.

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
| Flat executable/manifests and `SHA256SUMS` | `scripts/release-assets.py` checks prepared component packages; the manual Release workflow seals 11 files for schema 3 or 16 for schema 4. Historical schema 1/2 releases retain seven/eight files |
| Debian, RPM and AUR | `scripts/linux-packages.py --descriptor-sha256 …` derives one lock from the sealed Linux release, including source/license pins; optional native builders preserve the same payloads. See the [Linux generator](../packaging/linux/README.md#prepare-a-later-sealed-linux-release) |
| Homebrew source formulas | `packaging/homebrew/render.py` checks the full source archive against its reviewed SHA-256 and generates both formulas. See the [tap maintenance instructions](../packaging/homebrew/README.md#maintain-and-validate) |
| Scoop, WinGet and Chocolatey | `scripts/windows-manifests.py` verifies the Windows companion, builds one deterministic ZIP and shares its exact version/URL/hash among all three recipes. See [Windows preparation](../packaging/windows/README.md) |

A Linux release cannot supply an absent Windows payload, and Windows recipes do
not package the core. The unnotarized Homebrew binary casks remain separate from
the source tap. These generators do not update Nix pins or publish a channel.

The Release workflow now [prepares a Linux/Homebrew recipe artifact](releasing.md#prepared-recipe-artifacts)
after public verification, using these generators and an exact source/descriptor
receipt. Offline fixtures and generation from verified v0.3.3 assets passed;
hosted v0.3.5, v0.3.6 and v0.3.7 recipe generation passed in runs `34941117734`,
`34953872776` and `34958704741`. Preparation does not
publish recipes or change channel pins. The same release separately supplies the
verified Windows companion ZIP.

Review the exact allowlisted files, test installation/upgrade/removal on their
real platform, and publish downloads
before any manifest that references them. Verify each anonymous download after
publication. A checksum detects changed bytes; it is not an independent publisher
signature or proof of platform acceptance.

Never publish a development cache directory wholesale. It may include unrelated
state, logs, credentials or private Git history. Each generator emits a small,
explicit set of distributable files; retain operational receipts privately.

See [active work](../TODO-ACTIVE.md) for remaining channels and branding/link
follow-up. The [release process](releasing.md) describes the implemented manual
target-profile workflow, its completed hosted publication, and remaining signing/channel setup.
