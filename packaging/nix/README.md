# Nix packaging proposal

This is a **draft** for the released Flere v0.3.0 source, targeting native
x86_64 Linux. It is not a Nixpkgs submission or a supported installation method.
Native x86_64 Linux Docker validation passed parsing, evaluation, both builds,
each declared build-info install check and the final output inventory. Each output
contained exactly its executable and three license files, with the expected modes
and ownership; every file was hashed. The first core build exposed a missing zlib
input, now declared by the recipe. An intermediate inventory attempt failed on a
missing utility; the corrected validator completed all 15 phases.

Validation used the official Nix 2.35.2 image pinned to
`sha256:617d914dba5384bf75adf17081583b69371031ec7defce36c34c5fa14fc819b0`
and Nixpkgs `eaad089433ca2bb662274377d33df3d0e51ef28b`, which provided
Rust/Cargo 1.98.1. The image's default Nix and Docker security settings were
retained, including `sandbox = false`. A later minimal derivation with
`sandbox = true` and `sandbox-fallback = false` could not start because the
container lacked the required kernel namespaces. No security settings were
weakened and no upstream suite ran in that attempt. Sandbox acceptance needs an
isolated Linux/Nix host or VM that supports those namespaces. This container
evidence does not establish NixOS runtime acceptance.

`default.nix` exposes `flere` and the optional `flere-connect` separately. It accepts
a caller-provided `pkgs` set and otherwise uses `<nixpkgs>`. That package set must
provide Rust 1.98 or newer. This draft does not pin Nixpkgs or a Rust toolchain;
record the exact Nixpkgs revision when validating it.

The [release archive](https://github.com/robert-cronin/flere/releases/download/v0.3.0/flere-0.3.0-source.tar.gz)
is pinned by its verified compressed-file SHA-256:

```text
5ccc74765af98ae38f51be21709e3e703312ebde3eadb59d5b4cf8adbd31e94f
```

`fetchurl` uses that raw-file digest. Both lockfiles are byte-for-byte copies from
the archive; all external dependencies have crates.io checksums. Nixpkgs supports
[`cargoLock.lockFile`](https://nixos.org/manual/nixpkgs/stable/#rust), so this draft
needs no aggregate vendor hash or Git dependency output hashes.

| Archive member | Copied lockfile SHA-256 |
| --- | --- |
| `Cargo.lock` | `e903af6add90f2d5f8eb581fc157da4ef63c440db15227f2c3d50e949bca2e6e` |
| `companion/Cargo.lock` | `0f81427e49be361d39dec82622b8591df7a6361a1e9d911ad76faed4dedf1334` |

Each derivation unpacks the **full archive**. The core imports
`companion/src/sixel.rs`; the companion imports files under `src/` and uses the
shared build script. `cargoRoot` selects the correct lockfile, while
[`buildAndTestSubdir`](https://raw.githubusercontent.com/NixOS/nixpkgs/nixos-unstable/pkgs/build-support/rust/build-rust-package/default.nix)
selects the crate without discarding its siblings.

The only source adjustment replaces three Linux `/usr/bin/sha256sum` literals with
the absolute Nix coreutils path. No user PATH wrapper is added: Git, the configured
editor, shell, OpenSSH and optional `gh`, `curl` and desktop open helpers remain
user-environment tools. The inspected Linux feature trees select the Rust Wayland
and X11 implementations; no native Wayland/X11 library input is assumed. The
core declares zlib for the C ABI used by pixel compression.

On a disposable x86_64 Linux Nix environment, these are the package checks to run
from this directory. They create build outputs, not a user-profile installation:

```sh
nix-instantiate --parse default.nix
nix-instantiate --strict default.nix -A flere
nix-instantiate --strict default.nix -A flere-connect
nix-build default.nix -A flere -o result-flere
nix-build default.nix -A flere-connect -o result-flere-connect
```

The declared install check reads each binary's `--build-info` JSON and checks its
component and version. That command creates no application state or native chat.
The upstream test suite is disabled here because its home-cache, PTY and absolute
fixture-tool assumptions have not been adapted to the Nix sandbox. It must be
addressed before promoting this draft to supported packaging.

Further acceptance needs a real terminal: shell start, detach/reopen, editor and
Git use, X11/Wayland clipboard, and optional companion SSH with fixtures. Use
disposable home-cache state and stand-ins; do not launch paid model sessions.
Neither NixOS runtime behavior nor macOS/cross compilation has been validated.

Update ownership also remains pending: v0.3.0 does not recognize Nix-managed
executables. Manage any evaluation installation through Nix and do not use Flere's
installation/update controls to replace it. The runtime acceptance must verify
that updating cannot create a second per-user installation or refresh into the
wrong executable. This draft adds no update-policy patch.

Finally, the upstream build script deliberately embeds a random/time/process build
generation stamp. Pinned inputs therefore do not imply bit-identical outputs.
This draft preserves that identity behavior.
