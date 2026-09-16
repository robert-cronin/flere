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
# Evaluate all four exact package derivations before realizing their closure.
stage=complete-closure-budget
args=(--option allow-import-from-derivation false
      --argstr source "$PWD/product" --argstr sourceArchive "$FLERE_NIX_PROOF/work/source.tar")
derivations=()
for component in previous-flere previous-flere-connect flere flere-connect; do
  export FLERE_NIX_LOG="$FLERE_NIX_PROOF/$component.eval.log"
  timeout --signal=TERM --kill-after=15s 120s \
    nix-instantiate packaging/nix/owner-build.nix -A "$component" "${args[@]}" \
    > "$FLERE_NIX_PROOF/$component.drv" 2> "$FLERE_NIX_LOG"
  derivation=$(cat "$FLERE_NIX_PROOF/$component.drv")
  [[ "$derivation" =~ ^/nix/store/[0-9a-z]{32}-[^/[:space:]]+\.drv$ ]]
  derivations+=("$derivation")
done
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/complete-dry-run.log"
timeout --signal=TERM --kill-after=15s 300s \
  nix-store --realise --dry-run "${derivations[@]}" \
  > "$FLERE_NIX_PROOF/complete-dry-run.out" 2> "$FLERE_NIX_LOG"
python3 - <<'PY_BUDGET'
import importlib.util, json, os, shutil
from pathlib import Path
p = Path(os.environ['FLERE_NIX_PROOF'])
spec = importlib.util.spec_from_file_location('owner', 'packaging/nix/owner-probe.py')
owner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owner)
budget = owner.build_budget((p/'complete-dry-run.log').read_text(), shutil.disk_usage('/nix/store').free)
budget['selected_derivations'] = {name:(p/f'{name}.drv').read_text().strip()
    for name in ('previous-flere','previous-flere-connect','flere','flere-connect')}
assert len(set(budget['selected_derivations'].values())) == 4, 'expected four distinct version/component derivations'
(p/'budget.json').write_text(json.dumps(budget,indent=2,sort_keys=True)+'\n')
assert budget['passed'], 'complete closure exceeds reviewed disk/build budget'
PY_BUDGET
# A fresh input prevents an earlier probe result from standing in for this VM.
# Normal derivations must use a private network namespace; fixed-output fetches
# may use networking only for checksum-pinned source/dependency inputs.
stage=sandbox
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/sandbox.log"
nix-build packaging/nix/sandbox-probe.nix --no-out-link \
  --argstr hostNetworkNamespace "$(readlink /proc/self/ns/net)" \
  > "$FLERE_NIX_PROOF/sandbox.out" 2> "$FLERE_NIX_LOG"
for component in previous-flere previous-flere-connect flere flere-connect; do
  stage="$component"
  export FLERE_NIX_LOG="$FLERE_NIX_PROOF/$component.log"
  nix-store --realise "$(cat "$FLERE_NIX_PROOF/$component.drv")" \
    > "$FLERE_NIX_PROOF/$component.out" 2> "$FLERE_NIX_LOG"
done
# The previous source is already a dependency of both previous packages.
stage=previous-source
export FLERE_NIX_LOG="$FLERE_NIX_PROOF/previous-source.log"
nix-build packaging/nix/owner-build.nix -A previous-source --no-out-link "${args[@]}" \
  > "$FLERE_NIX_PROOF/previous-source.out" 2> "$FLERE_NIX_LOG"
stage=inventory
python3 - <<'PY_INVENTORY'
import hashlib, importlib.util, json, os, re, stat, subprocess, tarfile
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
source_lines = (proof/'previous-source.out').read_text().splitlines()
assert len(source_lines) == 1 and re.fullmatch('/nix/store/[0-9a-z]{32}-[^/]+', source_lines[0])
source_output = Path(source_lines[0])
assert source_output.is_file() and not source_output.is_symlink()
with source_output.open('rb') as stream:
    source_bytes = stream.read(64*1024*1024 + 1)
assert len(source_bytes) <= 64*1024*1024
assert hashlib.sha256(source_bytes).hexdigest() == '461bbe1e3fa88027c2ea7191c34adbd7eceaadbf6f091a270769b1cdc0ad6a9e'
# The shared release inspector requires the original public archive basename.
previous_archive = proof/'work/flere-0.3.3-source.tar.gz'
previous_archive.write_bytes(source_bytes)
spec = importlib.util.spec_from_file_location('release_source', 'scripts/release-source.py')
source_helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source_helper)
previous_source = source_helper.inspect(previous_archive, '0.3.3', '4ceafa043b2c79b862d1ca1dd56715055b9447968411045c69ee0c1c85673801')
assert previous_source['sha256'] == '461bbe1e3fa88027c2ea7191c34adbd7eceaadbf6f091a270769b1cdc0ad6a9e'
previous_source['commit'] = 'ce6bb62ca6051d8bfc385c2df16bc62cbd65c738'
previous_source['parser_patch_sha256'] = hashlib.sha256(Path('packaging/nix/patches/wrapped-path-delimiter.patch').read_bytes()).hexdigest()
assert previous_source['parser_patch_sha256'] == '3e1bcd7b259d047974a85f2ff0cd8aae965e90207ceba1253e9637e8ef701275'
previous_source['full_suites_repeated'] = False
previous_source['fixture_build_override'] = 'doCheck=false; original install checks retained'
(proof/'previous-source.json').write_text(json.dumps(previous_source,indent=2,sort_keys=True)+'\n')
licenses = [('LICENSE','LICENSE'),('OFL.txt','src/assets/fonts/OFL.txt'),('LICENSE-Nerd-Fonts','src/assets/fonts/LICENSE-Nerd-Fonts')]
with tarfile.open(previous_archive,'r:gz') as archive:
    previous_licenses = {name:archive.extractfile('flere-0.3.3/'+original).read() for name,original in licenses}
outputs = {}
for selection in ('previous-flere','previous-flere-connect','flere','flere-connect'):
    component = selection.removeprefix('previous-')
    lines = (proof/f'{selection}.out').read_text().splitlines()
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
    for name, original in licenses:
        expected_license = previous_licenses[name] if selection.startswith('previous-') else (Path('product')/original).read_bytes()
        assert (root/f'share/licenses/{component}/{name}').read_bytes() == expected_license
    closure = subprocess.check_output(['nix-store','--query','--requisites',str(root)],timeout=30)
    assert len(closure) <= 1048576
    (proof/f'{selection}.closure').write_bytes(closure)
    outputs[selection] = {'output':str(root),'derivation':(proof/f'{selection}.drv').read_text().strip(),
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
assert owner['profile_upgrade']['from_version'] == '0.3.3' and owner['profile_upgrade']['to_version'] == '0.3.4'
assert owner['profile_upgrade']['state_unchanged']
assert owner['profile_upgrade']['before']['generation'] != owner['profile_upgrade']['after']['generation']
assert owner['profile_upgrade']['before']['environment'] != owner['profile_upgrade']['after']['environment']
assert len(owner['previous_components']) == len(owner['components']) == 2
builds = json.loads((proof/'builds.json').read_text())
for label, rows in [('previous-', owner['previous_components']), ('', owner['components'])]:
    for name, info in rows.items():
        assert info['output'] == builds[label+name]['output']
        assert info['sha256'] == builds[label+name]['files']['bin/'+name]['sha256']
receipt = {'schema':'flere-nix-installed-owner-build-v1','status':'passed',
           **json.loads((proof/'context.json').read_text()),
           'source':json.loads((proof/'source.json').read_text()),
           'builds':builds,'previous_source':json.loads((proof/'previous-source.json').read_text()),
           'budget':json.loads((proof/'budget.json').read_text()),
           'owner_receipt_sha256':hashlib.sha256((proof/'owner-receipt.json').read_bytes()).hexdigest(),
           'sandbox':True,'sandbox_fallback':False,'cargo_offline':True,'full_suites_repeated':False,
           'packaging_changes':['0.3.4: three declared /usr/bin/sha256sum replacements only',
                                '0.3.3: fixed historical source/locks/patch, doCheck=false for separate upgrade fixture'],
           'limits':owner['limits']}
(proof/'receipt.json').write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n')
PY_FINAL
