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
- [x] Prepare and validate the core Cargo source package at `19637da`: normal
  extracted-package verification, all 186 library tests, a release build,
  stateless CLI checks and an unchanged cached-build stamp passed on macOS arm64.
  The archive contains 124 reviewed files and matches the exact clean source.
- [x] Publish the verified core Cargo package as v0.3.1 from matching source
  `7f5c5eb`. Normal Cargo publication succeeded; the registry checksum and
  anonymous archive both match the reviewed 124-file package. Installation is
  `cargo install flere --locked --version 0.3.1`. The owner has configured the
  exact GitHub Trusted Publisher for subsequent releases.
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
- [x] Automate manifest/checksum generation from verified release assets and
  document upgrade ownership for each installation method. The Linux generator
  now derives a shared Debian/RPM/AUR lock from an explicitly pinned sealed release;
  Homebrew and Windows generators retain their own verified inputs. Generated
  recipes remain local until their channel's lifecycle/publication checks pass.
- [x] Connect the configured crates.io Trusted Publisher to the manual Release
  workflow. The dedicated core job uses normal Cargo verification, exact source
  selection and short-lived OIDC credentials. A real read-only retry verified the
  published v0.3.1 checksum and all 124 source files; conflicting versions stop.
- [ ] Complete the first hosted OIDC Cargo publication on a subsequent selected
  release; local API-token upload and read-only retry checks do not establish it.
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
- [x] Complete local integrated validation and install the corrected update
  from `19637da`. All 485 macOS arm64 tests, both strict Clippy/format checks
  and release builds passed. Both packages were independently verified and both
  live supervisors applied the update; all six sessions present at installation
  and their selections were preserved. Existing frontends need their normal UI
  reload. The earlier `70083bd` installation receipt remains retained separately.
- [ ] Complete native Linux manager and Windows runtime ownership acceptance.
- [ ] Automate the release process: exact version/commit selection, native builds
  and tests, final-package checksums, publication and per-channel updates. The owner
  selected a **Release button with an explicit version and commit**; ordinary
  pushes must not publish. The Linux workflow includes the complete source
  archive and has 32 source/promotion tests. Version 0.3.1 is prepared locally;
  approved release immutability and the main-only release environment are enabled.
  Hosted run `34825445057` stopped before publication: one test assumed GitHub CLI
  was absent from the runner. The corrected candidate `19637da` passes all
  485 macOS and 482 native Linux tests in the serial baseline, both components'
  formatting/strict Clippy checks and both release builds. All 213 native Linux
  live tests also pass with four test threads. Final Linux artifact hashes, six
  stateless CLI calls and four ELF inspections passed; the binaries meet the
  glibc 2.39 baseline and left private state unchanged. The signed source is
  public on main. Hosted run `34835313350` passed all Rust checks but stopped
  before publication because an optional packaging test assumed Ubuntu's
  `rpmbuild` implied an RPM database with its build dependencies. The prerequisite
  check is corrected while retaining all RPM payload assertions. All 78 Python
  cases complete on macOS (74 passed, four platform/tool skips); actual Fedora
  queries confirm both provider-present and missing-provider behavior. The owner
  has authorized routine release/distribution decisions. Run `34837935289`
  published immutable v0.3.1 from `7f5c5eb`; its first attempt stopped on a
  transient post-publication tag lookup. The failed-job retry passed, preserving
  the same tested assets. All seven public downloads also matched independent
  checksum/source validation. Version 0.3.0 stays latest. The Linux release path
  has completed its hosted run; other targets and automatic channel activation
  remain open. Earlier failed receipts are retained; the hosted workflow uses
  the serial test baseline.
- [x] Resolve and revalidate UI tests under parallel load. The eight-row archive
  picker now retains a visible result row, and sidebar tests wait for the actual
  click acknowledgement. Both focused tests and the full serial/parallel suites
  pass at `19637da`: all 213 native Linux live tests passed with four threads,
  with no surviving owned children or harness errors. The prior `70083bd` run
  and its two failures remain retained; no assertions were removed or weakened.
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
