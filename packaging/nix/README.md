# Nix packaging draft

This draft pins the published **v0.3.3** full source archive with the explicit
production correction below. Native strict-sandbox builds, all **485 tests** and
both output inventories passed in hosted run
[34869221522](https://github.com/robert-cronin/flere/actions/runs/34869221522)
at workflow commit `2d52985845ed322b1c6c0f3018eaedf38d6bcead`. Separate native
Nix-on-Ubuntu ownership/profile acceptance passed for the newer v0.3.4 development
source, as described below. Actual private-profile version upgrades and emulated
NixOS core runtime checks also passed. Physical clipboard and external companion
SSH acceptance remain open; this is not a Nixpkgs submission or a supported
installation method yet.

The [manual workflow](../../.github/workflows/nix-acceptance.yml) used a disposable
native x86_64 Ubuntu 24.04.5 GitHub VM. The 485 passing tests comprise 183 core
library, 213 live, 13 installer and 76 companion/example tests, with none failed,
ignored or filtered. Independent review matched every summary to its actual test
assembly and verified both build-info records, exact executable/license
inventories and the artifact digest. Both packages built locally in the real
Nix sandbox; the namespace probe passed before the package builds.

Earlier failures remain separate: run
[34863649615](https://github.com/robert-cronin/flere/actions/runs/34863649615)
rejected an installer flag, and
[34864791262](https://github.com/robert-cronin/flere/actions/runs/34864791262)
failed 16 live tests after passing the sandbox probe and core library tests.
All 16 cases pass in the corrected run. The fixture corrections and production
fix below preserve the original assertions, deadlines and isolation settings.

The earlier v0.3.0 recipe passed all 15 native Linux Docker parsing, evaluation,
build, build-info and inventory checks. That evidence used the official
Nix 2.35.2 image `sha256:617d914dba5384bf75adf17081583b69371031ec7defce36c34c5fa14fc819b0`
with `sandbox = false`. A later strict sandbox probe could not create the
required namespaces under unchanged Docker security. Neither result is a
v0.3.3 sandbox test-suite pass.

The passing workflow pins Nixpkgs `eaad089433ca2bb662274377d33df3d0e51ef28b`,
which provided Rust/Cargo 1.98.1. It verifies the official NixOS installer
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

Five focused Python fixture/vendor/patch regressions and a dependency-free
offline Cargo output-layout reproduction also passed during preparation. These
are distinct from the subsequent native hosted full-suite acceptance above.

The separate [installed-owner workflow](../../.github/workflows/nix-owner-acceptance.yml)
passed [run 34874904110](https://github.com/robert-cronin/flere/actions/runs/34874904110)
at workflow commit `8fb9d7db7461d2fb5d8884d1791fe4caf2f1d440`. It built exact public
source `2d52985845ed322b1c6c0f3018eaedf38d6bcead`, the unreleased v0.3.4 development
version, on native Ubuntu 24.04.5 x86_64 with Nix 2.33.3 and the same pinned
Nixpkgs providing Rust 1.98.1. Both builds used the strict sandbox, original locks
and only the three declared hash-helper substitutions. This targeted check did not repeat
the full suites or change `default.nix` and its v0.3.3 source pin.

An ordinary user installed both actual store outputs through a private
`nix-env --profile`. Six stateless flags passed. Exact empty supervisor/frontend
identities and real core/companion update UIs confirmed verified Nix ownership,
passive owning-configuration guidance and refusal before staging. The product's
unchanged two-second live-content verification ran on the actual package outputs.
Both UIs detached normally, the empty supervisor exited cleanly, and normal
profile removal preserved synthetic Flere and unrelated user state. No process
needed a forced kill. UI evidence is retained terminal output; the companion used
an exact local SSH stand-in, not an external connection.

The same workflow passed the real-version profile upgrade in
[run 34883927251](https://github.com/robert-cronin/flere/actions/runs/34883927251)
at `c8107ad`. It built a fresh v0.3.3 pair from the pinned archive and declared
parser patch, then used literal `nix-env --profile … --upgrade --lt` with both
exact v0.3.4 outputs. The profile changed from generation 1 to 2; both selected
package names, paths, hashes and versions matched. All 12 stateless checks and
both upgraded owner/refusal UIs passed. Normal profile removal and exact process
cleanup preserved synthetic state. A successful no-op is rejected by the checks.

All four derivations passed a complete closure/disk gate before realization and
built under the same strict sandbox and limits. The previous pair skipped full
suites only for this separate fixture and retained install checks; its fresh
bytes are not the recorded 485-test outputs. The default recipe and full-suite
workflow remain unchanged. This is native Nix-on-Ubuntu profile acceptance,
not live old-process refresh or NixOS/Home Manager activation; no channel was repinned.

The initial [run 34874043087](https://github.com/robert-cronin/flere/actions/runs/34874043087)
remains a failed pre-compilation check: the verified archive was referenced by
its host filename instead of a store input. Importing both the archive and locks
into the store corrected the harness without weakening sandboxing or changing
product code.

The separate [emulated runtime workflow](../../.github/workflows/nixos-tcg-runtime.yml)
passed [run 34883876197](https://github.com/robert-cronin/flere/actions/runs/34883876197)
at `c8107ad`, testing exact core source `2d52985` in a headless NixOS guest.
Real shell output, a configured Vim edit/save, Git inspection and detach/reattach
with the exact session identities and an unsubmitted draft passed. Frontends,
owned editor/shell sessions, supervisor and QEMU exited normally. Actual QEMU
arguments selected TCG and its monitor confirmed KVM disabled. This is emulated
NixOS filesystem/terminal evidence, separate from the native profile checks.

The first runtime attempt failed before boot because the maintained driver
rejected a deprecated API call. Replacing it with the equivalent current method
retained type checks and all runtime assertions. The separate hosted KVM probe
remains blocked by device permissions; no permissions or security settings changed.

Physical desktop clipboard and external companion SSH acceptance remain open.
The emulated guest tested the core only. Neither macOS nor cross compilation is
covered by these runs. The pinned v0.3.3 recipe
still lacks proof-based Nix update ownership: do not use its installation controls
to replace Nix-owned files. No profile or NixOS/Home Manager upgrade command can
be inferred safely from a store path alone. The published recipe remains
v0.3.3 plus its explicit parser patch, separate from the newer ownership proof.

The build script embeds a random/time/process build identity. Pinned source and
dependencies do not imply bit-identical executable outputs.
