#!/usr/bin/env python3
"""Prepared native Nix-on-Ubuntu profile/owner acceptance; no real SSH or workspaces."""
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import selectors
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import termios
import time

COMMIT = '2d52985845ed322b1c6c0f3018eaedf38d6bcead'
VERSION = '0.3.4'
DETAIL = 'Update the owning Nix configuration or profile, then reopen Flere. In-app Apply is disabled.'
NIX = '/nix/var/nix/profiles/default/bin'


def require(value, message):
    if not value:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def tree(path):
    result = {}
    if not path.exists():
        return result
    for entry in sorted(path.rglob('*')):
        mode = entry.lstat().st_mode
        result[str(entry.relative_to(path))] = (
            stat.S_IMODE(mode),
            os.readlink(entry) if stat.S_ISLNK(mode)
            else sha(entry) if stat.S_ISREG(mode) else 'directory')
    return result


def nix_trust_metadata():
    """Read only the fixed root-controlled candidates/socket used by the product."""
    def trusted(path):
        rows = []
        valid = True
        for part in [path, *path.parents]:
            info = part.lstat()
            mode = info.st_mode
            allowed = info.st_uid == 0 and (stat.S_ISLNK(mode) or mode & 0o022 == 0
                or part == Path('/nix/store') and stat.S_ISDIR(mode) and mode & 0o1002 == 0o1000)
            rows.append({'path': str(part), 'uid': info.st_uid,
                         'mode': stat.S_IMODE(mode), 'trusted': allowed})
            valid &= allowed
        return valid, rows
    result = {'programs': []}
    for value in (NIX + '/nix-store', '/run/current-system/sw/bin/nix-store'):
        fixed = Path(value)
        try:
            canonical = fixed.resolve(strict=True)
            valid, rows = trusted(fixed)
            canonical_valid, canonical_rows = trusted(canonical)
            info = canonical.lstat()
            valid &= (canonical_valid and str(canonical).startswith('/nix/store/')
                      and stat.S_ISREG(info.st_mode) and bool(info.st_mode & 0o111))
            result['programs'].append({'fixed': value, 'canonical': str(canonical),
                                       'trusted': valid, 'fixed_chain': rows,
                                       'canonical_chain': canonical_rows})
        except FileNotFoundError:
            result['programs'].append({'fixed': value, 'available': False, 'trusted': False})
    socket = Path('/nix/var/nix/daemon-socket/socket')
    info = socket.lstat()
    valid, rows = trusted(socket.parent)
    valid &= stat.S_ISSOCK(info.st_mode) and info.st_uid == 0 and socket.resolve(strict=True) == socket
    result['socket'] = {'path': str(socket), 'uid': info.st_uid,
                        'mode': stat.S_IMODE(info.st_mode), 'trusted': valid, 'parent_chain': rows}
    return result


class Runtime:
    def __init__(self, argv, env, directory):
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 36, 180, 0, 0))
        try:
            self.child = subprocess.Popen(argv, env=env, cwd=directory, stdin=slave,
                                          stdout=slave, stderr=slave, start_new_session=True)
        except BaseException:
            os.close(self.master)
            raise
        finally:
            os.close(slave)
        os.set_blocking(self.master, False)
        self.output = bytearray()

    def drain(self):
        while True:
            try:
                data = os.read(self.master, 65536)
            except BlockingIOError:
                return
            except OSError as error:
                if error.errno == errno.EIO:
                    return
                raise
            if not data:
                return
            self.output.extend(data)
            require(len(self.output) <= 1024 * 1024, 'PTY proof exceeds 1 MiB')

    def wait(self, *markers, start=0, fresh=False):
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            self.drain()
            raw = self.output[start:]
            begin = raw.rfind(b'\x1b[?2026h')
            end = raw.rfind(b'\x1b[?2026l')
            if all(marker.encode() in raw for marker in markers) and (end > begin >= 0 if fresh else begin < 0 or end > begin):
                return
            require(self.child.poll() is None, 'owned UI exited before readiness')
            select.select([self.master], [], [], 0.05)
        raise RuntimeError('owned UI readiness timeout: ' + repr(markers))

    def send(self, data):
        require(os.write(self.master, data) == len(data), 'short owned PTY input')

    def finish(self):
        # Same normal detach sequence used by tests/live.rs::finish_ui.
        self.send(b'\0q')
        deadline = time.monotonic() + 12
        while self.child.poll() is None and time.monotonic() < deadline:
            self.drain()
            select.select([self.master], [], [], 0.05)
        require(self.child.poll() == 0, 'owned UI did not detach successfully')
        result = self.close()
        require(not result['forced_kill'], 'successful UI required forced cleanup')
        return result

    def close(self):
        forced = False
        if self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait(timeout=3)
                forced = True
        try:
            self.drain()
        finally:
            os.close(self.master)
        return {'pid': self.child.pid, 'exit': self.child.returncode, 'forced_kill': forced,
                'output_bytes': len(self.output),
                'output_sha256': hashlib.sha256(self.output).hexdigest()}


def main():
    proof = Path(os.environ['FLERE_NIX_PROOF'])
    require(os.environ.get('GITHUB_ACTIONS') == 'true' and os.environ.get('GITHUB_EVENT_NAME') == 'workflow_dispatch'
            and os.environ.get('GITHUB_REPOSITORY') == 'robert-cronin/flere'
            and os.environ.get('GITHUB_REF') == 'refs/heads/main', 'manual public-main workflow required')
    context = json.loads((proof / 'context.json').read_text())
    require(os.getuid() != 0 and os.uname().machine == 'x86_64', 'ordinary native user required')
    require(context['host']['runner_environment'] == 'github-hosted', 'hosted-only proof')
    root = Path(tempfile.mkdtemp(prefix='no-', dir=Path.home() / '.cache/flere/tmp'))
    os.chmod(root, 0o700)
    user = root / 'h'
    user.mkdir(mode=0o700)
    profile = root / 'profile'
    state = root / 's'
    env = {'HOME': str(user), 'PATH': NIX + ':/usr/bin:/bin', 'LC_ALL': 'C',
           'TERM': 'xterm-256color', 'NIX_REMOTE': 'daemon', 'NIX_CONFIG': '',
           'NIX_USER_CONF_FILES': '/dev/null'}
    for key, name in [('XDG_CONFIG_HOME', 'config'), ('XDG_CACHE_HOME', 'cache'),
                      ('XDG_DATA_HOME', 'data'), ('XDG_STATE_HOME', 'state'), ('TMPDIR', 'tmp')]:
        directory = user / name
        directory.mkdir(mode=0o700)
        env[key] = str(directory)
    sentinel = user / 'config/unrelated.txt'
    sentinel.write_bytes(b'owned synthetic user state\n')
    protected = sha(sentinel)
    receipt = {'schema': 'flere-nix-installed-owner-v1', 'status': 'running', **context,
               'source_commit': COMMIT, 'source_version': VERSION, 'root': str(root),
               'profile': str(profile), 'steps': [], 'components': {}, 'runtime': {},
               'limits': ['Native Nix on Ubuntu, not NixOS', 'No version upgrade or physical desktop/SSH acceptance',
                          'Unpublished 0.3.4 development source; not the separate 0.3.3 sandbox proof']}
    supervisor = ui = None
    bridge_fd = None
    profile_installed = False

    def run(argv, ok=(0,), timeout=15):
        # Only the fixed CLI commands below run; all receive a null stdin.
        started = time.monotonic()
        proc = subprocess.Popen(argv, env=env, cwd=user, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        streams = selectors.DefaultSelector()
        captured = [bytearray(), bytearray()]
        streams.register(proc.stdout, selectors.EVENT_READ, 0)
        streams.register(proc.stderr, selectors.EVENT_READ, 1)
        try:
            while streams.get_map():
                require(time.monotonic() - started < timeout, 'bounded CLI timeout')
                for event, _ in streams.select(0.1):
                    chunk = os.read(event.fileobj.fileno(), 65536)
                    if not chunk:
                        streams.unregister(event.fileobj)
                    else:
                        captured[event.data].extend(chunk)
                        require(sum(map(len, captured)) <= 256 * 1024, 'bounded CLI output exceeded')
            proc.wait(timeout=3)
            stdout, stderr = map(bytes, captured)
        finally:
            streams.close()
            if proc.poll() is None:
                proc.kill()
                proc.wait(timeout=3)
            proc.stdout.close()
            proc.stderr.close()
        receipt['steps'].append({'argv': [str(a) for a in argv], 'exit': proc.returncode,
                                 'stdout_sha256': hashlib.sha256(stdout).hexdigest(),
                                 'stderr_sha256': hashlib.sha256(stderr).hexdigest(),
                                 'seconds': round(time.monotonic() - started, 3)})
        require(proc.returncode in ok, 'command failed: ' + repr(argv) + ' ' + repr(stderr[-4096:]))
        return stdout, stderr

    outputs = {}
    for component in ('flere', 'flere-connect'):
        lines = (proof / f'{component}.out').read_text().splitlines()
        require(len(lines) == 1 and re.fullmatch(r'/nix/store/[0-9a-z]{32}-[^/]+', lines[0]), 'bad output path')
        outputs[component] = Path(lines[0])
    core = profile / 'bin/flere'
    companion = profile / 'bin/flere-connect'
    prefix = [str(core), '--state', str(state)]

    def app(args):
        return json.loads(run(prefix + args)[0])

    def registered(want):
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            require(supervisor.poll() is None, 'owned supervisor exited')
            status = app(['build-status'])
            current = status.get('supervisor', {})
            tracked = status.get('frontends', {})
            if current.get('status') == 'known' and tracked.get('status') == 'known':
                require(current['pid'] == supervisor.pid and status['comparison'] == 'same_build', 'supervisor identity changed')
                require(current['build'] == receipt['components']['flere']['build'], 'supervisor build differs')
                require(re.fullmatch('[0-9a-f]{32}', current['epoch']), 'invalid runtime epoch')
                rows = tracked['tracked']
                if len(rows) == want and tracked['untracked'] == 0:
                    for row in rows:
                        require(row['build'] == current['build'], 'frontend build differs')
                        require(Path(f'/proc/{row["pid"]}/exe').resolve() == outputs['flere'] / 'bin/flere', 'frontend executable differs')
                    empty = app(['list'])
                    require(empty['epoch'] == current['epoch'] and empty['workspaces'] == [], 'probe started work')
                    return current['epoch'], rows
            time.sleep(0.05)
        raise RuntimeError('exact empty runtime registration timed out')

    installer_paths = [user / 'data/flere/install', user / '.local/share/flere/install',
                       user / '.local/bin', user / 'cache/flere/install']
    def installations():
        return {str(path): tree(path) for path in installer_paths}

    try:
        require(hasattr(os, 'pidfd_open') and hasattr(signal, 'pidfd_send_signal'), 'owned bridge pidfd support required')
        receipt['nix_trust_metadata'] = nix_trust_metadata()
        trust = receipt['nix_trust_metadata']
        require(trust['socket']['trusted'] and any(row['trusted'] for row in trust['programs']), 'fixed Nix trust preflight failed')
        profile_installed = True
        run([NIX + '/nix-env', '--profile', str(profile), '--install',
             str(outputs['flere']), str(outputs['flere-connect'])], timeout=180)
        query = run([NIX + '/nix-env', '--profile', str(profile), '--query', '--out-path'])[0].decode()
        require({tuple(line.split()) for line in query.splitlines()} ==
                {(name + '-' + VERSION, str(path)) for name, path in outputs.items()}, 'profile inventory differs')
        for component, output in outputs.items():
            binary = profile / 'bin' / component
            actual = output / 'bin' / component
            require(binary.resolve() == actual and actual.stat().st_uid == 0, 'profile does not select real Nix output')
            info = actual.stat()
            require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o555, 'Nix executable mode differs')
            before = tree(user)
            for flag in ('--version', '--help', '--build-info'):
                stdout, _ = run([str(binary), flag])
                if flag == '--build-info':
                    build = json.loads(stdout)
                    require(build['component'] == component and build['package_version'] == VERSION
                            and build['target'] == 'x86_64-unknown-linux-gnu' and build['profile'] == 'release', 'binary metadata differs')
            require(tree(user) == before, 'stateless flags changed synthetic user state')
            receipt['components'][component] = {'output': str(output), 'executable': str(actual),
                                                 'sha256': sha(actual), 'mode': 0o555, 'build': build}
        log = (proof / 'supervisor.log').open('wb')
        supervisor = subprocess.Popen(prefix + ['serve'], env=env, cwd=user, stdin=subprocess.DEVNULL,
                                      stdout=log, stderr=subprocess.STDOUT)
        log.close()
        epoch, _ = registered(0)
        ui = Runtime(prefix + ['attach'], env, user)
        ui.wait('FLERE')
        same, rows = registered(1)
        require(same == epoch and rows[0]['pid'] == ui.child.pid, 'sole local frontend differs')
        expected = {'kind': 'manager', 'executable': receipt['components']['flere']['executable'],
                    'sha256': receipt['components']['flere']['sha256'], 'attempt': None,
                    'guidance': 'Remote Nix: ' + DETAIL}
        request = ['update-coordinated-v1', str(ui.child.pid)]
        owner = app(request + ['["update-ownership-v1"]'])
        require(owner == {'schema_version': 1, 'epoch': epoch, 'pid': supervisor.pid,
                          'frontend': expected, 'supervisor': expected}, 'core Nix owner report differs')
        before = installations()
        missing = str(root / 'MUST_NOT_BE_READ')
        stdout, stderr = run(prefix + request + [json.dumps(['update-prepare-v1', missing])], ok=(1,))
        require(not stdout and stderr.decode().strip() == 'flere: Remote Nix: ' + DETAIL, 'core prepare did not refuse by manager owner')
        require(installations() == before, 'core preparation staged or installed files')
        ui.send(b'\0K')
        ui.wait('Installed with Nix', 'commands are not run by Flere')
        ui.send(b'MUST_NOT_BECOME_SOURCE\r')
        # An observed redraw after Escape, then normal FIFO detach, acknowledges
        # all preceding modal input before checking that it stayed ignored.
        ui.drain()
        mark = len(ui.output)
        ui.send(b'\x1b')
        ui.wait(start=mark, fresh=True)
        # Closing Update preserves NAV; leave it before the shared detach toggle.
        mark = len(ui.output)
        ui.send(b'\x1b')
        ui.wait(start=mark, fresh=True)
        receipt['runtime']['core_owner'] = owner
        receipt['runtime']['core_ui'] = ui.finish()
        require(installations() == before, 'core modal created installer state')
        require(b'MUST_NOT_BECOME_SOURCE' not in ui.output, 'manager modal accepted source text')
        (proof / 'core-ui.pty').write_bytes(ui.output)
        ui = None
        require(registered(0)[0] == epoch, 'core detach changed empty supervisor')
        # This is an argv-validating local fixture, never an SSH connection.
        ssh = user / 'fixture-ssh'
        bridge_record = user / 'bridge.json'
        ssh.write_text('#!/usr/bin/python3\nimport json,os,pathlib,shlex,sys\n'
                       + 'assert sys.argv[1:4] == ["-T","--","fixture-host"]\n'
                       + 'args=shlex.split(sys.argv[4])\n'
                       + 'assert args == ' + repr(['exec', str(core), '--state', str(state), '_bridge']) + '\n'
                       + 'pathlib.Path(' + repr(str(bridge_record)) + ').write_text(json.dumps({"pid":os.getpid()}))\n'
                       + 'os.execv(args[1],args[1:])\n')
        ssh.chmod(0o700)
        ui = Runtime([str(companion), 'fixture-host', '--remote', str(core), '--state', str(state), '--ssh', str(ssh)], env, user)
        ui.wait('FLERE')
        same, rows = registered(1)
        require(same == epoch and len(rows) == 1, 'companion bridge identity changed')
        bridge = json.loads(bridge_record.read_text())['pid']
        require(rows[0]['pid'] == bridge, 'bridge registration differs from owned fixture')
        status = Path(f'/proc/{bridge}/status').read_text()
        require(re.search(r'^PPid:\s+' + str(ui.child.pid) + r'$', status, re.M), 'bridge is not the companion child')
        identity = Path(f'/proc/{bridge}/stat').read_bytes()
        candidate_fd = os.pidfd_open(bridge)
        try:
            require(Path(f'/proc/{bridge}/stat').read_bytes().rsplit(b') ', 1)[1].split()[19] == identity.rsplit(b') ', 1)[1].split()[19]
                    and re.search(r'^PPid:\s+' + str(ui.child.pid) + r'$', Path(f'/proc/{bridge}/status').read_text(), re.M)
                    and Path(f'/proc/{bridge}/exe').resolve() == outputs['flere'] / 'bin/flere'
                    and not select.select([candidate_fd], [], [], 0)[0], 'owned bridge changed during handle capture')
        except BaseException:
            os.close(candidate_fd)
            raise
        bridge_fd = candidate_fd
        require(Path(f'/proc/{ui.child.pid}/exe').resolve() == outputs['flere-connect'] / 'bin/flere-connect', 'companion executable differs')
        before = installations()
        ui.send(b'\0K')
        ui.wait('LOCAL', 'Update Flere + companion', 'Enter prepare')
        ui.send((missing + '\t' + missing + '-companion\r').encode())
        ui.wait('Local Nix:', 'Update the owning Nix configuration', 'Tab field · Enter prepare')
        require(b'Nix ownership could not be verified' not in ui.output, 'unverified ownership is not a pass')
        require(installations() == before, 'companion preparation staged or installed files')
        require(registered(1)[0] == epoch, 'companion prepare changed empty runtime')
        receipt['runtime']['companion_owner_guidance'] = 'Local Nix: ' + DETAIL
        receipt['runtime']['companion_pid'] = ui.child.pid
        receipt['runtime']['bridge_pid'] = bridge
        # Match the existing coordinated-owner test: cancel the local form,
        # leave its preserved NAV mode, then detach through the normal command.
        mark = len(ui.output)
        ui.send(b'\x03')
        ui.wait('Update cancelled', start=mark, fresh=True)
        mark = len(ui.output)
        ui.send(b'\x1b')
        ui.wait(start=mark, fresh=True)
        receipt['runtime']['companion_ui'] = ui.finish()
        (proof / 'companion-ui.pty').write_bytes(ui.output)
        ui = None
        require(select.select([bridge_fd], [], [], 3)[0], 'owned bridge remained after companion exit')
        os.close(bridge_fd)
        bridge_fd = None
        require(registered(0)[0] == epoch, 'companion detach changed empty supervisor')
        for component, output in outputs.items():
            require(sha(output / 'bin' / component) == receipt['components'][component]['sha256'], 'installed bytes changed')
        receipt['runtime'].update(empty_workspaces=True, core_prepare_blocked_before_staging=True,
                                  companion_prepare_blocked_before_staging=True,
                                  rendered_guidance_bytes_checked=True, physical_terminal_checked=False)
        receipt['status'] = 'passed'
    except BaseException as error:
        receipt['status'] = 'failed'
        receipt['error'] = {'type': type(error).__name__, 'message': str(error)}
        raise
    finally:
        cleanup = []
        cleanup_errors = []
        try:
            if ui is not None:
                cleanup.append(ui.close())
                (proof / 'failed-ui.pty').write_bytes(ui.output)
        except BaseException as error:
            cleanup_errors.append('UI cleanup: ' + str(error))
        try:
            if bridge_fd is not None:
                if not select.select([bridge_fd], [], [], 3)[0]:
                    signal.pidfd_send_signal(bridge_fd, signal.SIGKILL)
                require(select.select([bridge_fd], [], [], 3)[0], 'owned bridge did not exit')
                os.close(bridge_fd)
        except BaseException as error:
            cleanup_errors.append('bridge cleanup: ' + str(error))
        try:
            if supervisor is not None:
                if supervisor.poll() is None:
                    supervisor.terminate()
                    try:
                        supervisor.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        supervisor.kill()
                        supervisor.wait(timeout=3)
                cleanup.append({'supervisor_pid': supervisor.pid, 'exit': supervisor.returncode})
        except BaseException as error:
            cleanup_errors.append('supervisor cleanup: ' + str(error))
        if profile_installed:
            try:
                preserved = [state, user / 'config', user / 'state', user / 'data', user / '.local/bin', user / 'cache/flere']
                saved_state = {str(path): tree(path) for path in preserved}
                run([NIX + '/nix-env', '--profile', str(profile), '--uninstall', 'flere', 'flere-connect'], timeout=30)
                require(not run([NIX + '/nix-env', '--profile', str(profile), '--query', '--out-path'], timeout=10)[0], 'profile removal incomplete')
                require(not core.exists() and not companion.exists(), 'profile commands remain installed')
                require({str(path): tree(path) for path in preserved} == saved_state, 'profile removal changed synthetic Flere/user state')
                receipt['profile_removed'] = True
            except BaseException as error:
                receipt['status'] = 'failed'
                receipt['cleanup_error'] = str(error)
        receipt['cleanup'] = cleanup
        try:
            require(sha(sentinel) == protected, 'synthetic unrelated state changed')
        except BaseException as error:
            cleanup_errors.append('synthetic state cleanup: ' + str(error))
        if cleanup_errors:
            receipt['status'] = 'failed'
            receipt['cleanup_errors'] = cleanup_errors
        (proof / 'owner-receipt.json').write_text(json.dumps(receipt, indent=2, sort_keys=True) + '\n')
    require(receipt['status'] == 'passed', 'owner acceptance failed')


if __name__ == '__main__':
    def terminate(_signal, _frame):
        raise RuntimeError('owned validation received termination')
    signal.signal(signal.SIGTERM, terminate)
    main()
