#!/usr/bin/env bash
# Manual hosted VM only; explicit TCG/emulated NixOS terminal acceptance.
set -euo pipefail
# Refuse accidental local, self-hosted, non-main or non-dispatch execution
# before creating proof files, changing process limits or invoking any Nix tool.
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
python3 - <<'PY_CONTEXT'
import json, os, platform, re, stat
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
assert proof.is_absolute() and proof.resolve() == proof
assert proof.parent == Path.home() / '.cache/flere/tmp'
assert re.fullmatch('nixos-tcg-' + os.environ['GITHUB_RUN_ID'] + '-' + os.environ['GITHUB_RUN_ATTEMPT'] + '-[A-Za-z0-9]+', proof.name)
info = proof.stat()
assert stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o700 and info.st_uid == os.getuid()
os_release = platform.freedesktop_os_release()
assert os_release['ID'] == 'ubuntu' and os_release['VERSION_ID'] == '24.04'
assert os.environ['ImageOS'] and os.environ['ImageVersion']
uname = os.uname()
context = {
    'host': {'kernel': {'system': uname.sysname, 'release': uname.release, 'version': uname.version, 'machine': uname.machine},
             'os': {key: os_release[key] for key in ('ID', 'VERSION_ID', 'PRETTY_NAME')},
             'image': {'os': os.environ['ImageOS'], 'version': os.environ['ImageVersion']},
             'runner_environment': os.environ['FLERE_RUNNER_ENVIRONMENT']},
    'run': {'id': os.environ['GITHUB_RUN_ID'], 'attempt': os.environ['GITHUB_RUN_ATTEMPT'],
            'workflow_sha': os.environ['FLERE_WORKFLOW_SHA'], 'workflow_ref': os.environ['GITHUB_WORKFLOW_REF'],
            'repository': os.environ['GITHUB_REPOSITORY'], 'ref': os.environ['GITHUB_REF']},
}
(proof / 'context.json').write_text(json.dumps(context, indent=2, sort_keys=True) + '\n')
PY_CONTEXT
# Nix builds run in the existing daemon; never propagate a compiler file-size limit.
stage=configuration
failure() {
  code=$?
  trap - ERR
  python3 - "$stage" "$code" <<'PY_FAILURE'
import json, os, sys
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
context = json.loads((proof / 'context.json').read_text())
receipt = {'schema': 'flere-nixos-tcg-acceptance-v1', 'status': 'failed',
           'stage': sys.argv[1], 'exit_code': int(sys.argv[2]), **context}
(proof / 'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
log = os.environ.get('FLERE_NIX_LOG')
if log and Path(log).is_file():
    with Path(log).open('rb') as stream:
        stream.seek(max(0, Path(log).stat().st_size - 8192))
        tail = stream.read(8192).decode('utf-8', errors='backslashreplace')
    # A single escaped JSON line cannot replay child escape/workflow commands.
    print(json.dumps({'failure_log_tail': tail}, ensure_ascii=True))
PY_FAILURE
  exit "$code"
}
trap failure ERR
stage=source
export FLERE_TCG_RUN="$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT-$FLERE_WORKFLOW_SHA"
python3 - <<'PY_SOURCE'
import hashlib, json, os, subprocess
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
source = Path('product')
expected = {'commit': '2d52985845ed322b1c6c0f3018eaedf38d6bcead', 'git_tree': 'af352b755d225157cee4546835566254bd9e5fd0', 'source_archive_sha256': '2028219a227c66b28ed8e4a0e2bd485908fa82451a20f315137ae493a2c3564d', 'source_archive_bytes': 28579840, 'entries': {'Cargo.lock': {'bytes': 25468, 'sha256': '1764da34352b1f4ffa56fb8c61acc4eb56aeef9d2a8e2b1a90dd175b6633686c'}, 'companion/Cargo.lock': {'bytes': 27719, 'sha256': 'e68c4bcd8638d673a5d65647ff2bd286edfa43e6efec0230a15bd7c5120b3306'}, 'Cargo.toml': {'bytes': 1190, 'sha256': 'f6a1dfcef9a156e4dc8255e1a19bb8806d63b0946a0d8de7e1314039468e791d'}, 'companion/Cargo.toml': {'bytes': 564, 'sha256': '03dac110cb5c1d5bb15c648aa946a7a3d22019b2c0e87d30ac94b8cef69d1e4f'}}}
assert subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD'],text=True).strip() == expected['commit']
assert subprocess.check_output(['git','-C',str(source),'rev-parse','HEAD^{tree}'],text=True).strip() == expected['git_tree']
assert not subprocess.check_output(['git','-C',str(source),'status','--porcelain'])
for name, info in expected['entries'].items():
    data = (source/name).read_bytes()
    assert len(data) == info['bytes'] and hashlib.sha256(data).hexdigest() == info['sha256'], name
archive = subprocess.check_output(['git','-C',str(source),'archive','--format=tar','--prefix=flere-source/',expected['commit']])
assert len(archive) == expected['source_archive_bytes'] and hashlib.sha256(archive).hexdigest() == expected['source_archive_sha256']
work = proof/'work'
work.mkdir(mode=0o700)
(work/'source.tar').write_bytes(archive)
(proof/'source.json').write_text(json.dumps(expected,indent=2,sort_keys=True)+'\n')
PY_SOURCE
export NIX_PATH="nixpkgs=$PWD/nixpkgs"
export NIX_REMOTE=daemon
nix --version > "$FLERE_NIX_PROOF/nix-version.txt"
nix config show --json > "$FLERE_NIX_PROOF/nix-config.json"
python3 - <<'PY'
import json, os
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
assert (p/'nix-version.txt').read_text().strip() == 'nix (Nix) 2.33.3'
config = json.loads((p / 'nix-config.json').read_text())
assert config['sandbox']['value'] in (True, 'true'), config['sandbox']
assert config['sandbox-fallback']['value'] is False, config['sandbox-fallback']
assert config['require-sigs']['value'] is True, config['require-sigs']
assert config['trusted-users']['value'] == ['root'], config['trusted-users']
assert {'nix-command', 'flakes'} <= set(config['experimental-features']['value'])
PY
# A fresh input prevents an earlier probe result from standing in for this VM.
# Normal derivations must use a private network namespace; fixed-output fetches
# may use networking only for checksum-pinned source/dependency inputs.
stage=sandbox
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/sandbox.log"
(ulimit -f 8192
 nix-build packaging/nix/sandbox-probe.nix --no-out-link \
  --argstr hostNetworkNamespace "$(readlink /proc/self/ns/net)" \
  > "$FLERE_NIX_PROOF/sandbox.out" 2> "$FLERE_NIX_LOG")
stage=complete-closure-budget
args=(--option allow-import-from-derivation false
      --argstr source "$PWD/product" --argstr sourceArchive "$FLERE_NIX_PROOF/work/source.tar"
      --argstr runIdentity "$FLERE_TCG_RUN")
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/complete-dry-run.log"
(ulimit -f 8192
 timeout --signal=TERM --kill-after=15s 300s \
  nix-build packaging/nix/runtime-tcg.nix -A test --no-out-link --dry-run "${args[@]}" \
  > "$FLERE_NIX_PROOF/complete-dry-run.out" 2> "$FLERE_NIX_LOG")
python3 - <<'PY_BUDGET'
import json, os, re, shutil
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
log = (p/'complete-dry-run.log').read_text()
units = {'B':1,'KiB':1024,'MiB':1024**2,'GiB':1024**3}
rows = re.findall(r'\((\d+(?:\.\d+)?) (B|KiB|MiB|GiB) download, (\d+(?:\.\d+)?) (B|KiB|MiB|GiB) unpacked\)',log)
assert len(rows)<=1 and (rows or 'will be fetched' not in log), 'unparsed/ambiguous complete fetch estimate'
unpacked = int(float(rows[0][2])*units[rows[0][3]]) if rows else 0
builds = re.findall(r'^\s+(/nix/store/[0-9a-z]{32}-[^/\n]+\.drv)\s*$',log,re.M)
assert builds and len(builds)==len(set(builds)), 'missing/ambiguous complete build list'
assert any('vm-test-run-flere-tcg' in name for name in builds), 'complete test derivation not planned'
heavy = [name for name in builds if re.search(r'-(?:linux-[0-9]|qemu[^/]*-[0-9]|rustc?-[0-9]|gcc-[0-9]|llvm-[0-9]|clang-[0-9])',name)]
free = shutil.disk_usage('/nix/store').free
budget = {'scope':'complete core + guest system/kernel/QEMU/driver/tool closure',
          'missing_unpacked_bytes':unpacked,'local_derivations':builds,'unexpected_heavy_builds':heavy,
          'scratch_and_local_output_budget_bytes':8*1024**3,'guest_disk_budget_bytes':2*1024**3,
          'free_reserve_bytes':4*1024**3,'required_bytes':unpacked+14*1024**3,'free_bytes':free,
          'passed':not heavy and free>=unpacked+14*1024**3}
(p/'budget.json').write_text(json.dumps(budget,indent=2,sort_keys=True)+'\n')
assert budget['passed'], 'complete closure exceeds reviewed disk/build budget: '+json.dumps(budget)
PY_BUDGET
stage=realize-guest-dependencies
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/driver.log"
# test.driver realizes the same complete guest inputs but does not boot QEMU.
(ulimit -f 8192
 timeout --signal=TERM --kill-after=30s 1800s \
  nix-build packaging/nix/runtime-tcg.nix -A test.driver --no-out-link "${args[@]}" \
  > "$FLERE_NIX_PROOF/driver.out" 2> "$FLERE_NIX_LOG")
stage=core-inventory
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/core.log"
(ulimit -f 8192
 nix-build packaging/nix/runtime-tcg.nix -A core --no-out-link "${args[@]}" \
  > "$FLERE_NIX_PROOF/core.out" 2> "$FLERE_NIX_LOG")
nix-instantiate packaging/nix/runtime-tcg.nix -A core "${args[@]}" > "$FLERE_NIX_PROOF/core.drv"
python3 - <<'PY_INVENTORY'
import hashlib, json, os, re, shutil, stat, subprocess
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
lines = (p/'core.out').read_text().splitlines()
assert len(lines)==1 and re.fullmatch('/nix/store/[0-9a-z]{32}-flere-0.3.4',lines[0])
core = Path(lines[0])
expected = {'bin/flere',*(f'share/licenses/flere/{n}' for n in ('LICENSE','OFL.txt','LICENSE-Nerd-Fonts'))}
files = {q.relative_to(core).as_posix():q for q in core.rglob('*') if not q.is_dir()}
assert set(files)==expected
entries = {}
for name, path in files.items():
    info = path.lstat()
    assert stat.S_ISREG(info.st_mode) and info.st_uid==0
    assert stat.S_IMODE(info.st_mode)==(0o555 if name.startswith('bin/') else 0o444)
    entries[name] = {'bytes':info.st_size,'mode':stat.S_IMODE(info.st_mode),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
for license, original in [('LICENSE','LICENSE'),('OFL.txt','src/assets/fonts/OFL.txt'),('LICENSE-Nerd-Fonts','src/assets/fonts/LICENSE-Nerd-Fonts')]:
    assert (core/f'share/licenses/flere/{license}').read_bytes()==(Path('product')/original).read_bytes()
closure = subprocess.check_output(['nix-store','--query','--requisites',str(core)],timeout=30)
assert len(closure)<=1048576
(p/'core.closure').write_bytes(closure)
(p/'core.json').write_text(json.dumps({'output':str(core),'derivation':(p/'core.drv').read_text().strip(),
                                     'files':entries,'closure_sha256':hashlib.sha256(closure).hexdigest()},indent=2,sort_keys=True)+'\n')
free = shutil.disk_usage('/nix/store').free
required = 6*1024**3  # guest2GiB plus untouched4GiB reserve; dependencies now realized.
(p/'boot-budget.json').write_text(json.dumps({'free_bytes':free,'required_bytes':required,'passed':free>=required},indent=2)+'\n')
assert free>=required, 'insufficient free reserve after realization; guest not booted'
PY_INVENTORY
stage=tcg-guest-runtime
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/runtime.log"
(ulimit -f 8192
 timeout --signal=TERM --kill-after=30s 660s \
  nix-build packaging/nix/runtime-tcg.nix -A test --no-out-link "${args[@]}" \
  > "$FLERE_NIX_PROOF/runtime.out" 2> "$FLERE_NIX_LOG")
stage=final
python3 - <<'PY_FINAL'
import hashlib, json, os, re, subprocess
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
lines = (p/'runtime.out').read_text().splitlines()
assert len(lines)==1 and re.fullmatch('/nix/store/[0-9a-z]{32}-vm-test-run-flere-tcg',lines[0])
for name in ('runtime.json','acceleration.json'):
    source = Path(lines[0])/name
    assert source.is_file() and not source.is_symlink() and source.stat().st_size<=65536
    (p/name).write_bytes(source.read_bytes())
runtime = json.loads((p/'runtime.json').read_text())
acceleration = json.loads((p/'acceleration.json').read_text())
assert runtime['status']=='passed' and runtime['execution']=='TCG/emulated NixOS'
assert runtime['run_identity']==os.environ['FLERE_TCG_RUN']==acceleration['run_identity']
assert runtime['source_commit']=='2d52985845ed322b1c6c0f3018eaedf38d6bcead'
assert runtime['owned_sessions_exited'] and runtime['supervisor_exit']==0
assert runtime['before']==runtime['after'] and len(runtime['steps'])==4
assert runtime['first_frontend']['exit']==runtime['second_frontend']['exit']==0
assert not runtime['first_frontend']['forced_kill'] and not runtime['second_frontend']['forced_kill']
assert acceleration['execution']=='TCG/emulated NixOS' and acceleration['kvm_enabled'] is False and acceleration['qemu_exit']==0
core = json.loads((p/'core.json').read_text())
assert runtime['core']['path']==core['output']+'/bin/flere'
assert runtime['core']['sha256']==core['files']['bin/flere']['sha256']
assert hashlib.sha256(Path(runtime['core']['path']).read_bytes()).hexdigest()==runtime['core']['sha256']
source = json.loads((p/'source.json').read_text())
assert subprocess.check_output(['git','-C','product','rev-parse','HEAD'],text=True).strip()==source['commit']
assert subprocess.check_output(['git','-C','product','rev-parse','HEAD^{tree}'],text=True).strip()==source['git_tree']
assert not subprocess.check_output(['git','-C','product','status','--porcelain'])
for name, info in source['entries'].items():
    data = (Path('product')/name).read_bytes()
    assert len(data)==info['bytes'] and hashlib.sha256(data).hexdigest()==info['sha256']
assert hashlib.sha256((p/'work/source.tar').read_bytes()).hexdigest()==source['source_archive_sha256']
receipt = {'schema':'flere-nixos-tcg-acceptance-v1','status':'passed','execution':'TCG/emulated NixOS',
           **json.loads((p/'context.json').read_text()),'source':source,'core':core,
           'runtime_sha256':hashlib.sha256((p/'runtime.json').read_bytes()).hexdigest(),
           'acceleration_sha256':hashlib.sha256((p/'acceleration.json').read_bytes()).hexdigest(),
           'sandbox':True,'sandbox_fallback':False,'kvm_used':False,'full_rust_suites_repeated':False,
           'limits':['Emulated NixOS filesystem/terminal runtime, not native/KVM acceptance',
                     'Core only; no companion rebuild/connection, physical desktop, clipboard or external SSH']}
(p/'receipt.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n')
print(json.dumps({'status':'passed','execution':'TCG/emulated NixOS','steps':runtime['steps']}))
PY_FINAL
