#!/usr/bin/env python3
"""First Chocolatey install/remove of an exact retained ZIP, on a disposable hosted VM.

This captures manager records; it does NOT prove in-app Windows manager detection,
version upgrade, a public download URL, signing, physical UI, or SSH acceptance.
"""
import argparse
import base64
from contextlib import contextmanager
import http.server
import importlib.util
import io
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import stat
import subprocess
import sys
import threading
import time
import traceback
import zipfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("candidate", Path(__file__).with_name("hosted-candidate.py"))
candidate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(candidate)
require, sha = candidate.require, candidate.sha
REPO = "robert-cronin/flere"
RUN, ARTIFACT, ARTIFACT_BYTES = 34912671194, 10374816947, 1806160
WORKFLOW = "a445b8fce5f8ad17faebc3f518005f87fc9bb27a"
ARTIFACT_SHA = "e7fba2e1cec9bce69fd2696848a7275285edbbb0a207acb7ddb0bd81307bdf38"
PRODUCT = "422058c0fa4dda3cff7693a32953fea1b2c5404e"
SOURCE = "ef49b87e4b4ce6975c22ca9ce7c9f00e061495ff5658137655be60ff08834562"
VERSION, PACKAGE = "0.3.4", "flere-connect"
ZIP_NAME = "flere-connect-0.3.4-x86_64-pc-windows-msvc.zip"
ZIP_SHA = "e562f99168a34202ecaa2dc163db265e9df3c0007699fbb7fea6b24c6c8e7392"
PAYLOAD_SHA = "9622d73775ce583009a05d0ce6ee57e1ff1ec84e6d83f12ff04dd0f6968ce21f"
NUSPEC_SHA = "8ba0dd77f894cdfebfa093019dad91e3bd50aafe7c1f180a2665c0a5bc6dc681"
SCRIPT_SHA = "cdb7bc527b18583fb251dc0d278231dde4271585d0eeef7b2aaf3170fe05c6bf"
NUPKG_SHA = "93834412a8d70dd52bd9317b919f518508e3eb78f70ffb6659c7b2fdf018d0b1"
PUBLIC_URL = f"https://github.com/{REPO}/releases/download/v{VERSION}/{ZIP_NAME}".encode()
ANNOUNCEMENT = b"The package flere-connect wants to run 'chocolateyInstall.ps1'."
PROMPT = b"Do you want to run the script?([Y]es/[A]ll scripts/[N]o/[P]rint): "
MAX_LOG = 256 * 1024


@contextmanager
def command_logs(path, separate_stderr=False):
    """PowerShell JSON needs its own stdout; ordinary install prompts stay merged."""
    with path.open("xb") as stdout:
        if separate_stderr:
            with path.with_name(path.stem + "-stderr.log").open("xb") as stderr:
                yield stdout, stderr
        else:
            yield stdout, subprocess.STDOUT


def parse_features(data):
    result = {}
    for line in data.decode("utf-8-sig").splitlines():
        fields = line.split("|", 2)
        require(len(fields) == 3 and re.fullmatch(r"[A-Za-z][A-Za-z0-9]+", fields[0])
                and fields[1] in ("Enabled", "Disabled") and fields[0] not in result,
                "unexpected Chocolatey feature output")
        result[fields[0]] = fields[1] == "Enabled"
    require(result.get("checksumFiles") is True and "allowGlobalConfirmation" in result,
            "checksum policy or confirmation feature unavailable")
    return result


def parse_packages(data):
    result = {}
    for line in data.decode("utf-8-sig").splitlines():
        fields = line.split("|")
        require(len(fields) == 2 and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", fields[0])
                and re.fullmatch(r"[0-9][A-Za-z0-9.+_-]*", fields[1])
                and fields[0].lower() not in result, "unexpected Chocolatey package output")
        result[fields[0].lower()] = fields[1]
    require("chocolatey" in result, "normal installed Chocolatey record absent")
    return result


def prompt_ready(data, answered):
    """Never prefeed approval; match the single exact ordinary 2.7.4 script prompt."""
    clean = data.replace(b"\r\n", b"\n")
    requests = re.findall(rb"The package [^\n]+ wants to run '[^\n]+'.", clean)
    require(all(item == ANNOUNCEMENT for item in requests) and len(requests) <= 1,
            "unexpected package/script confirmation")
    count = clean.count(PROMPT)
    require(count <= 1 and clean.count(b"Do you want to run ") <= 1,
            "unexpected or repeated script prompt")
    if count:
        require(len(requests) == 1 and clean.index(ANNOUNCEMENT) < clean.index(PROMPT),
                "script prompt lacks exact package announcement")
    return count == 1 and not answered


def verify_api(value):
    require(value["id"] == ARTIFACT and value["name"] == f"windows-recipes-{RUN}-1"
            and value["size_in_bytes"] == ARTIFACT_BYTES and not value["expired"]
            and value["digest"] == "sha256:" + ARTIFACT_SHA
            and value["workflow_run"]["id"] == RUN
            and value["workflow_run"]["head_sha"] == WORKFLOW, "recipe artifact API identity differs")


def inputs(data, work):
    require(len(data) == ARTIFACT_BYTES and sha(data) == ARTIFACT_SHA, "recipe artifact bytes differ")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        names = archive.namelist()
        require(len(names) == 26 and len(set(names)) == 26, "recipe artifact inventory differs")
        total = 0
        for item in archive.infolist():
            require(not item.is_dir() and item.file_size <= 8 * 1024 * 1024
                    and not item.filename.startswith("/") and "\\" not in item.filename
                    and all(p not in ("", ".", "..") for p in item.filename.split("/"))
                    and ((item.external_attr >> 16) & 0o170000) in (0, 0o100000), "unsafe artifact entry")
            total += item.file_size
        require(total < 16 * 1024 * 1024, "artifact expanded bound exceeded")
        receipt = json.loads(archive.read("receipt.json"))
        require(receipt["schema"] == "flere-windows-recipes-only-v1"
                and receipt["status"] == "recipes_validated_not_published"
                and receipt["run_id"] == str(RUN) and receipt["run_attempt"] == "1"
                and receipt["workflow_sha"] == WORKFLOW and receipt["product_commit"] == PRODUCT
                and receipt["source_sha256"] == SOURCE and receipt["zip_sha256"] == ZIP_SHA
                and receipt["chocolatey_version"] == "2.7.4", "recipe proof identity differs")
        require(len(receipt["checks"]) == 6, "recipe check count differs")
        for row in receipt["checks"]:
            require(row["status"] == "passed" and row["exit"] == 0
                    and re.fullmatch(r"[a-z-]+", row["name"]), "recipe check failed")
            for stream in ("stdout", "stderr"):
                if row["name"] == "artifact-download" and stream == "stdout":
                    continue  # This binary is retained by its original artifact pin, not as a log.
                log = archive.read("logs/" + row["name"] + ("-stderr" if stream == "stderr" else "") + ".log")
                require(len(log) == row[stream + "_bytes"] and sha(log) == row[stream + "_sha256"], "recipe log differs")
        for name, pin in receipt["files"].items():
            value = archive.read("windows/" + name)
            require(len(value) == pin["bytes"] and sha(value) == pin["sha256"], "recipe file differs")
        source = json.loads(archive.read("input/source.json"))
        require(source["commit"] == PRODUCT and source["source_sha256"] == SOURCE
                and len(source["entries"]) == 365
                and candidate.module("release-source").fingerprint(source["entries"]) == SOURCE,
                "source inventory differs")
        portable = archive.read("windows/" + ZIP_NAME)
        spec = archive.read("windows/chocolatey/flere-connect/flere-connect.nuspec")
        script = archive.read("windows/chocolatey/flere-connect/tools/chocolateyInstall.ps1")
        packed = archive.read("windows/chocolatey-package/flere-connect.0.3.4.nupkg")
    require(sha(portable) == ZIP_SHA and len(portable) == 1771884
            and sha(spec) == NUSPEC_SHA and sha(script) == SCRIPT_SHA and sha(packed) == NUPKG_SHA,
            "reviewed ZIP/recipe pins differ")
    zip_path = work / ZIP_NAME; zip_path.write_bytes(portable)
    package = work / "flere-connect.0.3.4.nupkg"; package.write_bytes(packed)
    candidate.verify_nupkg(package, script)
    with zipfile.ZipFile(io.BytesIO(portable)) as archive:
        payload = {name: archive.read(name) for name in ("flere.exe", "flere-connect.exe", "manifest.json", "LICENSE")}
    manifest = json.loads(payload["manifest.json"])
    require(manifest["source"]["git_commit"] == PRODUCT and not manifest["source"]["dirty"]
            and manifest["source"]["source_sha256"] == SOURCE
            and manifest["payload"]["sha256"] == PAYLOAD_SHA
            and sha(payload["LICENSE"]) == source["entries"]["LICENSE"]["sha256"], "payload provenance differs")
    candidate.verify_zip(zip_path, manifest, payload["LICENSE"])
    return portable, spec, script, manifest, payload, receipt


def local_script(script, port):
    require(sha(script) == SCRIPT_SHA and script.count(PUBLIC_URL) == 1
            and 49152 <= port <= 65535, "script or loopback port differs")
    url = f"http://127.0.0.1:{port}/{ZIP_NAME}".encode()
    result = script.replace(PUBLIC_URL, url)
    require(result.replace(url, PUBLIC_URL) == script and ZIP_SHA.encode() in result,
            "non-URL script change")
    return result


def request_failure(method, path, status):
    """Bounded metadata only: never retain headers, arbitrary paths, query values or parser messages."""
    raw = path if isinstance(path, str) else ""
    clean = raw.split("?", 1)[0].split("#", 1)[0]
    return {"method": method if method in ("GET", "HEAD", "POST", "PUT", "DELETE", "OPTIONS", "CONNECT", "TRACE", "PATCH") else "<other>",
            "path": clean if clean == "/" + ZIP_NAME else "<other>",
            "path_form": "absolute" if raw.startswith(("http://", "https://")) else "origin" if raw.startswith("/") else "other",
            "path_characters": len(raw), "query_present": "?" in raw, "fragment_present": "#" in raw,
            "status": status, "reason": http.server.BaseHTTPRequestHandler.responses.get(status, ("Unknown",))[0]}


def verify_mirror(responses, attempts, rejections, dropped, internal_errors):
    """Accept only completed exact-object responses; rejected traffic is an observation."""
    require(not internal_errors, "loopback internal error")
    require(len(rejections) <= 8 and not dropped, "loopback diagnostic overflow")
    require(1 <= attempts <= 8 and attempts == len(responses), "loopback response cap or incomplete transfer")
    for row in rejections:
        require(400 <= row["status"] <= 499 or row["status"] == 501, "unexpected loopback error status")
    for row in responses:
        require(row["status"] == 200 and row["path"] == "/" + ZIP_NAME
                and row["method"] in ("GET", "HEAD") and row["content_length"] == 1771884,
                "loopback response identity differs")
        require((row["method"] == "GET" and row["bytes"] == 1771884 and row["sha256"] == ZIP_SHA)
                or (row["method"] == "HEAD" and row["bytes"] == 0 and row["sha256"] is None),
                "loopback response body differs")
    require(any(row["method"] == "GET" for row in responses), "complete exact loopback GET was not observed")


class Mirror(http.server.HTTPServer):
    """One immutable object, one loopback listener; no filesystem HTTP handler."""
    allow_reuse_address = False

    def __init__(self, data):
        require(sha(data) == ZIP_SHA, "mirror payload differs")
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def handle_one_request(self):
                self.connection.settimeout(3)
                super().handle_one_request()

            def send_error(self, code, message=None, explain=None):
                owner = self.server
                if len(owner.rejections) < 8:
                    owner.rejections.append(request_failure(getattr(self, "command", None), getattr(self, "path", None), code))
                else:
                    owner.rejections_dropped += 1
                # Standard error body only; never echo arbitrary parser text or filesystem data.
                super().send_error(code)

            def do_HEAD(self):
                self.respond(False)

            def do_GET(self):
                self.respond(True)

            def respond(self, body):
                owner = self.server
                if self.path != "/" + ZIP_NAME:
                    self.send_error(404)
                    return
                owner.attempts += 1
                if owner.attempts > 8:
                    self.send_error(429)
                    return
                row = {"status": 200, "method": self.command, "path": self.path,
                       "content_length": len(data), "bytes": 0, "sha256": None}
                self.send_response(200)
                self.send_header("Content-Type", "application/zip")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                if body:
                    require(self.wfile.write(data) == len(data), "incomplete loopback body write")
                    row.update(bytes=len(data), sha256=sha(data))
                self.wfile.flush()
                owner.requests.append(row)  # No success record exists before headers/body finish writing.
        # Windows SO_EXCLUSIVEADDRUSE prevents competing binds on the owned listener.
        super().__init__(("127.0.0.1", 0), Handler, bind_and_activate=False)
        if os.name == "nt":
            self.socket.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        try:
            self.server_bind(); self.server_activate()
            require(49152 <= self.server_port <= 65535, "OS did not assign a reviewed high port")
        except BaseException:
            self.server_close()
            raise
        self.requests, self.attempts, self.internal_errors = [], 0, 0
        self.rejections, self.rejections_dropped = [], 0
        self.worker = threading.Thread(target=self.serve_forever, kwargs={"poll_interval": 0.1}, daemon=True)
        self.worker.start()

    def handle_error(self, request, client_address):
        self.internal_errors += 1  # Fail closed without exporting exception/request text.

    def close_owned(self):
        self.shutdown(); self.server_close(); self.worker.join(timeout=5)
        require(not self.worker.is_alive(), "loopback worker did not stop")
        with socket.socket() as probe:
            probe.settimeout(1)
            require(probe.connect_ex(("127.0.0.1", self.server_port)) != 0, "loopback port still listening")


def file_record(path):
    before = path.lstat()
    require(not (getattr(before, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT)
            and (path.is_file() or path.is_dir()), "unexpected reparse/nonregular manager file")
    record = {"path": str(path), "directory": path.is_dir(), "device": before.st_dev,
              "file_id": before.st_ino, "mtime_ns": before.st_mtime_ns, "bytes": before.st_size}
    require(before.st_ino != 0, "native file identity unavailable")
    if path.is_file():
        require(before.st_size <= 8 * 1024 * 1024, "owned file exceeds capture bound")
        record["sha256"] = sha(path.read_bytes())
    after = path.lstat()
    require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns)
            == (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns), "file changed while observed")
    return record


def owned_paths(root):
    """Check each directory before descending; never follow a manager reparse point."""
    result, pending = [], [root]
    while pending:
        path = pending.pop()
        row = file_record(path)
        result.append(path)
        require(len(result) + len(pending) <= 128, "owned metadata inventory exceeds bound")
        if row["directory"]:
            children = sorted(path.iterdir())
            require(len(result) + len(pending) + len(children) <= 128, "owned directory exceeds bound")
            pending.extend(reversed(children))
    return result


def path_hashes():
    import winreg
    result = {"process": sha(os.environ.get("PATH", "").encode("utf-8"))}
    for name, key, path in (("user", winreg.HKEY_CURRENT_USER, r"Environment"),
                            ("machine", winreg.HKEY_LOCAL_MACHINE,
                             r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment")):
        try:
            with winreg.OpenKey(key, path) as opened:
                value, kind = winreg.QueryValueEx(opened, "Path")
            result[name] = {"sha256": sha(value.encode("utf-8")), "registry_type": kind}
        except FileNotFoundError:
            result[name] = {"present": False}
    return result



def verify_removal(value):
    """Stored history is distinct from installed files, according to observed policy."""
    require(set(value["aliases"]) == {"flere.exe", "flere-connect.exe"}, "both alias predicates required")
    errors = [name for name in ("installed_package_present", "package_directory_exists", "payload_directory_exists")
              if value[name]]
    errors += [name for name in ("package_inventory_preserved", "path_preserved", "synthetic_state_preserved")
               if not value[name]]
    for name, alias in value["aliases"].items():
        if alias["shim_exists"] or alias["path_resolution"] is not None:
            errors.append("alias remains: " + name)
    if value["remove_package_information_on_uninstall"] and value["registration_paths"]:
        errors.append("registration remains despite removal policy")
    require(not errors, "removal checks failed: " + ", ".join(errors))


def runtime_env(environment):
    # Keep ordinary OS/tool/module paths, but never give package scripts CI credentials.
    return {k: v for k, v in environment.items()
            if not any(word in k.upper() for word in ("TOKEN", "PASSWORD", "SECRET", "CREDENTIAL"))}


def main(output):
    candidate.hosted(os.environ)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64")
            and sys.version_info >= (3, 12), "native Windows AMD64/Python3.12+ required")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=candidate.PROJECT, text=True,
                                    timeout=15).strip() == os.environ["FLERE_WORKFLOW_SHA"], "workflow checkout differs")
    require(output.parent.resolve() == Path.home().resolve() / ".cache/flere/tmp"
            and not output.exists(), "fresh private HOME-cache output required")
    output.mkdir(); work = output / "work"; proof = output / "proof"
    work.mkdir(); proof.mkdir(); (proof / "logs").mkdir(); (proof / "records").mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    receipt = {"schema": "flere-chocolatey-lifecycle-v1", "status": "running", "checks": [],
               "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"], "run_id": os.environ["GITHUB_RUN_ID"],
               "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "input_artifact": ARTIFACT,
               "artifact_sha256": ARTIFACT_SHA, "product_commit": PRODUCT, "source_sha256": SOURCE,
               "zip_sha256": ZIP_SHA, "limits": ["First private-loopback install/remove, not version upgrade or public URL proof.",
               "No Windows manager detector, coordinated UI refusal, physical UI, SSH, signing or publication claim."],
               "machine": {"platform": platform.platform(), "image_os": os.environ.get("ImageOS"),
                           "image_version": os.environ.get("ImageVersion"), "python": platform.python_version()}}
    environment = runtime_env(os.environ)
    mirror = None; changed_confirmation = False; attempted_install = False; uninstalled = False
    baseline_features = baseline_packages = None
    deadline = time.monotonic() + 360  # Leave a separate 150s normal cleanup budget inside the 10min job.

    def save():
        (proof / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def record(name, value):
        (proof / "records" / (name + ".json")).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def command(name, argv, *, env=environment, seconds=120, maximum=MAX_LOG, confirm=False, destination=None, cwd=work, separate_stderr=False):
        require(time.monotonic() < deadline, "lifecycle phase budget exhausted")
        seconds = min(seconds, deadline - time.monotonic())
        log = destination or proof / "logs" / (name + ".log")
        row = {"name": name, "argv": list(map(str, argv)), "status": "running", "confirmed_script": False}
        receipt["checks"].append(row); save(); start = time.monotonic(); failure = None
        stderr_log = log.with_name(log.stem + "-stderr.log") if separate_stderr else None
        outputs = [log] + ([stderr_log] if stderr_log else [])
        with command_logs(log, separate_stderr) as (stream, errors):
            proc = subprocess.Popen(list(map(str, argv)), cwd=cwd, env=env,
                                    stdin=subprocess.PIPE if confirm else subprocess.DEVNULL,
                                    stdout=stream, stderr=errors)
            row["pid"] = proc.pid
            try:
                while proc.poll() is None:
                    require(time.monotonic() - start < seconds, name + " timed out")
                    require(all(path.stat().st_size <= maximum for path in outputs), name + " output exceeds bound")
                    if confirm and prompt_ready(log.read_bytes(), row["confirmed_script"]):
                        proc.stdin.write(b"y\r\n"); proc.stdin.flush()
                        row["confirmed_script"] = True
                        # Keep stdin open, send no further data; repeated prompts are rejected.
                    time.sleep(0.05)
            except BaseException as error:
                failure = error
            finally:
                row["forced_stop"] = proc.poll() is None
                if row["forced_stop"]:
                    killed = subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"),
                                             "/PID", str(proc.pid), "/T", "/F"], stdin=subprocess.DEVNULL,
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
                    require(killed.returncode == 0, "exact command tree could not be stopped")
                proc.wait(timeout=15)
                if proc.stdin:
                    proc.stdin.close()
        for path in outputs:
            if path.stat().st_size > maximum:
                failure = failure or ValueError(name + " output exceeds bound")
                with path.open("r+b") as stream:
                    stream.truncate(maximum)
        if stderr_log:
            errors = stderr_log.read_bytes()
            row.update(stderr_bytes=len(errors), stderr_sha256=sha(errors), stderr_log=stderr_log.name)
        data = log.read_bytes()
        row.update(exit=proc.returncode, seconds=round(time.monotonic() - start, 3),
                   log_bytes=len(data), log_sha256=sha(data), status="failed")
        if confirm and failure is None:
            try:
                prompt_ready(data, row["confirmed_script"])
                require(row["confirmed_script"], "ordinary script confirmation was not observed")
            except ValueError as error:
                failure = error
        if failure is None and proc.returncode == 0:
            row["status"] = "passed"
        save()
        if failure is not None:
            raise failure
        require(proc.returncode == 0, name + " failed; see bounded proof log")
        return data

    def features(name):
        return parse_features(command(name, [choco, "feature", "list", "--limit-output"]))

    def packages(name):
        return parse_packages(command(name, [choco, "list", "--limit-output"]))

    def powershell(name, code):
        encoded = base64.b64encode(("$ErrorActionPreference='Stop'; " + code).encode("utf-16-le")).decode()
        return json.loads(command(name, [ps, "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded], separate_stderr=True))

    def owned_registration():
        path = root / ".chocolatey"
        if not path.exists():
            return []
        file_record(path)
        return sorted(path.glob("flere-connect.0.3.4*"))

    def removal_snapshot(name, installed):
        # Capture each predicate before asserting, including normal historical registration retention.
        registrations = owned_registration()
        value = {"installed_package_present": PACKAGE in installed,
                 "package_inventory_preserved": installed == baseline_packages,
                 "package_directory_exists": os.path.lexists(root / "lib" / PACKAGE),
                 "payload_directory_exists": os.path.lexists(root / "lib" / PACKAGE / "tools/app"),
                 "aliases": {alias: {"shim_exists": os.path.lexists(root / "bin" / alias),
                                      "path_resolution": shutil.which(alias)}
                             for alias in ("flere.exe", "flere-connect.exe")},
                 "path_preserved": path_hashes() == paths_before,
                 "synthetic_state_preserved": candidate.tree(fixture) == before_state,
                 "remove_package_information_on_uninstall": baseline_features["removePackageInformationOnUninstall"],
                 "registration_paths": list(map(str, registrations)),
                 "historical_registration_retained": bool(registrations)}
        record(name, value)  # A later diagnostic read failure must not hide the removal predicates.
        retained = []
        for registration in registrations:
            retained.extend(owned_paths(registration))
        require(len(retained) <= 128, "retained manager history exceeds bound")
        value["historical_registration_inventory"] = [file_record(path) for path in retained]
        record(name, value)
        receipt["historical_registration_retained"] = bool(registrations)
        return value

    try:
        gh = shutil.which("gh.exe"); require(gh, "existing GitHub CLI unavailable")
        api = f"repos/{REPO}/actions/artifacts/{ARTIFACT}"
        metadata = json.loads(command("artifact-api", [gh, "api", api], env=os.environ.copy(), maximum=65536))
        verify_api(metadata); record("artifact-api", metadata)
        portable, spec, script, manifest, payload, original = inputs(command("artifact-download", [gh, "api", api + "/zip"],
            env=os.environ.copy(), destination=work / "artifact.zip", maximum=ARTIFACT_BYTES), work)
        record("input-recipe-receipt", original); record("manifest", manifest)
        root = Path(os.environ.get("ChocolateyInstall", ""))
        require(root.resolve() == Path(r"C:\ProgramData\chocolatey").resolve(), "normal Chocolatey root required")
        choco = Path(shutil.which("choco.exe") or "missing")
        require(choco.resolve() == (root / "bin/choco.exe").resolve(), "normal Chocolatey command required")
        ps = Path(shutil.which("pwsh.exe") or "missing")
        require(ps.is_file(), "existing native PowerShell7 unavailable; no provisioning attempted")
        ps_info = powershell("powershell-version", "[pscustomobject]@{edition=$PSVersionTable.PSEdition;major=$PSVersionTable.PSVersion.Major;version=$PSVersionTable.PSVersion.ToString();home=$PSHOME} | ConvertTo-Json -Compress")
        require(ps_info["edition"] == "Core" and ps_info["major"] == 7
                and (Path(ps_info["home"]) / "pwsh.exe").resolve() == ps.resolve(), "native PowerShell7 identity differs")
        record("powershell", dict(ps_info, executable=str(ps)))
        admin = powershell("elevation", "$identity=[Security.Principal.WindowsIdentity]::GetCurrent(); $principal=[Security.Principal.WindowsPrincipal]$identity; $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) | ConvertTo-Json -Compress")
        require(admin is True, "disposable hosted runner is not already elevated; no elevation requested")
        require(command("choco-version", [choco, "--version"]).decode().strip() == "2.7.4", "reviewed Chocolatey2.7.4 required")
        receipt["machine"]["chocolatey_version"] = "2.7.4"
        record("manager", {"root": file_record(root), "command": file_record(choco), "elevated": admin})
        baseline_features = features("features-before"); baseline_packages = packages("packages-before")
        require("removePackageInformationOnUninstall" in baseline_features, "package-information retention policy unavailable")
        require(PACKAGE not in baseline_packages and not (root / "lib" / PACKAGE).exists()
                and not owned_registration(), "package or registration already exists")
        require(all(shutil.which(alias) is None and not (root / "bin" / alias).exists()
                    for alias in ("flere.exe", "flere-connect.exe")), "alias collision")
        paths_before = path_hashes(); record("path-before", paths_before)
        fixture = work / "profile"; fixture.mkdir()
        for folder in ("state", ".ssh", "AppData/Local", "AppData/Roaming", ".cache", "temp"):
            (fixture / folder).mkdir(parents=True, exist_ok=True)
        (fixture / "state/preserved.json").write_text('{"synthetic":true,"selection":"retained"}\n')
        (fixture / ".ssh/config").write_text("# synthetic sentinel; no SSH is launched\n")
        before_state = candidate.tree(fixture); record("state-before", before_state)
        child_env = dict(environment, HOME=str(fixture), USERPROFILE=str(fixture),
                         LOCALAPPDATA=str(fixture / "AppData/Local"), APPDATA=str(fixture / "AppData/Roaming"),
                         XDG_CACHE_HOME=str(fixture / ".cache"), TEMP=str(fixture / "temp"), TMP=str(fixture / "temp"))
        if baseline_features["allowGlobalConfirmation"]:
            changed_confirmation = True  # Restore even if the command changes state then fails.
            command("require-script-confirmation", [choco, "feature", "disable", "-n", "allowGlobalConfirmation"])
        require(not any("ignorechecksum" in key.lower() and value.lower() not in ("", "false", "0")
                        for key, value in environment.items()), "inherited checksum override is unsupported")
        require(features("features-during") == dict(baseline_features, allowGlobalConfirmation=False),
                "features changed beyond confirmation tightening")
        mirror = Mirror(portable)
        install_script = local_script(script, mirror.server_port)
        recipe = work / "recipe"; (recipe / "tools").mkdir(parents=True)
        (recipe / "flere-connect.nuspec").write_bytes(spec)
        (recipe / "tools/chocolateyInstall.ps1").write_bytes(install_script)
        packed = work / "packages"; packed.mkdir()
        command("pack", [choco, "pack", recipe / "flere-connect.nuspec", "--outputdirectory", packed], cwd=recipe)
        nupkg = packed / "flere-connect.0.3.4.nupkg"
        require(list(packed.iterdir()) == [nupkg], "unexpected packed output")
        with zipfile.ZipFile(nupkg) as private, zipfile.ZipFile(work / "flere-connect.0.3.4.nupkg") as original_package:
            require(private.read("flere-connect.nuspec") == original_package.read("flere-connect.nuspec"),
                    "normal pack changed the reviewed package metadata")
        record("private-package", {"sha256": sha(nupkg.read_bytes()), "port": mirror.server_port,
                "original_script_sha256": SCRIPT_SHA, "private_script_sha256": sha(install_script),
                "inventory": candidate.verify_nupkg(nupkg, install_script)})
        attempted_install = True
        command("install", [choco, "install", PACKAGE, "--version=" + VERSION,
                            "--source=" + str(packed), "--no-progress"], confirm=True)
        require(packages("packages-installed") == dict(baseline_packages, **{PACKAGE: VERSION}), "unrelated package change")
        package_root = root / "lib" / PACKAGE; app = package_root / "tools/app"
        require(app.is_dir() and {p.name for p in app.iterdir()} == set(payload), "installed app inventory differs")
        for name, data in payload.items():
            require(file_record(app / name)["sha256"] == sha(data), "installed payload differs")
        with zipfile.ZipFile(nupkg) as archive:
            packed_spec = archive.read("flere-connect.nuspec")
        require((package_root / "flere-connect.nuspec").read_bytes() == packed_spec
                and (package_root / "tools/chocolateyInstall.ps1").read_bytes() == install_script,
                "installed package spec/script differs")
        aliases = []
        for alias in ("flere.exe", "flere-connect.exe"):
            path = Path(shutil.which(alias) or "missing")
            require(path.resolve() == (root / "bin" / alias).resolve(), "normal PATH alias not the Chocolatey shim")
            require(file_record(path)["sha256"] != PAYLOAD_SHA, "alias is a payload copy, not a generated shim")
            aliases.append(path)
            for flag in ("--help", "--version", "--build-info"):
                data = command(alias[:-4] + "-" + flag[2:], [path, flag], env=child_env, seconds=20, maximum=32768)
                if flag == "--build-info":
                    require(json.loads(data) == manifest["build"], "alias full build info differs")
                elif flag == "--version":
                    require(data.decode().strip().startswith("flere-connect " + VERSION + " ")
                            and manifest["build"]["build_id"] in data.decode(), "alias version/build differs")
                else:
                    require(b"--build-info" in data and b"ssh" in data, "alias help differs")
        # Capture only this install's package files and registration, not other packages/configuration.
        paths = [root, choco, package_root, *aliases]
        paths.extend(owned_paths(package_root))
        registrations = owned_registration()
        paths.extend(registrations)
        for path in registrations:
            paths.extend(owned_paths(path))
        paths = list(dict.fromkeys(paths)); require(len(paths) <= 128, "owned metadata inventory exceeds bound")
        records = [file_record(path) for path in paths]
        record("installed-layout", {"files": records, "registration_observed": list(map(str, registrations)),
               "native_file_id": "Python os.stat st_ino/st_dev from Windows file identity; observation, not ownership proof"})
        # Keep small manager-generated ledger bytes for the detector review; never copy executables.
        ledger = []
        for index, path in enumerate(paths):
            if any(path == base or base in path.parents for base in registrations) and path.is_file():
                require(len(ledger) < 12 and path.stat().st_size <= 128 * 1024, "registration record exceeds bound")
                data = path.read_bytes(); target = proof / "records" / f"registration-{index}.bin"
                target.write_bytes(data); ledger.append({"path": str(path), "retained": target.name, "sha256": sha(data)})
        record("registration-files", ledger)
        # Optional diagnostic: capture needed manager records first, without changing module paths/policy.
        selected = work / "metadata-paths.json"; selected.write_text(json.dumps(list(map(str, paths))), encoding="utf-8")
        escaped = str(selected).replace("'", "''")
        try:
            acl = powershell("owned-acls", "$paths = Get-Content -Raw -LiteralPath '" + escaped + "' | ConvertFrom-Json; @($paths | ForEach-Object { $a=Get-Acl -LiteralPath $_; [pscustomobject]@{path=$_;owner=$a.Owner;sddl=$a.Sddl} }) | ConvertTo-Json -Depth 3 -Compress")
            require(len(acl) == len(paths) and all(row["sddl"] and row["owner"] for row in acl), "owned ACL capture incomplete")
            record("owned-acls", acl)
            receipt["acl_diagnostic"] = {"status": "passed", "required_for_lifecycle": False}
        except Exception as error:
            receipt["acl_diagnostic"] = {"status": "failed", "required_for_lifecycle": False, "error": str(error)}
        save()
        require(candidate.tree(fixture) == before_state, "stateless alias calls changed synthetic state")
        command("uninstall", [choco, "uninstall", PACKAGE, "--version=" + VERSION, "--no-progress"])
        uninstalled = True
        verify_removal(removal_snapshot("removal", packages("packages-after")))
        record("path-after", path_hashes()); record("state-after", candidate.tree(fixture))
        receipt.update(status="lifecycle_complete_pending_mirror", aliases_checked=6, synthetic_state_preserved=True,
                       package_inventory_preserved=True, no_product_ownership_claim=True)
    except BaseException as error:
        receipt.update(status="failed", error=str(error), traceback=traceback.format_exc())
    finally:
        deadline = time.monotonic() + 150
        cleanup_errors = []
        # Normal package-manager cleanup only. A failure is retained, never papered over by deleting its files.
        if attempted_install and not uninstalled:
            try:
                command("failure-uninstall", [choco, "uninstall", PACKAGE, "--version=" + VERSION, "--no-progress"])
                verify_removal(removal_snapshot("failure-removal", packages("failure-packages-after")))
            except BaseException as error:
                cleanup_errors.append("normal uninstall: " + str(error))
        if changed_confirmation:
            try:
                command("restore-confirmation-feature", [choco, "feature", "enable", "-n", "allowGlobalConfirmation"])
            except BaseException as error:
                cleanup_errors.append("confirmation restore: " + str(error))
        if baseline_features is not None:
            try:
                require(features("features-restored") == baseline_features, "full original feature list not restored")
                receipt["features_restored"] = True
            except BaseException as error:
                cleanup_errors.append("feature verification: " + str(error))
        if mirror is not None:
            try:
                mirror.close_owned(); receipt["loopback_closed"] = True
            except BaseException as error:
                cleanup_errors.append("loopback cleanup: " + str(error))
            receipt["loopback_requests"] = mirror.requests
            receipt["loopback_response_attempts"] = mirror.attempts
            receipt["loopback_internal_errors"] = mirror.internal_errors
            receipt["loopback_rejections"] = mirror.rejections
            receipt["loopback_rejections_dropped"] = mirror.rejections_dropped
            try:
                require(receipt.get("loopback_closed") is True, "loopback must close before final verification")
                verify_mirror(mirror.requests, mirror.attempts, mirror.rejections,
                              mirror.rejections_dropped, mirror.internal_errors)
                receipt["loopback_verified"] = True
                if receipt["status"] == "lifecycle_complete_pending_mirror":
                    receipt["status"] = "lifecycle_passed"
            except Exception as error:
                receipt["status"] = "failed"
                receipt.setdefault("error", str(error))
                receipt["loopback_verification_error"] = str(error)
        receipt["cleanup_errors"] = cleanup_errors
        if cleanup_errors:
            receipt["status"] = "failed"
        files = [p for p in proof.rglob("*") if p.is_file() and p.name != "receipt.json"]
        if len(files) > 64 or sum(p.stat().st_size for p in files) > 8 * 1024 * 1024:
            receipt.update(status="failed", error="curated proof bound exceeded")
        receipt["files"] = {p.relative_to(proof).as_posix(): {"bytes": p.stat().st_size, "sha256": sha(p.read_bytes())}
                            for p in sorted(files)}
        save()
    require(receipt["status"] == "lifecycle_passed", "Chocolatey lifecycle failed; see bounded receipt")


def self_test():
    """Safety/format regressions; no manager, Windows payload, network or application state."""
    import unittest
    import tempfile

    class Guards(unittest.TestCase):
        def test_hosted_main_only(self):
            env = {"GITHUB_ACTIONS": "true", "FLERE_RUNNER_ENVIRONMENT": "github-hosted",
                   "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REPOSITORY": REPO,
                   "GITHUB_REF": "refs/heads/main", "RUNNER_OS": "Windows", "RUNNER_ARCH": "X64",
                   "FLERE_WORKFLOW_SHA": "a" * 40, "GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "1"}
            candidate.hosted(env)
            for key in env:
                with self.subTest(key=key), self.assertRaises(ValueError):
                    candidate.hosted(dict(env, **{key: "unexpected"}))

        def test_only_complete_prompt_at_every_split(self):
            text = ANNOUNCEMENT + b"\r\nNote: ordinary script confirmation\r\n" + PROMPT
            for split in range(len(text)):
                self.assertFalse(prompt_ready(text[:split], False))
                self.assertTrue(prompt_ready(text[:split] + text[split:], False))
                self.assertFalse(prompt_ready(text, True))

        def test_other_or_repeated_prompt_rejected(self):
            for text in (PROMPT, ANNOUNCEMENT.replace(b"flere-connect", b"other") + PROMPT,
                         ANNOUNCEMENT + PROMPT + PROMPT, ANNOUNCEMENT + ANNOUNCEMENT + PROMPT):
                with self.subTest(text=text), self.assertRaises(ValueError):
                    prompt_ready(text, False)

        def test_features_preserve_explicit_boolean_semantics(self):
            raw = b"allowGlobalConfirmation|Enabled|Prompt policy\r\nchecksumFiles|Enabled|Validate hashes\r\n"
            self.assertEqual(parse_features(raw), {"allowGlobalConfirmation": True, "checksumFiles": True})
            for bad in (raw.replace(b"checksumFiles|Enabled", b"checksumFiles|Disabled"),
                        raw + raw, raw.replace(b"|Enabled|", b"|unknown|")):
                with self.assertRaises(ValueError):
                    parse_features(bad)

        def test_packages_reject_diagnostics_duplicates_and_missing_manager(self):
            raw = b"chocolatey|2.7.4\r\nflere-connect|0.3.4\r\n"
            self.assertEqual(parse_packages(raw)[PACKAGE], VERSION)
            for bad in (raw + b"warning text\n", raw + raw, b"flere-connect|0.3.4\n"):
                with self.assertRaises(ValueError):
                    parse_packages(bad)

        def test_credentials_removed_but_system_module_paths_remain(self):
            self.assertEqual(runtime_env({"PATH": "normal", "PSModulePath": "normal-modules", "SystemRoot": "system",
                                          "GH_TOKEN": "test", "ACTIONS_RUNTIME_TOKEN": "test", "OTHER_PASSWORD": "test"}),
                             {"PATH": "normal", "PSModulePath": "normal-modules", "SystemRoot": "system"})

        def test_removal_respects_observed_history_policy(self):
            value = {"installed_package_present": False, "package_directory_exists": False,
                     "payload_directory_exists": False, "package_inventory_preserved": True,
                     "path_preserved": True, "synthetic_state_preserved": True,
                     "aliases": {name: {"shim_exists": False, "path_resolution": None}
                                 for name in ("flere.exe", "flere-connect.exe")},
                     "remove_package_information_on_uninstall": False,
                     "registration_paths": [r"C:\ProgramData\chocolatey\.chocolatey\flere-connect.0.3.4"]}
            verify_removal(value)  # Native2.7.4 default: metadata history may outlive the installed package.
            with self.assertRaisesRegex(ValueError, "registration remains"):
                verify_removal(dict(value, remove_package_information_on_uninstall=True))
            verify_removal(dict(value, remove_package_information_on_uninstall=True, registration_paths=[]))

        def test_history_does_not_relax_actual_removal_or_preservation(self):
            base = {"installed_package_present": False, "package_directory_exists": False,
                    "payload_directory_exists": False, "package_inventory_preserved": True,
                    "path_preserved": True, "synthetic_state_preserved": True,
                    "aliases": {name: {"shim_exists": False, "path_resolution": None}
                                for name in ("flere.exe", "flere-connect.exe")},
                    "remove_package_information_on_uninstall": False, "registration_paths": ["own-history"]}
            for name in ("installed_package_present", "package_directory_exists", "payload_directory_exists",
                         "package_inventory_preserved", "path_preserved", "synthetic_state_preserved"):
                with self.subTest(name=name), self.assertRaisesRegex(ValueError, name):
                    verify_removal(dict(base, **{name: not base[name]}))
            for alias in ({"shim_exists": True, "path_resolution": None},
                          {"shim_exists": False, "path_resolution": r"C:\other\flere.exe"}):
                with self.subTest(alias=alias), self.assertRaisesRegex(ValueError, "alias remains"):
                    verify_removal(dict(base, aliases=dict(base["aliases"], **{"flere.exe": alias})))
            with self.assertRaisesRegex(ValueError, "both alias"):
                verify_removal(dict(base, aliases={}))

        def test_mirror_accepts_exact_bytes_with_rejected_traffic(self):
            get = {"status": 200, "method": "GET", "path": "/" + ZIP_NAME,
                   "content_length": 1771884, "bytes": 1771884, "sha256": ZIP_SHA}
            head = dict(get, method="HEAD", bytes=0, sha256=None)
            verify_mirror([get, head], 2, [{"status": code} for code in (400, 404, 501)], 0, 0)
            with self.assertRaisesRegex(ValueError, "GET"):
                verify_mirror([head], 1, [], 0, 0)

        def test_mirror_rejects_mismatched_incomplete_and_unbounded_results(self):
            get = {"status": 200, "method": "GET", "path": "/" + ZIP_NAME,
                   "content_length": 1771884, "bytes": 1771884, "sha256": ZIP_SHA}
            for key, wrong in (("status", 201), ("method", "POST"), ("path", "/other"),
                               ("content_length", 1), ("bytes", 1), ("sha256", "0" * 64)):
                with self.subTest(key=key), self.assertRaises(ValueError):
                    verify_mirror([dict(get, **{key: wrong})], 1, [], 0, 0)
            with self.assertRaisesRegex(ValueError, "body"):
                verify_mirror([dict(get, method="HEAD")], 1, [], 0, 0)
            for args in (([get], 2, [], 0, 0), ([get] * 9, 9, [], 0, 0),
                         ([get], 1, [], 1, 0), ([get], 1, [{"status": 400}] * 9, 0, 0), ([get], 1, [], 0, 1),
                         ([get], 1, [{"status": 500}], 0, 0)):
                with self.subTest(args=args[1:]), self.assertRaises(ValueError):
                    verify_mirror(*args)

        def test_rejected_request_diagnostics_never_retain_arbitrary_text(self):
            row = request_failure("GET", "/" + ZIP_NAME + "?token=private#private", 404)
            self.assertEqual(row["path"], "/" + ZIP_NAME)
            self.assertTrue(row["query_present"] and row["fragment_present"])
            self.assertEqual((row["method"], row["status"], row["reason"]), ("GET", 404, "Not Found"))
            self.assertNotIn("private", json.dumps(row))
            for method, path in (("private", "/private"), (None, None),
                                 ("GET", "http://user:private@127.0.0.1:50000/private?private")):
                with self.subTest(method=method):
                    result = request_failure(method, path, 400)
                    self.assertEqual(result["path"], "<other>")
                    self.assertNotIn("private", json.dumps(result))
            self.assertEqual(request_failure("HEAD", "/" + ZIP_NAME, 501)["method"], "HEAD")

        def test_actual_powershell_progress_stays_out_of_json(self):
            # Exact first-use CLIXML progress bytes retained from native run34915209443.
            progress = (b'#< CLIXML\r\n<Objs Version="1.1.0.1" xmlns="http://schemas.microsoft.com/powershell/2004/04">'
                        b'<Obj S="progress" RefId="0"><TN RefId="0"><T>System.Management.Automation.PSCustomObject</T>'
                        b'<T>System.Object</T></TN><MS><I64 N="SourceId">1</I64><PR N="Record">'
                        b'<AV>Preparing modules for first use.</AV><AI>0</AI><Nil /><PI>-1</PI><PC>-1</PC>'
                        b'<T>Completed</T><SR>-1</SR><SD> </SD></PR></MS></Obj></Objs>')
            native_merged = progress[:11] + b"true\r\n" + progress[11:]
            self.assertEqual(sha(native_merged), "1f0811f57c3afe1272c800316b52c9bdb91835fd4e58bc00fa30a1650570f702")
            with self.assertRaises(ValueError):
                json.loads(native_merged)  # The exact pre-fix native failure.
            root = Path.home() / ".cache/flere/tmp"; root.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(prefix="choco-json-", dir=root) as folder:
                log = Path(folder) / "elevation.log"
                program = "import os; os.write(1, b'true\\r\\n'); os.write(2, " + repr(progress) + ")"
                with command_logs(log, True) as (stdout, stderr):
                    subprocess.run([sys.executable, "-I", "-B", "-c", program], check=True, timeout=10,
                                   stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, env=runtime_env(os.environ))
                self.assertIs(json.loads(log.read_bytes()), True)
                self.assertEqual((Path(folder) / "elevation-stderr.log").read_bytes(), progress)

        def test_ordinary_prompt_output_remains_merged(self):
            root = Path.home() / ".cache/flere/tmp"; root.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(prefix="choco-prompt-", dir=root) as folder:
                log = Path(folder) / "install.log"
                program = "import os; os.write(1, " + repr(ANNOUNCEMENT + b"\r\n") + "); os.write(2, " + repr(PROMPT) + ")"
                with command_logs(log) as (stdout, stderr):
                    subprocess.run([sys.executable, "-I", "-B", "-c", program], check=True, timeout=10,
                                   stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr, env=runtime_env(os.environ))
                self.assertTrue(prompt_ready(log.read_bytes(), False))
                self.assertFalse((Path(folder) / "install-stderr.log").exists())

        def test_artifact_api_must_match_run_revision_digest(self):
            value = {"id": ARTIFACT, "name": f"windows-recipes-{RUN}-1", "size_in_bytes": ARTIFACT_BYTES,
                     "expired": False, "digest": "sha256:" + ARTIFACT_SHA,
                     "workflow_run": {"id": RUN, "head_sha": WORKFLOW}}
            verify_api(value)
            for key, wrong in (("expired", True), ("digest", "sha256:" + "0" * 64),
                               ("workflow_run", {"id": RUN, "head_sha": "0" * 40})):
                with self.assertRaises(ValueError):
                    verify_api(dict(value, **{key: wrong}))

    require(unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Guards)).wasSuccessful(),
            "lifecycle pure guards failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, nargs="?")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        require(args.output is None, "self-test takes no output path")
        self_test()
    else:
        require(args.output is not None, "fresh output directory required")
        main(args.output.resolve())
