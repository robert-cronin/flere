#!/usr/bin/env python3
"""Opt-in native Windows Scoop refusal fixture: owned console and passive local peer.

No remote core, actual SSH, chat, clipboard, installation or product test API.
The caller owns ordinary package installation/removal and its preservation checks.
"""
import argparse
import ctypes
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import secrets
import shutil
import struct
import subprocess
import sys
import time

VERSION = b"flere-remote-v6"
PROBE = b"coordinated-update-owner-v1?"
CAPABILITY = b"coordinated-update-owner-v1"
GUIDANCE = ("Local Scoop: This companion is installed by Scoop. Use Scoop with the next reviewed Flere "
            "manifest/package to upgrade, or remove it with Scoop, then reopen the companion. In-app Apply is disabled.")
HEADER = "LOCAL · Update Flere + companion"
FOOTER = "Tab field · Enter prepare · Ctrl+U clear · Esc cancel"
CANCELLED = "Update cancelled; installed components retained"
REMOTE_ARGS = ["-T", "--", "fixture-host", "exec 'fixture-core' --state 'fixture-state' _bridge"]
MAX_PACKET, MAX_RECORD = 65536, 512 * 1024


def require(ok, message):
    if not ok:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def save(path, value):
    data = (json.dumps(value, indent=2, ensure_ascii=True) + "\n").encode()
    require(len(data) <= MAX_RECORD, "UI record bound")
    temporary = path.with_suffix(".tmp")
    temporary.write_bytes(data)
    temporary.replace(path)


def load(path):
    require(path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_RECORD, "UI input/receipt bound")
    return json.loads(path.read_bytes())


def hosted(environment):
    for key, value in {"GITHUB_ACTIONS":"true", "FLERE_RUNNER_ENVIRONMENT":"github-hosted", "CI":"true",
                       "GITHUB_EVENT_NAME":"workflow_dispatch", "GITHUB_REPOSITORY":"robert-cronin/flere",
                       "GITHUB_REF":"refs/heads/main", "RUNNER_OS":"Windows", "RUNNER_ARCH":"X64"}.items():
        require(environment.get(key) == value, "UI fixture requires hosted manual Windows main: " + key)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64"), "native Windows AMD64 required")
    require(not environment.get("FLERE_COORDINATED_UPDATE") and not environment.get("FLERE_CONNECT_UPDATE_ACK"),
            "resumed-update environment is outside the fresh UI fixture")


def enabled(selected, requested):
    require(type(requested) is bool, "UI flag must be boolean")
    if requested:
        require(selected["name"] == "candidate-scoop-bucket-owner" and selected["owner_check"] is True
                and selected["product"] == "4f3b693914dd291da5d7f103e5166f37f8691d1f", "UI fixture requires reviewed4f3 Scoop bucket candidate")
    return requested


def packet(tag, ident, body=b""):
    require(type(ident) is int and 0 <= ident < 2**64 and 0 <= tag < 256 and len(body)+9 <= MAX_PACKET, "frame bound")
    return struct.pack(">IBQ", len(body)+9, tag, ident) + body


def read_exact(stream, count):
    result = bytearray()
    while len(result) < count:
        part = stream.read(count-len(result))
        require(part, "peer input closed before cancellation")
        result.extend(part)
    return bytes(result)


def read_packet(stream):
    size, = struct.unpack(">I", read_exact(stream, 4))
    require(9 <= size <= MAX_PACKET, "invalid frame length")
    body = read_exact(stream, size)
    return body[0], int.from_bytes(body[1:9], "big"), body[9:]


class Peer:
    """Only passive handshake/probes and one exact cancellation; no RPC handler."""
    def __init__(self, token):
        require(re.fullmatch(r"[0-9a-f]{32}", token), "challenge differs")
        self.hello = VERSION + token.encode()
        self.request = int(token[:15], 16) + 1
        self.handshake = self.offered = self.cancelled = self.cancel_resize_drained = False
        self.dimensions = None
        self.rows = []

    def receive(self, tag, ident, body):
        require(len(self.rows) < 64, "peer packet count exceeded")
        self.rows.append({"tag":tag, "id":ident, "bytes":len(body), "sha256":digest(body)})
        require(tag not in (42, 43, 45, 46, 47, 48), "update RPC/activation/transfer reached peer")
        if not self.handshake:
            require(tag == 1 and ident == 0 and body[:-4] == self.hello and len(body) == len(self.hello)+4,
                    "handshake challenge mismatch")
            width, height = struct.unpack(">HH", body[-4:])
            require(80 <= width <= 320 and 24 <= height <= 106, "console dimensions outside fixture bound")
            self.dimensions = (width, height)
            self.handshake = True
        elif self.cancelled:
            require(not self.cancel_resize_drained and tag == 3 and ident == 0
                    and body == struct.pack(">HH", *self.dimensions), "post-cancel resize mismatch")
            self.cancel_resize_drained = True
        elif tag == 8 and ident == 0 and body == PROBE:
            require(not self.offered, "ownership probe repeated")
            self.offered = True
            return packet(9, 0, CAPABILITY) + packet(41, self.request)
        elif tag == 44:
            require(self.offered and not self.cancelled and ident == self.request
                    and json.loads(body) == {"message":CANCELLED}, "cancellation result mismatch")
            self.cancelled = True
        else:
            require(ident == 0 and tag in (2, 3, 8, 9) and len(body) <= 1024, "unexpected peer packet")
            if tag == 3:
                require(len(body) == 4, "resize differs")
                width, height = struct.unpack(">HH", body)
                require(80 <= width <= 320 and 24 <= height <= 106, "resize dimensions outside fixture bound")
                self.dimensions = (width, height)
        return b""


def peer(config, arguments):
    require(arguments == REMOTE_ARGS, "local peer arguments differ; no command is executed")
    state = Peer(config["token"])
    report = {"schema":"flere-owner-ui-peer-v1", "pid":os.getpid(), "request":state.request, "status":"running"}
    out = Path(config["directory"])/"peer.json"
    try:
        sys.stdout.buffer.write(packet(1, 0, state.hello)); sys.stdout.buffer.flush()
        # Production cancel queues RESULT44 then RESIZE3. Drain both before EOF.
        while not state.cancel_resize_drained:
            response = state.receive(*read_packet(sys.stdin.buffer))
            report.update(handshake=state.handshake, offered=state.offered, cancelled=state.cancelled,
                          cancel_resize_drained=state.cancel_resize_drained, dimensions=state.dimensions, packets=state.rows)
            save(out, report)
            if response:
                sys.stdout.buffer.write(response); sys.stdout.buffer.flush()
        report["status"] = "passed"
    except BaseException as error:
        report["error"] = str(error); report["status"] = "failed"
        raise
    finally:
        report["packets"] = state.rows
        save(out, report)


def chunks(text, width):
    result = [""]
    for char in text:
        if result[-1] and len((result[-1]+char).encode()) > width:
            result.append("")
        result[-1] += char
    return result


def form(rows, refused):
    """Match real header/status/footer geometry, including hard-wrap boundaries."""
    if len(rows) < 24 or not 80 <= len(rows[0]) <= 320:
        return False
    width = len(rows[0])-4
    status = chunks(GUIDANCE if refused else "fixture-host · Enter prepares both packages", width)
    expected = [HEADER, "> Remote core: ", "  Local companion: ",
                "Package path / HTTPS manifest; blank = saved source", "Use --rollback for retained compatible packages",
                *status, FOOTER]
    return all(rows[index+1][2:].rstrip() == text.rstrip() and rows[index+1][:2] == "  "
               for index, text in enumerate(expected))


def tree(root):
    rows = {}
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), "UI profile symlink")
        if path.is_file():
            require(path.stat().st_size <= 2*1024*1024 and len(rows) < 128, "UI profile bound")
            rows[path.relative_to(root).as_posix()] = {"bytes":path.stat().st_size, "sha256":digest(path.read_bytes())}
        else:
            require(path.is_dir(), "unexpected UI profile entry")
    require(sum(row["bytes"] for row in rows.values()) <= 4*1024*1024, "UI profile total bound")
    return rows


def preservation(before, after):
    require(all(after.get(key) == value for key, value in before.items()), "UI changed seeded/initialized files")
    added = set(after)-set(before)
    require(all(key.startswith(("AppData/Local/flere-connect/", "AppData/Local/Flere/logs/")) for key in added),
            "UI wrote outside its connection/diagnostic records")


def no_stage(home):
    require(not os.path.lexists(home/"AppData/Local/Flere/install"), "local installer staging observed")


def console_stream(fd, name):
    require(name in ("CONIN$", "CONOUT$"), "unknown owned console stream")
    # Explicit raw FileIO avoids os.fdopen's automatic _WindowsConsoleIO wrapper:
    # CPython3.12 rejects reading CONOUT$ and borrows integer console fds.
    return io.FileIO(fd, "rb" if name == "CONIN$" else "wb", closefd=True)


class Console:
    """Win32 x64 console ABI; owned CONIN$/CONOUT$ handles, never a user console."""
    def __init__(self):
        from ctypes import wintypes as w
        import msvcrt
        class Coord(ctypes.Structure):
            _fields_ = [("x", w.SHORT), ("y", w.SHORT)]
        class Rect(ctypes.Structure):
            _fields_ = [("left", w.SHORT), ("top", w.SHORT), ("right", w.SHORT), ("bottom", w.SHORT)]
        class Info(ctypes.Structure):
            _fields_ = [("size", Coord), ("cursor", Coord), ("attributes", w.WORD), ("window", Rect), ("maximum", Coord)]
        class Key(ctypes.Structure):
            _fields_ = [("down", w.BOOL), ("repeat", w.WORD), ("vk", w.WORD), ("scan", w.WORD), ("char", w.WCHAR), ("control", w.DWORD)]
        class Event(ctypes.Union):
            _fields_ = [("key", Key), ("padding", w.DWORD*4)]
        class Input(ctypes.Structure):
            _fields_ = [("kind", w.WORD), ("event", Event)]
        require(ctypes.sizeof(Info) == 22 and ctypes.sizeof(Key) == 16 and ctypes.sizeof(Input) == 20, "console ABI differs")
        self.Coord, self.Info, self.Input = Coord, Info, Input
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        signatures = {
            "CreateFileW":([w.LPCWSTR,w.DWORD,w.DWORD,w.LPVOID,w.DWORD,w.DWORD,w.HANDLE],w.HANDLE),
            "CloseHandle":([w.HANDLE],w.BOOL),
            "GetConsoleMode":([w.HANDLE,ctypes.POINTER(w.DWORD)],w.BOOL),
            "GetConsoleCP":([],w.UINT), "GetConsoleOutputCP":([],w.UINT),
            "SetConsoleCP":([w.UINT],w.BOOL), "SetConsoleOutputCP":([w.UINT],w.BOOL),
            "GetConsoleScreenBufferInfo":([w.HANDLE,ctypes.POINTER(Info)],w.BOOL),
            "ReadConsoleOutputCharacterW":([w.HANDLE,w.LPWSTR,w.DWORD,Coord,ctypes.POINTER(w.DWORD)],w.BOOL),
            "WriteConsoleInputW":([w.HANDLE,ctypes.POINTER(Input),w.DWORD,ctypes.POINTER(w.DWORD)],w.BOOL),
            "GetConsoleProcessList":([ctypes.POINTER(w.DWORD),w.DWORD],w.DWORD),
        }
        for name, (arguments, result) in signatures.items():
            function=getattr(self.api,name); function.argtypes=arguments; function.restype=result
        self.files=[]; self.codepages_before=None
        try:
            for name in ("CONIN$", "CONOUT$"):
                handle=self.api.CreateFileW(name,0xC0000000,3,None,3,0,None)
                require(handle not in (None,ctypes.c_void_p(-1).value), "owned console open failed")
                # open_osfhandle transfers handle ownership to the file object.
                try:
                    fd=msvcrt.open_osfhandle(handle,os.O_BINARY | (os.O_RDONLY if name == "CONIN$" else os.O_WRONLY))
                except BaseException:
                    self.api.CloseHandle(handle); raise
                try:
                    self.files.append(console_stream(fd,name))
                except BaseException:
                    os.close(fd); raise
            self.input,self.output=[msvcrt.get_osfhandle(value.fileno()) for value in self.files]
            require(self.processes()==[os.getpid()], "console already shared with another process")
            self.codepages_before=self.codepages()
            require(all(self.codepages_before), "console code page query failed")
            require(all((self.api.SetConsoleCP(65001),self.api.SetConsoleOutputCP(65001))), "owned UTF-8 console setup failed")
        except BaseException:
            self.close(); raise

    def processes(self):
        from ctypes import wintypes as w
        pids=(w.DWORD*16)(); count=self.api.GetConsoleProcessList(pids,16)
        require(0<count<=16,"console process inventory bound")
        return sorted(pids[:count])

    def modes(self):
        from ctypes import wintypes as w
        modes=[]
        for handle in (self.input,self.output):
            value=w.DWORD(); require(self.api.GetConsoleMode(handle,ctypes.byref(value)),"console mode query failed")
            modes.append(value.value)
        return modes

    def screen(self):
        from ctypes import wintypes as w
        info=self.Info(); require(self.api.GetConsoleScreenBufferInfo(self.output,ctypes.byref(info)),"console screen query failed")
        width=info.window.right-info.window.left+1; height=info.window.bottom-info.window.top+1
        require(80 <= width <= 320 and 24 <= height <= 106,"owned console dimensions outside bound")
        rows=[]
        for y in range(info.window.top,info.window.bottom+1):
            buf=ctypes.create_unicode_buffer(width); count=w.DWORD()
            require(self.api.ReadConsoleOutputCharacterW(self.output,buf,width,self.Coord(info.window.left,y),ctypes.byref(count))
                    and count.value == width,"console cell read failed")
            rows.append(buf[:width])
        return rows

    def key(self, vk, char):
        from ctypes import wintypes as w
        events=(self.Input*2)()
        for index,down in enumerate((1,0)):
            events[index].kind=1
            key=events[index].event.key; key.down=down; key.repeat=1; key.vk=vk; key.char=char
        count=w.DWORD()
        require(self.api.WriteConsoleInputW(self.input,events,2,ctypes.byref(count)) and count.value == 2,"owned console input failed")

    def codepages(self):
        return [self.api.GetConsoleCP(),self.api.GetConsoleOutputCP()]

    def close(self):
        try:
            if self.codepages_before:
                require(all((self.api.SetConsoleCP(self.codepages_before[0]),self.api.SetConsoleOutputCP(self.codepages_before[1]))),
                        "owned console code page restoration failed")
                require(self.codepages()==self.codepages_before,"owned console code pages not restored")
        finally:
            for stream in self.files:
                stream.close()


def stop_owned(proc):
    if proc.poll() is None:
        result=subprocess.run([str(Path(os.environ["SystemRoot"])/"System32/taskkill.exe"),"/PID",str(proc.pid),"/T","/F"],
                              stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10)
        require(result.returncode == 0,"owned console process tree cleanup failed")
    proc.wait(timeout=5)


def console_child(config):
    hosted(os.environ)
    directory=Path(config["directory"]); home=Path(config["home"])
    result={"schema":"flere-windows-owner-ui-v1","alias":config["alias"],"pid":os.getpid(),"status":"running", "forced_stop":False,
            "product":config["product"],"build":config["build"],"payload_sha256":config["payload_sha256"],"frames":{}}
    console=None; proc=None
    try:
        no_stage(home)
        console=Console(); result["modes_before"]=console.modes()
        result["codepages_before"]=console.codepages_before; result["codepages_during"]=console.codepages()
        result["dimensions"]=[len(console.screen()[0]),len(console.screen())]
        actual=Path(config["executable"])
        require(actual.is_file() and actual.stat().st_size == config["payload_bytes"]
                and digest(actual.read_bytes()) == config["payload_sha256"],"installed executable changed")
        argv=[config["shim"],"fixture-host","--remote","fixture-core","--state","fixture-state","--ssh",str(directory/"peer.cmd")]
        proc=subprocess.Popen(argv,cwd=home,stdin=console.files[0],stdout=console.files[1],stderr=console.files[1])
        result["installed_alias_pid"]=proc.pid
        deadline=time.monotonic()+30
        for phase,refused in (("initial",False),("refused",True)):
            stable=0
            while True:
                no_stage(home)
                require(proc.poll() is None,"installed companion exited before "+phase)
                require(time.monotonic()<deadline,"console form timed out: "+phase)
                rows=console.screen(); stable=stable+1 if form(rows,refused) else 0
                if stable>=2:
                    result["frames"][phase]=rows; save(directory/"console.json",result); break
                time.sleep(0.02)
            console.key(13,"\r") if not refused else console.key(27,"\x1b")
        proc.wait(timeout=max(0.1,deadline-time.monotonic()))
        require(proc.returncode == 0,"installed companion/peer did not exit normally")
        observed=load(directory/"peer.json")
        require(observed["status"] == "passed" and observed["handshake"] and observed["offered"] and observed["cancelled"]
                and observed["cancel_resize_drained"] and [row["tag"] for row in observed["packets"][-2:]] == [44,3]
                and observed["request"] == Peer(config["token"]).request
                and all(row["tag"] not in (42,43,45,46,47,48) for row in observed["packets"]),"peer refusal proof incomplete")
        result["peer"]=observed
        result["modes_after"]=console.modes()
        require(result["modes_after"]==result["modes_before"],"console modes not restored")
        no_stage(home)
        require(actual.stat().st_size == config["payload_bytes"] and digest(actual.read_bytes()) == config["payload_sha256"],"payload changed during UI")
        result.update(status="passed",exit=proc.returncode,no_staging=True)
    except BaseException as error:
        result.update(status="failed",error=str(error))
        raise
    finally:
        if console:
            try:
                if "refused" not in result["frames"]:
                    result["frames"]["last"]=console.screen()
            except BaseException as error:
                result["screen_error"]=str(error)
            try:
                if proc and proc.poll() is None:
                    result["forced_stop"]=True; stop_owned(proc)
                result["console_pids_final"]=console.processes()
                require(result["console_pids_final"]==[os.getpid()],"owned console has remaining attached child")
                result["modes_final"]=console.modes()
            except BaseException as error:
                result["cleanup_error"]=str(error); result["status"]="failed"
            finally:
                try:
                    console.close(); result["codepages_restored"]=console.codepages()
                except BaseException as error:
                    result["cleanup_error"]=str(error); result["status"]="failed"
        save(directory/"console.json",result)


def check_pair(run, root, selected, manifest):
    hosted(os.environ); enabled(selected,True)
    deadline=min(run.deadline,time.monotonic()+90)
    home_root=run.work/"owner-ui"; home_root.mkdir()
    results=[]
    for alias in ("flere","flere-connect"):
        directory=home_root/alias; directory.mkdir(); home=directory/"home"; home.mkdir()
        environment=dict(run.env,HOME=str(home),USERPROFILE=str(home))
        locations={"APPDATA":"AppData/Roaming","LOCALAPPDATA":"AppData/Local","XDG_CONFIG_HOME":"AppData/Roaming",
                   "XDG_DATA_HOME":"AppData/Local","XDG_STATE_HOME":".local/state","XDG_CACHE_HOME":".cache",
                   "TEMP":"temp","TMP":"temp","FLERE_STATE_DIR":".cache/flere/state","SCOOP":"scoop"}
        for key, relative in locations.items():
            path=home/relative; path.mkdir(parents=True,exist_ok=True); environment[key]=str(path)
        (home/".cache/flere/state/sentinel").write_bytes(b"owned Windows UI fixture\n")
        shell=shutil.which("powershell.exe",path=environment["PATH"]); require(shell,"existing PowerShell missing")
        require(run.command(alias+"-ui-initialize",[shell,"-NoProfile","-NonInteractive","-Command","[Console]::Out.Write('flere-ui-baseline')"],
                            env=environment,seconds=min(15,deadline-time.monotonic()),maximum=4096)==b"flere-ui-baseline","UI tool initialization differs")
        before=tree(home); no_stage(home)
        config={"alias":alias,"directory":str(directory),"home":str(home),"token":secrets.token_hex(16),
                "shim":str(root/"shims"/(alias+".exe")),"executable":str((root/"apps/flere"/selected["version"]/(alias+".exe")).resolve()),
                "product":selected["product"],"build":manifest["build"],"payload_sha256":selected["payload_sha"],"payload_bytes":manifest["payload"]["bytes"]}
        save(directory/"config.json",config)
        script=str(Path(__file__).resolve()); python=str(Path(sys.executable).resolve()); config_path=str(directory/"config.json")
        require(not any(char in python+script+config_path for char in '\r\n"%!'),"unsupported fixture wrapper path")
        # All forwarded arguments are the fixed literal REMOTE_ARGS, checked by peer().
        (directory/"peer.cmd").write_bytes((f'@echo off\r\n"{python}" -I -B "{script}" --peer "{config_path}" -- %*\r\nexit /b %errorlevel%\r\n').encode("utf-8"))
        stdout=run.proof/"logs"/(alias+"-ui-console.log"); stderr=run.proof/"logs"/(alias+"-ui-console-stderr.log")
        row={"alias":alias,"status":"running","forced_stop":False,"config":config,"profile_before":before}
        proc=None; start=time.monotonic()
        try:
            with stdout.open("xb") as out,stderr.open("xb") as err:
                require(time.monotonic()<deadline,"90-second console pair deadline exhausted before start")
                proc=subprocess.Popen([python,"-I","-B",script,"--console",config_path],cwd=directory,env=environment,
                                      stdin=subprocess.DEVNULL,stdout=out,stderr=err,creationflags=subprocess.CREATE_NEW_CONSOLE)
                row["console_pid"]=proc.pid
                while proc.poll() is None:
                    require(time.monotonic()<deadline,"90-second console pair deadline exceeded")
                    require(stdout.stat().st_size<=65536 and stderr.stat().st_size<=65536,"console diagnostic bound")
                    time.sleep(0.05)
                row["exit"]=proc.returncode
                require(proc.returncode==0,"owned console fixture failed; inspect cells/peer record")
            receipt=load(directory/"console.json")
            require(receipt["status"]=="passed" and not receipt["forced_stop"] and "cleanup_error" not in receipt,"console proof incomplete")
            after=tree(home); preservation(before,after); no_stage(home)
            row.update(status="passed",profile_after=after,console=receipt)
        except BaseException as error:
            row["error"]=str(error); row["status"]="failed"
            raise
        finally:
            if proc and proc.poll() is None:
                row["forced_stop"]=True
                try:
                    stop_owned(proc)
                except BaseException as error:
                    row["cleanup_error"]=str(error)
            for name in ("console","peer"):
                path=directory/(name+".json")
                if path.exists():
                    try:
                        row[name]=load(path)
                    except BaseException as error:
                        row["evidence_error"]=str(error); row["status"]="failed"
            row["seconds"]=round(time.monotonic()-start,3)
            for name,path in (("stdout",stdout),("stderr",stderr)):
                size=path.stat().st_size if path.exists() else 0
                if path.exists():
                    with path.open("rb") as stream: data=stream.read(65536)
                else: data=b""
                row[name]={"observed_bytes":size,"retained_bytes":len(data),"sha256":digest(data)}
                if size>65536:
                    path.write_bytes(data); row["diagnostic_overflow"]=True; row["status"]="failed"
            run.record(alias+"-ui-refusal",row)
        require(row["status"]=="passed" and not row["forced_stop"] and "cleanup_error" not in row,"UI cleanup/evidence failed")
        results.append({"alias":alias,"console_pid":row["console_pid"],"request":row["peer"]["request"],"status":"passed"})
    return results


def self_test():
    import unittest
    from unittest import mock
    class Checks(unittest.TestCase):
        def ready(self):
            state=Peer("a"*32)
            state.receive(1,0,state.hello+struct.pack(">HH",80,25))
            state.receive(8,0,PROBE)
            return state
        def test_directional_raw_streams_own_and_close_their_descriptors(self):
            import tempfile
            cache=Path(os.environ.get("XDG_CACHE_HOME",Path.home()/".cache"))/"flere/tests"
            cache.mkdir(parents=True,exist_ok=True)
            with tempfile.TemporaryDirectory(dir=cache) as folder:
                for name in ("CONIN$", "CONOUT$"):
                    path=Path(folder)/("input.bin" if name=="CONIN$" else "output.bin"); path.write_bytes(b"old")
                    fd=os.open(path,os.O_RDWR)
                    try:
                        stream=console_stream(fd,name)
                    except BaseException:
                        os.close(fd); raise
                    with stream:
                        self.assertIs(type(stream),io.FileIO); self.assertTrue(stream.closefd)
                        self.assertEqual(stream.fileno(),fd)
                        self.assertEqual((stream.readable(),stream.writable()),(name=="CONIN$",name=="CONOUT$"))
                        if name=="CONIN$":
                            self.assertEqual(stream.read(),b"old")
                            with self.assertRaises(io.UnsupportedOperation):stream.write(b"bad")
                        else:
                            self.assertEqual(stream.write(b"new"),3)
                            with self.assertRaises(io.UnsupportedOperation):stream.read(1)
                    with self.assertRaises(OSError):os.fstat(fd)
                with self.assertRaises(ValueError):console_stream(-1,"other")

        def test_frame_partial_reads_and_bounds(self):
            class Short(io.BytesIO):
                def read(self,n=-1): return super().read(min(n,2))
            self.assertEqual(read_packet(Short(packet(9,17,b"abc"))),(9,17,b"abc"))
            for raw in (b"",struct.pack(">I",8),struct.pack(">I",65537),packet(9,17,b"abc")[:-1]):
                with self.assertRaises(ValueError): read_packet(io.BytesIO(raw))
        def test_wrapper_separator_preserves_fixed_peer_arguments(self):
            args=parse_args(["--peer","config.json","--",*REMOTE_ARGS])
            self.assertEqual(args.arguments,["--",*REMOTE_ARGS])
            with self.assertRaises(ValueError): peer({"token":"a"*32},["-T","--","other","anything"])

        def test_no_update_rpc_or_transfer_handler(self):
            for tag in (42,43,45,46,47,48,99):
                with self.assertRaises(ValueError): self.ready().receive(tag,1,b"anything")
        def test_exact_challenge_capability_and_cancel_binding(self):
            state=Peer("b"*32)
            with self.assertRaises(ValueError): state.receive(1,0,VERSION+b"c"*32+struct.pack(">HH",80,25))
            state=self.ready()
            for ident,body in ((state.request+1,{"message":CANCELLED}),(state.request,{"message":"prepared"})):
                with self.assertRaises(ValueError): state.receive(44,ident,json.dumps(body).encode())
            state.receive(44,state.request,json.dumps({"message":CANCELLED}).encode()); self.assertTrue(state.cancelled)
        def test_real_peer_loop_drains_exact_resize_before_eof(self):
            import tempfile
            from types import SimpleNamespace
            cache=Path(os.environ.get("XDG_CACHE_HOME",Path.home()/".cache"))/"flere/tests"
            cache.mkdir(parents=True,exist_ok=True)
            state=Peer("d"*32)
            prefix=(packet(1,0,state.hello+struct.pack(">HH",80,25))+packet(8,0,PROBE)
                    +packet(3,0,struct.pack(">HH",100,30))
                    +packet(44,state.request,json.dumps({"message":CANCELLED}).encode()))
            for tail,passed in ((packet(3,0,struct.pack(">HH",100,30)),True), (b"",False),
                                (packet(3,1,struct.pack(">HH",100,30)),False),
                                (packet(3,0,struct.pack(">HH",80,25)),False),(packet(9,0,b"other"),False)):
                with tempfile.TemporaryDirectory(dir=cache) as folder:
                    config={"token":"d"*32,"directory":folder}
                    with mock.patch.object(sys,"stdin",SimpleNamespace(buffer=io.BytesIO(prefix+tail))), \
                         mock.patch.object(sys,"stdout",SimpleNamespace(buffer=io.BytesIO())):
                        if passed:
                            peer(config,REMOTE_ARGS)
                        else:
                            with self.assertRaises(ValueError):peer(config,REMOTE_ARGS)
                    report=load(Path(folder)/"peer.json")
                    self.assertEqual(report["status"],"passed" if passed else "failed")
                    self.assertEqual(report["cancel_resize_drained"],passed)
                    if passed:
                        self.assertEqual([row["tag"] for row in report["packets"][-2:]],[44,3])
                        self.assertEqual(report["dimensions"],[100,30])

        def test_initial_form_is_not_refusal_and_wrap_must_be_complete(self):
            def screen(refused):
                content=[HEADER,"> Remote core: ","  Local companion: ","Package path / HTTPS manifest; blank = saved source",
                         "Use --rollback for retained compatible packages",*chunks(GUIDANCE if refused else "fixture-host · Enter prepares both packages",76),FOOTER]
                return [" "*80]+[("  "+line).ljust(80) for line in content]+[" "*80]*(24-len(content))
            first,done=screen(False),screen(True)
            self.assertTrue(form(first,False)); self.assertFalse(form(first,True)); self.assertTrue(form(done,True))
            for index in (1,6,6+len(chunks(GUIDANCE,76))):
                bad=done.copy();bad[index]=" "*80;self.assertFalse(form(bad,True))
        def test_only_documented_ui_profile_additions(self):
            before={"sentinel":{"sha256":"a","bytes":1}}
            preservation(before,dict(before,**{"AppData/Local/flere-connect/connections.json":{}}))
            for after in ({},dict(before,sentinel={}),dict(before,**{"AppData/Local/Flere/install/pending.json":{}})):
                with self.assertRaises(ValueError): preservation(before,after)
            with mock.patch.object(os.path,"lexists",return_value=True),self.assertRaises(ValueError):no_stage(Path("home"))
        def test_only_fixed_bucket_candidate_opt_in(self):
            selected={"name":"candidate-scoop-bucket-owner","owner_check":True,"product":"4f3b693914dd291da5d7f103e5166f37f8691d1f"}
            self.assertTrue(enabled(selected,True)); self.assertFalse(enabled({},False))
            for change in ({"name":"candidate-scoop-owner"},{"owner_check":False},{"product":"a"*40}):
                with self.assertRaises(ValueError):enabled(dict(selected,**change),True)
        def test_hosted_guard_rejects_other_surfaces(self):
            values={"GITHUB_ACTIONS":"true","FLERE_RUNNER_ENVIRONMENT":"github-hosted","CI":"true","GITHUB_EVENT_NAME":"workflow_dispatch",
                    "GITHUB_REPOSITORY":"robert-cronin/flere","GITHUB_REF":"refs/heads/main","RUNNER_OS":"Windows","RUNNER_ARCH":"X64"}
            with mock.patch.object(os,"name","nt"),mock.patch.object(platform,"machine",return_value="AMD64"):
                hosted(values)
                for key in values:
                    with self.assertRaises(ValueError):hosted(dict(values,**{key:"wrong"}))
                for key in ("FLERE_COORDINATED_UPDATE","FLERE_CONNECT_UPDATE_ACK"):
                    with self.assertRaises(ValueError):hosted(dict(values,**{key:"unexpected"}))
    require(unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Checks)).wasSuccessful(),"UI fixture checks failed")


def parse_args(words=None):
    parser=argparse.ArgumentParser(description=__doc__)
    modes=parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--self-test",action="store_true")
    modes.add_argument("--console",type=Path)
    modes.add_argument("--peer",type=Path)
    parser.add_argument("arguments",nargs=argparse.REMAINDER)
    return parser.parse_args(words)


if __name__ == "__main__":
    args=parse_args()
    if args.self_test:
        self_test()
    else:
        hosted(os.environ)
        config=load(args.console or args.peer)
        if args.console:
            require(not args.arguments,"unexpected console arguments"); console_child(config)
        else:
            require(args.arguments[:1]==["--"],"peer argument separator missing")
            peer(config,args.arguments[1:])
