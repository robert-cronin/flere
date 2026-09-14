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

The workflow runs both Rust suites serially with `RUST_TEST_THREADS=1`. Each
fixture retains its internal process, thread and PTY concurrency. If candidate
validation fails, the job prints a bounded tail of its validation log with
credential redaction and literal line prefixes. Missing, ambiguous or unsafe logs
leave the original failure intact. Full validation logs are excluded from public
release assets.

The final executables run `--build-info` and undergo managed installation and an
exact-payload reinstall in a disposable home-cache directory. ELF library and
symbol checks enforce the recorded system-library set and glibc 2.39 minimum.
Passing these checks establishes native CI behavior;
it does not establish interactive desktop behavior, physical clipboard/SSH
acceptance, upgrades from another release or package-manager installation tests.
The temporary installation is removed with its disposable directory.

The current workflow uses schema 1 with seven allowlisted files:

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
Homebrew, Debian/AUR or Nix lifecycle acceptance or advance their versions. A separate
core Cargo job follows public release verification, as described below.

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

## Debian wrapper support

Schema 2 adds `flere_X.Y.Z-1_amd64.deb` to the seven files above. The optional
`--with-debian` candidate step builds it from the five already verified base
inputs: both executables, their manifests and the source archive. It uses the
same maintained Debian builder as the standalone packaging command. No enclosing
release-descriptor digest is embedded in the package, avoiding a checksum cycle.

The [Debian inspector](../scripts/release-debian.py) reads the archive as bounded
data without extraction or execution. It verifies exact payload/control files,
licenses, ownership, modes and source timestamps; links, install hooks, extra
members and changed bytes are rejected. Sealing and publication each repeat this
inspection. Schema 2's final checksum list, immutable retry and anonymous download
checks cover all eight files. Historical schema 1 releases retain their exact
seven-file validation and release text.

Workflow activation remains pending native Debian lifecycle acceptance. Adding
a `.deb` to a future release does not submit AUR recipes, create an APT repository
or change the latest-release pointer. Published immutable releases remain unchanged.

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
is not an ordinary build button.

The workflow's actions are pinned to exact official implementation commits. The
runner label is `ubuntu-24.04`, whose image can change; the receipt records the
actual image version. [Run 34837935289](https://github.com/robert-cronin/flere/actions/runs/34837935289)
published immutable v0.3.1 from `7f5c5eb`. Its first attempt completed builds and
publication, then an immediate tag lookup returned HTTP 404. Rerunning only the
failed jobs verified the same tag/assets and passed every anonymous public-download
check; no rebuild, asset replacement or latest-pointer change occurred. Independent
anonymous downloads also matched all seven sealed checksums. Earlier failed runs
remain recorded separately.

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

A failed anonymous download check leaves the release intact and prevents the Cargo
job from starting. The Cargo job publishes only the explicitly selected core
version; it does not change a moving channel pointer. Future package-manager jobs
must consume verified release bytes and serialize their own monotonic updates.
Keep mutable channel acceptance status outside the frozen release descriptor.

## Core Cargo Trusted Publishing

After `verify-public`, the separate `publish-core` job checks the selected clean
source and public registry. If that version already exists, its API/index checksum,
downloaded archive, complete core file inventory, Git commit and normalized Cargo
manifest/lock must match. A matching retry skips Cargo builds and authentication;
a conflicting publication stops without uploading or changing the GitHub release.

For a new version, the job provisions Rust 1.98.0 and locked core dependencies,
then obtains a short-lived token through the pinned official
`rust-lang/crates-io-auth-action`. Normal `cargo publish --locked --registry crates-io -p flere`
builds and verifies the selected source before uploading. The final public archive
must match both Cargo's exact upload and the selected Git source. The companion
is excluded. This dedicated Cargo job can build the reviewed source; the separate
GitHub asset publisher continues to execute no candidate payloads.

Only this job receives `id-token: write`; it uses the existing `release`
environment and the same manual version/commit selection. Ordinary pushes do not
publish. The owner configured the crates.io Trusted Publisher for repository
`robert-cronin/flere`, workflow filename `release.yml`, environment `release`.
No stored Cargo API token or `cargo login` step is needed.

[Run 34843553467](https://github.com/robert-cronin/flere/actions/runs/34843553467)
published v0.3.2 from `1be2dab` using OIDC and normal Cargo verification. Native
Linux checks, immutable GitHub publication and anonymous downloads passed. The
first attempt uploaded the crate, then the local-archive comparison looked in
Cargo's final-package directory instead of its upload scratch directory. The
failed-job retry verified the existing public crate and all 124 source files,
skipping authentication and duplicate publication. An independent offline Cargo
package build reproduced the public archive exactly.

The workflow now fixes both Cargo output directories and compares the archive in
`target/package/tmp-crate`, as used by the pinned
[Cargo 1.98 implementation](https://github.com/rust-lang/cargo/blob/rust-1.98.0/src/cargo/ops/cargo_package/mod.rs).
The correction passed local archive verification and review; a future new-version
upload will exercise that corrected hosted path. The successful retry exercised
registry reconciliation. Offline fixtures cover missing, delayed, conflicting
and matching registry data. No owner API token is needed for future releases.

## Blocked targets and channels

| Target/channel | Required work before automation can publish it |
| --- | --- |
| macOS arm64 prebuilt | Developer ID Application identity, accepted notarization and ordinary quarantined download/Gatekeeper acceptance. No unsigned fallback. |
| macOS x86-64 prebuilt | Native Intel acceptance plus the same signing/notarization requirements. Cross-compilation alone does not qualify. |
| Windows x86-64 companion | Physical clipboard, SSH, draft, image, resize, held-control and cleanup acceptance tied to final payload hashes. No Windows core claim. |
| Homebrew source formulas | Exact source archive, real formula install/test/removal and narrowly scoped tap writer. Existing source formulas remain the current strategy. |
| crates.io | Core v0.3.2 is published through the configured Trusted Publisher and independently verified. Exact version/commit selection and a matching core package layout are required for subsequent releases. |
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
