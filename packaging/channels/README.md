# Default release selection

`stable.json` selects independently verified releases per platform. Linux and
Windows currently select 0.3.8. macOS arm64 explicitly retains the unsigned
0.3.0 prebuilt; current Mac source builds are available through Homebrew.
Intel Mac prebuilt selection is unavailable.

The Unix installer and source-built SSH companion read this bounded target map
when no explicit package source was supplied. They construct immutable release
URLs and verify the exact manifest size, SHA-256, version, component and target
before using the existing package checks. An invalid or unavailable channel stops
selection; it does not silently choose an older release. A compatible existing
remote installation still attaches without channel discovery.

Companion binaries from 0.3.7 use this index for missing-remote bootstrap.
Earlier binaries retain their previous discovery behavior. The shell
loader pins the self-contained Python installer at commit `48e1cf9` with its
verified size and SHA-256. Old saved loader copies keep their original pin.

Version 0.3.8 default installs retain `default_channel` in the managed receipt.
A later explicit Update resolves this map during preparation; Apply uses that
already verified package. Explicit URLs and old Public receipts stay pinned;
local packages and manager-owned installations keep their existing ownership.
This does not migrate old receipts, update in the background or change the legacy
Mac pin. Supporting candidate and retained-launcher readers are required before
the policy can be saved or restored.

## Prepare a promotion

Run `scripts/release-channel.py --help` from the reviewed source. Supply the
sealed release directory, exact release descriptor SHA, version/commit/run and
workflow identity, plus the current channel file and its SHA. It validates the
release and all anonymous public downloads before writing `stable.json` and a
private `promotion.json` receipt in a fresh home-cache output directory.

The generator advances only targets present in that release profile. It preserves
other platform records, refuses downgrades and refuses different bytes for an
already selected version. Identical promotions are idempotent. It prepares files;
it does not publish or execute downloaded programs.

Before committing, compare the current channel bytes with the receipt's previous
SHA, review the changed targets, then use the normal signed main commit. Initial
creation uses a private seed with independently verified legacy Mac pins and
unavailable Linux/Windows records; subsequent promotions must use the actual
published channel. Verify the published bytes after pushing.

The Release workflow prepares a channel artifact after verifying a published
Linux/Windows or complete release. It records current main's exact channel
preimage and uploads only the proposal and its provenance; historical Linux-only
profiles explicitly skip this step. Apply the artifact through the signed local
main workflow above. Hosted runners have no configured path to the existing
hardware SSH signer, so unattended signed promotion remains separate work.
Package-bucket updates also remain separate. GitHub's shared latest pointer
stays at 0.3.0; this map does not alter that release.
