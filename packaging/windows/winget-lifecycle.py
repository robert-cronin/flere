#!/usr/bin/env python3
"""Normal first WinGet portable install/remove on one disposable hosted Windows VM.

Exact retained 422 ZIP; no public URL, upgrade, owner-detector, UI or SSH claim.
The community source is contacted normally. Only LocalManifestFiles may change,
through normal settings commands with readback and original-state restoration.
"""
import argparse
import base64
import copy
import importlib.util
import io
import json
import ntpath
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import traceback
import zipfile

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("choco_fixture", Path(__file__).with_name("chocolatey-lifecycle.py"))
base = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(base)
candidate, require, sha = base.candidate, base.require, base.sha
PACKAGE, VERSION = "RobertCronin.FlereConnect", "0.3.4"
WINGET_VERSION = "v1.11.510"
SOURCE_URL = "https://cdn.winget.microsoft.com/cache"
SOURCE_ID = "Microsoft.Winget.Source_8wekyb3d8bbwe"
SETTING_SCHEMA = "https://aka.ms/winget-settings-export.schema.json"
MANIFESTS = {
    PACKAGE + ".installer.yaml": "e780fb0839499e3ad196410a552c667877ca6f5779126fb81883640710fabe82",
    PACKAGE + ".locale.en-US.yaml": "782cdfeff696c8fc19920270acef8b7cc5ab6d950c5eab5bbb9b282491ddd8a9",
    PACKAGE + ".yaml": "4a12e5e84b053b265931e1f5970dce9134f4fc89893c9ffd260850a86d78e049",
}
SOURCE_TITLE = b"The `winget` source requires that you view the following agreements before using."
SOURCE_PROMPT = b"Do you agree to all the source agreements terms?"
OPTIONS = b"[Y] Yes  [N] No: "
ARP = r"Software\Microsoft\Windows\CurrentVersion\Uninstall"


def clean_console(data):
    # Interpretation only; retain original bytes. Only ordinary SGR/progress
    # cursor sequences from the known manager are ignored for text matching.
    return re.sub(rb"\x1b\[[0-9;?]*[mGKJABCDHf]", b"", data).replace(b"\r", b"")


def prompt_ready(data, answered, allow_source):
    text = clean_console(data)
    titles = re.findall(rb"The `[^`\n]+` source requires that you view the following agreements before using\.", text)
    require(all(title == SOURCE_TITLE for title in titles) and len(titles) <= 1,
            "unexpected source agreement context")
    require(b"The publisher requires that you view" not in text and b"Do you want to" not in text,
            "unexpected package or other prompt")
    count = text.count(OPTIONS)
    require(count <= 1 and text.count(SOURCE_PROMPT) <= 1, "repeated confirmation prompt")
    if count:
        require(allow_source and titles == [SOURCE_TITLE] and text.count(SOURCE_PROMPT) == 1
                and text.index(SOURCE_TITLE) < text.index(SOURCE_PROMPT) < text.index(OPTIONS),
                "prompt lacks reviewed community-source context")
        # Retain the actual community terms; Store market consent is outside this fixture.
        require(b"2-letter geographic region" not in text, "unexpected source market agreement")
    return count == 1 and not answered


def source_export(data):
    decoder = json.JSONDecoder(); text = data.decode("utf-8-sig"); rows = []
    while text.strip():
        value, end = decoder.raw_decode(text.lstrip())
        require(isinstance(value, dict), "source export entry is not an object")
        rows.append(value); text = text.lstrip()[end:]
        require(len(rows) <= 8, "source inventory exceeds bound")
    require(rows and len({row.get("Name") for row in rows}) == len(rows), "duplicate/empty source inventory")
    matches = [row for row in rows if row.get("Name") == "winget"]
    require(len(matches) == 1, "normal community source missing")
    source = matches[0]
    require(source.get("Arg") == SOURCE_URL and source.get("Type") == "Microsoft.PreIndexed.Package"
            and source.get("Identifier") == SOURCE_ID and source.get("Explicit") is False,
            "community source identity differs")
    return sorted(rows, key=lambda row: row["Name"])


def setting_value(value):
    require(isinstance(value, dict) and value.get("$schema") == SETTING_SCHEMA
            and isinstance(value.get("userSettingsFile"), str) and value["userSettingsFile"]
            and isinstance(value.get("adminSettings"), dict)
            and type(value["adminSettings"].get("LocalManifestFiles")) is bool,
            "unexpected settings export")
    return value["adminSettings"]["LocalManifestFiles"]


def verify_settings(before, after, enabled):
    setting_value(before); setting_value(after)
    expected = copy.deepcopy(before); expected["adminSettings"]["LocalManifestFiles"] = enabled
    require(type(enabled) is bool and after == expected, "settings changed beyond LocalManifestFiles")


def verify_user_settings(value):
    require(isinstance(value, dict), "user settings must be an object")
    for section, key, default in (("installBehavior", "portablePackageUserRoot", ""),
                                 ("installBehavior", "portablePackageMachineRoot", ""),
                                 ("interactivity", "disable", False),
                                 ("uninstallBehavior", "purgePortablePackage", False)):
        group = value.get(section, {})
        require(isinstance(group, dict) and group.get(key, default) == default
                and type(group.get(key, default)) is type(default), "unsupported custom setting: " + key)


def local_manifest(name, original, port):
    require(type(port) is int and 49152 <= port <= 65535, "owned high loopback port required")
    require(name in MANIFESTS and sha(original) == MANIFESTS[name], "frozen manifest differs")
    updated = original
    if name.endswith(".installer.yaml"):
        require(original.count(base.PUBLIC_URL) == 1, "one installer URL required")
        local = f"http://127.0.0.1:{port}/{base.ZIP_NAME}".encode()
        updated = original.replace(base.PUBLIC_URL, local)
        require(updated.replace(local, base.PUBLIC_URL) == original, "non-URL manifest change")
    return updated


def local_manifests(archive_bytes, destination, port):
    require(len(archive_bytes) == base.ARTIFACT_BYTES and sha(archive_bytes) == base.ARTIFACT_SHA,
            "recipe artifact differs")
    original_prefix = "windows/winget/manifests/r/RobertCronin/FlereConnect/0.3.4/"
    destination.mkdir()
    changes = {}
    with zipfile.ZipFile(io.BytesIO(archive_bytes)) as archive:
        for name, digest in MANIFESTS.items():
            original = archive.read(original_prefix + name)
            updated = local_manifest(name, original, port)
            (destination / name).write_bytes(updated)
            changes[name] = {"original_sha256": digest, "private_sha256": sha(updated)}
    return changes


def own_record(records, *, complete=True):
    rows = [row for row in records if row["values"].get("WinGetPackageIdentifier") == PACKAGE]
    require(len(rows) == 1, "exactly one active package record required")
    row = rows[0]; values = row["values"]
    require(row["hive"] == "HKCU" and row["view"] == "64"
            and values.get("DisplayVersion") == VERSION and values.get("DisplayName") == "Flere Connect"
            and values.get("WinGetInstallerType") == "portable"
            and values.get("WinGetSourceIdentifier") == "*DefaultSource",
            "active user portable record differs")
    if complete:
        require(values.get("InstallDirectoryCreated") == 1
                and values.get("InstallDirectoryAddedToPath", 0) == 0,
                "portable install incomplete or alias fallback occurred")
    return row


def verify_inventory(before, after, installed):
    old = {row["key"]: row["sha256"] for row in before}
    new = {row["key"]: row["sha256"] for row in after}
    require(len(old) == len(before) and len(new) == len(after), "duplicate installed record")
    if installed:
        owned = own_record(after)
        require(owned["key"] not in old, "owned registry entry existed before install")
        del new[owned["key"]]
    require(new == old, "unrelated installed-program inventory changed")


def verify_removal(value):
    require(all(value[key] is True for key in ("installed_inventory_preserved", "path_preserved", "state_preserved",
                                              "links_preserved", "package_directories_preserved"))
            and value["package_directory_exists"] is False
            and set(value["aliases"]) == {"flere.exe", "flere-connect.exe"}
            and all(alias["link_exists"] is False and alias["resolution"] is None for alias in value["aliases"].values()),
            "normal uninstall/preservation check failed")


def registry_inventory():
    import winreg
    result = []
    for hive_name, hive, view_name, view in (("HKCU", winreg.HKEY_CURRENT_USER, "64", winreg.KEY_WOW64_64KEY),
                                            ("HKLM", winreg.HKEY_LOCAL_MACHINE, "64", winreg.KEY_WOW64_64KEY),
                                            ("HKLM", winreg.HKEY_LOCAL_MACHINE, "32", winreg.KEY_WOW64_32KEY)):
        try:
            parent = winreg.OpenKey(hive, ARP, 0, winreg.KEY_READ | view)
        except FileNotFoundError:
            continue
        with parent:
            count = winreg.QueryInfoKey(parent)[0]; require(count <= 1024, "installed registry bound exceeded")
            names = sorted(winreg.EnumKey(parent, i) for i in range(count))
            for name in names:
                with winreg.OpenKey(parent, name) as child:
                    count_values = winreg.QueryInfoKey(child)[1]
                    require(count_values <= 128, "installed record value bound exceeded")
                    values, typed = {}, {}
                    for index in range(count_values):
                        key, value, kind = winreg.EnumValue(child, index)
                        values[key] = value
                        typed[key] = [kind, {"base64": base64.b64encode(value).decode()} if isinstance(value, bytes) else value]
                    raw = json.dumps(typed, sort_keys=True).encode(); require(len(raw) <= 65536, "installed record too large")
                row = {"key": hive_name + "/" + view_name + "/" + name, "hive": hive_name,
                       "view": view_name, "subkey": ARP + "\\" + name, "sha256": sha(raw)}
                # Export raw values only for the exact own ID; other software stays hashed.
                row["values"] = values if values.get("WinGetPackageIdentifier") == PACKAGE else {}
                require("flere" not in str(values.get("DisplayName", "")).lower() or row["values"],
                        "conflicting existing Flere package")
                result.append(row)
    require(len(result) <= 2048, "installed inventory exceeds bound")
    return sorted(result, key=lambda row: row["key"])


def normal_path():
    import winreg
    parts = []
    for hive, key in ((winreg.HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
                       (winreg.HKEY_CURRENT_USER, r"Environment")):
        with winreg.OpenKey(hive, key) as opened:
            try:
                value, kind = winreg.QueryValueEx(opened, "Path")
            except FileNotFoundError:
                continue
        require(kind in (winreg.REG_SZ, winreg.REG_EXPAND_SZ) and isinstance(value, str), "PATH registry type differs")
        parts.append(os.path.expandvars(value))
    return ";".join(parts)


def verify_link_path_baseline(path_value, links, before_links):
    def key(value):
        return ntpath.normpath(value.strip().strip('"')).casefold()
    present = key(str(links)) in {key(value) for value in path_value.split(";") if value.strip()}
    require(not (present and not before_links), "normal uninstall would remove a preexisting empty-links PATH entry")


def link_inventory(root):
    if not root.exists():
        require(not os.path.lexists(root), "dangling portable root")
        return {}
    base.file_record(root)
    paths = sorted(root.iterdir()); require(len(paths) <= 256, "portable links bound exceeded")
    result = {}
    for path in paths:
        st = path.lstat()
        if path.is_symlink():
            result[path.name] = {"link": os.readlink(path), "file_id": st.st_ino, "device": st.st_dev}
        else:
            result[path.name] = base.file_record(path)
    return result


class Run:
    def __init__(self, work, proof, receipt):
        self.work, self.proof, self.receipt = work, proof, receipt
        self.env = base.runtime_env(os.environ)
        self.deadline = time.monotonic() + 900  # Reserve cleanup inside a 20-minute job.

    def save(self):
        (self.proof / "receipt.json").write_text(json.dumps(self.receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def record(self, name, value):
        raw = json.dumps(value, indent=2, sort_keys=True).encode()
        require(len(raw) <= 1024 * 1024, "record exceeds bound")
        (self.proof / "records" / (name + ".json")).write_bytes(raw + b"\n")

    def command(self, name, argv, *, env=None, seconds=120, maximum=base.MAX_LOG, source_prompt=False, destination=None, accepted=(0,)):
        seconds = min(seconds, self.deadline - time.monotonic()); require(seconds > 0, "lifecycle deadline exhausted")
        log = destination or self.proof / "logs" / (name + ".log")
        errlog = self.proof / "logs" / (name + "-stderr.log")
        row = {"name": name, "argv": list(map(str, argv)), "status": "running", "confirmed_source": False}
        self.receipt["checks"].append(row); self.save(); start = time.monotonic(); failure = None
        with log.open("xb") as out, errlog.open("xb") as err:
            proc = subprocess.Popen(row["argv"], cwd=self.work, env=self.env if env is None else env,
                                    stdin=subprocess.PIPE, stdout=out, stderr=err)
            row["pid"] = proc.pid
            try:
                while proc.poll() is None:
                    require(time.monotonic() - start < seconds, name + " timed out")
                    require(log.stat().st_size <= maximum and errlog.stat().st_size <= base.MAX_LOG, name + " output bound")
                    if destination is None and prompt_ready(log.read_bytes() + errlog.read_bytes(), row["confirmed_source"], source_prompt):
                        require(not any(check["confirmed_source"] for check in self.receipt["checks"]), "source confirmation repeated across commands")
                        proc.stdin.write(b"Y\r\n"); proc.stdin.flush(); row["confirmed_source"] = True
                    time.sleep(0.05)
            except BaseException as error:
                failure = error
            finally:
                row["forced_stop"] = proc.poll() is None
                if row["forced_stop"]:
                    killed = subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"),
                        "/PID", str(proc.pid), "/T", "/F"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL, timeout=15)
                    require(killed.returncode == 0, "owned child tree could not be stopped")
                proc.wait(timeout=15); proc.stdin.close()
        for path, bound in ((log, maximum), (errlog, base.MAX_LOG)):
            if path.stat().st_size > bound:
                failure = failure or ValueError(name + " output bound")
                with path.open("r+b") as stream:
                    stream.truncate(bound)
        data, errors = log.read_bytes(), errlog.read_bytes()
        if destination is None and failure is None:
            try:
                require(not prompt_ready(data + errors, row["confirmed_source"], source_prompt), "prompt exited before response")
            except ValueError as error:
                failure = error
        row.update(exit=proc.returncode, seconds=round(time.monotonic()-start, 3),
                   stdout_bytes=len(data), stdout_sha256=sha(data), stderr_bytes=len(errors), stderr_sha256=sha(errors),
                   status="passed" if failure is None and proc.returncode in accepted else "failed")
        self.save()
        if failure:
            raise failure
        require(proc.returncode in accepted, name + " failed; inspect retained log")
        return data

    def ps(self, name, code):
        text = "$ErrorActionPreference='Stop'; " + code
        encoded = base64.b64encode(text.encode("utf-16-le")).decode()
        return json.loads(self.command(name, [self.pwsh, "-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", encoded]))


def main(output):
    candidate.hosted(os.environ)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64") and sys.version_info >= (3, 12), "native Windows AMD64 required")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=candidate.PROJECT, text=True, timeout=15).strip()
            == os.environ["FLERE_WORKFLOW_SHA"], "workflow checkout differs")
    require(output.parent.resolve() == Path.home().resolve()/".cache/flere/tmp" and not output.exists(), "fresh private home-cache output required")
    output.mkdir(); work = output/"work"; proof = output/"proof"
    for path in (work, proof, proof/"logs", proof/"records"):
        path.mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    receipt = {"schema": "flere-winget-lifecycle-v1", "status": "running", "checks": [],
        "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"], "run_id": os.environ["GITHUB_RUN_ID"],
        "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "input_artifact": base.ARTIFACT,
        "artifact_sha256": base.ARTIFACT_SHA, "product_commit": base.PRODUCT, "source_sha256": base.SOURCE,
        "zip_sha256": base.ZIP_SHA, "limits": ["First private-loopback install/remove; no public URL or upgrade proof.",
        "No WinGet detector/update-refusal, signing, physical UI, clipboard or SSH claim.",
        "Normal Microsoft community-source network access and OS archive malware scanning are enabled."],
        "machine": {"platform": platform.platform(), "image_os": os.environ.get("ImageOS"),
                    "image_version": os.environ.get("ImageVersion"), "python": platform.python_version()}}
    run = Run(work, proof, receipt); mirror = None; settings_before = None; changed_setting = False
    user_before = sources_before = None
    attempted = False; removed = False; manifests = work/"manifests"; installed_root = None

    def settings(name):
        value = json.loads(run.command(name, [winget, "settings", "export"]))
        setting_value(value); run.record(name, value); return value

    def user_file(name):
        # Do not parse arbitrary files; only the reported standard runner profile settings path.
        path = Path(settings_before["userSettingsFile"])
        require(path.is_absolute() and path.resolve().is_relative_to(local_appdata.resolve()) and path.name == "settings.json",
                "user settings path is outside normal runner LocalAppData")
        if not path.exists():
            require(not os.path.lexists(path), "settings path is dangling")
            # Invalid/missing primary settings can load the backup: reject that ambiguity.
            require(not path.parent.exists() or not list(path.parent.glob("settings.json*")), "backup-only user settings unsupported")
            return {"path": str(path), "present": False}
        value = base.file_record(path); require(value["bytes"] <= 65536, "user settings bound")
        escaped = str(path).replace("'", "''")
        parsed = run.ps(name, "$o=[System.Text.Json.JsonDocumentOptions]::new(); $o.AllowTrailingCommas=$true; "
            "$o.CommentHandling=[System.Text.Json.JsonCommentHandling]::Skip; "
            "$d=[System.Text.Json.JsonDocument]::Parse([IO.File]::ReadAllText('"+escaped+"'),$o); "
            "$d.RootElement.GetRawText() | ConvertFrom-Json -AsHashtable | ConvertTo-Json -Depth 20 -Compress")
        verify_user_settings(parsed)
        return {"path": str(path), "present": True, "bytes": value["bytes"], "sha256": value["sha256"]}

    def removal(name):
        inventory = registry_inventory(); normal = normal_path()
        facts = {"installed_inventory_preserved": inventory == before_inventory,
            "path_preserved": base.path_hashes() == before_path, "state_preserved": candidate.tree(fixture) == before_state,
            "links_preserved": link_inventory(links) == before_links,
            "package_directories_preserved": link_inventory(packages) == before_package_dirs,
            "package_directory_exists": installed_root is not None and os.path.lexists(installed_root),
            "aliases": {alias: {"link_exists": os.path.lexists(links/alias), "resolution": shutil.which(alias, path=normal)}
                        for alias in ("flere.exe", "flere-connect.exe")}}
        run.record(name, facts); run.record(name+"-inventory", inventory)
        verify_removal(facts)
        return facts

    try:
        gh = shutil.which("gh.exe"); require(gh, "existing GitHub CLI missing")
        api = f"repos/{base.REPO}/actions/artifacts/{base.ARTIFACT}"
        value = json.loads(run.command("artifact-api", [gh,"api",api], env=os.environ.copy(), maximum=65536))
        base.verify_api(value); run.record("artifact-api", value)
        artifact = run.command("artifact-download", [gh,"api",api+"/zip"], env=os.environ.copy(),
                               destination=work/"artifact.zip", maximum=base.ARTIFACT_BYTES)
        portable, _, _, manifest, payload, original = base.inputs(artifact, work)
        run.record("input-recipe-receipt", original); run.record("manifest", manifest)
        run.pwsh = Path(shutil.which("pwsh.exe") or "missing"); require(run.pwsh.is_file(), "native PowerShell7 unavailable")
        info = run.ps("machine", "$i=[Security.Principal.WindowsIdentity]::GetCurrent(); $p=[Security.Principal.WindowsPrincipal]$i; "
            "[pscustomobject]@{edition=$PSVersionTable.PSEdition;major=$PSVersionTable.PSVersion.Major;home=$PSHOME;"
            "elevated=$p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);"
            "caption=(Get-CimInstance Win32_OperatingSystem).Caption;locale=[Globalization.CultureInfo]::CurrentUICulture.Name} | ConvertTo-Json -Compress")
        require(info["edition"] == "Core" and info["major"] == 7 and info["elevated"] is True
                and info["locale"] == "en-US" and "Windows Server 2025" in info["caption"]
                and (Path(info["home"])/"pwsh.exe").resolve() == run.pwsh.resolve(),
                "already elevated hosted Windows2025/native PowerShell7 required")
        run.record("machine", info)
        located = shutil.which("winget.exe"); require(located, "preinstalled WinGet unavailable")
        winget = Path(located); local_appdata = Path(os.environ["LOCALAPPDATA"])
        require(winget == local_appdata/"Microsoft/WindowsApps/winget.exe", "normal preinstalled WinGet app alias required")
        identity = winget.lstat()
        run.record("manager", {"path": str(winget), "file_id": identity.st_ino, "device": identity.st_dev,
                   "reparse_tag": getattr(identity, "st_reparse_tag", None), "version_expected": WINGET_VERSION})
        require(run.command("winget-version", [winget,"--version"]).decode("utf-8-sig").strip() == WINGET_VERSION, "reviewed WinGet version differs")
        run.command("winget-info", [winget,"--info"]); receipt["machine"]["winget"] = WINGET_VERSION
        links = local_appdata/"Microsoft/WinGet/Links"
        packages = local_appdata/"Microsoft/WinGet/Packages"
        settings_before = settings("settings-before"); user_before = user_file("user-settings-before")
        sources_before = source_export(run.command("sources-before", [winget,"source","export"]))
        run.record("sources-before", sources_before)
        # No source reset, account, automatic agreements or custom source. Normal metadata access is explicit.
        run.command("community-source-update", [winget,"source","update","--name","winget"], seconds=180, source_prompt=True)
        run.command("installed-list-before", [winget,"list","--source","winget"], seconds=180, source_prompt=True)
        before_inventory = registry_inventory(); require(not any(row["values"] for row in before_inventory), "package already installed")
        run.record("installed-before", before_inventory)
        before_path = base.path_hashes(); before_links = link_inventory(links)
        before_normal_path = normal_path()
        verify_link_path_baseline(before_normal_path, links, before_links)
        require(all(shutil.which(alias, path=before_normal_path) is None and not os.path.lexists(links/alias)
                    for alias in ("flere.exe", "flere-connect.exe")), "alias collision")
        before_package_dirs = link_inventory(packages)
        require(not any("flere" in name.lower() for name in before_package_dirs), "existing portable Flere directory")
        run.record("path-before", before_path); run.record("links-before", before_links)
        fixture = work/"profile"; fixture.mkdir()
        for name in ("state", ".ssh", "AppData/Local", "AppData/Roaming", ".cache", "temp"):
            (fixture/name).mkdir(parents=True, exist_ok=True)
        (fixture/"state/preserved.json").write_text('{"synthetic":true,"selection":"retained"}\n')
        (fixture/".ssh/config").write_text("# synthetic sentinel; no SSH is launched\n")
        before_state = candidate.tree(fixture); run.record("state-before", before_state)
        if not setting_value(settings_before):
            changed_setting = True
            run.command("enable-local-manifest", [winget,"settings","--enable","LocalManifestFiles"])
        verify_settings(settings_before, settings("settings-during"), True)
        mirror = base.Mirror(portable)
        run.record("private-manifests", local_manifests(artifact, manifests, mirror.server_port))
        run.command("validate", [winget,"validate",manifests])
        attempted = True
        run.command("install", [winget,"install","--manifest",manifests,"--scope","user","--architecture","x64"], seconds=180)
        after_inventory = registry_inventory(); run.record("installed-after", after_inventory)
        verify_inventory(before_inventory, after_inventory, True); owned = own_record(after_inventory)
        installed_root = Path(owned["values"]["InstallLocation"])
        require(installed_root.is_absolute() and installed_root.parent.resolve() == packages.resolve(), "portable path outside default user root")
        base.file_record(packages); base.file_record(installed_root)
        entries = list(installed_root.iterdir()); extras = [p for p in entries if p.name not in payload]
        require(len(entries) == len(payload)+1 and len(extras) == 1 and extras[0].suffix == ".db", "portable payload/index inventory differs")
        installed_files = [base.file_record(path) for path in entries]
        for name, data in payload.items():
            require(base.file_record(installed_root/name)["sha256"] == sha(data), "installed payload differs")
        require(extras[0].stat().st_size <= 512*1024, "portable index exceeds bound")
        (proof/"records/portable-index.bin").write_bytes(extras[0].read_bytes())
        run.record("installed-layout", {"registry": owned, "files": installed_files,
                   "index_retained": "portable-index.bin", "identity_note": "Observed native file IDs/hashes; no product manager detector claim."})
        listing = run.command("installed-list-exact", [winget,"list","--id",PACKAGE,"--exact","--scope","user","--source","winget"], source_prompt=True)
        require(PACKAGE.encode() in clean_console(listing) and VERSION.encode() in listing, "normal installed manager identity missing")
        after_links = link_inventory(links)
        require({k:v for k,v in after_links.items() if k not in ("flere.exe","flere-connect.exe")} == before_links, "unrelated portable links changed")
        child_env = dict(run.env, PATH=normal_path(), HOME=str(fixture), USERPROFILE=str(fixture),
            LOCALAPPDATA=str(fixture/"AppData/Local"), APPDATA=str(fixture/"AppData/Roaming"),
            XDG_CACHE_HOME=str(fixture/".cache"), TEMP=str(fixture/"temp"), TMP=str(fixture/"temp"))
        aliases = {}
        for alias in ("flere.exe", "flere-connect.exe"):
            path = Path(shutil.which(alias, path=child_env["PATH"]) or "missing")
            require(path == links/alias and path.is_symlink() and path.resolve() == (installed_root/alias).resolve(), "normal WinGet PATH symlink differs")
            aliases[alias] = {"link": after_links[alias], "resolved": base.file_record(path.resolve())}
            for flag in ("--help", "--version", "--build-info"):
                data = run.command(alias[:-4]+"-"+flag[2:], [path,flag], env=child_env, seconds=20, maximum=32768)
                if flag == "--build-info":
                    require(json.loads(data) == manifest["build"], "alias full build identity differs")
                elif flag == "--version":
                    require(data.decode().strip().startswith("flere-connect "+VERSION+" ") and manifest["build"]["build_id"] in data.decode(), "alias version differs")
                else:
                    require(b"--build-info" in data and b"ssh" in data, "alias help differs")
            require(base.file_record(path.resolve()) == aliases[alias]["resolved"], "alias payload changed")
        run.record("aliases", aliases); require(candidate.tree(fixture) == before_state, "stateless calls changed state")
        run.command("uninstall", [winget,"uninstall","--manifest",manifests,"--scope","user","--source","winget"], seconds=180, source_prompt=True)
        removed = True; removal("removal")
        run.command("installed-list-after", [winget,"list","--source","winget"], source_prompt=True)
        require(link_inventory(packages) == before_package_dirs, "unrelated portable package directories changed")
        receipt.update(status="lifecycle_complete_pending_cleanup", aliases_checked=6,
            installed_inventory_preserved=True, state_preserved=True, path_preserved=True, no_product_owner_claim=True)
    except BaseException as error:
        receipt.update(status="failed", error=str(error), traceback=traceback.format_exc())
    finally:
        run.deadline = time.monotonic()+180; errors = []
        if attempted and not removed:
            try:
                current = registry_inventory()
                if any(row["values"] for row in current):
                    owned = own_record(current, complete=False)
                    # RegisterARPEntry precedes directory creation; a partial
                    # owned record still receives ordinary manager cleanup.
                    if owned["values"].get("InstallLocation"):
                        path = Path(owned["values"]["InstallLocation"])
                        require(path.is_absolute() and path.parent.resolve() == packages.resolve(), "cleanup record path differs")
                        installed_root = path
                    run.command("failure-uninstall", [winget,"uninstall","--manifest",manifests,"--scope","user","--source","winget"], source_prompt=True)
                removal("failure-removal")
            except BaseException as error:
                errors.append("normal uninstall: "+str(error))
        if changed_setting:
            try:
                run.command("restore-local-manifest", [winget,"settings","--disable","LocalManifestFiles"])
            except BaseException as error:
                errors.append("settings restore: "+str(error))
        if settings_before is not None:
            try:
                verify_settings(settings_before, settings("settings-restored"), setting_value(settings_before))
                if user_before is not None:
                    require(user_file("user-settings-restored") == user_before, "user settings changed")
                if sources_before is not None:
                    require(source_export(run.command("sources-restored", [winget,"source","export"])) == sources_before, "source definitions changed")
                    receipt["sources_preserved"] = True
                receipt["settings_restored"] = True
            except BaseException as error:
                errors.append("settings/source verification: "+str(error))
        if mirror is not None:
            try:
                mirror.close_owned(); receipt["loopback_closed"] = True
                base.verify_mirror(mirror.requests, mirror.attempts, mirror.rejections, mirror.rejections_dropped, mirror.internal_errors)
                receipt["loopback_verified"] = True
            except BaseException as error:
                errors.append("loopback: "+str(error))
            receipt.update(loopback_requests=mirror.requests, loopback_response_attempts=mirror.attempts,
                loopback_rejections=mirror.rejections, loopback_rejections_dropped=mirror.rejections_dropped, loopback_internal_errors=mirror.internal_errors)
        receipt["cleanup_errors"] = errors
        if errors:
            receipt["status"] = "failed"
        elif receipt["status"] == "lifecycle_complete_pending_cleanup":
            receipt["status"] = "lifecycle_passed"
        files = [path for path in proof.rglob("*") if path.is_file() and path.name != "receipt.json"]
        if len(files) > 96 or sum(path.stat().st_size for path in files) > 8*1024*1024:
            receipt.update(status="failed", error="curated proof bound exceeded")
        receipt["files"] = {path.relative_to(proof).as_posix(): {"bytes":path.stat().st_size,"sha256":sha(path.read_bytes())} for path in sorted(files)}
        run.save()
    require(receipt["status"] == "lifecycle_passed", "WinGet lifecycle failed; inspect bounded proof")



def self_test():
    """Pure protocol/preservation regressions; no manager, native payload or network."""
    import unittest
    class Guards(unittest.TestCase):
        def source(self):
            return {"Name":"winget", "Arg":SOURCE_URL, "Type":"Microsoft.PreIndexed.Package",
                    "Identifier":SOURCE_ID, "Explicit":False, "Data":SOURCE_ID, "TrustLevel":["Trusted","StoreOrigin"]}

        def settings(self):
            return {"$schema":SETTING_SCHEMA,"userSettingsFile":"C:\\fixture\\settings.json",
                    "adminSettings":{"LocalManifestFiles":False,"InstallerHashOverride":False}}

        def owned(self):
            return {"key":"HKCU/64/observed","hive":"HKCU","view":"64","sha256":"a"*64,
                    "values":{"WinGetPackageIdentifier":PACKAGE,"DisplayVersion":VERSION,"DisplayName":"Flere Connect",
                              "WinGetInstallerType":"portable","WinGetSourceIdentifier":"*DefaultSource",
                              "InstallDirectoryCreated":1}}

        def test_hosted_guard_each_identity(self):
            env={"GITHUB_ACTIONS":"true","FLERE_RUNNER_ENVIRONMENT":"github-hosted","GITHUB_EVENT_NAME":"workflow_dispatch",
                 "GITHUB_REPOSITORY":base.REPO,"GITHUB_REF":"refs/heads/main","RUNNER_OS":"Windows","RUNNER_ARCH":"X64",
                 "FLERE_WORKFLOW_SHA":"a"*40,"GITHUB_RUN_ID":"1","GITHUB_RUN_ATTEMPT":"1"}
            candidate.hosted(env)
            for key in env:
                with self.subTest(key=key), self.assertRaises(ValueError):
                    candidate.hosted(dict(env, **{key:"unexpected"}))

        def test_source_prompt_every_split_and_one_answer(self):
            raw=SOURCE_TITLE+b"\r\n"+SOURCE_PROMPT+b"\r\n"+OPTIONS
            for at in range(len(raw)):
                self.assertFalse(prompt_ready(raw[:at],False,True))
                self.assertTrue(prompt_ready(raw[:at]+raw[at:],False,True))
            self.assertFalse(prompt_ready(raw,True,True))
            self.assertFalse(prompt_ready(b"Found Flere Connect. Installing...",False,False))

        def test_changed_unknown_repeated_and_out_of_phase_prompts(self):
            good=SOURCE_TITLE+b"\n"+SOURCE_PROMPT+b"\n"+OPTIONS
            for raw,allowed in ((good,False),(OPTIONS,True),(good+OPTIONS,True),(good.replace(b"`winget`",b"`msstore`"),True),
                                (good.replace(SOURCE_PROMPT,b"2-letter geographic region\n"+SOURCE_PROMPT),True),
                                (b"The publisher requires that you view terms.\n"+OPTIONS,True)):
                with self.subTest(raw=raw), self.assertRaises(ValueError): prompt_ready(raw,False,allowed)

        def test_actual_source_export_object_stream(self):
            other=dict(self.source(),Name="msstore",Arg="https://storeedgefd.dsx.mp.microsoft.com/v9.0")
            raw=(json.dumps(other)+"\n"+json.dumps(self.source())+"\n").encode()
            self.assertEqual(len(source_export(raw)),2)
            for bad in (raw+b"warning",json.dumps([self.source()]).encode(),raw+json.dumps(self.source()).encode(),
                        json.dumps(dict(self.source(),Arg="http://127.0.0.1")).encode()):
                with self.assertRaises((ValueError,json.JSONDecodeError)):source_export(bad)

        def test_observed_boolean_setting_only_and_restoration(self):
            old=self.settings();new=copy.deepcopy(old);new["adminSettings"]["LocalManifestFiles"]=True
            verify_settings(old,new,True);verify_settings(old,old,False);verify_settings(new,new,True)
            for key,val in (("InstallerHashOverride",True),("LocalManifestFiles","false")):
                bad=copy.deepcopy(old);bad["adminSettings"][key]=val
                with self.assertRaises(ValueError):verify_settings(old,bad,False)
            bad=copy.deepcopy(new);bad["userSettingsFile"]="C:\\other\\settings.json"
            with self.assertRaises(ValueError):verify_settings(old,bad,True)

        def test_custom_root_prompt_and_purge_settings_rejected(self):
            verify_user_settings({"visual":{"progressBar":"accent"}})
            for value in ({"installBehavior":{"portablePackageUserRoot":"C:\\other"}},
                          {"interactivity":{"disable":True}},{"uninstallBehavior":{"purgePortablePackage":True}},
                          {"interactivity":{"disable":"false"}}):
                with self.assertRaises(ValueError):verify_user_settings(value)

        def test_existing_empty_links_path_rejected_with_windows_spelling(self):
            root=r"C:\Users\fixture\AppData\Local\Microsoft\WinGet\Links"
            for value in (root, root.upper()+"\\", '"'+root+'\\"', root.replace("\\","/")):
                with self.assertRaises(ValueError):verify_link_path_baseline("C:\\bin;"+value,root,{})
            verify_link_path_baseline("C:\\bin",root,{})
            verify_link_path_baseline(root,root,{"unrelated.exe":{"link":"unchanged"}})

        def test_active_owner_record_and_wrong_identity(self):
            own=self.owned();self.assertEqual(own_record([own]),own)
            for key,val in (("WinGetSourceIdentifier","elsewhere"),("DisplayVersion","9.0.0"),("WinGetInstallerType","exe")):
                bad=copy.deepcopy(own);bad["values"][key]=val
                with self.assertRaises(ValueError):own_record([bad])
            with self.assertRaises(ValueError):own_record([own,own])

        def test_partial_owned_record_can_receive_normal_cleanup(self):
            own=self.owned();del own["values"]["InstallDirectoryCreated"]
            with self.assertRaises(ValueError):own_record([own])
            self.assertEqual(own_record([own],complete=False),own)
            own["values"]["InstallDirectoryAddedToPath"]=1
            self.assertEqual(own_record([own],complete=False),own)
            with self.assertRaises(ValueError):own_record([own])

        def test_unrelated_inventory_and_own_key_preservation(self):
            old=[{"key":"HKLM/64/other","sha256":"b"*64,"values":{}}];own=self.owned()
            verify_inventory(old,old+[own],True);verify_inventory(old,old,False)
            for after,installed in (([dict(old[0],sha256="c"*64),own],True),(old+[own],False),(old,True)):
                with self.assertRaises(ValueError):verify_inventory(old,after,installed)
            with self.assertRaises(ValueError):verify_inventory(old+[own],old+[own],True)

        def test_removal_rejects_each_residual_or_preservation_failure(self):
            value={key:True for key in ("installed_inventory_preserved","path_preserved","state_preserved",
                                       "links_preserved","package_directories_preserved")}
            value.update(package_directory_exists=False,aliases={name:{"link_exists":False,"resolution":None}
                         for name in ("flere.exe","flere-connect.exe")})
            verify_removal(value)
            for key in ("installed_inventory_preserved","path_preserved","state_preserved","links_preserved","package_directories_preserved"):
                with self.assertRaises(ValueError):verify_removal(dict(value,**{key:False}))
            with self.assertRaises(ValueError):verify_removal(dict(value,package_directory_exists=True))
            changed=copy.deepcopy(value);changed["aliases"]["flere.exe"]["resolution"]="C:\\unexpected\\flere.exe"
            with self.assertRaises(ValueError):verify_removal(changed)
            changed=copy.deepcopy(value);del changed["aliases"]["flere-connect.exe"]
            with self.assertRaises(ValueError):verify_removal(changed)

        def test_actual_frozen_manifest_single_url_substitution(self):
            original=base64.b64decode("IyB5YW1sLWxhbmd1YWdlLXNlcnZlcjogJHNjaGVtYT1odHRwczovL2FrYS5tcy93aW5nZXQtbWFuaWZlc3QuaW5zdGFsbGVyLjEuMTAuMC5zY2hlbWEuanNvbg0KUGFja2FnZUlkZW50aWZpZXI6IFJvYmVydENyb25pbi5GbGVyZUNvbm5lY3QNClBhY2thZ2VWZXJzaW9uOiAiMC4zLjQiDQpJbnN0YWxsZXJUeXBlOiB6aXANCk5lc3RlZEluc3RhbGxlclR5cGU6IHBvcnRhYmxlDQpJbnN0YWxsZXJzOg0KLSBBcmNoaXRlY3R1cmU6IHg2NA0KICBOZXN0ZWRJbnN0YWxsZXJGaWxlczoNCiAgLSBSZWxhdGl2ZUZpbGVQYXRoOiBmbGVyZS5leGUNCiAgICBQb3J0YWJsZUNvbW1hbmRBbGlhczogZmxlcmUNCiAgLSBSZWxhdGl2ZUZpbGVQYXRoOiBmbGVyZS1jb25uZWN0LmV4ZQ0KICAgIFBvcnRhYmxlQ29tbWFuZEFsaWFzOiBmbGVyZS1jb25uZWN0DQogIEluc3RhbGxlclVybDogImh0dHBzOi8vZ2l0aHViLmNvbS9yb2JlcnQtY3JvbmluL2ZsZXJlL3JlbGVhc2VzL2Rvd25sb2FkL3YwLjMuNC9mbGVyZS1jb25uZWN0LTAuMy40LXg4Nl82NC1wYy13aW5kb3dzLW1zdmMuemlwIg0KICBJbnN0YWxsZXJTaGEyNTY6IEU1NjJGOTkxNjhBMzQyMDJFQ0FBMkRDMTYzREIyNjVFOURGM0MwMDA3Njk5RkJCN0ZFQTZCMjRDNkM4RTczOTINCk1hbmlmZXN0VHlwZTogaW5zdGFsbGVyDQpNYW5pZmVzdFZlcnNpb246IDEuMTAuMA0K")
            name=PACKAGE+".installer.yaml";port=49152
            changed=local_manifest(name,original,port)
            local=f"http://127.0.0.1:{port}/{base.ZIP_NAME}".encode()
            self.assertEqual(changed.replace(local,base.PUBLIC_URL),original)
            self.assertIn(base.ZIP_SHA.upper().encode(),changed)
            for data in (original+b"\n",original.replace(base.ZIP_SHA.upper().encode(),b"A"*64)):
                with self.assertRaises(ValueError):local_manifest(name,data,port)
            for bad in (True,80,65536,"49152"):
                with self.assertRaises(ValueError):local_manifest(name,original,bad)

        def test_completed_object_mirror_contract_is_reused(self):
            row={"status":200,"path":"/"+base.ZIP_NAME,"method":"GET","content_length":1771884,
                 "bytes":1771884,"sha256":base.ZIP_SHA}
            base.verify_mirror([row],1,[{"status":400}],0,0)
            for rows,attempts,dropped,errors in (([dict(row,bytes=1)],1,0,0),([],1,0,0),([row],1,1,0),([row],1,0,1)):
                with self.assertRaises(ValueError):base.verify_mirror(rows,attempts,[],dropped,errors)

        def test_runtime_credentials_removed_without_module_override(self):
            self.assertEqual(base.runtime_env({"GH_TOKEN":"fake","PATH":"normal","PSModulePath":"normal-modules"}),
                             {"PATH":"normal","PSModulePath":"normal-modules"})

    result=unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Guards))
    require(result.wasSuccessful(),"WinGet lifecycle guard checks failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        require(args.output is not None, "output is required")
        main(args.output)
