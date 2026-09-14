# Flere

Flere is a terminal workbench for persistent shells, peer agents, editors, and
Git. It owns its UI, terminal renderer, navigation, explorer, and session
supervisor. Shells keep running when you detach the UI.

This source package builds the **`flere` core executable** for Linux x86_64 or
macOS arm64/x86_64. It requires Rust 1.98 or newer and a system C linker. macOS
builds need Xcode Command Line Tools. Git and your configured Vim/Neovim are
external programs; agent CLIs are optional and run only when explicitly opened.

## Install from crates.io

```sh
cargo install flere --locked --version 0.3.1
```

[Version 0.3.1](https://crates.io/crates/flere/0.3.1) is published from the matching
GitHub release at `7f5c5eb`. Its public registry checksum and anonymous archive
match the verified upload. Installation needs no crates.io account and normally
places `flere` in `~/.cargo/bin`; use Cargo for upgrades and removal.

## Build from the source package

After extracting `flere-0.3.1.crate`, enter its `flere-0.3.1` directory:

```sh
cargo build --release --locked
./target/release/flere --version
./target/release/flere --build-info
```

Use `--offline` when the locked dependencies are already cached. To install this
extracted source with Cargo, run `cargo install --path . --locked`. Cargo owns
that installation, normally in `~/.cargo/bin`; this does not register a Flere
managed-update receipt or development source.

The packaging verifier prepares and checks an archive without publishing it.
Each later registry version needs its own upload and public-download verification.

Run `flere` in an ordinary terminal to open the workbench. **Ctrl+Space** enters
navigation, **Space** shows actions, and **q** in navigation detaches while
leaving shells alive. See the [handbook](https://github.com/robert-cronin/flere/tree/main/docs)
and [keyboard reference](https://github.com/robert-cronin/flere/blob/main/docs/reference/keyboard.md).

## Companion and platform scope

`flere ssh` delegates to the separate `flere-connect` executable. The companion
is not included in this crate; obtain its matching verified release package or
build it from the complete Git checkout. The Windows companion is also separate;
this core crate does not build a Windows workbench.

The archive includes the complete core source, build script, embedded assets,
their licenses, and the two image fixtures needed by library unit tests.
`cargo test --locked --lib` runs those tests. The full checkout contains the
additional PTY, companion, installer, and UI integration tests. Large design
explorations, recordings, release archives, and the companion source are excluded
from the crate.

## Build and source identity

`--build-info` reports an opaque Cargo generation stamp. Unchanged cached builds
retain it; an independent build can have a different stamp. It is not a source
digest, payload checksum, or promise of byte-for-byte reproducible binaries.

Cargo records the source commit in `.cargo_vcs_info.json`, preserves the original
manifest as `Cargo.toml.orig`, normalizes `Cargo.toml`, and includes a minimized
lockfile. The registry archive checksum identifies these packaged bytes. It is
distinct from the source-input receipt and payload hashes of Flere's prebuilt
release packages. Source packaging changes are bound to their own commit rather
than attributed to an earlier binary release.

Maintainers can run `python3 packaging/cargo/verify.py --output DIR` from a clean
Git checkout. Use a new private directory under your home. The verifier packages
offline with Cargo's normal verification enabled, checks archive contents and
source provenance, builds and tests the extraction, and checks stateless CLI
commands and cached build identity. It never publishes or installs the result.

## License

Flere's code is MIT licensed; see `LICENSE`. The embedded JetBrains Mono Nerd
Font is under the SIL Open Font License 1.1. Its attribution and the Nerd Fonts
license notice are included in `src/assets/fonts/`. The package license
expression is `MIT AND OFL-1.1` to represent both components.
