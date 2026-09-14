# Unpublished binary cask candidate — macOS blocked

Do not publish this directory as a working macOS installation channel. The tested
v0.3.0 downloads are ad-hoc signed and not notarized: ordinary Homebrew installation
succeeded, but macOS Gatekeeper blocked the executable on first launch. No quarantine
or security controls were bypassed. Developer ID signing, notarization, a new immutable
release, and fresh acceptance are required before publishing the macOS casks.

The primary Homebrew channel is the source-built tap in `../homebrew`. These casks
use the same tokens and must not be published beside those formulas.

`render.py` retains the reproducible, checksummed binary recipe preparation. Run
`python3 packaging/homebrew-prebuilt/render.py "$RELEASE_ASSETS" --check` from the
Flere source to verify all four original payloads. Run
`python3 scripts/test_homebrew_distribution.py` for offline recipe and libc guard tests.
Nothing in this directory installs, commits or publishes by itself.

Validation retained: Homebrew 6.0.19 parsed both OS variants, restricted architectures
correctly, and reported no cask style offenses. Actual GitHub downloads and macOS
installation/uninstallation succeeded in a disposable home-cache prefix. Native Linux
Homebrew install and macOS runtime acceptance did not pass this candidate's release gate.
