# Windows distribution preparation

The native Windows package is the OpenSSH/clipboard **companion**. It connects
to a Linux or macOS workbench. Both `flere.exe` and `flere-connect.exe` launch
that companion; this does not claim a native Windows core.

No Windows package-manager channel is published yet. Physical Windows acceptance
is tracked in [the handoff](../../docs/windows-acceptance-handoff.md).

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
