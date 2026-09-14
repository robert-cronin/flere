# Flere — active work

## Distribution

- [x] Publish the [Homebrew source tap](https://github.com/robert-cronin/homebrew-flere)
  using the pinned, checksummed v0.3.0 archive. Published tap commit: `754e4de`.
- [x] Validate the isolated macOS arm64 source-formula lifecycle for core and
  companion: install, formula tests, stateless CLI checks, same-source revision
  upgrade and full removal. Disposable state/config stayed unchanged. This used
  cached Rust dependencies with `--ignore-dependencies`.
- [ ] Validate fresh Homebrew dependency provisioning, upgrades between release
  versions, and the native Linux x86_64 Homebrew lifecycle. Older macOS acceptance
  remains pending; Intel Macs and Linux ARM are excluded from this tap.
- [ ] Prepare and validate Cargo source packages; publish after registry access
  and package-name availability are established.
  The core 0.3.1 archive passed verification at `3c24227`: 184 extracted library
  tests, a release build, stateless CLI checks and an unchanged cached-build stamp
  on macOS arm64. This does not validate the companion crate. Registry publication
  remains pending and must use the matching version/tag without relabeling v0.3.0.
- [x] Prepare the Linux `.deb` and AUR recipe with the glibc 2.39+ requirement.
  Corrected Debian archive/hash/mode checks and six native stateless CLI checks
  passed; the archive matches an independent Docker reproduction byte for byte.
  See [package validation](packaging/linux/README.md#validation-and-publication-follow-up).
- [x] Validate offline Debian-package installation, a same-payload revision
  upgrade, removal and purge using real APT/dpkg in disposable emulated amd64
  Ubuntu 24.04 containers. Both jobs passed their six installed CLI checks;
  synthetic state and all 222 unrelated package records were preserved.
- [ ] Publish the verified `.deb`, complete Arch `makepkg`/pacman checks and submit
  the AUR recipe. Both remain unpublished. Debian dependency downloading and
  upgrades between different Flere runtime versions remain untested.
- [ ] Validate the prepared [Nix packaging draft](packaging/nix/README.md). No
  Nix parsing, evaluation or build has run; native runtime and update ownership
  acceptance also remain pending. Do not claim Nix support.
- [ ] Finish current Windows physical clipboard/SSH/draft/image acceptance, then
  publish the companion through Scoop and submit a WinGet manifest.
- [ ] Revalidate the public source on native Windows after incorporating the
  CRLF bootstrap fix. The September 14 handoff tested the earlier private base
  plus that patch, not a fresh public checkout; preserve that evidence distinction.
- [ ] Fix the initial horizontal/ladder hold pause on Windows legacy input;
  investigate console-owned press/release events. Do not lengthen the legacy
  timeout blindly, alter system repeat settings or add global keyboard monitoring.
- [ ] Add RPM/Chocolatey and additional channels after their package/runtime
  checks pass and publisher access is available.
- [ ] Automate manifest/checksum generation from verified release assets and
  document upgrade ownership for each installation method.
- [x] Make the local in-app updater identify package-manager installations and
  display the appropriate upgrade command. Ambiguous ownership and removed
  executables keep Apply disabled; a Cargo receipt advancing beyond the running
  version retains manager guidance. Unit and combined core checks pass.
- [ ] Extend installation-owner checks to coordinated remote updates. The current
  development checks cover the local updater only; published v0.3.0 still requires
  the documented package-manager upgrade instructions.
- [ ] Automate the release process: exact version/commit selection, native builds
  and tests, final-package checksums, publication and per-channel updates. The owner
  selected a **Release button with an explicit version and commit**; ordinary
  pushes must not publish. The Linux workflow includes the complete source
  archive and has 32 source/promotion tests. Version 0.3.1 is prepared locally;
  final candidate validation, release immutability/environment setup, a hosted
  run, other targets and channel activation remain open. The latest native Linux
  serial run passed 180 core unit tests and 209 of 210 live tests, both components'
  strict Clippy checks, companion tests and both release builds. The remaining
  live failure observed an empty search-cursor proof file before its write
  completed; the retained file contains the expected value. The fixture fix is
  prepared and awaits validation.
  The hosted workflow is configured for the same serial baseline; full native
  serial acceptance has not passed.
- [ ] Investigate six UI test failures observed under parallel/load conditions.
  A private runner error interrupted collection of their panic details; the root
  causes remain unconfirmed. Serial validation does not establish acceptance
  under parallel test load.
- [ ] Add Developer ID signing and notarization for prebuilt macOS packages after
  Apple enrollment and credential setup. Current binary casks are not published:
  ordinary Gatekeeper blocked their unnotarized executable in testing.

## Add project

- [x] Replace exact-path recall with a terminal directory browser and path
  completion in Add project. Remove the editable Name field; discover the project
  name from Git remote metadata, with a local directory-name fallback for offline
  folders or projects without a remote. Real PTY tests cover browsing, completion,
  cancellation, automatic naming and literal paths; inherited Git selectors cannot
  redirect the selected checkout. Published on `main` at `8e7b34a`; the verified
  local packages at `f9335c7` are installed and the selected supervisor has applied
  them. An already-open older frontend still needs its normal UI reload.

## Terminal hyperlinks

- [x] Preserve bounded HTTP(S) hyperlink metadata through terminal rendering,
  both panes, styled history and supervisor refresh. Older clients retain their
  text/style protocol; raw child escapes are never replayed. The outer terminal
  owns explicit link activation. Source `80bc6c2` passed all 476 tests, core and
  companion strict Clippy, formatting and release builds, plus Linux core and
  Windows companion all-target compile checks. The approved update from
  `80bc6c2` is installed; physical link activation remains unchecked.
- [x] Request host Shift mouse gestures before capture and during core/companion
  cleanup, including companions attached to older cores. Focused PTY/bridge and
  hyperlink/output checks passed; this does not establish the cause of the
  reported Ghostty failure.
- [ ] Confirm physical hyperlink activation in Ghostty and Windows Terminal with
  the updated build. Ghostty activation still fails in the reported session,
  including Shift+Command-click. Automated PTY/remote tests verify targets,
  clipping, history, exact-session refresh and local-prompt/disconnect cleanup;
  physical browser interaction remains unresolved. Previously discarded targets
  need child redraw/output.

## Branding and link audit

- [ ] Clean up obsolete local Railhand files and installation references after
  the move to Flere. Identify live dependencies before relocating state; preserve
  private history separately from the clean public repository. The development
  source reference and stale test-fixture connection entries are corrected;
  live legacy state and active working directories remain retained.

- [x] Audit current rendered branding and use Flere captures in the product tour.
  Current workbench/intro images and capture cells are verified; old recordings
  and measurements remain clearly labeled historical evidence.
- [x] Run the parallel source/docs/assets and product-link audit. The final checks
  passed 259 handbook links and a broader 323 local links/anchors; all five
  checked public product URLs returned HTTP 200. Legacy isolation tests and
  truthful historical records remain intact. This was a bounded product audit,
  not an exhaustive crawl of every external site.

An unchecked channel is not a claim that its installation command is available.
Published downloads and platform limits are documented in the installation guide.
