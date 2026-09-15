# Flere — active work

## Distribution

- [x] Publish the verified [shell installer](scripts/install.sh) for `wget | sh`
  and `curl | sh` (also Bash). Source `83237c4` is public; its raw URL matches
  the reviewed bytes. Eight offline installer checks passed. Python 3.9+ is
  required; the bootstrap pins and verifies the published Python installer.
- [x] Publish the [Homebrew source tap](https://github.com/robert-cronin/homebrew-flere)
  using the pinned, checksummed v0.3.5 release archive. Signed tap commit `3e844f5`
  advances both formulas using the verified prepared recipe bytes;
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
- [x] Validate default-prefix Homebrew source installation and version upgrade
  on native hosted macOS 15.7.9 and 26.6.2 arm64. Run `34862741037` passed both
  jobs at `520c50c`: freshly provisioned direct Homebrew Rust, four formula tests,
  fourteen CLI checks, strict linkage, literal v0.3.0 → v0.3.2 upgrades and removal.
  Independent review verified all 94 command results and log hashes per job;
  synthetic state stayed unchanged. Preinstalled transitive dependencies and
  verified tap checkout are retained limits; physical UI/clipboard/SSH,
  notarization, Intel Macs and Linux ARM remain separate or excluded.
- [x] Prepare and validate the core Cargo source package at `19637da`: normal
  extracted-package verification, all 186 library tests, a release build,
  stateless CLI checks and an unchanged cached-build stamp passed on macOS arm64.
  The archive contains 124 reviewed files and matches the exact clean source.
- [x] Publish the verified core Cargo package as v0.3.1 from matching source
  `7f5c5eb`. Normal Cargo publication succeeded; the registry checksum and
  anonymous archive both match the reviewed 124-file package. After the hosted
  release below, the current core command is
  `cargo install flere --locked --version 0.3.5`. The exact GitHub Trusted
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
  Arch hardware and interactive terminal features were not tested. The original
  package's licence metadata replaced MIT with a filename; correct installed
  licence files did not establish correct package metadata.
- [x] Preserve the AUR licence array while installing licence files. The Bash
  regression reproduced the overwritten MIT entry and passes after renaming the
  loop variable. Packaging checks: 12 passed, four platform skips. Source/payload
  pins and `.SRCINFO` stayed unchanged. Native corrected-package validation
  passed for both the v0.3.3 wrapper and a private v0.3.4 fixture.
- [x] Validate native Arch package installation, a real v0.3.0 → v0.3.3
  pacman upgrade and removal, followed by a separate private v0.3.4 package from
  exact source `3401a77`. All 18 stateless checks, exact payloads and corrected
  licence metadata passed. Both development update UIs recognized pacman and
  refused preparation before staging. Synthetic state, unrelated package records
  and repository/keyring configuration stayed unchanged; owned application
  processes exited normally and the exact container was removed. The initial
  helper failure on inherited signature policy is retained; no package-security
  setting was relaxed. These packages are unsigned and not submitted to AUR.
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
- [x] Publish immutable v0.3.4 with the explicit Linux/Windows profile from
  `32108e3`. Run `34930826557` passed native producers, publication, anonymous
  verification, Cargo OIDC publication and hosted recipe generation. Independent
  review matched all 11 public assets, all 383 source files/modes and all 129
  Cargo source/metadata files. The separate ten-file recipe artifact is verified
  `prepared_not_published`. The public Windows MSVC ZIP includes both companion
  aliases; no Windows core is included.
  Latest remains v0.3.0; the separate Homebrew tap update publishes v0.3.4.
  Mac signing/notarization and package-manager catalogue publication remain separate.
- [x] Publish immutable v0.3.5 with the explicit Linux/Windows profile from
  `8744d35`. Run `34941117734` passed native Linux/Windows checks, publication,
  anonymous verification, core Cargo OIDC publication and recipe generation.
  All 11 downloads, 388 source files/modes and 132 Cargo archive files were
  independently verified. Windows passed 81 tests and six alias checks, with
  one physical clipboard ignore. The separately published Homebrew tap at
  signed `3e844f5` matches both hosted source formulas exactly. Latest/default
  installer remains v0.3.0; Apple signing and Windows catalogues remain separate.
- [ ] Configure the AUR maintainer identity and publishing access.
- [x] Check the intended AUR package name: official AUR exact-info/name-search
  and Arch package APIs returned no matching `flere-bin` package on 2026-09-14
  at 19:00 UTC. This does not reserve the name; recheck before submission.
- [ ] Submit the verified AUR recipe after maintainer/publisher access is ready.
  Successful Arch package lifecycle checks do not establish publication or
  account setup.
- [x] Finish output inventory for the [Nix packaging draft](packaging/nix/README.md).
  All 15 native x86_64 Linux Docker phases passed: parsing, evaluation, both builds,
  declared install checks and exact output files/modes/ownership/hashes. The core's
  zlib input and the validator's missing-utility assumption are corrected.
- [x] Complete native strict-sandbox Nix builds and full test suites for the
  pinned v0.3.3 recipe plus the explicit wrapped-image parser patch. Hosted run
  `34869221522` at `2d52985` passed all 485 tests, both release builds/build-info
  checks and exact executable/license inventories. Independent review verified
  the logs, archive digest, declared inputs and real sandbox namespace probe.
  All 16 previously failing live tests now pass; earlier failures remain retained.
  Fixture corrections preserve assertions, deadlines and isolation settings.
- [x] Validate native Nix-on-Ubuntu core and companion installation ownership
  and normal profile install/remove. Hosted run `34874904110` at workflow
  `8fb9d7d` built exact v0.3.4 development source `2d52985` in the strict sandbox.
  Both actual update UIs showed verified Nix guidance and refused preparation
  before staging; six stateless flags, exact empty runtime identities, clean
  detach/exit and profile removal preserving synthetic state passed.
- [x] Validate the core in an explicitly emulated NixOS guest. Run `34883876197`
  at workflow `c8107ad` tested product `2d52985`: shell, Vim edit/save, Git
  inspector and detach/reattach with exact identities and a retained draft passed.
  TCG with KVM disabled was confirmed; frontends, sessions, supervisor and QEMU
  exited normally. The earlier deprecated-driver-call failure is retained.
- [x] Validate actual Nix v0.3.3 → v0.3.4 private-profile upgrade. Run
  `34883927251` at `c8107ad` built both component/version pairs and verified the
  literal `nix-env --upgrade --lt` changed the generation and exact outputs.
  All 12 stateless checks, both owner/refusal UIs and normal cleanup passed;
  synthetic state stayed unchanged. The previous pair used the declared parser
  patch; fresh outputs are separate from the earlier 485-test binaries.
- [x] Validate real OpenSSH from the installed macOS companion to a Nix-built
  Linux core at exact source `2d52985`. Two normal attachments preserved the
  exact shell/supervisor and unsubmitted draft; one Enter after reconnect
  produced exactly one appended result. Independent review verified normal
  session/supervisor shutdown, exact container removal and unchanged companion
  bytes. This separate native check used Nix 2.35.2's container default settings;
  it does not extend the earlier strict-sandbox suite or test a Nix-packaged
  companion. Earlier fixture failures remain retained.
- [ ] Complete physical desktop clipboard acceptance before promoting the Nix
  draft to supported status. TCG is emulated core
  runtime evidence; native Nix-on-Ubuntu profile upgrades do not establish
  NixOS/Home Manager activation. The pinned v0.3.3 recipe still lacks the newer
  ownership code and was not repinned.
- [ ] Finish current Windows physical clipboard/SSH/draft/image acceptance, then
  publish the companion through Scoop and submit a WinGet manifest.
  The public v0.3.5 ZIP is available from exact source `8744d35`; its native
  producer passed 81 MSVC tests and six alias checks with one physical clipboard
  ignore. Manager lifecycle results below use the earlier candidate `7e43fa1`;
  runtime code matches, but public release bytes have their own verified hashes.
  The physical handoff is updated to the actual public ZIP. Catalogue submissions
  remain pending.
- [x] Revalidate the public source on native Windows after incorporating the
  CRLF bootstrap fix. Run `34911230794` tested exact public source `422058c`:
  native MSVC formatting, strict Clippy, 59 tests and release build passed, with
  one explicit physical clipboard ignore. The module-independent Windows hash
  regression, packaging and all six portable alias checks passed. The overall
  job then failed on WinGet manifest warnings; recipe validation and physical
  acceptance remain separate. Earlier private-source and failed-run evidence
  are retained.
- [x] Build and verify the Windows candidate from exact public source
  `21cf68c`. Run `34922477345` passed all 65 native MSVC tests, formatting,
  strict Clippy, release packaging and six portable alias checks. One physical
  screenshot/clipboard test was explicitly ignored. Real WinGet manifest
  validation and Chocolatey packing also passed. Independent artifact inspection
  matched all 376 source files and modes, payloads and log hashes. This candidate
  remains unpublished; installed-manager and physical acceptance are separate.
- Deferred by the owner: the Windows legacy-input arcade hold pause is not a
  release requirement. Preserve the console-input investigation for a future
  arcade pass; do not alter system repeat settings or add global monitoring.
- [x] Prepare the upstream RPM wrapper from the verified v0.3.0 release. Real
  rpmbuild, payload/license/dependency checks and offline RPM install/remove
  passed in emulated amd64 Fedora 44; six installed stateless CLI checks and
  synthetic-state preservation passed. The historical comparison of 220 unrelated
  package headers is unproven: its unsupported `HDRID` query printed diagnostics
  with exit status 0. The original receipt remains invalid for that assertion;
  corrected native proof is recorded below.
- [x] Validate native Fedora 44 container-on-Linux v0.3.0 → v0.3.3 RPM install,
  literal DNF upgrade and removal. All 63 lifecycle checks passed, including 12
  stateless CLI checks. Exact core/companion payloads, licenses and real owner UIs passed;
  both guards refused replacement before staging and detached normally. All 223
  unrelated real SHA256-header records, synthetic state and repository/keyring
  configuration were preserved. Exact container cleanup was independently checked.
  The tested unsigned v0.3.3 RPM has SHA-256
  `85993e02bd4ecca190bcb91b954313a44bb84b723764bec35643373b1fcbd5f8`;
  the checked-in recipe remains v0.3.0. No physical desktop or external SSH claim.
- [ ] Set up RPM signing, then sign/publish the reviewed wrapper with its checksum
  and provenance. The tested v0.3.3 RPM remains unsigned and unpublished. Fedora
  desktop/physical terminal and other RPM-distribution acceptance remain open;
  this is an upstream wrapper, not a Fedora repository submission.
- [x] Prepare Chocolatey companion recipes from the same verified Windows ZIP,
  version and checksum as Scoop/WinGet. Nine offline generator tests pass, covering
  package inventory, provenance, escaping and AMD64 selection. No Windows core is
  included and no package is published.
- [x] Validate the retained Windows ZIP recipes natively. Run `34912671194`
  passed warning-free WinGet 1.11.510 validation and Chocolatey 2.7.4 packing;
  the exact five-member `.nupkg`, logs and source provenance were independently
  inspected. It reused the byte-identical ZIP from run `34911230794`, without
  rebuilding or executing the companion. Both earlier failures remain retained.
- [ ] Complete Windows package-manager install/upgrade/remove and ownership
  checks before publication. Chocolatey runs `34918170067` and `34919246013`
  proved normal installation, six shim calls, removal, unchanged synthetic state,
  PATH, unrelated package inventory and feature restoration. Historical records
  remained as required by the observed retention setting. Both overall runs
  remain failed: the loopback monitor rejected incidental malformed HTTP traffic,
  whose source is unproven. The corrected monitor verifies completed exact-ZIP
  responses after shutdown and records rejected traffic separately; 14 pure and
  five owned-loopback checks pass. Fresh run `34921228036` passed all 23 native
  commands and final download/removal/preservation checks on the retained exact
  `422058c` ZIP. Its three completed downloads matched the reviewed hash; two
  rejected requests remain recorded observations. Earlier failed receipts remain
  unchanged. The current-candidate Chocolatey workflow now checks production
  ownership through the normally installed shim, and a separate WinGet workflow
  exercises normal install/remove with exact package records and state preservation.
  Corrected WinGet run `34929525551` passed normal local-manifest installation,
  all six alias calls, exact installed records, normal removal and state/PATH
  preservation: 25 native commands passed. The original failed lookup/removal
  run is retained. Current-candidate Chocolatey runs `34927861846` and
  `34929532961` proved actual installed ownership and normal removal but failed
  cold-profile equality. Retained diagnostics show only PowerShell startup data
  and an empty Chocolatey temporary directory were added; existing state and
  SSH files stayed unchanged. A reviewed fixture change initializes those normal
  tools before measuring preservation. Corrected run `34930361884` passed
  all 28 native commands, actual installed ownership, six aliases, exact later
  state/PATH preservation and normal removal; 25 helper checks also passed.
  Cold-profile no-write behavior is not claimed. Corrected WinGet installed-owner
  run `34936386953` passed both aliases and all 28 commands at `9d53f96`,
  with normal removal and initialized-profile/PATH preservation. Scoop run
  `34935706908` passed the public ZIP install, six alias calls, metadata rename
  and normal Flere removal/preservation. Its overall result remains failed
  because extra Scoop self-removal timed out. The fixture now retains Scoop for
  disposable VM teardown, without relaxing Flere removal checks; seven helper
  checks pass. New native Scoop run `34939641214` passed actual public 0.3.4 →
  candidate 0.3.5 upgrade, 12 alias checks, both production owner reports and
  normal removal of both retained versions. All 40 commands and preservation
  checks passed; Scoop itself remains until disposable VM teardown. Chocolatey
  run `34939646345` passed the same actual version upgrade, 12 alias checks,
  both owner reports and normal removal: all 41 commands passed, with all 45
  preexisting package records and initialized profile/PATH preserved. These
  checks used source `7e43fa1`, local recipes and a checksummed runner-local
  target download; catalogue and physical desktop acceptance remain separate.
  WinGet run `34943275275` also passed normal 0.3.4 → 0.3.5 local-manifest
  upgrade, all 41 commands, 12 alias checks, both installed owner reports and
  normal removal/preservation of all 456 unrelated registry records. The private
  installer projected the verified baseline ProductCode and runner-local URL;
  the provisioned community-only source set remained until VM teardown.
  These three lifecycle routes are verified; ordinary catalogue/bucket ownership
  support and physical Windows acceptance remain open.
- [x] Automate manifest/checksum generation from verified release assets and
  document upgrade ownership for each installation method. The Linux generator
  now derives a shared Debian/RPM/AUR lock from an explicitly pinned sealed release;
  Homebrew and Windows generators retain their own verified inputs. A separate
  Release job now prepares an allowlisted Linux/Homebrew recipe artifact after
  public verification, with exact source/descriptor pins and a
  `prepared_not_published` receipt. Seven offline checks and generation from the
  verified v0.3.3 assets passed; hosted v0.3.4 generation passed in run
  `34930826557`. It does not publish recipes or advance checked-in channel pins.
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
  checks and Linux/Windows compile checks now pass. The correction is included
  in public v0.3.4 and the current Homebrew source formulas; the historical
  v0.3.2 tap contained the earlier parser.
- [x] Implement verified local Nix ownership for core and companion, with
  commandless owning-configuration guidance and in-app Apply disabled. Exact
  registered-content queries passed on native Nix 2.35.2 as root and an ordinary
  user, with corrupt/unregistered outputs rejected and owned containers removed.
  The default root-controlled daemon/store is the verified scope. Actual core
  and companion ownership/UI/profile checks also passed on native Nix 2.33.3 in
  run `34874904110`, separately from the v0.3.3 full-suite recipe. This source
  change is included in public v0.3.4; the Nix draft remains separately pinned.
- [x] Correct wrapped image-path selection when the closing delimiter occupies
  its own row. The regression reproduced the defect, and exact caption checks
  pass at 32/33/64/65 columns while unrelated clicks/text stay rejected. This
  source change is included in public v0.3.4; the older Nix recipe retains it
  as an explicit patch.
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
- [x] Install the verified development update from `2d52985` after all 493
  macOS tests, strict Clippy/format checks, Linux/Windows compile checks and both
  release builds passed. Both managed commands match the reviewed payloads;
  both known supervisors applied the update with all five exact session identities
  preserved. Existing frontends remain attached and need their normal UI reload.
  The unidentified legacy runtime was excluded. This installation predates the
  separately published v0.3.4 release at `32108e3`.
- [x] Validate actual native Linux Cargo v0.3.3 installed ownership: normal
  registry installation, exact empty supervisor/frontend owner reports,
  coordinated preparation rejected before staging, and normal Cargo uninstall.
  Installed executable/tracking files and synthetic state stayed unchanged during
  rejection; both owned children and the validation container were removed.
  Cached locked dependencies were preseeded. No active-session or modal-painting
  acceptance is claimed; one non-authoritative receipt summary field is annotated
  invalid while the independent ownership predicates and original receipts remain
  retained.
- [x] Validate native Linux Homebrew core v0.3.2 ownership with a normal source
  install and real receipt. Exact empty supervisor/frontend reports, blocked
  coordinated preparation, unchanged binary/receipt/state, normal uninstall and
  exact child/container cleanup passed. The published tap/source remained pinned;
  companion, rendered-modal and active-session behavior were not part of this check.
- [x] Validate native Linux Homebrew core and companion ownership at exact
  then-unreleased v0.3.4 source `2d52985`. Both normal source installs, formula tests,
  six stateless checks, actual owner UIs and preparation refusal before staging
  passed. Binaries, manager receipts and synthetic state stayed unchanged; both
  frontends, the local bridge and supervisor exited normally, both kegs uninstalled
  and the exact container was removed. Formulas used the verified source via a
  private local URL; the published tap was then v0.3.2. No active-session or
  external SSH acceptance is claimed.
- [x] Integrate the reviewed session-reliability branch: UI watches tolerate
  temporary output stalls without corrupting partial frames, SSH failures report
  their exit status, and automatic approval-review activity renders correctly.
  The final four-thread core run passed all 424 tests; 89 companion tests, strict
  Clippy/format checks and Windows companion compilation also passed. The earlier
  long fixture-path failure, one transient probe timeout and toolchain stripping
  warning remain retained; corrected release relinks passed. Exact source
  `21cf68c` subsequently passed all 514 native macOS tests and both components'
  formatting, strict Clippy and release checks. After correcting a local toolchain
  symbol-stripping invocation, both verified packages were installed and applied
  to the two known supervisors: all five exact session identities and selections
  survived. The two attached frontends still need their own UI reload.
  Subsequent branch tip `f30ab18` was merged in `fc7a864` and pushed. Its exact
  ancestry and lack of an open branch PR were checked before deleting the fully
  merged branch with an exact-tip lease; only `main` remains. Recheck for new
  branch work before future releases.
- [ ] Implement and validate Windows package-manager ownership recognition.
  Normal Scoop bucket and WinGet catalogue installation records still need
  support and validation before those channels are published. Current recognition
  covers the verified local-manifest routes and default Chocolatey packages;
  unsupported records remain Unknown with Apply disabled.
  Chocolatey detection now binds the running payload and manifest to the ordinary
  default installation and an exact active package query, with fixed upgrade
  guidance and Apply disabled. The companion's existing `update-status` JSON now
  includes the production ownership report while preserving its existing fields;
  the added regression checks all owner kinds without changing fixture state.
  The native records informed the parser; Windows
  all-target compilation passed. Actual installed-product recognition passed
  through the normal shim in both retained Windows runs above; coordinated
  preparation refusal remains unverified. Custom Chocolatey roots remain Unknown;
  The initial WinGet detector checked the observed user-scope local-manifest installation
  through a fixed read-only HKCU/64 query, the actual Windows profile location,
  and exact manifest/build/payload identity. It invokes no WinGet catalogue query
  and provides commandless manifest guidance with Apply disabled. Two functional
  record regressions, all 306 core/companion Mac unit tests, formatting, strict
  Clippy and Windows cross-compilation pass. Native MSVC candidate run
  `34931604932` at `9a9183c` passed 76 tests, both aliases and native recipe
  checks, with one physical ignore. Installed check `34933659610` at workflow `d7e6619` failed:
  normal installation and six alias checks passed, but the first owner query
  returned Unknown. The second owner query was not reached. Normal removal,
  state/PATH/settings preservation and loopback cleanup passed. Diagnostic
  run `34934498384` reproduced the failure and showed GetFolderPath resolving
  the synthetic profile instead of the active package root; cleanup passed again.
  The correction reads the current Windows token SID and its unexpanded HKLM
  profile path, supporting the literal default AppData/Local location. All 308
  core/companion Mac unit tests, both format/strict Clippy checks and Windows GNU
  cross-compilation pass. Corrected native candidate `34935547435` at
  `9d53f96` passed 77 tests, six aliases and native recipe checks, with one
  physical ignore. Installed run `34936386953` passed both actual owner reports
  under synthetic profile overrides, all 28 commands, exact payload/registry
  preservation and normal removal; both failed receipts remain retained.
  Catalogue/custom-root installations stay Unknown; UI/coordinated refusal
  remains unverified. Scoop run `34935706908` proved native metadata preservation,
  both aliases and normal Flere removal, while extra manager self-removal failed
  as recorded above. Scoop ownership detection now checks default-user local
  manifests, the active junction, both alias shims and exact release/build/payload
  identity without executing package metadata. Missing, stale or unsupported
  records remain Unknown with Apply disabled. The shared profile extraction
  preserves the corrected WinGet resolver. Thirty-two focused manager tests,
  formatting and strict companion/Windows Clippy pass. Native Scoop recognition
  and actual Scoop/Chocolatey 0.3.4 → 0.3.5 upgrades passed as recorded above;
  WinGet local-manifest upgrade passed below; catalogue/bucket support and
  actual Windows UI/coordinated refusal remain separate. Integrated
  source `7e43fa1` passed all 541 native macOS tests and both release builds;
  strict checks and formatting pass. Normal Scoop/WinGet upgrade workflows
  are prepared with exact version/build/alias and preservation checks; all 64
  combined helper checks pass. Native candidate `34938451094` at `7e43fa1`
  passed 81 tests, six aliases and package recipe checks; one physical clipboard
  test remains ignored. First upgrade run `34939643780` installed and checked
  public 0.3.4, then stopped when WinGet opened Microsoft Store terms during
  upgrade. No terms were accepted and 0.3.5 was not installed; normal removal
  and state/settings/source preservation passed. The next attempt explicitly
  selected the existing `winget` source, but run `34940075188` then showed that
  WinGet 1.11 rejects `--source` together with `--manifest`. It stopped before
  the target download; normal removal and preservation passed. Both failures
  remain retained. Upgrade-only hosted VM provisioning now normally removes
  the exact default Store source before recording its protected source baseline;
  the community-only configuration remains until VM teardown. No Store terms
  are accepted and stock-source restoration is not claimed. All 25 helper checks
  and replay of the observed source inventory pass. Run `34941913400` verified
  that provisioning and normal cleanup, but WinGet could not correlate its
  local portable record with the target manifest; no target download occurred.
  The private upgrade manifest now supplies the exact baseline ProductCode,
  in addition to its runner-local URL. Public catalogue recipes and payloads
  remain unchanged; 26 helper checks and actual CRLF manifest/record replay pass.
  Run `34943275275` independently verified that projection, normal upgrade,
  both actual owner reports, unchanged state/settings and normal removal; all
  41 commands passed. This proves the local-manifest route, not a public
  catalogue upgrade or physical Windows behavior. Native
  Linux checks passed for Debian, Cargo core, Homebrew core/companion, RPM,
  pacman and Nix at their separately recorded versions and scopes above. Physical
  desktop and external companion SSH acceptance remain separate.
- [x] Automate the Linux manual Release path: explicit main version/commit
  selection, native core/companion checks, final payload/source checksums,
  immutable GitHub publication and anonymous download verification. Hosted
  v0.3.1 and v0.3.2 releases passed; v0.3.3 passed all jobs on its first attempt,
  including the Debian package and core Cargo OIDC publication. Earlier retries
  reconciled the original assets/uploads without rebuilding or publishing twice.
  Ordinary pushes do not publish. Earlier failed receipts remain retained.
- [ ] Extend release automation to the remaining targets and package channels
  after their own acceptance checks. The Windows candidate producer now accepts
  an explicit commit/version pair while preserving its historical defaults,
  including the manual candidate workflow. Linux/Windows artifact assembly now
  validates one complete eleven-file release from matching source/version/run
  evidence; 78 offline release/packaging checks pass and historical schema 1/2
  behavior is retained. Complete-release assembly additionally requires all
  sixteen Linux/Windows/macOS assets and pinned accepted macOS signing/notary
  evidence; it checks every companion against both core protocols and preserves
  historical formats. All 101 offline release tests pass. The manual workflow
  now connects explicit `linux-windows` and `complete` profiles to the maintained
  producers, aggregate and publisher. Twelve focused orchestration tests,
  workflow linting and independent review pass. The complete profile requires
  Apple configuration before builds and retains an exact submission checkpoint
  for resume-only notarization. The first hosted Linux/Windows profile passed
  as the v0.3.4 release above; complete-profile native signing and external
  catalogue updates remain unfinished.
  The Linux manual Release path is complete; v0.3.0 remains latest, and
  macOS/Windows channels were not advanced by v0.3.3.
- [x] Resolve and revalidate UI tests under parallel load. The eight-row archive
  picker now retains a visible result row, and sidebar tests wait for the actual
  click acknowledgement. Both focused tests and the full serial/parallel suites
  pass at `19637da`: all 213 native Linux live tests passed with four threads,
  with no surviving owned children or harness errors. The prior `70083bd` run
  and its two failures remain retained; no assertions were removed or weakened.
- [x] Resolve the Linux executable-busy error in installer package inspection.
  Metadata probes now retry only a pre-spawn executable-busy error within their
  existing deadline; other errors and already-started commands are never replayed.
  Exact `aa3b582` passed all 201 native Linux library tests, all 13 update tests
  with four threads, formatting, strict Clippy and the core release build. Both
  real held-writer regressions and the originally failing update test passed.
  Source and locks stayed unchanged and no owned processes remained. The earlier
  `22555bb` run's 215 live UI passes remain valid at that source; its failed
  update receipt is retained, and the historical writer identity remains unknown.
  The complete native macOS package at `aa3b582` passed all 527 tests and both
  components' formatting, strict Clippy and release builds. Both managed commands
  and the two known supervisors now run that verified local package; all five
  exact session identities, tab selections and frontend attachments survived.
  Existing frontends still need their normal UI reload.
- [x] Package and install the local macOS v0.3.5 development update from
  `6e87f05`. Both ordinary incremental release builds and complete 388-file
  source seals passed; prior 541-test evidence covers the unchanged runtime.
  Normal installation and both supervisor updates completed with all five exact
  sessions, selections and frontend attachments preserved. Installed core and
  companion hashes match the verified packages; frontends still require their
  normal UI reload. This is separate from public macOS signing/notarization.
- [ ] Add Developer ID signing and notarization for prebuilt macOS packages after
  Apple enrollment and credential setup. The staged producer now separates build,
  signing, resumable notarization and final signed-byte verification; 17 pure
  boundary tests pass. Entitlement output is bounded, and signing cleanup checks
  the restored keychain search list and removal. The complete Release profile
  now wires these phases with separate credential scopes and exact artifact pins;
  actual Apple execution and hosted checkpoint retries remain unverified.
  Current binary casks are not
  published: ordinary Gatekeeper blocked their unnotarized executable in testing.

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
