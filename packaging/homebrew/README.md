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

Homebrew downloads the pinned v0.3.2 full source archive and its Rust build
dependencies, then compiles each executable locally. The first install takes
longer than downloading a prebuilt binary and needs Rust 1.98 or newer, which
Homebrew supplies as a build dependency. Dependencies follow the checked-in lock
files; installation compiles in Cargo's offline mode after fetching them. On Linux,
the core also declares `zlib-ng-compat` as a runtime dependency; the companion does
not need it.

This tap supports macOS Apple Silicon and Linux x86_64. Intel Macs and Linux ARM
are excluded pending runtime acceptance. Older macOS runtime acceptance remains
pending; Homebrew and its Rust dependency also impose their own supported OS
requirements. See [platform limits](https://github.com/robert-cronin/flere/blob/main/MACOS.md)
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

Native Linux x86_64 and isolated macOS arm64 checks completed source installation,
both formula tests, stateless CLI and license checks, upgrade from core `0.3.0_1`
and companion `0.3.0` to `0.3.2`, and full removal. Both new executables and selected
kegs were checked; disposable state and configuration stayed unchanged.

Linux used the pinned Homebrew 7.0.1 image, fresh dependencies, normal
`brew upgrade`, and strict linkage checks before and after upgrade. The core's
Linux `zlib-ng-compat` dependency is declared; the companion does not need it.
Default security settings remained enabled and the exact isolated container was
removed. Scoped formula trust preceded a verified Git checkout, so this does not
establish Homebrew's automatic tap-cloning flow.

macOS used a nonstandard private prefix, cached Rust/Cargo dependencies and
Homebrew's unsupported `--ignore-dependencies` option, with its sandbox enabled. The between-release
upgrade used `brew install`'s automatic upgrade behavior, not literal
`brew upgrade`. Fresh macOS dependency provisioning and the default prefix remain
unvalidated. The retained Rust toolchain emitted an LLVM stripping warning while
builds succeeded. Neither platform's stateless checks establish interactive
terminal behavior or preservation of live sessions.

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

Run Homebrew style, installation and formula tests in an isolated prefix on each
supported OS. Use disposable home-cache directories for build/test state. During
local offline validation, preseed a disposable Cargo cache and set
`CARGO_NET_OFFLINE=true`; the formula's fetch step then checks the cache without
network access. A fresh user's Homebrew installation fetches the locked dependencies.

Publish only `Formula/`, `README.md`, `LICENSE`, `render.py`, `verify.py` and
`formula.rb.in` from this directory.
Do not include `packaging/homebrew-prebuilt` or an enclosing workspace or Git history.
There are no bottles or binary casks in this tap.
