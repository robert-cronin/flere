# Windows fixes reconciled and Linux validation

This follow-up starts from `main` at
`979bdc75ca204f77c73a20da4461dd00a1060da5` and reconciles the Windows developer's
`0325fbe693236c8b5e2f2d6f3653f6e5dd425318`. The original
[Windows acceptance report](windows-acceptance-20260913.md) remains a record of
that earlier build, including its six Linux failures and incomplete physical
acceptance. Personal connection details and retained sessions stay private.

## Product changes

- Package creation, staging and launcher replacement now close writable file
  handles before executing or publishing those files. This fixes Linux
  `ETXTBSY` failures without changing verification or activation ordering.
- A fresh core or companion installation creates its missing Unix bin directory
  with private permissions. Previously, an ambient `umask 0002` created a
  group-writable directory that the installer then correctly rejected. Existing
  shared or linked directories still fail without permission changes or takeover.
- Neovim close inspection uses its supported child-process API. Walking the
  shared libuv loop with `vim.loop.walk` aborts Neovim 0.9.5 when it encounters
  non-Lua handles; an isolated direct reproduction confirmed the crash.
  Child PIDs still pass through Flere's existing process/registered-helper
  verification. Both LSP discovery APIs are supported, and unknown `jobstart`
  and raw `uv.spawn` jobs still require confirmation. Dirty buffers and quit
  hooks retain their existing protection.
- Windows screenshots publish original PNG, native DIB and text under one
  clipboard lock. The original PNG is offered first. Pure encoding checks cover
  bottom-up BGR pixels, odd-width padding, alpha matting and rejected dimensions;
  the original PNG retains alpha. Final Windows runtime of these follow-up
  changes has not been rerun on the physical machine.
- Flere uses the same compact white disc and continuous close halo in the pet
  dock and original intro screensaver. The wing-shaped field sheets are removed;
  wandering, petting, throwing, animated reactions and keyboard wake remain.
  Actual UI captures include the original intro at 236×55 and narrower layouts.

## Test corrections

The other three reported Linux failures came from assumptions in UI fixtures:
the file list was used before its asynchronous load finished, a resized `/bin/sh`
was expected to redraw its pending kernel-echo line, and package activation was
given the ordinary three-second redraw deadline. Fixtures now wait for the
visible list, verify the deliberately submitted harmless shell command, and
allow 30 seconds for the package copy/hash/activation transaction. Exact session,
draft, selection and update acknowledgement assertions remain.

Fresh full-suite runs exposed two further fixture races. Repeated attachment
paths had identical prompt titles, so a stale prompt could make the fixture send
its choice before the new consent gate was ready. The fixture now observes the
old modal disappear and the new modal's controls appear. The product still
rejects coalesced paste/Enter and early approval input. The graphics fixture now
waits for Escape's visible mode transition before sending Enter, and continues
draining terminal graphics output while awaiting the native result.

## Validation and retained evidence

Native Linux checks use Ubuntu 24.04 x86_64, Rust 1.98.0 and Neovim 0.9.5 in a new
private validation directory on the owner's authorized SSH host. Existing
installations, acceptance state and shells were not changed. All Cargo work is
offline. The initial archive's preserved mtimes caused Cargo to reuse an old
embedded editor hook; only the disposable package target was cleaned, and the
fresh binary was checked for the exact new Lua bytes before rerunning.

Full runs and focused reruns are recorded separately rather than described as
one uninterrupted green run. macOS arm64 coverage is **123 unit + 197 live +
11 installer tests**, with its one initial inspector-fixture failure corrected
and rerun. The companion's **48 tests** pass. Native release builds, strict
all-target Clippy and formatting pass; Intel Mac core and Windows companion
all-target compile checks also pass.

The fresh Linux full run passed **119 unit, 197 live and 10 installer tests**;
the two additional live fixture failures are described above and both corrected
fixtures pass independently. All six failures from the original handoff passed
in that full run. The final installer suite adds one regression: aggregate
coverage is **119 unit + 199 live + 11 installer tests**, plus **47 companion
tests**. Both installer suites pass under `umask 0002`, including their explicit
private-directory and existing-owner rejection checks. Both Linux release builds,
strict all-target Clippy and formatting pass. The final Linux and macOS manifests
have identical hashes for all product source inputs.

Process-heavy installer fixtures run with `--test-threads=1`, matching the earlier
full-run conditions. An exploratory parallel invocation hit transient lock-busy
errors after a previous lock was dropped. A separate reproduction on both OSes
confirmed that another thread's fork inherits the lock descriptor until exec,
temporarily retaining the lock after the parent closes it. Serialization removes
this cross-fixture interference; the parallel failures remain in the evidence.
This is not a claim that the default parallel suite passed. Initial permissive-
umask checks also exposed a development-checkout fixture that needed to create
its intended trusted directories explicitly; ownership checks were not weakened.

An additional isolated headless X11 run exercised the actual UI screenshot
action and detached clipboard owner. Independent `xclip` PNG and UTF-8 text
consumers matched the retained PNG/path before and after UI detach. The exact
fixture supervisor epoch, shell PID/run and unsubmitted draft survived. Replacing
the selection caused the original owner to exit successfully. The container had
no network or real desktop mount, and its supervisor, Xvfb and container were
cleaned up. This proves X11 ownership/runtime behavior, not Wayland or physical
terminal/native-chat acceptance.

Detailed logs and exact local installation observations are retained privately.
The technical findings above belong to this historical reconciliation build.
Current naming, bootstrap and package status are recorded in the
[Flere 0.3.0 validation summary](releases/0.3.0-validation.md). Physical terminal,
clipboard and native-agent acceptance remain in the
[acceptance checklist](acceptance.md).
