# Windows companion downloads

The native Windows package is the OpenSSH/clipboard **companion**. It connects
to a Linux or macOS workbench. Both `flere.exe` and `flere-connect.exe` launch
that companion; this does not claim a native Windows core.

The [v0.3.6 Windows ZIP](https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere-connect-0.3.6-x86_64-pc-windows-msvc.zip)
and [Flere's Scoop bucket](https://github.com/robert-cronin/scoop-flere) are
published. WinGet and Chocolatey catalogue publication remains pending.
Physical Windows acceptance is tracked in
[the handoff](../../docs/windows-acceptance-handoff.md).

## Install with Scoop

With [Scoop](https://scoop.sh/) installed on Windows x86_64, run in PowerShell:

```powershell
scoop bucket add flere https://github.com/robert-cronin/scoop-flere
scoop install flere/flere
flere ssh dev
```

Replace `dev` with your existing OpenSSH hostname or configured alias. Both
command names launch the companion. The published bucket at
[commit `982c8dc`](https://github.com/robert-cronin/scoop-flere/commit/982c8dcf8666f7ae2e5cbd9d1e0c58a890e1407a)
contains the exact native-generated v0.3.6 recipe and verified ZIP checksum.
This is Flere's own bucket, not a Scoop Main/Extras submission.

Detach the companion before upgrading or removing it. Use `scoop update`, then
`scoop update flere` for upgrades, or `scoop uninstall flere` for removal. Scoop
owns both command aliases; SSH configuration and workbench state stay separate.
The earlier owned-Git-bucket lifecycle and UI evidence below remains tied to its
candidate; a native install from this public bucket has not yet been exercised.

## Install the public Windows ZIP

Use Windows x86_64, an installed OpenSSH client and your existing SSH configuration.
In PowerShell, download into a fresh user directory, verify the exact published
checksum, then extract:

```powershell
$Download = Join-Path $env:LOCALAPPDATA ('Flere\downloads\' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Download | Out-Null
$Zip = Join-Path $Download 'flere-connect-0.3.6-x86_64-pc-windows-msvc.zip'
Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere-connect-0.3.6-x86_64-pc-windows-msvc.zip' -OutFile $Zip
if ((Get-FileHash -LiteralPath $Zip -Algorithm SHA256).Hash -ne 'e1d4a75467a30e55452fa85be6f56ab2aab349517771449c07a448067043ce0a') { throw 'ZIP checksum differs.' }
$Portable = Join-Path $Download 'app'
Expand-Archive -LiteralPath $Zip -DestinationPath $Portable
$env:Path = "$Portable;$env:Path"  # This PowerShell session only.
flere --build-info
flere ssh dev
```

Replace `dev` with your SSH hostname or configured alias. `flere-connect ssh dev`
is equivalent. The ZIP is 1,856,627 bytes and contains only `flere.exe`,
`flere-connect.exe`, `manifest.json` and `LICENSE`. Both executable aliases have
SHA-256 `19346b7b524bba24f224c253b90ea30c78563d581959671b88e93389ff5bad4e`.
No Rust build, WSL or package-manager catalogue is required. Add the extracted
folder to your user PATH separately if desired; these commands change only the
current terminal's PATH. Keep the folder outside an existing managed installation
and detach before replacing or removing it.

## Install the Windows companion from source

To build a newer source revision, on Windows x86_64 install Rust 1.98+ using
<https://rustup.rs> and its Visual Studio C++ prerequisites, then reopen
PowerShell. The native MSVC toolchain is the normal choice. An installed OpenSSH
client and your existing SSH configuration provide the connection.

```powershell
cargo install --git https://github.com/robert-cronin/flere --locked flere-connect
flere-connect ssh dev
```

Replace `dev` with your SSH hostname or configured alias. Cargo adds
`flere-connect.exe` to its user bin directory; this source-install route does not
create the `flere.exe` alias. It follows public `main`, so record `--build-info`
when reporting a problem. For acceptance tied to a reviewed commit, add
`--rev <FULL_REVIEWED_SOURCE_COMMIT>` to `cargo install`.

This is a native Windows client for a Linux/macOS workbench, not a local Windows
workbench. It does not require WinGet, Scoop or WSL. The public portable ZIP
above provides both command names.

## Release and candidate evidence

The public v0.3.6 ZIP was built from exact source `8729733` in
[run 34953872776](https://github.com/robert-cronin/flere/actions/runs/34953872776).
Native MSVC formatting, strict Clippy, 85 tests, release packaging, six portable
alias checks, WinGet validation and Chocolatey packing passed. One physical
clipboard test was explicitly ignored. All 11 release downloads, including this
ZIP and its source/manifest, were independently verified without authentication.
The earlier public v0.3.5 ZIP at `8744d35` passed 81 native tests in
[run 34941117734](https://github.com/robert-cronin/flere/actions/runs/34941117734),
with its separate physical clipboard test ignored.
The earlier public v0.3.4 ZIP at `32108e3` passed 74 native tests in
[run 34930826557](https://github.com/robert-cronin/flere/actions/runs/34930826557);
its separate physical clipboard test was also ignored.

The manual **Windows companion candidate** workflow separately accepts an explicit
reviewed source commit/version and never publishes it. It verifies Git blob/mode
identity, runs native MSVC checks and retains `candidate.json`, source proof,
bounded logs, ZIP and proposed recipes in `windows-candidate-RUN-ATTEMPT`.
Only `prepared_not_published` with passing native/recipe checks is a successful
candidate. This job does not install its package, create manager aliases or prove
physical behavior.

Historical source `422058c` passed its build and six aliases in
[run 34911230794](https://github.com/robert-cronin/flere/actions/runs/34911230794),
then failed on WinGet warnings. The unchanged ZIP's corrected recipes passed
[run 34912671194](https://github.com/robert-cronin/flere/actions/runs/34912671194).
Later `21cf68c` candidate checks and separate Chocolatey ownership acceptance
remain tied to those exact bytes. The corrected `9d53f96` candidate passed native checks in
[run 34935547435](https://github.com/robert-cronin/flere/actions/runs/34935547435)
and both installed WinGet owner reports, normal removal and initialized-profile
preservation in [run 34936386953](https://github.com/robert-cronin/flere/actions/runs/34936386953).
**It is not the public v0.3.4 ZIP.**
Its release-shaped recipe URL must not be used to substitute the different public
ZIP for candidate testing.

Actions artifacts are temporary evidence, not published package-manager channels.
Physical clipboard, terminal graphics, existing-chat drafts and external SSH
remain separate acceptance work. Arcade held controls remain deferred. Both Windows
command names run the companion; no native Windows core is supplied.

## Prepare package-manager recipes

After a matching Windows release package has been built and verified, run:

```sh
python3 scripts/windows-manifests.py "$RELEASE_ASSETS" "$PRIVATE_OUTPUT" \
  --target x86_64-pc-windows-msvc
```

The GNU target is also supported explicitly. Do not combine targets or substitute
an untested toolchain merely to obtain a package. The generator checks the payload
size, SHA-256, release source provenance and PE architecture without executing it.
It creates a deterministic ZIP containing only the two command names, the original
manifest and the MIT license. Extra files in the input directory are excluded.

Generated outputs include a Scoop bucket manifest, three WinGet manifests for
`RobertCronin.FlereConnect`, a Chocolatey recipe for `flere-connect`, checksums
and a preparation receipt. Public v0.3.6 uses the verified immutable ZIP above;
a newly generated recipe for an unpublished version is not evidence that its URL
exists or that a catalogue has accepted it.
Neither generator nor manifests invokes the managed self-installer, modifies SSH
configuration, launches sessions or requests automatic native trust approval.

## Chocolatey recipe

`chocolatey/flere-connect/` contains `flere-connect.nuspec` and the UTF-8-BOM
`tools/chocolateyInstall.ps1`. The spec includes only that explicit script path;
it cannot sweep nearby logs or binaries into the package. The script uses the
same immutable release ZIP URL and SHA-256 as Scoop/WinGet, requires AMD64
(including 32-bit PowerShell on AMD64 via the native WOW64 architecture), rejects
forced x86, and extracts through `Install-ChocolateyZipPackage` into the
Chocolatey package's `tools/app` directory. Both commands run the companion.

Chocolatey creates the command shims and removes the package-owned files and
shims on uninstall. There is no custom uninstaller because the payload stays
inside the package directory. User state, SSH configuration and
remote sessions are not removed. Detach the companion before upgrading or
removing it; the recipe does not kill a process to replace a locked executable.
Use `choco upgrade flere-connect` and `choco uninstall flere-connect` once that
channel is available. Do not combine channels that provide the same aliases.

The generator prepares recipe sources; native `choco pack` and exact `.nupkg`
inventory checks passed in the public v0.3.6 producer. Separate retained-candidate
runs passed normal Chocolatey install/remove and both shims. Run `34930361884`
also verified actual installed ownership diagnostics on `21cf68c` and preserved
an explicitly initialized synthetic PowerShell/Chocolatey profile. Cold-profile
no-write behavior is not claimed. WinGet run `34929525551` passed normal
local-manifest install/remove and both aliases on `422058c`; corrected installed
ownership passed on `9d53f96` as recorded above. Scoop run `34935706908` passed
public-ZIP install, both aliases, the release-manifest preservation hook and normal
Flere removal/preservation. Its overall result remains failed because extra
Scoop self-removal timed out. Subsequent fixtures retain Scoop for disposable VM
teardown. Candidate `7e43fa1` (0.3.5) passed 81 native tests and six
aliases in [run 34938451094](https://github.com/robert-cronin/flere/actions/runs/34938451094),
with one physical clipboard ignore. Actual public 0.3.4 → candidate 0.3.5
upgrades, both installed owner reports and normal removal/preservation passed
for [Scoop](https://github.com/robert-cronin/flere/actions/runs/34939641214)
and [Chocolatey](https://github.com/robert-cronin/flere/actions/runs/34939646345).
These used local recipes and checksummed runner-local target downloads, not
catalogue installations. Earlier WinGet attempts remain failed: a local-manifest
upgrade requested unrelated Store terms, and adding `--source` was rejected as
incompatible with `--manifest`. Both attempts removed Flere normally. The corrected
[WinGet run 34943275275](https://github.com/robert-cronin/flere/actions/runs/34943275275)
passed normal public 0.3.4 → candidate 0.3.5 local-manifest upgrade, both installed
owner reports, normal removal and initialized-profile/PATH/package preservation.
The target manifest used the exact observed baseline ProductCode and a checksummed
runner-local ZIP URL. The disposable VM removed the Store source before recording
its community-only source baseline and retained that setup until VM teardown.
Default two-source and catalogue upgrades remain outside that proof. Published
v0.3.5 supports the local-manifest owner routes above. Later source `4f3b693`
adds ordinary default-user Scoop bucket and official WinGet community ownership
recognition; all eight macOS checks and 549 tests passed. The native Windows
candidate passed 85 tests (one physical clipboard test ignored). Registered
Scoop bucket [run 34947765377](https://github.com/robert-cronin/flere/actions/runs/34947765377)
passed normal installation, upgrade, both installed-owner reports and removal,
with state and PATH preserved. It used an owned local Git bucket; WinGet
catalogue acceptance remains pending. These source changes are included in public
v0.3.6; the v0.3.5 ZIP lacks them. The manager checks retain their exact candidate
payload scope.
The same `4f3b693` candidate passed local coordinated-update refusal through
both installed Scoop aliases in [run 34953623319](https://github.com/robert-cronin/flere/actions/runs/34953623319)
(workflow `65fc06c`). Each owned native Windows console displayed the complete
Scoop guidance after Enter, then cancelled with the exact request identity.
No update RPC or installer staging was observed. Console modes/code pages,
installed files and synthetic state were preserved; normal process exit,
package/bucket removal and listener cleanup passed. The passive local peer did
not exercise a remote core or actual SSH. Earlier [run 34952666337](https://github.com/robert-cronin/flere/actions/runs/34952666337)
remains failed: a Python console-wrapper error occurred before UI startup;
normal package/bucket cleanup passed.

Physical acceptance, remote-core refusal, WinGet/Chocolatey UI refusal and their
catalogue publication remain open. Scoop's owned public bucket is available.
The public v0.3.6 ZIP is rebuilt from `8729733`; its hashes differ from the
candidates used in those manager checks.

Package-manager publication and physical acceptance are separate tracks. Keep
physical clipboard, SSH, draft-retention and image/resize/cleanup checks open
against their exact payload digest until observed; catalogue availability does
not prove them. The deferred arcade investigation is not a release requirement.

For each package-manager channel:

1. Run `winget validate` on the generated version directory. Test install,
   command aliases, upgrade and removal in a disposable Windows user/profile.
2. Test the generated Scoop manifest and both aliases in an isolated Scoop setup.
3. Run `choco pack .\flere-connect.nuspec` from the generated Chocolatey recipe
   directory. Inspect the `.nupkg` inventory: NuGet metadata, the spec and only
   `tools/chocolateyInstall.ps1`; no embedded executable or local files.
4. Keep recipes bound to an anonymously verified published ZIP. Public v0.3.6
   satisfies that download step; future versions require their own verification.
   Preserve user state and SSH configuration through normal version upgrades.
   Retain the exact recipe/download scope of the candidate upgrade checks above;
   they do not establish catalogue installation or a different download route.
5. Maintain the published owned Scoop bucket, and submit the WinGet and Chocolatey
   packages through their normal contribution processes. Keep pending/accepted
   status explicit.

Use Scoop/WinGet/Chocolatey to upgrade their installations. The in-app updater owns a
separate per-user managed installation and must not be used as the package
manager's upgrade mechanism. Existing OpenSSH authentication and host-key trust
remain under user control.

Formats follow [Scoop manifests](https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests),
[Microsoft's manifest documentation](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest),
and Chocolatey's [package format](https://docs.chocolatey.org/en-us/create/create-packages/),
[ZIP helper](https://docs.chocolatey.org/en-us/create/functions/install-chocolateyzippackage/),
[automatic shims](https://docs.chocolatey.org/en-us/features/shim/),
Microsoft's [WOW64 architecture variables](https://learn.microsoft.com/en-us/windows/win32/winprog64/wow64-implementation-details),
and [package removal](https://docs.chocolatey.org/en-us/choco/commands/uninstall/).
