# Windows acceptance procedure

Validate the Windows companion against an explicitly selected source commit.
Record the real machine, terminal and toolchain. Compilation is useful evidence;
physical clipboard, SSH, input and cleanup checks require the actual environment.

Select an existing SSH alias and explicit remote Flere executable/state. Use
owner-provisioned disposable shell fixtures for lifecycle tests. Any native chat,
trust prompt or actual message test requires the user’s separate choice. Do not
change SSH trust, inspect private keys or touch unrelated installations/sessions.

Supply an accessible repository URL and exact reviewed commit when running this
procedure; release availability is separate from source compilation.

## 1. Obtain exactly the validated source

Run in an ordinary PowerShell window, outside any Flere child. Replace the
commit placeholder with the **full reviewed source hash**. This procedure creates
a fresh private checkout with only local `main`; it never reuses or changes an
existing checkout. The reviewed commit must be reachable from fetched
`origin/main`, but may precede later documentation or automation commits.

```powershell
$ExpectedCommit = '<FULL_REVIEWED_SOURCE_COMMIT>'
$Checkout = Join-Path $env:LOCALAPPDATA ('Flere\acceptance-source\' + [guid]::NewGuid().ToString('N'))
$RepoUrl = 'https://github.com/robert-cronin/flere.git'
if ($ExpectedCommit -notmatch '^[0-9a-f]{40}$') { throw 'Supply the exact reviewed source commit.' }
function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit $LASTEXITCODE" }
}
# No -Force: an existing path must stop before any Git operation.
New-Item -ItemType Directory -Path $Checkout -ErrorAction Stop | Out-Null
Invoke-Checked git @('init', '--initial-branch=main', $Checkout)
Set-Location -LiteralPath $Checkout
Invoke-Checked git @('remote', 'add', '-t', 'main', 'origin', $RepoUrl)
Invoke-Checked git @('fetch', '--no-tags', 'origin', 'main')
Invoke-Checked git @('merge-base', '--is-ancestor', $ExpectedCommit, 'refs/remotes/origin/main')
# main is still unborn; create it at the reviewed commit without detach or reset.
Invoke-Checked git @('switch', '--create', 'main', '--no-track', $ExpectedCommit)
$ActualCommit = (Invoke-Checked git @('rev-parse', 'HEAD')).Trim()
$Branches = @(Invoke-Checked git @('for-each-ref', '--format=%(refname)', 'refs/heads/'))
if ($ActualCommit -ne $ExpectedCommit -or $Branches.Count -ne 1 -or $Branches[0] -ne 'refs/heads/main') {
    throw 'Checkout identity differs from the reviewed source. Preserve it and report.'
}
if (@(Invoke-Checked git @('status', '--porcelain')).Count -ne 0) {
    throw 'Unexpected checkout changes. Preserve them and report; no stash/reset/clean.'
}
```

Keep any existing work and its Git remote untouched. Retain a failed fresh
checkout as evidence; do not repair it by resetting or cleaning another tree.
Read the checked-out `AGENTS.md` and `docs/acceptance.md`, then continue at
section 2 below. If the reviewed commit contains an older copy of this procedure,
keep the obtain-source step above rather than repeating its checkout commands.

## 2. Record the machine and validate the companion offline

Use the installed **native host toolchain**, normally Windows MSVC. An installed
Windows GNU toolchain is also a distinct test target; do not force GNU merely
because macOS previously cross-checked it. `companion/Cargo.toml` specifies the
minimum Rust version. Do not run root `cargo test` on Windows.

```powershell
$Evidence = Join-Path $env:LOCALAPPDATA ('Flere\acceptance\' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $Evidence -ErrorAction Stop | Out-Null
$BuildDir = Join-Path $Evidence 'cargo-target'
Start-Transcript -Path (Join-Path $Evidence 'build.log')
$PSVersionTable
Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture
Get-AppxPackage '*WindowsTerminal*' | Select-Object Name, Version
Invoke-Checked rustc @('-vV')
Invoke-Checked cargo @('--version')
Invoke-Checked rustup @('show', 'active-toolchain')
$NativeTarget = ((rustc -vV | Select-String '^host: ').Line -replace '^host: ', '').Trim()
if ($NativeTarget -notmatch '-pc-windows-(msvc|gnu)$') { throw 'Select an installed native Windows toolchain.' }
$Common = @('--offline', '--locked', '--manifest-path', 'companion/Cargo.toml', '--target', $NativeTarget, '--target-dir', $BuildDir)
Invoke-Checked cargo @('fmt', '--manifest-path', 'companion/Cargo.toml', '--check')
Invoke-Checked cargo (@('test') + $Common + @('--', '--test-threads=4'))
Invoke-Checked cargo (@('clippy') + $Common + @('--all-targets', '--', '-D', 'warnings'))
Invoke-Checked cargo (@('build') + $Common + @('--release'))
$Candidate = Join-Path $BuildDir "$NativeTarget\release\flere-connect.exe"
Invoke-Checked $Candidate @('--build-info')
Get-FileHash -LiteralPath $Candidate -Algorithm SHA256
Invoke-Checked git @('status', '--short')
Stop-Transcript
```

Keep build logs, actual test counts and failures. Several tests are Unix-only;
their absence on Windows is **not** a Windows pass. If a fixture alone fails to
start under load, retain the failure and rerun that exact test sequentially; do
not weaken assertions. Record terminal profile, font, font size, zoom, scaling
and actual paste bindings separately. Sixel plus cell-geometry replies are
required for companion graphics; record a text fallback honestly when absent.

## 3. Package and install the native stable launcher

This installs only the companion under the current user's `LOCALAPPDATA` and
starts no SSH connection. Inspect an existing installation first. A foreign,
linked or busy manual executable must not be forcibly replaced. Only add
`--adopt` after the owner explicitly selects that recognized manual installation;
detach its old manual companion first if Windows denies replacement.

```powershell
$PackageA = Join-Path $Evidence 'package-release'
Invoke-Checked $Candidate @('package', $Candidate, $PackageA)
Invoke-Checked $Candidate @('update-status')
Invoke-Checked $Candidate @('install', $PackageA)
$Launcher = Join-Path $env:LOCALAPPDATA 'Flere\bin\flere.exe'
$ReceiptPath = Join-Path $env:LOCALAPPDATA 'Flere\install\flere-connect.json'
Copy-Item -LiteralPath $ReceiptPath -Destination (Join-Path $Evidence 'install-before.json')
Invoke-Checked $Launcher @('update-status')
Get-FileHash -LiteralPath $Launcher -Algorithm SHA256
Get-Acl -LiteralPath (Split-Path $ReceiptPath) | Format-List
$Installed = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
Invoke-Checked $Installed.current.executable @('--build-info')
```

Use `$Launcher` explicitly; do not edit PATH or invoke a different shadowing
command. The stable launcher waits for an immutable worker. Its own
`--build-info` may remain older after an update: record **launcher hash, current
receipt, worker executable/build and running process path separately**. Matching
version/build stamps alone are insufficient. Local packaging supports the native
MSVC or GNU target; `scripts/install-companion.ps1` accepts either native x86_64
manifest. Both `flere.exe` and `flere-connect.exe` must be installed stable
launchers for the same worker. Verify both commands and matching launcher hashes.
Do not claim that public HTTPS download was tested by local package installation.

## 4. Select the exact remote target and capture identities

The new name is a clean installation: old Railhand state/commands are not used.
In an owner-provisioned disposable remote home, also test `& $Launcher ssh $SshAlias`
(or `--package LOCAL_VERIFIED_LINUX_PACKAGE` before public release). It should
reuse native OpenSSH trust, identify the remote OS/architecture, install only if
absent, start a supervisor with no native chats, then attach. Disconnect and repeat:
no implicit update, adoption or duplicate supervisor. A public channel that is
not yet published must report that failure, not reuse an old Railhand payload.
Keep this isolated bootstrap test separate from the selected existing chat below.


The supervisor must already be running. Have the owner select an existing state;
connecting is not authority to start chats or restore unknown conversations.
Use a **separate, owner-provisioned disposable remote state with ordinary shells**
for update, rollback, cancelled transfer, overwrite, and port tests. If it is not
available, record those cases blocked instead of repurposing a production state.

```powershell
$SshAlias = Read-Host 'Owner-selected existing SSH alias'
$RemoteExe = Read-Host 'Owner-selected absolute remote Flere executable'
$RemoteState = Read-Host 'Owner-selected absolute remote state'
$SshExe = (Get-Command ssh -CommandType Application -ErrorAction Stop).Source
$FlereProfile = 'windows-acceptance-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
function Quote-Posix([string]$Value) {
    if (-not $Value -or $Value -match '[\x00-\x1f\x7f]') { throw 'Invalid remote argument.' }
    return "'" + $Value.Replace("'", "'\''") + "'"
}
function Save-RemoteJson([string]$Action, [string]$Label) {
    if ($Action -notin @('build-status', 'list', 'install-status')) { throw 'Read-only commands only.' }
    $RemoteCommand = 'exec ' + (Quote-Posix $RemoteExe) + ' --state ' + (Quote-Posix $RemoteState) + ' ' + $Action
    $Text = (& $SshExe -T -- $SshAlias $RemoteCommand) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw 'Remote observation failed; do not substitute another target.' }
    $Text | ConvertFrom-Json | Out-Null
    [IO.File]::WriteAllText((Join-Path $Evidence ($Label + '.json')), $Text, [Text.UTF8Encoding]::new($false))
}
Save-RemoteJson 'build-status' 'remote-build-before'
Save-RemoteJson 'list' 'remote-sessions-before'
Invoke-Checked $Launcher @('connections', 'save', $FlereProfile, $SshAlias, '--remote', $RemoteExe, '--state', $RemoteState, '--ssh', $SshExe)
Invoke-Checked $Launcher @('--connection', $FlereProfile)
# After detaching with Ctrl+Space, q:
Save-RemoteJson 'build-status' 'remote-build-after'
Save-RemoteJson 'list' 'remote-sessions-after'
```

For each detach/reconnect/update compare supervisor epoch/PID and every original
workspace/tab ID, run token, PID, order and selection. Record split membership,
focused pane and unsent drafts visually; `list` does not expose the complete
split layout. Never use `send`, `--image`, Enter, or clipboard automation to
probe an unselected native chat. Keep transcripts closed during chat interaction;
evidence needs identities and observations, not chat text or clipboard contents.

## 5. Physical terminal acceptance

Record each case separately as pass/fail/blocked, following [acceptance.md](acceptance.md).

- **Both paste gestures:** only in the owner's selected existing chat, with an
  approved harmless draft already unsubmitted, copy a known image. Test Ctrl+V
  and the terminal's configured paste shortcut separately. Expect exactly one
  attachment in that conversation, intact draft and **no Enter/submission**.
  Then test plain text. Cancel a separate transfer/change focus in the disposable
  state; nothing may arrive in the newly selected target. Do not activate hooks,
  change native trust, launch a harness or send a model request for this test.
- **Local file drops, when `chat-drop-v1` is available:** use a harmless local
  PNG, JPEG and text/PDF fixture, including a filename with spaces and Unicode.
  Drop each file into the selected existing chat. The local prompt must identify
  the intended chat and offer Attach, Paste as text and Cancel. Before Attach,
  there must be no local file read or transfer. Attach must retain exact bytes
  privately on the remote host and insert one completed path into the same draft
  without Enter. Native image/file presentation depends on the file type and
  harness; record what actually appears instead of counting every path as an
  image badge. Check Cancel, literal text choice, queued input, repeated drops,
  target-away-and-back, disconnect and refresh in the permitted fixtures. Paths
  pasted into ordinary shells/editors and path-entry forms must remain text.
  On older builds without this capability, record the feature as unavailable;
  clipboard-image success does not establish generic file-drop acceptance.
- **PNG/JPEG and badges:** use owner-provided harmless fixtures. Open from Files
  (Ctrl+Space, l; Enter/p) and visible terminal paths; navigate the gallery.
  Check portrait/wide/transparent images, fallback initials, fixed badge slots,
  selected/busy/attention cards, wide/narrow windows and font/zoom changes. Close
  independently with Esc, q and Ctrl+Space; menus, hover cards, workspace changes,
  detach/reconnect and the idle intro must clear old graphics without child input.
- **Screenshots:** Ctrl+Space, s copies Flere's composed frame. Check the local
  retained PNG/path and clipboard image. Paste it only into a scratch image app
  or the separately approved existing draft; no automatic submission. No desktop
  screen recording is required.
- **Flere screensaver:** select **Mascot: Flere (screensaver + pet)** in Actions;
  it immediately opens the original animated Flere intro. Check the small
  drifting white disc, moving rings and yellow-green fields beneath the wordmark
  at wide and narrow sizes. A gentle click may produce a warm glow or
  white irritation flecks; dragging must move/throw it. Hover and clicks keep
  the saver open. A key or paste wakes it without changing the underlying draft.
  Check reduced motion, the Duck choice and Flere in the enabled sidebar pet.
  This uses terminal cells; success does not depend on Sixel or a desktop overlay.
- **Arcade:** Actions → Arcade: Context Ruins (`Ctrl+Space, &`), choose each avatar,
  move with arrows/h/l, jump with Space, climb with Up/Down/k/j, collect a key and
  open its door, then pause and exit. Check the 40×20 camera, resize and focus
  loss pause, full-game screenshots, and that queued paste/Enter never reaches
  the underlying shell or approved draft.
  Hold Left/Right or h/l while tapping Space: the jump must retain horizontal
  movement. On a terminal reporting keyboard release events, releasing the
  direction must stop it; record the actual terminal and negotiated capability.
  Legacy terminals retain launch direction until landing. Pause/resume and a
  quick close/reopen must not revive stale held keys. After leaving Arcade,
  opening a local companion prompt, or disconnecting, ordinary typing must work
  and game key reports must not change the underlying draft.
- **Reconnect:** Ctrl+Space, q, then `& $Launcher reconnect $FlereProfile`. Verify exact
  remote identities/drafts, one foreground worker, restored console modes and
  no competing parent shell input. Saved profiles must contain only connection
  arguments, never a one-shot image, draft, password or queued keystrokes.
- **Files/Ports, disposable state only:** NAV 6/7/8/9 opens upload/download/local
  open/Ports. Local forms must require local choices; cancel preserves files.
  Check exact harmless file content, conflict/cancellation cleanup, loopback-only
  forwarding and Stop affecting only the owned SSH child. Never expose a service
  publicly, open untrusted executables, overwrite live files or kill unrelated SSH.

## 6. Windows update and rollback, disposable remote state only

Use two validated **same-host-target** companion packages. A second offline debug
build of this same commit is a practical distinct test payload; label it debug,
not a second release. Keep both manifests and SHA-256 values.

```powershell
Invoke-Checked cargo (@('build') + $Common)
$DebugCandidate = Join-Path $BuildDir "$NativeTarget\debug\flere-connect.exe"
$PackageB = Join-Path $Evidence 'package-debug-update-fixture'
Invoke-Checked $Candidate @('package', $DebugCandidate, $PackageB)
```

Attach through the **stable launcher**. Press Ctrl+Space, K. The owner supplies
an exact prepared remote-core package (or explicitly approved source); choose
`$PackageB` locally. Review both identities, then choose Update. Preparation must
not change installed commands. Successful apply must retain the stable launcher,
replace the immutable worker through its foreground wait, reconnect the exact
saved target, apply/acknowledge the remote supervisor and initiating UI, and
preserve every original PID/run/draft/split. A mismatched component or unavailable
ACK must report partial completion. No second SSH login is needed by this flow.

Capture the receipts and actual running worker path from another ordinary
PowerShell window (`Get-CimInstance Win32_Process` filtered to
`flere-connect.exe` gives PID, parent PID, executable path and argv). Verify
the parent's console does not accept input while its worker runs. Test argv paths
with spaces and `%!&^` in the disposable fixture; no `cmd.exe` reinterpretation.

After detaching, the companion-only rollback/reconnect command is:

```powershell
Invoke-Checked $Launcher @('update', '--rollback', '--reconnect', $FlereProfile)
# After the reconnect test is detached:
Invoke-Checked $Launcher @('update-status')
Copy-Item -LiteralPath $ReceiptPath -Destination (Join-Path $Evidence 'install-after-rollback.json')
Save-RemoteJson 'build-status' 'remote-build-after-rollback'
Save-RemoteJson 'list' 'remote-sessions-after-rollback'
```

This CLI performs a read-only SSH compatibility probe before installation; if
the saved authentication cannot work in that probe, report the limitation rather
than changing credentials or claiming success. It rolls back the companion,
not the remote core. Never force a rejected handoff/protocol downgrade. Test
corrupt/wrong-target packages and a changed installation during Review only in
the disposable fixture: they must fail without replacing the newer installation
or falsely acknowledging the old plan. Leave the owner-selected final package
installed and report which one; do not silently end on the debug fixture.

## 7. Return evidence, not an unsupported completion claim

Write `acceptance-result.json` under `$Evidence`, with paths to logs/receipts and
cropped terminal images when useful. Do not commit personal logs, credentials,
chat contents or clipboard payloads. Report source issues separately from missing
tools, unsupported terminal capabilities, blocked human checks and physical
Windows failures. Fix localized Windows defects on `main` when needed, preserve
existing changes, and repeat the affected checks before reporting the patch.
No commits/pushes/publication are part of this Windows procedure.

```json
{
  "schema_version": 1,
  "commit": "FULL_REVIEWED_SOURCE_COMMIT",
  "platform": {"windows_build": "", "architecture": "", "powershell": "", "rustc": "", "cargo": "", "host_target": "", "compiler_linker": "", "openssh": ""},
  "terminal": {"name_version": "", "profile": "", "font_size_scaling": "", "paste_bindings": [], "sixel": "pass|fail|blocked", "cell_geometry": "pass|fail|blocked"},
  "checks": [{"name": "", "command_or_gesture": "", "result": "pass|fail|blocked|not_applicable", "expected": "", "observed": "", "evidence": []}],
  "components": {"launcher_sha256": "", "worker_executable": "", "worker_build": {}, "package_sha256": "", "install_attempt": "", "remote_before": "remote-build-before.json", "remote_after": "remote-build-after.json"},
  "preservation": {"epoch_pid_same": null, "tab_run_pid_order_same": null, "split_selection_same": null, "approved_draft_retained": null, "no_enter_or_submission": null},
  "remaining_limits": [],
  "final_installed_package": ""
}
```
