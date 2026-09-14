# Windows distribution preparation

The native Windows package is the OpenSSH/clipboard **companion**. It connects
to a Linux or macOS workbench. Both `flere.exe` and `flere-connect.exe` launch
that companion; this does not claim a native Windows core.

No Windows package-manager channel is published yet. Physical Windows acceptance
is tracked in [the handoff](../../docs/windows-acceptance-handoff.md).

## Private hosted candidate

The manual **Windows companion candidate** workflow prepares the unpublished
v0.3.4 companion from exact public source
`3401a77193821b2c33127d2001239fb983a3712b` on a disposable Windows Server 2025
AMD64 runner. It compares actual checkout bytes with Git blobs, installs Rust
1.98.0 MSVC, fetches locked dependencies, then runs offline formatting, strict
Clippy, companion tests and the release build. It packages those same bytes with
the source receipt and existing generators, checks the ZIP inventory, and runs
six stateless checks across both executable aliases. No Windows core is built.

The retained `windows-candidate-RUN-ATTEMPT` artifact contains `candidate.json`,
source proof, command logs and `windows/` with the ZIP and proposed catalogue
manifests. Only `status: prepared_not_published` is a successful candidate;
`winget.status` separately records real manifest validation or an unavailable-tool
blocker. WinGet validation failure fails the run. Hosted checks do not establish
physical clipboard, terminal graphics/held controls, existing-chat drafts,
external SSH, or package-manager install/upgrade/remove and ownership acceptance.
No release, catalogue entry, PATH change or self-install is performed.

When Chocolatey is available, the same job runs `choco pack` and verifies that
the `.nupkg` contains only NuGet metadata, the spec and the exact generated
installation script. It retains that package and records its inventory/hash;
this does not run the script or install its payload. Missing WinGet/Chocolatey
tools are recorded for that run. They can be provisioned on a later disposable
runner after review; they are not a requirement for a physical user machine.

After downloading a successful artifact from the reviewed Actions run, extract
it into a fresh private directory and verify the inner ZIP's SHA-256 against
`candidate.json`. To try its portable companion, use a new directory (outside
any existing managed installation):

```powershell
$Receipt = Get-Content -LiteralPath '.\candidate.json' -Raw | ConvertFrom-Json
if ($Receipt.status -ne 'prepared_not_published') { throw 'Candidate validation did not pass.' }
$Zip = Join-Path '.\windows' $Receipt.zip.name
if ((Get-FileHash -LiteralPath $Zip -Algorithm SHA256).Hash -ne $Receipt.zip.sha256) { throw 'ZIP differs.' }
$Portable = Join-Path $env:LOCALAPPDATA ('Flere\candidates\' + [guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $Portable) { throw 'Use a new candidate directory.' }
Expand-Archive -LiteralPath $Zip -DestinationPath $Portable
& (Join-Path $Portable 'flere.exe') --build-info
# When ready, use your existing OpenSSH alias; "dev" is an example.
& (Join-Path $Portable 'flere.exe') ssh dev
```

Both command names run the same companion. This portable candidate does not make
`winget install` available: the generated release URL is still a proposed location.
Keep physical acceptance tied to this executable digest. Publication and a normal
WinGet submission remain separate; no extra assets can be appended to an immutable
older release.

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
and a preparation receipt. Their immutable
GitHub URLs are proposed publication locations, not evidence of availability.
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

The generator prepares recipe sources, **not a tested `.nupkg`**. Offline tests
use a synthetic PE header that is never executed. Real `choco pack`, native
install/upgrade/remove, shim ownership and state preservation still require an
accepted Windows payload and a disposable Windows environment. A generated
URL/hash does not establish that its ZIP has been published.

Before publishing a package-manager channel, on Windows:

1. Complete physical clipboard, SSH, draft-retention, image/resize/cleanup and
   held-control acceptance against the exact payload digest.
2. Run `winget validate` on the generated version directory. Test install,
   command aliases, upgrade and removal in a disposable Windows user/profile.
3. Test the generated Scoop manifest and both aliases in an isolated Scoop setup.
4. Run `choco pack .\flere-connect.nuspec` from the generated Chocolatey recipe
   directory. Inspect the `.nupkg` inventory: NuGet metadata, the spec and only
   `tools/chocolateyInstall.ps1`; no embedded executable or local files.
5. Publish the ZIP only after its contents/privacy review; fetch it anonymously
   and verify its checksum before making catalogue manifests available.
   In an isolated Chocolatey setup, test local-source install, both command
   aliases, upgrade to a second verified version and uninstall. Check that a
   corrupt ZIP is rejected and synthetic user state and SSH configuration stay
   intact. These tests need the exact recipe download to exist.
6. Publish the Scoop bucket and submit the WinGet and Chocolatey packages through
   their normal contribution processes. Keep pending/accepted status explicit.

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
