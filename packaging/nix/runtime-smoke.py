#!/usr/bin/env python3
"""One synthetic installed-core PTY smoke inside the owned emulated NixOS guest."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import selectors
import shlex
import signal
import subprocess
import tempfile
import time


def require(value, message):
    if not value:
        raise RuntimeError(message)


def identity(tab):
    require(tab['kind'] in ('shell', 'editor') and tab['alive'], 'unexpected/stopped tab')
    require(isinstance(tab['id'], int) and tab['id'] > 0 and isinstance(tab['pid'], int) and tab['pid'] > 1,
            'invalid session PID/id')
    require(re.fullmatch('[0-9a-f]{32}', tab['run']), 'invalid session run')
    return {key: tab[key] for key in ('id', 'run', 'pid', 'kind', 'path')}


def workspace(snapshot, epoch):
    require(snapshot['protocol'] == 4 and snapshot['epoch'] == epoch, 'runtime epoch/protocol changed')
    require(len(snapshot['workspaces']) == 1, 'expected one synthetic workspace')
    row = snapshot['workspaces'][0]
    require(snapshot['active_workspace'] == row['id'], 'selected workspace changed')
    require(all(t['kind'] in ('shell', 'editor') for t in row['tabs']), 'native tab started')
    return row


def main(args):
    require(os.getuid() != 0 and os.uname().machine == 'x86_64', 'ordinary x86_64 guest user required')
    os_release = Path('/etc/os-release').read_text()
    require(re.search(r'^ID="?nixos"?$', os_release, re.M), 'actual NixOS guest required')
    require(Path.home() == Path('/home/tester'), 'dedicated guest home required')
    require(re.fullmatch('[0-9]+-[0-9]+-[0-9a-f]{40}', args.run_identity), 'invalid workflow identity')
    spec = importlib.util.spec_from_file_location('flere_owner_runtime', args.runtime_helper)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    Runtime = module.Runtime  # Reuse the already-reviewed bounded PTY/normal-detach helper.
    cache = Path.home()/'.cache/flere/tmp'
    cache.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='r-', dir=cache))
    home, repo, state = root/'h', root/'p', root/'s'
    home.mkdir(mode=0o700)
    repo.mkdir(mode=0o700)
    env = {'HOME':str(home), 'PATH':':'.join(str(Path(v).parent) for v in (args.git,args.shell,args.editor)) + ':/run/current-system/sw/bin',
           'SHELL':args.shell, 'VISUAL':shlex.join([args.editor,'-Nu','NONE','-n','--noplugin']),
           'TERM':'xterm-256color', 'LC_ALL':'C.UTF-8', 'ENV':'', 'BASH_ENV':'', 'PS1':'$ '}
    for key, name in [('XDG_CONFIG_HOME','config'),('XDG_CACHE_HOME','cache'),('XDG_DATA_HOME','data'),
                      ('XDG_STATE_HOME','state'),('TMPDIR','tmp')]:
        directory = home/name
        directory.mkdir(mode=0o700)
        env[key] = str(directory)
    sentinel = home/'config/unrelated'
    sentinel.write_bytes(b'owned synthetic state\n')
    output = Path(args.output)
    require(output.parent == cache and not output.exists(), 'new exact home-cache receipt required')
    receipt = {'schema':'flere-nixos-tcg-runtime-v1','status':'running','execution':'TCG/emulated NixOS',
               'run_identity':args.run_identity,'source_commit':'2d52985845ed322b1c6c0f3018eaedf38d6bcead',
               'root':str(root),'kernel':os.uname().release,'system':str(Path('/run/current-system').resolve()),
               'steps':[],'cleanup':[],'physical_graphics_clipboard_or_external_ssh':False}
    prefix = [args.core,'--state',str(state)]
    supervisor = ui = None
    handles = []

    def run(argv, ok=(0,), timeout=20):
        child = subprocess.Popen(argv, env=env, cwd=repo, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        streams = selectors.DefaultSelector()
        captured = [bytearray(), bytearray()]
        streams.register(child.stdout, selectors.EVENT_READ, 0)
        streams.register(child.stderr, selectors.EVENT_READ, 1)
        until = time.monotonic()+timeout
        try:
            while streams.get_map():
                require(time.monotonic() < until, 'bounded CLI deadline: '+str(argv[0]))
                if ui:
                    ui.drain()
                for event, _ in streams.select(0.05):
                    data = os.read(event.fileobj.fileno(), 65536)
                    if not data:
                        streams.unregister(event.fileobj)
                    else:
                        captured[event.data].extend(data)
                        require(sum(map(len,captured)) <= 256*1024, 'bounded CLI output exceeded')
            child.wait(timeout=3)
        finally:
            streams.close()
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
            child.stdout.close()
            child.stderr.close()
        stdout, stderr = map(bytes,captured)
        require(child.returncode in ok, 'CLI failed: '+repr(argv)+' '+repr(stderr[-2048:]))
        return stdout, child.returncode

    def app(argv):
        return json.loads(run(prefix+argv)[0])

    def wait(check, label):
        until = time.monotonic()+30
        while time.monotonic() < until:
            require(supervisor.poll() is None, 'owned supervisor exited')
            if ui:
                ui.drain()
                require(ui.child.poll() is None, 'owned frontend exited')
            value = check()
            if value:
                return value
            time.sleep(0.05)
        raise RuntimeError('condition deadline: '+label)

    def current():
        return workspace(app(['list']),epoch)

    def capture(tab):
        return run(prefix+['capture','--session',str(tab['id']),'--run',tab['run'],'--lines','80'])[0]

    def track(tab):
        pid = tab['pid']
        before = Path(f'/proc/{pid}/stat').read_bytes().rsplit(b') ',1)[1].split()
        require(int(before[1]) == supervisor.pid, 'session is not the owned supervisor child')
        fd = os.pidfd_open(pid)
        handles.append(fd)
        after = Path(f'/proc/{pid}/stat').read_bytes().rsplit(b') ',1)[1].split()
        require(before[19] == after[19] and before[1] == after[1] and not select.select([fd],[],[],0)[0],
                'session changed during owned-handle capture')
        return fd

    def registered(count):
        status = app(['build-status'])
        server, front = status['supervisor'], status['frontends']
        require(server['status'] == 'known' and server['pid'] == supervisor.pid and server['epoch'] == epoch,
                'supervisor identity changed')
        require(server['build'] == build and status['comparison'] == 'same_build', 'supervisor build changed')
        require(front['status'] == 'known' and front['untracked'] == 0, 'untracked frontend')
        rows = front['tracked']
        if len(rows) != count:
            return False
        require(not rows or (len(rows)==1 and ui and rows[0]['pid']==ui.child.pid and rows[0]['build']==build),
                'frontend registration identity changed')
        return True

    try:
        binary = Path(args.core).resolve(strict=True)
        require(str(binary).startswith('/nix/store/') and binary.stat().st_uid == 0, 'declared Nix core required')
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        build = json.loads(run([args.core,'--build-info'])[0])
        require(build['component']=='flere' and build['package_version']=='0.3.4'
                and build['target']=='x86_64-unknown-linux-gnu' and build['profile']=='release', 'core metadata differs')
        receipt['core'] = {'path':str(binary),'bytes':binary.stat().st_size,'sha256':digest,'build':build}
        file = repo/"--strange ' file.txt"
        file.write_text('starting line\n')
        run([args.git,'init','-q','-b','main'])
        run([args.git,'add','--',file.name])
        run([args.git,'-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-qm','base'])
        index = (repo/'.git/index').read_bytes()
        with (root/'supervisor.log').open('wb') as log:
            supervisor = subprocess.Popen(prefix+['serve'],env=env,cwd=repo,stdin=subprocess.DEVNULL,
                                          stdout=log,stderr=subprocess.STDOUT)
        wait(lambda: run(prefix+['list'],ok=(0,1))[1]==0, 'empty supervisor ready')
        empty = app(['list'])
        require(empty['protocol']==4 and empty['workspaces']==[], 'startup created work')
        epoch = empty['epoch']
        wait(lambda: registered(0), 'empty supervisor identity')
        run(prefix+['new','--cwd',str(repo),'--name','NixOS smoke'])
        row = current()
        require(len(row['tabs'])==1, 'one shell required')
        shell = identity(row['tabs'][0])
        require(shell['kind']=='shell' and row['selected_tab']==shell['id'], 'shell selection differs')
        shell_fd = track(shell)
        receipt['before'] = {'supervisor_pid':supervisor.pid,'epoch':epoch,'workspace':row['id'],'shell':shell}
        ui = Runtime(prefix+['attach'],env,repo)
        ui.wait('FLERE')
        wait(lambda: registered(1), 'first frontend identity')
        ui.send(b"printf 'SHELL_%s\\n' NIXOS\r")
        wait(lambda: b'SHELL_NIXOS' in capture(shell), 'shell execution through real frontend')
        receipt['steps'].append('real frontend shell sentinel')
        draft_file = repo/'draft-proof'
        ui.send(b"printf 'ONCE\\n' >> draft-proof; printf 'DRAFT_%s\\n' ONCE")
        wait(lambda: b'draft-proof' in capture(shell), 'unsubmitted draft delivered')
        require(not draft_file.exists(), 'draft executed prematurely')
        ui.drain()
        editor_mark = len(ui.output)
        run(prefix+['open-file',str(row['id']),str(file)])
        def editor_ready():
            tabs = current()['tabs']
            editors = [t for t in tabs if t['kind']=='editor']
            return identity(editors[0]) if len(tabs)==2 and len(editors)==1 else False
        editor = wait(editor_ready,'configured editor tab')
        require(editor['path']==str(file) and editor['id']!=shell['id'], 'editor canonical target differs')
        editor_fd = track(editor)
        wait(lambda: b'starting line' in capture(editor), 'real Vim ready')
        # Real keyboard input must not use CLI --text, which intentionally
        # wraps literal text in bracketed paste when an editor requests it.
        ui.wait('starting line',start=editor_mark,fresh=True)
        selected = current()
        require(selected['selected_tab']==editor['id']
                and identity(next(t for t in selected['tabs'] if t['id']==editor['id']))==editor,
                'editor selection/run/PID differs')
        ui.send(b'ggIedited \x1b:wq\r')
        wait(lambda: len(current()['tabs'])==1 and file.read_text()=='edited starting line\n','editor saved and exited')
        require(identity(current()['tabs'][0])==shell and select.select([editor_fd],[],[],3)[0], 'shell/editor lifetime differs')
        run(prefix+['send','--session',str(editor['id']),'--run',editor['run'],'--text','MUST_NOT_SEND'],ok=(1,))
        require(not draft_file.exists(), 'editor replayed shell draft')
        receipt['steps'].append('configured Vim exact edit, tab exit, stale-run refusal and same shell')
        ui.drain()
        mark = len(ui.output)
        ui.send(b'\0g')
        ui.wait('Working tree',file.name,start=mark,fresh=True)
        prefs = json.loads((state/'ui.json').read_text())
        require(prefs['inspector']=='Git', 'Git inspector preference not acknowledged')
        require((repo/'.git/index').read_bytes()==index and file.read_text()=='edited starting line\n', 'Git view mutated files')
        receipt['steps'].append('installed Git inspector, unchanged index/worktree')
        mark = len(ui.output)
        ui.send(b'\x1b')
        ui.wait(start=mark,fresh=True)
        receipt['first_frontend'] = ui.finish()
        ui = None
        wait(lambda: registered(0),'first frontend detached')
        require(identity(current()['tabs'][0])==shell and current()['selected_tab']==shell['id']
                and not select.select([shell_fd],[],[],0)[0] and not draft_file.exists(), 'detach changed/replayed shell')
        ui = Runtime(prefix+['attach'],env,repo)
        ui.wait('FLERE')
        wait(lambda: registered(1),'second frontend identity')
        require(identity(current()['tabs'][0])==shell and current()['selected_tab']==shell['id']
                and not draft_file.exists(), 'reattach changed/replayed shell')
        ui.send(b'\r')
        wait(lambda: draft_file.exists() and draft_file.read_bytes()==b'ONCE\n'
             and b'DRAFT_ONCE' in capture(shell),'exact retained draft submitted once')
        receipt['after'] = {'supervisor_pid':supervisor.pid,'epoch':epoch,'workspace':current()['id'],
                            'shell':identity(current()['tabs'][0])}
        require(receipt['before']==receipt['after'],'session identities changed')
        receipt['second_frontend'] = ui.finish()
        ui = None
        wait(lambda: registered(0),'final frontend detached')
        run(prefix+['close','--session',str(shell['id']),'--run',shell['run'],'--terminate'])
        require(select.select([shell_fd],[],[],10)[0], 'exact shell survived close')
        wait(lambda: current()['tabs']==[], 'exact shell tab removed')
        run(prefix+['stop','--terminate'])
        require(supervisor.wait(timeout=10)==0,'owned supervisor did not stop normally')
        receipt['steps'].append('detach/reattach preserves exact identities and unreplayed draft; exact cleanup')
        require(hashlib.sha256(binary.read_bytes()).hexdigest()==digest,'installed core changed')
        require(sentinel.read_bytes()==b'owned synthetic state\n','unrelated synthetic state changed')
        receipt['status']='passed'
    except BaseException as error:
        receipt['status']='failed'
        receipt['error']={'type':type(error).__name__,'message':str(error)}
        raise
    finally:
        try:
            if ui:
                receipt['cleanup'].append(ui.close())
            if supervisor and supervisor.poll() is None:
                receipt['status']='failed'
                supervisor.terminate()
                try:
                    supervisor.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    supervisor.kill()
                    supervisor.wait(timeout=3)
            for fd in handles:
                if not select.select([fd],[],[],3)[0]:
                    receipt['status']='failed'
                    signal.pidfd_send_signal(fd,signal.SIGKILL)
                require(select.select([fd],[],[],3)[0],'owned session survived cleanup')
                os.close(fd)
            receipt['owned_sessions_exited']=True
            receipt['supervisor_exit']=supervisor.returncode if supervisor else None
        except BaseException as error:
            receipt['status']='failed'
            receipt['cleanup_error']={'type':type(error).__name__,'message':str(error)}
            raise
        finally:
            output.write_text(json.dumps(receipt,indent=2,sort_keys=True)+'\n')
    require(receipt['status']=='passed','runtime smoke failed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('runtime-helper','core','shell','editor','git','output','run-identity'):
        parser.add_argument('--'+name,required=True)
    def terminate(_signal,_frame):
        raise RuntimeError('owned guest smoke received termination')
    signal.signal(signal.SIGTERM,terminate)
    main(parser.parse_args())
