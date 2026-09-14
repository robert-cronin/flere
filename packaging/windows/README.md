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
`RobertCronin.FlereConnect`, checksums and a preparation receipt. Their immutable
GitHub URLs are proposed publication locations, not evidence of availability.
Neither generator nor manifests invokes the managed self-installer, modifies SSH
configuration, launches sessions or requests automatic native trust approval.

Before publishing, on Windows:

1. Complete physical clipboard, SSH, draft-retention, image/resize/cleanup and
   held-control acceptance against the exact payload digest.
2. Run `winget validate` on the generated version directory. Test install,
   command aliases, upgrade and removal in a disposable Windows user/profile.
3. Test the generated Scoop manifest and both aliases in an isolated Scoop setup.
4. Publish the ZIP only after its contents/privacy review; fetch it anonymously
   and verify its checksum before making catalogue manifests available.
5. Publish the Scoop bucket and submit the WinGet manifests through their normal
   contribution process. Keep each channel's pending/accepted status explicit.

Use Scoop/WinGet to upgrade their installations. The in-app updater owns a
separate per-user managed installation and must not be used as the package
manager's upgrade mechanism. Existing OpenSSH authentication and host-key trust
remain under user control.

Formats follow [Scoop manifests](https://github.com/ScoopInstaller/Scoop/wiki/App-Manifests)
and [Microsoft's manifest documentation](https://learn.microsoft.com/en-us/windows/package-manager/package/manifest).
