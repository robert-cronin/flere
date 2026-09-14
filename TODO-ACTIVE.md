# Flere — active work

## Distribution

- [x] Publish the verified [shell installer](scripts/install.sh) for `wget | sh`
  and `curl | sh` (also Bash). Source `83237c4` is public; its raw URL matches
  the reviewed bytes. Eight offline installer checks passed. Python 3.9+ is
  required; the bootstrap pins and verifies the published Python installer.
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
  Cargo distribution is core-only: the companion's external build script/shared
  imports are not a standalone Cargo package. Its exact source-package attempt
  failed before creating a crate; companion downloads and other channels remain separate.
- [x] Prepare the Linux `.deb` and AUR recipe with the glibc 2.39+ requirement.
  Corrected Debian archive/hash/mode checks and six native stateless CLI checks
  passed; the archive matches an independent Docker reproduction byte for byte.
  See [package validation](packaging/linux/README.md#validation-and-publication-follow-up).
- [x] Validate offline Debian-package installation, a same-payload revision
  upgrade, removal and purge using real APT/dpkg in disposable emulated amd64
  Ubuntu 24.04 containers. Both jobs passed their six installed CLI checks;
  synthetic state and all 222 unrelated package records were preserved.
- [x] Validate the AUR recipe with real Arch `makepkg` source verification/build,
  exact `.SRCINFO` comparison and offline pacman installation/removal in a
  disposable emulated amd64 container. Six installed stateless CLI checks passed;
  synthetic state and all 175 unrelated package records were preserved. Native
  Arch hardware and interactive terminal features were not tested.
- [ ] Publish the verified `.deb` and submit the AUR recipe. Both remain
  unpublished. Debian dependency downloading and upgrades between different
  Flere runtime versions remain untested.
- [x] Finish output inventory for the [Nix packaging draft](packaging/nix/README.md).
  All 15 native x86_64 Linux Docker phases passed: parsing, evaluation, both builds,
  declared install checks and exact output files/modes/ownership/hashes. The core's
  zlib input and the validator's missing-utility assumption are corrected.
- [ ] Complete Nix sandbox test-suite, interactive NixOS runtime and update
  ownership acceptance before promoting the packaging draft to supported status.
- [ ] Finish current Windows physical clipboard/SSH/draft/image acceptance, then
  publish the companion through Scoop and submit a WinGet manifest.
- [ ] Revalidate the public source on native Windows after incorporating the
  CRLF bootstrap fix. The September 14 handoff tested the earlier private base
  plus that patch, not a fresh public checkout; preserve that evidence distinction.
- [ ] Fix the initial horizontal/ladder hold pause on Windows legacy input;
  investigate console-owned press/release events. Do not lengthen the legacy
  timeout blindly, alter system repeat settings or add global keyboard monitoring.
- [x] Prepare the upstream RPM wrapper from the verified v0.3.0 release. Real
  rpmbuild, payload/license/dependency checks and offline RPM install/remove
  passed in emulated amd64 Fedora 44; six installed stateless CLI checks passed,
  with synthetic state and all 220 unrelated package headers unchanged.
- [ ] Sign/publish the RPM after reviewing its checksum and provenance. Native
  Fedora hardware, interactive features and runtime-version upgrades remain
  untested; this is an upstream wrapper, not a Fedora repository submission.
- [x] Prepare Chocolatey companion recipes from the same verified Windows ZIP,
  version and checksum as Scoop/WinGet. Nine offline generator tests pass, covering
  package inventory, provenance, escaping and AMD64 selection. No Windows core is
  included and no package is published.
- [ ] Run native `choco pack`, package inventory and Windows install/upgrade/remove
  checks before publishing Chocolatey. Additional channels require their own
  package/runtime checks and publisher access.
- [ ] Automate manifest/checksum generation from verified release assets and
  document upgrade ownership for each installation method.
- [x] Make the local in-app updater identify package-manager installations and
  display the appropriate upgrade command. Ambiguous ownership and removed
  executables keep Apply disabled; a Cargo receipt advancing beyond the running
  version retains manager guidance. Unit and combined core checks pass.
- [x] Extend installation-owner checks to coordinated remote updates, including
  explicit manual adoption, stale-action rejection and owner rechecks under the
  installation lock. Older endpoints/candidates and unknown or manager-owned
  commands stop before installation. Both strict Clippy/format checks and 49
  focused Rust tests pass; Linux core/companion and Windows companion cross-target
  compile checks pass. Published v0.3.0 still requires manual manager upgrades.
- [x] Complete local integrated validation and install the coordinated-owner
  update from `70083bd`. All 485 macOS arm64 tests, both strict Clippy/format checks
  and release builds passed. Both packages were independently verified and both
  live supervisors applied the update; seven exact session identities and their
  selections were preserved. Existing frontends need their normal UI reload.
- [ ] Complete native Linux manager and Windows runtime ownership acceptance.
- [ ] Automate the release process: exact version/commit selection, native builds
  and tests, final-package checksums, publication and per-channel updates. The owner
  selected a **Release button with an explicit version and commit**; ordinary
  pushes must not publish. The Linux workflow includes the complete source
  archive and has 32 source/promotion tests. Version 0.3.1 is prepared locally;
  approved release immutability and the main-only release environment are enabled.
  Hosted run `34825445057` stopped before publication: one test assumed GitHub CLI
  was absent from the runner. The isolated fixture and corrected source `70083bd`
  pass all 485 macOS and 482 native Linux tests in the serial baseline, both
  components' formatting/strict Clippy checks and both release builds. Final Linux
  artifact hashes, six stateless CLI calls and four ELF inspections passed;
  the binaries meet the glibc 2.39 baseline and left private state unchanged.
  The parallel UI failures below remain open; finalize and validate the corrected
  candidate before changed-commit release approval. Other targets and channel
  activation remain open. Earlier failed receipts remain retained. The hosted
  workflow uses the serial baseline; hosted execution and publication of a
  corrected source commit still require review and approval.
- [ ] Resolve and revalidate UI tests under parallel load. A complete native
  Linux four-thread live run at `70083bd` captured 211 passes and two failures,
  with no harness error or surviving children. This exposed an eight-row archive
  picker with no visible result row and a sidebar test reading preferences before
  its click was acknowledged. Both fixes pass their focused live tests; complete
  serial and parallel validation of the final source remains pending. The other
  four previously failing tests passed the captured run. Preserve all evidence.
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
  owns explicit link activation. The current local update from `2788f64` passed
  all 477 tests, both components' strict Clippy, formatting and release builds on
  macOS arm64. It is installed and applied with three exact session identities
  and selections preserved; the frontend has loaded the same build. Physical
  link activation remains unresolved.
- [x] Request host Shift mouse gestures before capture and during core/companion
  cleanup, including companions attached to older cores. Focused PTY/bridge and
  hyperlink/output checks passed; this does not establish the cause of the
  reported Ghostty failure.
- [ ] Revisit physical hyperlink activation in Ghostty and Windows Terminal when
  requested; the owner has parked this investigation. Clicking works in plain
  Ghostty/fish but still fails inside Flere, including Shift+Command-click. Both
  the screenshot instance's frontend and supervisor now have the updated build. Automated PTY/remote tests verify targets,
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
