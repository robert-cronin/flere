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

## Package-manager channels

The initial channels are being prepared. An entry below does not imply an
installation command is already available in a public catalogue.

| Channel | Scope | Status |
| --- | --- | --- |
| [Homebrew tap](../packaging/homebrew/README.md) | Source builds for macOS arm64 and Linux x86_64 | Source archive/formulas and isolated Mac build checked; publication pending |
| [Cargo / crates.io](../packaging/cargo/README.md) | Core source package | Archive verified; registry publication pending |
| [Linux `.deb` and AUR](../packaging/linux/README.md) | Linux x86_64 with glibc 2.39+ | Corrected Debian package built; final artifact review pending. AUR submission and full Arch installation pending |
| [Scoop / WinGet](../packaging/windows/README.md) | Windows x86_64 companion | Generator prepared; physical Windows acceptance and publication pending |
| Nix / RPM / Chocolatey | Additional installations | Queued; not validated or published |

Homebrew currently builds from the pinned source archive. Its native binaries
use the build host's runtime; the prebuilt Linux glibc requirement does not apply
to those source formulas. Building successfully is still distinct from validating
all runtime features on a new OS/version.

The [prepared binary casks](../packaging/homebrew-prebuilt/README.md) remain
unpublished: normal macOS Gatekeeper blocked execution of the unnotarized download
during an isolated install test. The source formulas keep macOS security checks
intact and require no Apple Developer Program membership from users.

The verified Cargo archive comes from a later packaging commit than the v0.3.0
prebuilt binaries. It is not attached to the original release as if it shared that
source identity. Choose a new matching version/tag when enabling registry
publication; the current archive is preparation evidence only.

## Upgrade through the installation owner

Use the package manager that installed Flere to upgrade or remove that copy.
Package recipes do not invoke Flere's per-user installer, create application
state, start sessions, alter SSH settings or approve native agent prompts.

The published v0.3.0 **Update Flere** action manages a separate per-user
installation and does not automatically identify a package-manager installation.
Use the original manager for those installations to avoid creating an additional
`~/.local/bin/flere` that takes precedence on PATH.

The development version checks the running executable's installation receipts or
system-package ownership before offering a local update. Recognized Homebrew,
Cargo, Debian and RPM installations show instructions for their manager; Flere
does not run those commands. This check does not yet cover coordinated remote
updates. Use `command -v flere` and `flere --build-info` to inspect the command
your shell selects.

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
Linux workflow, its pending hosted validation, and remaining signing/channel setup.
