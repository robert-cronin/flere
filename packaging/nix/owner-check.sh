#!/usr/bin/env bash
# Manual hosted VM only; exact-source build and private profile ownership acceptance.
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
assert re.fullmatch('nix-owner-' + os.environ['GITHUB_RUN_ID'] + '-' + os.environ['GITHUB_RUN_ATTEMPT'] + '-[A-Za-z0-9]+', proof.name)
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
# Bound retained files as well as the daemon's per-build logs (at most 32 MiB).
ulimit -f 32768
stage=configuration
failure() {
  code=$?
  trap - ERR
  python3 - "$stage" "$code" <<'PY_FAILURE'
import json, os, sys
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
context = json.loads((proof / 'context.json').read_text())
receipt = {'schema': 'flere-nix-owner-acceptance-v1', 'status': 'failed',
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
config = json.loads((Path(os.environ['FLERE_NIX_PROOF']) / 'nix-config.json').read_text())
assert config['sandbox']['value'] in (True, 'true'), config['sandbox']
assert config['sandbox-fallback']['value'] is False, config['sandbox-fallback']
assert config['require-sigs']['value'] is True, config['require-sigs']
assert {'nix-command', 'flakes'} <= set(config['experimental-features']['value'])
PY
# A fresh input prevents an earlier probe result from standing in for this VM.
# Normal derivations must use a private network namespace; fixed-output fetches
# may use networking only for checksum-pinned source/dependency inputs.
stage=sandbox
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/sandbox.log"
nix-build packaging/nix/sandbox-probe.nix --no-out-link \
  --argstr hostNetworkNamespace "$(readlink /proc/self/ns/net)" \
  > "$FLERE_NIX_PROOF/sandbox.out" 2> "$FLERE_NIX_LOG"
for component in flere flere-connect; do
  stage="$component"
  export FLERE_NIX_LOG="$FLERE_NIX_PROOF/$component.eval.log"
  nix-instantiate packaging/nix/owner-build.nix -A "$component" \
    --argstr source "$PWD/product" --argstr sourceArchive "$FLERE_NIX_PROOF/work/source.tar" \
    > "$FLERE_NIX_PROOF/$component.drv" 2> "$FLERE_NIX_LOG"
  export FLERE_NIX_LOG="$FLERE_NIX_PROOF/$component.log"
  nix-build packaging/nix/owner-build.nix -A "$component" --no-out-link \
    --argstr source "$PWD/product" --argstr sourceArchive "$FLERE_NIX_PROOF/work/source.tar" \
    > "$FLERE_NIX_PROOF/$component.out" 2> "$FLERE_NIX_LOG"
done
stage=inventory
python3 - <<'PY_INVENTORY'
import hashlib, json, os, re, stat, subprocess
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
outputs = {}
for component in ('flere','flere-connect'):
    lines = (proof/f'{component}.out').read_text().splitlines()
    assert len(lines) == 1 and re.fullmatch(r'/nix/store/[0-9a-z]{32}-[^/]+',lines[0])
    root = Path(lines[0])
    expected = {f'bin/{component}',*(f'share/licenses/{component}/{n}' for n in ('LICENSE','OFL.txt','LICENSE-Nerd-Fonts'))}
    files = {p.relative_to(root).as_posix():p for p in root.rglob('*') if not p.is_dir()}
    assert set(files) == expected
    entries = {}
    for name, path in sorted(files.items()):
        info = path.lstat()
        mode = stat.S_IMODE(info.st_mode)
        assert stat.S_ISREG(info.st_mode) and info.st_uid == 0
        assert mode == (0o555 if name.startswith('bin/') else 0o444)
        entries[name] = {'bytes':info.st_size,'mode':mode,'sha256':hashlib.sha256(path.read_bytes()).hexdigest()}
    for name, original in [('LICENSE','LICENSE'),('OFL.txt','src/assets/fonts/OFL.txt'),('LICENSE-Nerd-Fonts','src/assets/fonts/LICENSE-Nerd-Fonts')]:
        assert (root/f'share/licenses/{component}/{name}').read_bytes() == (Path('product')/original).read_bytes()
    closure = subprocess.check_output(['nix-store','--query','--requisites',str(root)],timeout=30)
    assert len(closure) <= 1048576
    (proof/f'{component}.closure').write_bytes(closure)
    outputs[component] = {'output':str(root),'derivation':(proof/f'{component}.drv').read_text().strip(),
                          'files':entries,'closure_sha256':hashlib.sha256(closure).hexdigest()}
(proof/'builds.json').write_text(json.dumps(outputs,indent=2,sort_keys=True)+'\n')
PY_INVENTORY
stage=profile-owner
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/owner.log"
timeout --signal=TERM --kill-after=90s 480s python3 -I -B packaging/nix/owner-probe.py \
  > "$FLERE_NIX_LOG" 2>&1
stage=final
python3 - <<'PY_FINAL'
import hashlib, json, os, subprocess
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
source = json.loads((proof/'source.json').read_text())
assert subprocess.check_output(['git','-C','product','rev-parse','HEAD'],text=True).strip() == source['commit']
assert subprocess.check_output(['git','-C','product','rev-parse','HEAD^{tree}'],text=True).strip() == source['git_tree']
assert not subprocess.check_output(['git','-C','product','status','--porcelain'])
for name, info in source['entries'].items():
    data = (Path('product')/name).read_bytes()
    assert len(data) == info['bytes'] and hashlib.sha256(data).hexdigest() == info['sha256']
archive = (proof/'work/source.tar').read_bytes()
assert len(archive) == source['source_archive_bytes'] and hashlib.sha256(archive).hexdigest() == source['source_archive_sha256']
owner = json.loads((proof/'owner-receipt.json').read_text())
assert owner['status'] == 'passed' and owner['profile_removed']
receipt = {'schema':'flere-nix-installed-owner-build-v1','status':'passed',
           **json.loads((proof/'context.json').read_text()),
           'source':json.loads((proof/'source.json').read_text()),
           'builds':json.loads((proof/'builds.json').read_text()),
           'owner_receipt_sha256':hashlib.sha256((proof/'owner-receipt.json').read_bytes()).hexdigest(),
           'sandbox':True,'sandbox_fallback':False,'cargo_offline':True,'full_suites_repeated':False,
           'packaging_changes':['Three declared /usr/bin/sha256sum replacements only'],
           'limits':owner['limits']}
(proof/'receipt.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n')
PY_FINAL
