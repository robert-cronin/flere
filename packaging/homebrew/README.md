# Flere Homebrew tap

Source-built formulas for [Flere](https://github.com/robert-cronin/flere).

## Install

With Homebrew 7, trust the two formula names before the first install. This
allows Homebrew to load their recipes; it does not install the optional companion.

```sh
brew trust --formula robert-cronin/flere/flere robert-cronin/flere/flere-connect
brew install robert-cronin/flere/flere
flere --version
```

For the optional local SSH and clipboard companion:

```sh
brew install robert-cronin/flere/flere-connect
flere-connect --version
```

Homebrew downloads the pinned v0.3.4 full source archive and its Rust build
dependencies, then compiles each executable locally. The first install takes
longer than downloading a prebuilt binary and needs Rust 1.98 or newer, which
Homebrew supplies as a build dependency. Dependencies follow the checked-in lock
files; installation compiles in Cargo's offline mode after fetching them. On Linux,
the core also declares `zlib-ng-compat` as a runtime dependency; the companion does
not need it.

This tap supports macOS Apple Silicon and Linux x86_64. Source-formula lifecycle
checks passed on macOS 15.7.9 and 26.6.2; other macOS versions and physical terminal
behavior remain unverified by those checks. Intel Macs and Linux ARM are excluded
pending runtime acceptance. Homebrew and its Rust dependency also impose their own
supported OS requirements. See [platform limits](https://github.com/robert-cronin/flere/blob/main/MACOS.md)
and [release validation](https://github.com/robert-cronin/flere/blob/main/docs/releases/0.3.0-validation.md).
The source build uses the host runtime; the separate Linux GNU prebuilt package's
glibc 2.39 minimum does not apply to a native source build.

Ensure Homebrew's `bin` directory is on PATH. The core finds a separately installed
companion through PATH for `flere ssh`. Use `command -v flere` and
`command -v flere-connect` to identify the selected copies if you also have a manual
installation. These formulas do not adopt or delete another installation.

## Update and remove

```sh
brew update
brew upgrade robert-cronin/flere/flere
brew upgrade robert-cronin/flere/flere-connect  # if installed
```

Use Homebrew for these packages. A separate manual `~/.local/bin` installation can
take precedence on PATH. Upgrades and uninstall preserve Flere's saved state and
do not stop running sessions. An existing process uses its current executable
until you explicitly refresh or restart it.

```sh
brew uninstall robert-cronin/flere/flere-connect  # if installed
brew uninstall robert-cronin/flere/flere
```

## Why source-built formulas

The v0.3.0 macOS prebuilt executables are not Developer ID signed or notarized.
Homebrew could install them, but Gatekeeper blocked the executable at first launch.
They are not distributed by this tap. These formulas compile the audited source
locally without altering macOS security policy. Normal quarantine remains enabled.
The source archive SHA-256 detects corruption; it is not an independent publisher
signature. A future macOS binary channel needs signing, notarization and fresh
runtime acceptance.

## Maintain and validate

The current formulas use the immutable [v0.3.4 source release](https://github.com/robert-cronin/flere/releases/tag/v0.3.4).
Both recipes were verified against the released archive and maintained generator;
only their source URLs, versions and SHA-256 values changed from v0.3.2. Install
commands, dependencies, target restrictions, license handling and formula tests
are unchanged. The Cargo manifests and dependency locks differ only in Flere's
own version fields.

The matching Rust sources and test inputs passed a macOS arm64 developer-package
run with 527 tests and all core/companion checks. The [v0.3.4 native Linux release
run](https://github.com/robert-cronin/flere/actions/runs/34930826557) passed the
core/companion checks and final package verification. A new Homebrew
`0.3.2` to `0.3.4` install/upgrade/remove run was not performed; the established
package-manager lifecycle baseline below remains explicitly versioned.

Native Linux x86_64 and hosted macOS arm64 checks completed source installation,
both formula tests, stateless CLI and license checks, upgrade from core `0.3.0_1`
and companion `0.3.0` to `0.3.2`, and full removal. Both new executables and selected
kegs were checked; disposable state and configuration stayed unchanged.

Linux used the pinned Homebrew 7.0.1 image, fresh dependencies, normal
`brew upgrade`, and strict linkage checks before and after upgrade. The core's
Linux `zlib-ng-compat` dependency is declared; the companion does not need it.
Default security settings remained enabled and the exact isolated container was
removed. Scoped formula trust preceded a verified Git checkout, so this does not
establish Homebrew's automatic tap-cloning flow.

The [hosted macOS acceptance run](https://github.com/robert-cronin/flere/actions/runs/34862741037)
passed on native arm64 macOS 15.7.9 with Xcode 16.4 and macOS 26.6.2 with Xcode
26.6. Each disposable VM used the default `/opt/homebrew` prefix, Homebrew 7.0.1,
empty private caches and newly installed direct Homebrew Rust 1.98.1. Normal
`brew install`, literal `brew upgrade`, both formula tests, CLI/license checks,
strict linkage before and after upgrade, and normal uninstall passed with the
Homebrew sandbox enabled. No `--ignore-dependencies` option was used.

Preinstalled transitive dependencies were recorded, so this does not claim every
dependency was pristine. Scoped trust and verified tap checkouts at the two
reviewed commits were used; automatic tap cloning was not tested. Earlier local
macOS private-prefix checks used cached dependencies and install-driven upgrade;
they remain separate historical evidence. Neither platform's stateless checks
establish interactive terminal behavior or preservation of live sessions. These
source-formula results do not validate the separate v0.3.0 prebuilt downloads,
Intel Macs, Developer ID signing or notarization.

Both formulas intentionally use the full source archive: the companion references
shared code and build support outside its own directory. Preserve the font licenses
that accompany the embedded font. For a new release, update both source URLs,
versions and checksums together after its source and runtime validation using the generator.

From the Flere source checkout:

```sh
python3 packaging/homebrew/render.py "$SOURCE_ARCHIVE" --sha256 "$SOURCE_SHA256"
python3 packaging/homebrew/render.py "$SOURCE_ARCHIVE" --sha256 "$SOURCE_SHA256" --check
python3 packaging/homebrew/verify.py "$SOURCE_ARCHIVE"
python3 scripts/test_homebrew_distribution.py
```

Read `SOURCE_SHA256` from the verified release checksum list. The generator checks
the archive before writing either formula; it rejects mismatched versions and missing
shared inputs. The `--check` mode detects recipe drift without writing.

In this standalone tap, run the same commands without `packaging/homebrew/`; use `python3 verify.py "$SOURCE_ARCHIVE"`. Verification
hashes the archive and checks that the core, companion, shared build inputs and
licenses are present. It never extracts, installs, commits or publishes anything.

Changes to installation behavior, dependencies or supported platforms need
Homebrew style, installation and formula tests on disposable machines for each
supported OS, keeping build/test state in disposable home-cache directories. The
[manual macOS workflow](https://github.com/robert-cronin/flere/blob/main/.github/workflows/homebrew-macos.yml)
checks fresh direct Rust provisioning, the default prefix and literal version upgrade.
For separate local offline checks, preseed a disposable Cargo cache and set
`CARGO_NET_OFFLINE=true`; the formula's fetch step then checks the cache without
network access. Cached checks do not establish fresh dependency provisioning.

Publish only `Formula/`, `README.md`, `LICENSE`, `render.py`, `verify.py` and
`formula.rb.in` from this directory.
Do not include `packaging/homebrew-prebuilt` or an enclosing workspace or Git history.
There are no bottles or binary casks in this tap.
