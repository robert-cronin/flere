#!/usr/bin/env bash
# Only invoked by the manual disposable Ubuntu VM job; no profile installation.
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
assert re.fullmatch('nix-sandbox-' + os.environ['GITHUB_RUN_ID'] + '-' + os.environ['GITHUB_RUN_ATTEMPT'] + '-[A-Za-z0-9]+', proof.name)
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
receipt = {'schema': 'flere-nix-sandbox-acceptance-v1', 'status': 'failed',
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
  nix-instantiate packaging/nix/default.nix -A "$component" \
    > "$FLERE_NIX_PROOF/$component.drv" 2> "$FLERE_NIX_LOG"
  export FLERE_NIX_LOG="$FLERE_NIX_PROOF/$component.log"
  nix-build packaging/nix/default.nix -A "$component" --no-out-link \
    > "$FLERE_NIX_PROOF/$component.out" 2> "$FLERE_NIX_LOG"
done
stage=inventory
python3 - <<'PY'
import hashlib, json, os, re, stat, subprocess
from pathlib import Path
proof = Path(os.environ['FLERE_NIX_PROOF'])
context = json.loads((proof / 'context.json').read_text())
outputs = {}
for component in ('flere', 'flere-connect'):
    lines = (proof / f'{component}.out').read_text().splitlines()
    assert len(lines) == 1 and lines[0].startswith('/nix/store/')
    root = Path(lines[0])
    summaries = re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;', (proof / f'{component}.log').read_text())
    assert summaries and sum(int(row[0]) for row in summaries) > 0, 'missing full-suite log'
    assert all(all(int(value) == 0 for value in row[1:]) for row in summaries), summaries
    expected = {f'bin/{component}', *(f'share/licenses/{component}/{name}' for name in ('LICENSE', 'OFL.txt', 'LICENSE-Nerd-Fonts'))}
    files = {p.relative_to(root).as_posix(): p for p in root.rglob('*') if not p.is_dir()}
    assert set(files) == expected, sorted(files)
    entries = {}
    for name, p in sorted(files.items()):
        info = p.lstat()
        assert stat.S_ISREG(info.st_mode), name
        mode = stat.S_IMODE(info.st_mode)
        assert mode == (0o555 if name.startswith('bin/') else 0o444), (name, mode)
        entries[name] = {'bytes': info.st_size, 'mode': mode, 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
    outputs[component] = {'output': str(root), 'derivation': (proof / f'{component}.drv').read_text().strip(), 'files': entries, 'test_summaries': [[int(n) for n in row] for row in summaries]}
receipt = {'schema': 'flere-nix-sandbox-acceptance-v1', 'status': 'passed', **context,
           'source_version': '0.3.5', 'source_commit': '8744d358e62632490bbca10ddd9e82aa5b9c5e94',
           'source_archive_sha256': 'b47e0b741d3e15795a98ff9d107430e94f03d4713e9341d5e7b747f0396142c5',
           'nixpkgs': subprocess.check_output(['git', '-C', 'nixpkgs', 'rev-parse', 'HEAD'], text=True).strip(),
           'workflow_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
           'sandbox': True, 'sandbox_fallback': False, 'cargo_offline': True,
           'test_threads': 1, 'build_jobs': 2, 'outputs': outputs,
           'limits': ['No user-profile install', 'No interactive NixOS/desktop/SSH acceptance', 'No Nix updater-ownership acceptance']}
(proof / 'receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
PY
