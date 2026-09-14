# Nix packaging draft

This draft pins the published **v0.3.3** full source archive with the explicit
production correction below. It is not a
Nixpkgs submission or a supported installation method. The manual
[Nix sandbox workflow](../../.github/workflows/nix-acceptance.yml) is prepared for
a disposable native x86_64 Ubuntu 24.04 GitHub VM. The first run
[34863649615](https://github.com/robert-cronin/flere/actions/runs/34863649615)
verified the installer download but rejected an unsupported CLI flag before
installation. After the installer correction, run
[34864791262](https://github.com/robert-cronin/flere/actions/runs/34864791262)
passed the strict sandbox probe, core release build and 182 core library tests,
then failed 16 of 213 live tests. The installer and companion suites were not
reached. The fixture corrections and production fix below await a full hosted
rerun; this is not complete Nix acceptance.

The earlier v0.3.0 recipe passed all 15 native Linux Docker parsing, evaluation,
build, build-info and inventory checks. That evidence used the official
Nix 2.35.2 image `sha256:617d914dba5384bf75adf17081583b69371031ec7defce36c34c5fa14fc819b0`
with `sandbox = false`. A later strict sandbox probe could not create the
required namespaces under unchanged Docker security. Neither result is a
v0.3.3 sandbox test-suite pass.

The new workflow pins Nixpkgs `eaad089433ca2bb662274377d33df3d0e51ef28b`, previously
observed to provide Rust/Cargo 1.98.1. It verifies the official NixOS installer
2.33.3 Linux executable's published SHA-256 and size before installation. Only
the disposable VM is modified. It sets `sandbox = true`,
`sandbox-fallback = false`, and requires a real derivation to observe a separate
network namespace before running package checks. There is no Docker privilege,
AppArmor, sysctl, KVM, or fallback workaround. A namespace refusal fails the job.
See [GitHub's hosted VM documentation](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and the [Nix sandbox settings](https://nix.dev/manual/nix/2.33/command-ref/conf-file.html#conf-sandbox).

The [v0.3.3 source archive](https://github.com/robert-cronin/flere/releases/download/v0.3.3/flere-0.3.3-source.tar.gz)
contains source commit `ce6bb62ca6051d8bfc385c2df16bc62cbd65c738` and is pinned by
the compressed-file SHA-256:

```text
461bbe1e3fa88027c2ea7191c34adbd7eceaadbf6f091a270769b1cdc0ad6a9e
```

The derivations also apply the reviewed
[wrapped-path delimiter patch](patches/wrapped-path-delimiter.patch), SHA-256
`3e1bcd7b259d047974a85f2ff0cd8aae965e90207ceba1253e9637e8ef701275`.
It fixes a production parser bug where a clicked path fills a terminal row and
its closing delimiter wraps to the next row, and adds a focused regression.
The existing failing remote image assertion remains unchanged. This is v0.3.3
plus that explicit patch and the three existing hash-helper substitutions,
not an unmodified v0.3.3 runtime. The published source archive is unchanged.

| Archive member | Copied lockfile SHA-256 |
| --- | --- |
| `Cargo.lock` | `7a6a0ec936b4cd0a7fd82b85b08c5b2dbd52356fb6e8b8b8f0b8b9eebaa7c789` |
| `companion/Cargo.lock` | `659e826726ecaadc8ea1a5b19fa95770e1ac286f4e4a3353b58416004003a0cb` |

`default.nix` exposes `flere` and optional `flere-connect`. It accepts `pkgs`,
otherwise using `<nixpkgs>`, and requires Rust 1.98 or newer. Each derivation
unpacks the whole archive because the crates share sibling source files.
The core imports the companion Sixel implementation; the companion imports core
files and the shared build script.

Both original locks are preserved and compared before and after checking.
Nixpkgs [`importCargoLock`](https://raw.githubusercontent.com/NixOS/nixpkgs/eaad089433ca2bb662274377d33df3d0e51ef28b/pkgs/build-support/rust/import-cargo-lock.nix)
uses the crates.io checksum of each external dependency. The vendor merge keeps
both sets, rejects conflicting crate identities and retains the selected
package's exact lock. No vendor/NAR hash is invented. This is necessary because
the core suite performs a real offline sibling Cargo build and then executes
`companion/target/debug/flere-connect`. The fixture clears the inherited Nix
`CARGO_BUILD_TARGET` for that native nested build to preserve this output path.

Both derivations run their full `--locked --offline --all-targets` suites with
one test thread, two compilation jobs and unchanged assertions and deadlines.
Git, Python, Dash, Bash, Zsh, Vim, Neovim, coreutils and ncurses are declared fixture
inputs. Writable HOME/XDG state is beneath the short owned sandbox build-home
cache. `test-support.py` substitutes fixture tool paths only inside integration
tests or identified unit-test modules; the simulated remote receiver uses the
declared stat/hash tools while the embedded production SSH scripts remain
unchanged. A fixture-change record is included in the build log.
The Linux `/bin/sh` fixtures use Dash, retaining their non-bracketed shell input
semantics; explicit Bash fixtures still use Bash. The two full shell-return
path expectations include their exact 80-column capture wrap, with their
existing assertions and deadlines retained.

The three existing production hash-helper replacements retain absolute trusted
Nix coreutils paths. No runtime PATH wrapper is added. The core declares zlib.
Git, configured shells/editors, OpenSSH and optional desktop tools remain user
environment dependencies of an eventual installed package.

Checks can also run on an appropriate disposable Nix VM:

```sh
nix-build packaging/nix/default.nix -A flere --no-out-link
nix-build packaging/nix/default.nix -A flere-connect --no-out-link
```

The hosted workflow additionally requires strict sandbox proof, both complete
suite logs, declared build-info install checks and exact output inventory.
It retains derivations, file hashes/modes and bounded logs. Package outputs cannot
substitute a previous result for this acceptance run. No user profile, Flere
session, native model or release is installed or launched. Fixtures exercise the
product using owned stand-ins. A failed check remains failed.

Local preparation only: five Python fixture/vendor/patch regressions and a
dependency-free offline Cargo output-layout reproduction passed. The latter ran
on macOS arm64 and is not a native Nix check. The prior shell syntax and workflow
lint checks remain valid; full-suite acceptance of the corrected draft is pending.

Interactive NixOS shell/detach/editor/Git, desktop clipboard and optional
companion SSH acceptance remain separate. Neither macOS nor cross compilation
is covered. v0.3.3 still lacks proof-based Nix update ownership: do not use its
installation controls to replace Nix-owned files. No profile or NixOS/Home Manager
upgrade command can be inferred safely from a store path alone. This draft adds
no ownership policy patch and does not close that TODO.

The build script embeds a random/time/process build identity. Pinned source and
dependencies do not imply bit-identical executable outputs.
