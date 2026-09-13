# Historical Windows acceptance (2026-09-13)

Follow-up: [main reconciliation and fresh Linux validation](linux-reconciliation-20260913.md)
records subsequent fixes and results. The report below retains the original
acceptance findings for its tested build.

Acceptance started from `44b824f134c81326455655d8bcb35ea8a73194a6`.

The Windows companion builds and runs over real SSH to Ubuntu. Acceptance is
partial: the checks below distinguish the owner's physical Windows Terminal
observations from automated native Windows ConPTY checks. Personal logs,
screenshots, receipts and exact target identities remain in the private evidence
archive and are not part of this source snapshot.

Changes made:

- Publish Windows screenshot PNG, native bitmap and text path under one clipboard
  lock. The previous library call released its lock during nested image
  publication: pasting an image could work while PNG/path verification failed
  and the retained screenshot was deleted. A standard 24-bit DIB also supports
  Windows bitmap consumers; the original PNG retains alpha.
- Close writable core package/staging handles before executing or publishing
  their files. Linux acceptance exposed `ETXTBSY` failures with the handles open.
- Make two companion test fixtures use Windows home/path conventions.

Validated on Windows 11 Home build 26200, Windows Terminal 1.24.11911.0,
PowerShell 5.1 and native Rust 1.98.0 MSVC with the Windows SDK. Tools and the
initial locked dependency download were explicitly authorized. Subsequent Cargo
work ran offline.

- [x] Companion formatting, strict Clippy and release build.
- [x] 30 default Windows tests; one additional opt-in real clipboard regression
  executed separately and passed (31 executed, no failures).
- [x] Final installed release screenshot: retained PNG matches clipboard PNG
  byte for byte; text points to that file; independent Windows Forms bitmap
  reading succeeds at 1200x768. This was an automated native ConPTY check.
- [x] Physical Windows Terminal Sixel and cell-geometry replies (10x20 pixels).
- [x] Owner observed tiny PNG/JPEG fixtures and gallery changes in the physical
  terminal, reported narrow/wide resizing, screenshot paste into Paint and q exit.
  Those observations precede the final clipboard fix.
- [x] Physical terminal exit restored console input/output modes exactly.
- [x] Actual Windows ConPTY/SSH coordinated release-to-debug update, initiating
  frontend ACK, companion rollback/reconnect and remote return to release.
  Stable launcher remained; worker changed; controlled before/after snapshots
  retained both ordinary shells' PID/run identities, split selection and drafts.
- [x] Real upload/download with equal hashes, prompt cancellation, existing-file
  rejection, and cleanup of the test download.
- [x] Loopback-only forwarding and Stop terminating only its owned SSH child.
- [x] Corrupt/wrong-target package rejection without installation replacement;
  inert preparation; paths containing spaces and `%!&^`.
- [ ] Existing native chat image paste with Ctrl+V and the terminal paste gesture,
  one attachment, intact draft and no submission; plain text paste. No existing
  chat was selected, and no native chat was launched for these tests.
- [ ] Mid-transfer cancellation/focus changes, local open and overwrite approval.
- [ ] Larger portrait/wide/alpha fixtures, visible terminal-path opening, badge
  states, font/zoom/scaling, separate Esc and Ctrl+Space preview cleanup,
  menus/hover/workspace cleanup and idle intro interactions.
- [ ] Update in the owner's physical terminal, changed installation during
  Review, forced partial acknowledgement, published downloads/signing/bootstrap.
- [ ] Full Linux core test suite: 286 passed, 6 failed (details below).

The controlled update and rollback checks preserved two ordinary fixture shells.
This claim covers those controlled checks, not later manual UI activity.

Both installed components finish on release packages. Final Windows payload
SHA-256 is `9844aabda545170b65cec5db8e7b3ffed083d881f41cc7bed8929fb477085044`;
core payload SHA-256 is
`de5f2fd3c96ed6709adea5bb9a857c9e0bbd61a7576a150d9a5cbc16b3d34c2e`.
The Windows stable launcher's hash remains
`59f5fad63a69056e11280286a08ebc0ad51f2cf73b7b113afce0fe7b41ec87db`;
its own build stamp does not identify the current worker.

Linux x86_64 / Ubuntu 24.04 results after the handle fixes: 99 unit, 177 live and
10 installer tests passed; six live tests failed. Formatting, strict Clippy,
release/debug builds, all installer tests and the dedicated coordinated
companion/core update test passed. The full suite used
`cargo test --offline --locked --no-fail-fast -- --test-threads=1` with a private
home-cache test root and umask 077. Do not substitute historical macOS counts.

Remaining failures:

- `actual_ui_inspector_tabs_paging_and_context_preserve_native_draft`
- `actual_ui_tree_overflow_mouse_actions_and_narrow_details_are_reachable`
- `closing::clean_editors_quit_without_a_signal_and_dirty_hidden_buffers_stay_open`
- `closing::editor_quit_hooks_cannot_turn_a_clean_close_into_unsaved_data_loss`
- `closing::nvim_protects_user_jobs_but_allows_registered_language_server_helpers`
- `updating::ui_update_applies_the_verified_package_and_acknowledges_the_new_frontend`

The closing failures include Neovim 0.9.5/libuv assertion crashes; other failures
are UI expected-condition timeouts. Their root causes remain unverified. This
branch does not weaken assertions or claim these Linux failures are fixed.

At the time of this report, the next planned feature was a VS Code Remote-style
entry point, `railhand ssh <existing-alias>`. Flere 0.3.0 now implements
[`flere ssh ALIAS`](guides/remote.md); the following describes the earlier
requirements. Reuse normal OpenSSH configuration, keys and host
trust; discover the remote OS/architecture; reuse or install a compatible,
verified, versioned prebuilt core in user-owned storage; start/attach without
requiring Rust or a manually supplied remote executable. Keep connection and
installation progress understandable, with actionable authentication, missing
asset, offline and compatibility errors. The observed current failure was
`zsh: command not found: railhand` followed by a generic EOF error.

Preserve exact state/session identities and drafts, isolate versions, serialize
concurrent installations, support rollback and report incomplete activation.
Avoid modifying shell startup files or requiring root for normal bootstrap.
Keep the existing explicit executable/state route available. Automatic remote bootstrap had not been implemented or accepted at the time of
this historical report.
