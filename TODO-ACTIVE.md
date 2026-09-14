# Flere — active work

## Distribution

- [x] Publish the verified [shell installer](scripts/install.sh) for `wget | sh`
  and `curl | sh` (also Bash). Source `83237c4` is public; its raw URL matches
  the reviewed bytes. Eight offline installer checks passed. Python 3.9+ is
  required; the bootstrap pins and verifies the published Python installer.
- [x] Publish the [Homebrew source tap](https://github.com/robert-cronin/homebrew-flere)
  using the pinned, checksummed v0.3.2 archive. Signed tap commit `97cbcff`
  advances both formulas after native Linux and isolated macOS validation;
  the core retains its Linux `zlib-ng-compat` runtime dependency.
- [x] Validate the isolated macOS arm64 source-formula lifecycle for core and
  companion: install, formula tests, stateless CLI checks, same-source revision
  upgrade and full removal. Disposable state/config stayed unchanged. This used
  cached Rust dependencies with `--ignore-dependencies`.
- [x] Validate the native Linux x86_64 Homebrew lifecycle with fresh dependencies:
  scoped tap trust, source integrity, installation, formula tests, stateless CLI
  checks, strict linkage, same-source revision upgrade and uninstall. Disposable
  state stayed unchanged; the isolated container was removed.
- [x] Validate the native Linux Homebrew upgrade from v0.3.0 to v0.3.2 for
  both programs, with fresh dependencies, formula/CLI/license/strict-linkage
  checks before and after, exact selected kegs and complete removal. Synthetic
  state stayed unchanged and the isolated container was removed.
- [x] Validate the isolated macOS arm64 Homebrew v0.3.0 → v0.3.2 upgrade.
  Both formula phases, 14 stateless CLI checks, licenses, selected kegs and full
  removal passed; synthetic state stayed unchanged. This used install-driven
  upgrade in a cached private prefix with `--ignore-dependencies`, not literal
  `brew upgrade` or the default prefix.
- [ ] Validate fresh macOS Homebrew dependency provisioning and older macOS
  acceptance. The completed arm64 lifecycle used `--ignore-dependencies`;
  Intel Macs and Linux ARM remain excluded from this tap.
  The manual Homebrew macOS acceptance workflow now covers hosted macOS 15/26
  default-prefix installs, direct Rust dependency provisioning and literal
  version upgrades. Its actual hosted results remain pending.
- [x] Prepare and validate the core Cargo source package at `19637da`: normal
  extracted-package verification, all 186 library tests, a release build,
  stateless CLI checks and an unchanged cached-build stamp passed on macOS arm64.
  The archive contains 124 reviewed files and matches the exact clean source.
- [x] Publish the verified core Cargo package as v0.3.1 from matching source
  `7f5c5eb`. Normal Cargo publication succeeded; the registry checksum and
  anonymous archive both match the reviewed 124-file package. After the hosted
  release below, the current core command is
  `cargo install flere --locked --version 0.3.3`. The exact GitHub Trusted
  Publisher is configured for subsequent releases.
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
- [x] Complete native Ubuntu Debian v0.3.0 → v0.3.2 installation, upgrade,
  removal and purge. All 12 installed CLI checks, package-manager ownership
  detection and blocked coordinated-update staging passed. Synthetic state and
  unrelated packages stayed unchanged; owned children and the container exited.
  The tested archive was copied back and passed independent release inspection.
- [x] Validate normal APT resolution/download of a missing Git dependency for
  the exact tested Debian 0.3.2 package. A fresh native Ubuntu container fetched
  Git from signed Ubuntu repositories, installed and removed Flere, and preserved
  unrelated packages, synthetic state and repository/keyring files. No Flere
  runtime ran; other dependencies were preinstalled, so this is not an
  all-dependency pristine-machine check.
- [x] Publish the verified `.deb` through the eight-asset v0.3.3 release from
  `ce6bb62`. Hosted run `34853838902` passed every job on its first attempt,
  including Cargo OIDC and the corrected upload-archive check. Independent
  downloads verified all eight assets, 333 source files/modes and all 124 Cargo
  source files. Existing immutable releases and the v0.3.0 latest pointer stayed unchanged.
- [ ] Configure the AUR maintainer identity and publishing access.
- [ ] Check package-name availability and submit the verified AUR recipe.
  Successful Arch package lifecycle checks do not establish publication or
  account setup.
- [x] Finish output inventory for the [Nix packaging draft](packaging/nix/README.md).
  All 15 native x86_64 Linux Docker phases passed: parsing, evaluation, both builds,
  declared install checks and exact output files/modes/ownership/hashes. The core's
  zlib input and the validator's missing-utility assumption are corrected.
- [ ] Complete Nix sandbox test-suite, interactive NixOS runtime and update
  ownership acceptance before promoting the packaging draft to supported status.
  A forced-sandbox probe could not start under the current container security
  settings because required kernel namespaces were unavailable; use a suitable
  isolated Nix/NixOS host or VM. No security settings were weakened.
- [ ] Finish current Windows physical clipboard/SSH/draft/image acceptance, then
  publish the companion through Scoop and submit a WinGet manifest.
  The unpublished v0.3.4 source is reserved for the next Windows acceptance
  payload. Do not publish it with the Linux-only workflow before all intended
  Windows assets are ready: v0.3.3 is immutable and cannot receive extra files.
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
- [x] Complete the first hosted OIDC Cargo publication. Run `34843553467`
  published v0.3.2 from `1be2dab`; the failed-job retry reconciled the existing
  upload and all jobs passed. The first upload's archive-path verification error
  is corrected; an independent offline Cargo package exactly matches the registry
  checksum and all 124 source files. Future releases use no owner API token.
- [x] Make the local in-app updater identify package-manager installations and
  display the appropriate upgrade command. Ambiguous ownership and removed
  executables keep Apply disabled; a Cargo receipt advancing beyond the running
  version retains manager guidance. Unit and combined core checks pass.
- [x] Correct Homebrew companion ownership: the executable and formula must
  match, and the companion retains its own `brew upgrade` command. The regression
  first reproduced Unknown ownership; all 486 macOS tests, strict Clippy/format
  checks and Linux/Windows compile checks now pass. This correction is unreleased;
  published Homebrew v0.3.2 still contains the earlier parser.
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
- [ ] Complete remaining native Linux package-manager and Windows runtime
  ownership acceptance. The native Debian ownership protocol checks above passed.
- [x] Automate the Linux manual Release path: explicit main version/commit
  selection, native core/companion checks, final payload/source checksums,
  immutable GitHub publication and anonymous download verification. Hosted
  v0.3.1 and v0.3.2 releases passed; v0.3.3 passed all jobs on its first attempt,
  including the Debian package and core Cargo OIDC publication. Earlier retries
  reconciled the original assets/uploads without rebuilding or publishing twice.
  Ordinary pushes do not publish. Earlier failed receipts remain retained.
- [ ] Extend release automation to the remaining targets and package channels
  after their own acceptance checks. The Linux manual Release path is complete;
  v0.3.0 remains latest, and macOS/Windows channels were not advanced by v0.3.3.
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
