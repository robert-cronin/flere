[Documentation](README.md) · [Repository instructions](../AGENTS.md) · [Design](../DESIGN.md)

# Build, validate, and contribute

Flere is a Rust 2024 crate requiring Rust 1.98+. It owns its UI and terminal
emulator. Small infrastructure crates are allowed; native Git, editors, shells,
and harnesses remain external. Keep platform FFI in `src/os.rs`.

## Local validation

Fetch locked dependencies once, then use the repository's offline checks:

```sh
cargo fetch --locked
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo build --offline --locked --release
```

For the optional companion:

```sh
cargo test --offline --manifest-path companion/Cargo.toml
cargo clippy --offline --manifest-path companion/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path companion/Cargo.toml --check
```

Live tests need Git, Vim, and Python. macOS native identity fixtures additionally
use Xcode/Command Line Tools' Python framework, `install_name_tool`, and `codesign`.
Tests copy and sign disposable fixture executables; they start no real agents.
See [platform details](../MACOS.md) and the [current validation record](releases/0.3.0-validation.md)
for the distinction between complete baselines, focused reruns and physical
acceptance. Process-heavy runtime/installer fixtures are validated with
`--test-threads=1` to avoid cross-fixture process and inherited-lock interference.

A Linux musl build can be produced with the installed Rust musl target and a
suitable system linker. Cross-compilation proves build compatibility, not native
runtime behavior; record those separately.

## Documentation tools

| Tool | Purpose |
| --- | --- |
| [`latency_bench`](../examples/latency_bench.rs) | Direct-PTY baseline, completed-frame latency, and scoped resource samples |
| [`docs_demo`](../examples/docs_demo.rs) | Real workbench interactions in an owned generated Git fixture |
| [`capture_intro`](../examples/capture_intro.rs) | Six seconds of the actual first-use animation |
| [`capture_ui`](../examples/capture_ui.rs) | SVG/text/ANSI capture from an explicitly selected instance |
| [`history_bench`](../examples/history_bench.rs) | In-process terminal-history probe; different boundary from UI latency |
| [`render_demo.py`](tools/render_demo.py) | Captured cells → GIF and static screenshots |
| [`render_performance.py`](tools/render_performance.py) | Recorded benchmark JSON → SVG/PNG charts |
| [`check_docs.py`](tools/check_docs.py) | Local links, anchors, assets, and benchmark consistency |

Use home-cache output directories. The documentation recorders create and clean
up their own sessions. `capture_ui` is different: it attaches to the instance you
name and changes that instance's shared viewport; use a disposable state.

## Make a documentation change

Keep the README short and send detailed workflows to the handbook. Use relative
links, descriptive image alt text, and static alternatives for motion. Keep claims
next to their measurement scope. Do not replace local evidence with a performance
guarantee, a competitor comparison, or an unverified platform claim.

```sh
python3 docs/tools/check_docs.py
```

When changing runtime behavior, update the corresponding guide and run the
relevant tests. When changing a benchmark, retain its method, source/binary
identity, units, sample counts, and raw values. Read the architecture guide before
changing session ownership, native identity, or restart behavior.
