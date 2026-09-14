# Flere — active work

## Distribution

- [ ] Publish and validate a Homebrew tap using versioned, checksummed releases.
  Source formulas and the isolated Mac build/CLI are verified; the curated tap
  is prepared. Publication and final package lifecycle acceptance remain open.
- [ ] Prepare and validate Cargo source packages; publish after registry access
  and package-name availability are established.
  Core archive verification passed at `677971d`; first registry publication must
  use a new matching version/tag rather than relabel the original release source.
- [ ] Prepare Linux native packages and an AUR recipe; validate each supported
  platform and state the prebuilt Linux glibc requirement.
- [ ] Add Nix packaging and validate it with Nix before claiming support.
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
  pushes must not publish. The first Linux workflow is implemented and has
  16 promotion/validation tests; a hosted run, other targets and channel
  activation remain open.
- [ ] Add Developer ID signing and notarization for prebuilt macOS packages after
  Apple enrollment and credential setup. Current binary casks are not published:
  ordinary Gatekeeper blocked their unnotarized executable in testing.

## Add project

- [x] Replace exact-path recall with a terminal directory browser and path
  completion in Add project. Remove the editable Name field; discover the project
  name from Git remote metadata, with a local directory-name fallback for offline
  folders or projects without a remote. Real PTY tests cover browsing, completion,
  cancellation, automatic naming and literal paths; inherited Git selectors cannot
  redirect the selected checkout. Installation/publication of this change is pending.

## Branding and link audit

- [ ] Clean up obsolete local Railhand files and installation references after
  the move to Flere. Identify live dependencies before relocating state; preserve
  private history separately from the clean public repository.

- [ ] Replace remaining rendered Railhand branding with Flere in current terminal
  captures, illustrations, intro/screensaver frames and documentation art. Inspect
  actual images as well as text; keep historical evidence accurately labeled.
- [ ] Use parallel subagents to audit source, docs, generated assets and public
  GitHub links for stale references, missing targets and dead links; fix current
  product references without rewriting truthful historical records.

An unchecked channel is not a claim that its installation command is available.
Published downloads and platform limits are documented in the installation guide.
