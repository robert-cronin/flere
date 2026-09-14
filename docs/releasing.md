[Documentation](README.md) · [Distribution](distribution.md)

# Release process

The selected trigger is the **Release button**, with an explicit unused version
and full reviewed `main` commit. The
[workflow](../.github/workflows/release.yml) runs only on `workflow_dispatch` from
`main`; ordinary pushes, pull requests and version tags do not publish releases.
Both Cargo manifests must already declare the requested `X.Y.Z`. Existing tags,
drafts and published versions are rejected by initial selection. In particular,
the existing v0.3.0 assets must not be rebuilt, relabeled or overwritten.

## Current automation scope

The first implementation builds the Linux x86-64 GNU core and companion on native
Ubuntu 24.04, using Rust 1.98.0. Compiler components and locked dependencies are
provisioned explicitly before offline formatting, strict Clippy, tests and release
builds. It records the actual runner image, compiler, linker, source fingerprint
and final payload digests. There is no shared dependency/build cache or restore
from a pull-request run.

The final executables run `--build-info` and undergo managed installation and an
exact-payload reinstall in a disposable home-cache directory. ELF library and
symbol checks enforce the recorded system-library set and glibc 2.39 minimum.
These checks establish native CI behavior;
they do not establish interactive desktop behavior, physical clipboard/SSH
acceptance, upgrades from another release or package-manager installation tests.
The temporary installation is removed with its disposable directory.

Only seven allowlisted files can be published:

- `flere-x86_64-unknown-linux-gnu`
- `flere-x86_64-unknown-linux-gnu.manifest.json`
- `flere-connect-x86_64-unknown-linux-gnu`
- `flere-connect-x86_64-unknown-linux-gnu.manifest.json`
- `flere-X.Y.Z-source.tar.gz`, containing the complete reviewed public source tree
- `release.json`, recording source/run/workflow identity, hashes and acceptance
- `SHA256SUMS`, including the digest of `release.json`

The source archive contains exactly the selected commit's regular files under
`flere-X.Y.Z/`, including both components and lockfiles, shared build support,
public documentation/assets, the MIT license and both font licenses. Git metadata,
untracked files and private build/review output are excluded. Files are sorted;
Git executable modes are preserved, owner metadata is cleared, tar timestamps use
the selected commit time, and gzip has no original filename or timestamp.

The [source helper](../scripts/release-source.py) checks all archive members,
package identities, required shared inputs and licenses. It rejects links,
duplicate/unsafe paths, extra metadata and trailing data, and bounds compressed
size, expanded size and file count. The complete member inventory reproduces the
current `flere-source-v1` fingerprint used by both binary manifests: each sorted
path, Unix file mode and content digest is bound together. This fingerprint is
separate from the archive's SHA-256. The publisher repeats validation as data;
it never extracts or executes candidate source with publication credentials.
Content review of the selected public commit and final files remains necessary.
Including source enables subsequent channel preparation; it does not establish
Homebrew, Cargo, Debian/AUR or Nix lifecycle acceptance or advance their versions.

The upload is a new immutable Actions artifact from this exact run. Its numeric
artifact ID and separately recorded release-descriptor digest travel to the
publisher. The publisher revalidates the complete allowlist, source, version,
target pair, remote-protocol compatibility and every file digest before writing
GitHub. It uses automation code from the trusted workflow revision and never
builds, installs or executes candidate programs with release credentials.

All files go to a draft first. Every uploaded file is fetched and compared before
the completed draft is published. A separate job downloads every exact version URL
without authentication and checks the same hashes. The workflow explicitly leaves
the `latest` designation unchanged, so the existing managed installer feed does
not silently advance to this narrower platform release.

## Enable and run

Before the first dispatch, enable
[immutable releases in the repository](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases).
GitHub freezes assets and the associated tag when an immutable release is
published. Upload every intended file before that step. The workflow never edits
repository settings and needs only `contents: write` in its publication job; the
remaining jobs use repository read access. Restrict the `release` environment to
`main` and configure reviewer approval if required by repository policy.

The immutable-settings API requires administration read access, which is not
granted to the release token. The workflow checks `immutable: true` in the
publication response, so settings must be verified beforehand. If the feature is
disabled, the workflow reports failure **after publication** and stops; it does
not delete the release or replace files. Append-only upload rules and digest
verification apply regardless of the repository setting. This is a known
activation limitation, not a pre-publication guarantee about GitHub settings.

Choose **Actions → Release → Run workflow**, select `main`, and enter the unused
`X.Y.Z` and its complete lowercase 40-character commit SHA. The source commit and
workflow revision must both be ancestors of the fetched `main`. The run summary
shows the sealed descriptor digest and target/channel readiness. Dispatch requests
publication of the currently supported Linux assets once these checks pass; it
is not an ordinary build button. No fresh release was dispatched when adding this
automation.

The workflow's actions are pinned to exact official implementation commits. The
runner label is `ubuntu-24.04`, whose image can change; the receipt records the
actual image version. The pipeline has not yet been exercised on a hosted release
run. Local fixture tests cover its validation and promotion behavior without
network writes or native chat launches.

## Retry and recovery

Rerun failed jobs to retain the original successful build job, artifact ID and
descriptor digest. A partially uploaded matching draft resumes by adding only
missing files. Existing assets must already match; foreign assets, another run's
draft, a changed tag or a different checksum stop promotion. Nothing is deleted
or overwritten to make a retry fit. An ambiguous response after publication is
reconciled by verifying the same immutable release and tag, without republishing.

Starting the entire workflow again would rebuild the executable generation stamp
and create a new candidate. Initial selection therefore rejects a version whose
draft/tag already exists. Retain the original run; artifacts expire after 30
days. If those artifacts are lost or a published executable needs correction,
use a new reviewed version. Do not remove a version tag or published release to
work around this guard.

A failed anonymous download check leaves the release intact. No channel updater
runs in this workflow, so no delayed run can reset a newer package-manager version.
Future channel jobs must consume these same immutable bytes and serialize their
own monotonic updates. Keep their mutable delivery/acceptance status outside the
frozen release descriptor.

## Blocked targets and channels

| Target/channel | Required work before automation can publish it |
| --- | --- |
| macOS arm64 prebuilt | Developer ID Application identity, accepted notarization and ordinary quarantined download/Gatekeeper acceptance. No unsigned fallback. |
| macOS x86-64 prebuilt | Native Intel acceptance plus the same signing/notarization requirements. Cross-compilation alone does not qualify. |
| Windows x86-64 companion | Physical clipboard, SSH, draft, image, resize, held-control and cleanup acceptance tied to final payload hashes. No Windows core claim. |
| Homebrew source formulas | Exact source archive, real formula install/test/removal and narrowly scoped tap writer. Existing source formulas remain the current strategy. |
| crates.io | First publication, publisher ownership setup, then Trusted Publishing enrollment. |
| Debian/AUR | New reviewed release lock and real package-manager install/update/removal validation. The checked-in lock still pins v0.3.0. AUR also needs account/credential setup. |
| Scoop/WinGet | Physical Windows acceptance, native validators/install tests and catalogue publishing authority. Submission and acceptance remain separate statuses. |
| Managed latest feed | Target-specific public-download/runtime acceptance and a reviewed pointer update. This workflow keeps the existing latest release. |

These exclusions appear in the run summary and `release.json`; they are not
silently counted as successful targets. RPM, Nix and Chocolatey remain outside
the supported channel set.

## macOS signing and notarization setup

Apple Developer Program membership provides Developer ID and notarization.
The account holder creates a **Developer ID Application** certificate and securely
configures its private key and notary authentication in a dedicated signing
environment. Credentials must never enter the repository, release archive, build
logs or chat. Homebrew source builds and Linux packages do not require Apple
membership. [Developer ID setup](https://developer.apple.com/help/account/certificates/create-developer-id-certificates).

A future isolated signing job must consume the selected binaries and pre-signing
digests without compiling or executing repository code while Apple credentials
are present. Sign both executables with a secure timestamp and hardened runtime,
verify signatures, submit their ZIP with `xcrun notarytool`, and require an
`Accepted` result. A timeout remains pending. Remove temporary keychains and
credentials before runtime testing. Signing changes bytes: regenerate payload
manifests and every wrapper/catalogue checksum afterward.

Bare command-line executables and ZIP archives cannot be stapled. A future
stapled DMG/PKG channel requires its own packaging and acceptance work. Verify the
actual downloaded CLI with ordinary quarantine and Gatekeeper enabled; a launch
on the signing host is insufficient.
[Apple notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).

Developer ID enrollment, signing credentials, physical acceptance receipts and
first registry/catalogue onboarding remain external prerequisites. This workflow
does not provision them or claim the Mac/Windows pipeline is implemented.
