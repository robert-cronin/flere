#!/usr/bin/env bash
# Prerequisite only. No Flere build, guest closure, boot, runtime or profile.
set -euo pipefail
test "${GITHUB_ACTIONS-}" = true
test "${FLERE_RUNNER_ENVIRONMENT-}" = github-hosted
test "${GITHUB_EVENT_NAME-}" = workflow_dispatch
test "${GITHUB_REPOSITORY-}" = robert-cronin/flere
test "${GITHUB_REF-}" = refs/heads/main
test "${RUNNER_OS-}/${RUNNER_ARCH-}" = Linux/X64
test "$(uname -sm)" = 'Linux x86_64'
test "$(cat /proc/1/comm)" = systemd
[[ "${FLERE_WORKFLOW_SHA-}" =~ ^[0-9a-f]{40}$ ]]
[[ "${GITHUB_RUN_ID-}/${GITHUB_RUN_ATTEMPT-}" =~ ^[0-9]+/[0-9]+$ ]]
test "$(git rev-parse HEAD)" = "$FLERE_WORKFLOW_SHA"
test "$(git -C nixpkgs rev-parse HEAD)" = eaad089433ca2bb662274377d33df3d0e51ef28b
export NIX_PATH="nixpkgs=$PWD/nixpkgs" NIX_REMOTE=daemon
export FLERE_KVM_RUN="$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT-$FLERE_WORKFLOW_SHA"
python3 - <<'PY'
import json, os, platform, re, shutil, stat
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
assert p.is_absolute() and p.resolve() == p
assert p.parent == Path.home() / '.cache/flere/tmp'
assert re.fullmatch('nixos-kvm-' + os.environ['GITHUB_RUN_ID'] + '-' + os.environ['GITHUB_RUN_ATTEMPT'] + '-[A-Za-z0-9]+', p.name)
info = p.stat()
assert stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid == os.getuid()
assert platform.freedesktop_os_release()['ID'] == 'ubuntu'
context = {'run_identity': os.environ['FLERE_KVM_RUN'], 'workflow_sha': os.environ['FLERE_WORKFLOW_SHA'],
           'runner_image': os.environ['ImageOS'], 'runner_image_version': os.environ['ImageVersion'],
           'kernel': platform.release(), 'cpu_count': os.cpu_count(),
           'host_network_namespace': os.readlink('/proc/self/ns/net'),
           'disk_free_bytes_before': shutil.disk_usage(p).free,
           'nixpkgs': 'eaad089433ca2bb662274377d33df3d0e51ef28b',
           'future_product_commit': '2d52985845ed322b1c6c0f3018eaedf38d6bcead'}
(p/'context.json').write_text(json.dumps(context, indent=2, sort_keys=True)+'\n')
PY
stage=configuration
failure() {
  code=$?
  trap - ERR
  python3 - "$stage" "$code" <<'PY'
import json, os, sys
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
if not (p/'receipt.json').exists():
    receipt = {'schema':'flere-nixos-kvm-prerequisite-v1', 'status':'failed',
               'stage':sys.argv[1], 'exit_code':int(sys.argv[2]),
               **json.loads((p/'context.json').read_text())}
    (p/'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True)+'\n')
print(json.dumps({'status':json.loads((p/'receipt.json').read_text())['status'],
                  'stage':sys.argv[1], 'receipt':'receipt.json'}))
PY
  exit "$code"
}
trap failure ERR
nix --version > "$FLERE_NIX_PROOF/nix-version.txt"
nix config show --json > "$FLERE_NIX_PROOF/nix-config.json"
python3 - <<'PY'
import json, os
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
assert (p/'nix-version.txt').read_text().strip() == 'nix (Nix) 2.33.3'
c = json.loads((p/'nix-config.json').read_text())
assert c['sandbox']['value'] is True and c['sandbox-fallback']['value'] is False
assert c['require-sigs']['value'] is True and c['trusted-users']['value'] == ['root']
PY
stage=runner-probe
timeout --signal=TERM --kill-after=2s 15s python3 -I -B packaging/nix/kvm-probe.py \
  --scope runner --run-identity "$FLERE_KVM_RUN" > "$FLERE_NIX_PROOF/runner-probe.json"

# Probe dependencies only; no NixOS guest or product expression is evaluated.
# Bound only these Nix client processes and retained output; the existing
# NIX_REMOTE=daemon build service does not inherit this client file-size limit.
stage=probe-budget
(ulimit -f 8192
 nix-build packaging/nix/kvm-probe.nix --no-out-link --dry-run \
  --argstr runIdentity "$FLERE_KVM_RUN" \
  --argstr hostNetworkNamespace "$(readlink /proc/self/ns/net)" \
  > "$FLERE_NIX_PROOF/probe-dry-run.out" \
  2> "$FLERE_NIX_PROOF/probe-dry-run.log")
python3 - <<'PY'
import json, os, re, shutil
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
log = (p/'probe-dry-run.log').read_text()
units = {'B':1, 'KiB':1024, 'MiB':1024**2, 'GiB':1024**3}
rows = re.findall(r'\((\d+(?:\.\d+)?) (B|KiB|MiB|GiB) download, (\d+(?:\.\d+)?) (B|KiB|MiB|GiB) unpacked\)', log)
assert len(rows) <= 1, 'ambiguous Nix fetch estimate'
assert rows or 'will be fetched' not in log, 'unparsed Nix fetch estimate'
unpacked = int(float(rows[0][2]) * units[rows[0][3]]) if rows else 0
# Six GiB: four GiB kept free, plus two GiB for this small interpreter/probe
# realization. A later guest must use its own complete closure estimate.
required = unpacked + 6 * 1024**3
free = shutil.disk_usage('/nix/store').free
budget = {'scope':'KVM probe dependencies only', 'missing_unpacked_bytes':unpacked,
          'free_bytes':free, 'required_bytes':required, 'reserve_bytes':4*1024**3,
          'scratch_budget_bytes':2*1024**3, 'passed':free >= required,
          'full_guest_and_source_budget':'not evaluated; execution remains deferred'}
(p/'budget.json').write_text(json.dumps(budget, indent=2, sort_keys=True)+'\n')
if not budget['passed']:
    receipt = {'schema':'flere-nixos-kvm-prerequisite-v1', 'status':'blocked',
               'blocker':'insufficient probe disk budget', 'budget':budget,
               **json.loads((p/'context.json').read_text())}
    (p/'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True)+'\n')
    raise SystemExit(2)
PY
stage=sandbox-probe
(ulimit -f 8192
 timeout --signal=TERM --kill-after=30s 600s \
  nix-build packaging/nix/kvm-probe.nix --no-out-link \
  --argstr runIdentity "$FLERE_KVM_RUN" \
  --argstr hostNetworkNamespace "$(readlink /proc/self/ns/net)" \
  > "$FLERE_NIX_PROOF/sandbox-probe.out" \
  2> "$FLERE_NIX_PROOF/sandbox-probe.log")
stage=receipt
python3 - <<'PY'
import hashlib, json, os, re, shutil
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
lines = (p/'sandbox-probe.out').read_text().splitlines()
assert len(lines) == 1 and re.fullmatch('/nix/store/[0-9a-z]{32}-flere-nixos-kvm-prerequisite', lines[0])
raw = (Path(lines[0])/'probe.json').read_bytes()
assert len(raw) <= 16384
(p/'sandbox-probe.json').write_bytes(raw)
probes = {scope:json.loads((p/(scope+'-probe.json')).read_text()) for scope in ('runner','sandbox')}
for scope, row in probes.items():
    assert row['scope'] == scope and row['run_identity'] == os.environ['FLERE_KVM_RUN']
    assert row['owned_fds_closed'] and row['guest_memory_or_vcpus_created'] is False
config = json.loads((p/'nix-config.json').read_text())
features = config['system-features']['value']
status = 'passed' if all(row['status'] == 'passed' for row in probes.values()) and 'kvm' in features else 'blocked'
if any(row['status'] == 'failed' for row in probes.values()):
    status = 'failed'
receipt = {'schema':'flere-nixos-kvm-prerequisite-v1', 'status':status,
           **json.loads((p/'context.json').read_text()), 'probes':probes,
           'effective_client_system_features':features,
           'client_kvm_feature_missing':'kvm' not in features,
           'sandbox_output':lines[0], 'sandbox':True, 'sandbox_fallback':False,
           'disk_free_bytes_after':shutil.disk_usage('/nix/store').free,
           'budget':json.loads((p/'budget.json').read_text()),
           'files':{name:hashlib.sha256((p/name).read_bytes()).hexdigest() for name in ('runner-probe.json','sandbox-probe.json','nix-config.json')},
           'nixos_guest_booted':False, 'product_built_or_started':False,
           'security_or_device_permissions_changed':False,
           'next_step':'Review capability evidence; guest closure/source budget and actual NixOS runtime remain separate.'}
(p/'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True)+'\n')
print(json.dumps({'status':status, 'runner':probes['runner']['status'], 'sandbox':probes['sandbox']['status'],
                  'client_kvm_feature_missing':receipt['client_kvm_feature_missing']}))
raise SystemExit(0 if status == 'passed' else 2)
PY
