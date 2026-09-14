# Native macOS support

Flere builds as a native Mach-O executable for `aarch64-apple-darwin` and
`x86_64-apple-darwin`. Linux x86_64 remains supported. This port preserves the
existing UI, protocol, private state location and explicit session lifecycle.

## Implementation

- Platform constants come from `libc`: PTY allocation, controlling terminals,
  termios, window resizing, file locks, signals and no-follow/nonblocking opens.
- Darwin Unix sockets authenticate peers with `getpeereid`.
- The login-shell fallback uses `getpwuid_r`, including macOS directory services.
- Owned process discovery uses `libproc` for parent/child identity, executable,
  working directory and open descriptors, plus bounded `KERN_PROCARGS2` for argv
  and environment. The consumer retains only its existing configuration and
  session fields. No global process scan or transcript-directory search is added.
- Refresh validates the original parent and process start time before adopting
  an unreaped child. Detach and same-process refresh preserve PTYs and drafts;
  a supervisor restart still starts no saved terminal or agent automatically.
- Darwin cannot open another process's descriptor via `/proc/PID/fd`. Flere
  opens the kernel-reported path without following its final symlink, requires
  a regular file with the observed device/inode, then rechecks the descriptor.
  A renamed, replaced or inaccessible transcript cannot silently substitute
  another native target. Incomplete proofs fail closed.

Unsafe platform bindings stay in `src/os.rs`. Darwin structures use concrete
SDK-compatible C layouts and exact returned-size checks. The original platform port needed no Rust dependency or
lockfile change. Screenshot export now adds portable font/image dependencies. Native local graphics also link macOS's built-in
ImageIO, CoreGraphics and zlib libraries.

**Copy Flere screenshot** renders the complete composed Flere view using
portable Rust font/image codecs and a bundled licensed font. macOS clipboard
bindings write PNG data and a retained private PNG path for terminal paste; there
is no desktop capture, window picker, or screen-recording permission. Tests use a
private named pasteboard, including a separate AppKit reader after releasing the
writer; they do not replace the user's clipboard. Linux provides X11/Wayland
clipboard ownership, and current protocol v6 exports images to the local Windows/macOS/
Linux companion. Cross-compilation is not physical desktop clipboard acceptance.

## Validation

The [Flere 0.3.0 validation record](docs/releases/0.3.0-validation.md) distinguishes
complete native baselines, final focused checks, compile checks and remaining
physical acceptance. The held-control follow-up passed a macOS arm64 baseline
of 141 unit, 202 live and 13 installer tests, followed by 21 arcade units and five
real UI/remote tests after the final reply-attribution correction. The companion
passed 64 native tests. Strict Clippy, formatting, native release builds and Intel
Mac all-target compilation passed.

Live tests cover shell and editor PTYs, resizing, native metadata, exact-target
mailbox delivery, socket guards, Git/worktrees, scrollback and the remote bridge.
Darwin regressions cover argument parsing, peer credentials, stale start tokens,
descriptor bounds and transcript replacement/symlink rejection.

Tests require Git, Vim, Python 3 and the Xcode/Command Line Tools Python framework,
`install_name_tool` and `codesign`. Harmless native stand-ins copy that interpreter
into disposable home-cache directories, repair its local library reference and
ad-hoc sign the copy. Tests never modify the installed interpreter or start real
coding agents. PTY assertions wait for complete UI frames and allow short writes.

The published Homebrew v0.3.2 source formulas also passed
[hosted arm64 lifecycle checks](https://github.com/robert-cronin/flere/actions/runs/34862741037)
on macOS 15.7.9 (Xcode 16.4) and 26.6.2 (Xcode 26.6). Both fresh VMs used the default
`/opt/homebrew` prefix, empty private caches, normal Homebrew Rust 1.98.1
provisioning and the enabled Homebrew sandbox. Normal install, literal upgrade
from core 0.3.0_1/companion 0.3.0 to both 0.3.2, formula/stateless CLI/license and
strict linkage checks, and removal passed; disposable state stayed unchanged.
Preinstalled transitive dependencies were recorded. Scoped trust preceded a
verified tap checkout, so automatic tap cloning was not tested. See the
[Homebrew validation record](packaging/homebrew/README.md#maintain-and-validate).

## Acceptance limits

The Homebrew source-formula checks cover only the two macOS versions above; they
do not extend acceptance of the separate v0.3.0 prebuilt downloads or establish
physical UI/clipboard behavior. Other macOS versions and Intel Mac runtime
acceptance remain pending. Developer ID signing and notarization remain pending.
Real Codex/Claude/Copilot sessions and physical terminal-specific key handling
still need user acceptance. Full VT compatibility is not claimed.

The optional SSH companion builds on macOS using its Unix backend. It reads PNG
and TIFF images from the native macOS pasteboard only on an explicit paste gesture;
TIFF is converted to bounded PNG data. Its portable PNG/JPEG decoder supports
previews and badges in Sixel-capable terminals that report cell geometry. This
does not add Kitty support to the companion. Direct local Flere renders pets,
repository icons and PNG/JPEG previews with Kitty graphics (including Ghostty).
It probes support, consumes terminal replies locally, decodes images on a worker,
and clears attachment-owned placements on hide, resize, detach and refresh.
The local UI needs neither the companion nor Docker/Rosetta. Protocol/PTY tests
and Flere's own exported frames do not establish the exact visible appearance
of a particular terminal build.

See README.md for build, installation, state and navigation instructions, and the
[acceptance checklist](docs/acceptance.md) for the remaining physical-terminal and
native-agent checks. Compilation and protocol fixtures do not complete them.
