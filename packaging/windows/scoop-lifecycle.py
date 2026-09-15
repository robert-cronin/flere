#!/usr/bin/env python3
"""Normal Scoop public install/remove or reviewed034-to-035 candidate upgrade.

Elevated hosted per-user acceptance; optional installed owner JSON is not UI,
external SSH or ordinary non-admin desktop acceptance.
"""
import argparse
import copy
import io
import importlib.util
import json
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
SPEC = importlib.util.spec_from_file_location("winget_lifecycle", Path(__file__).with_name("winget-lifecycle.py"))
common = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(common)
base, candidate = common.base, common.candidate
require, sha = candidate.require, candidate.sha
packager = candidate.module("windows-manifests")
VERSION, TARGET = "0.3.4", "x86_64-pc-windows-msvc"
PRODUCT = "32108e352f4a51d809b1e5ed64cc9dba3bc0be88"
ZIP_NAME = "flere-connect-0.3.4-x86_64-pc-windows-msvc.zip"
PUBLIC_URL = "https://github.com/robert-cronin/flere/releases/download/v0.3.4/" + ZIP_NAME
ZIP_BYTES, ZIP_SHA = 1822693, "71fd96f768894d849c4773add52bd0e3131e032c77640a02aa1c4af36e5001db"
PAYLOAD_SHA = "ccf4985a7df7174206cb6ca9f6df21dd0619a4402f2f8f11736a7b0fd940b302"
MANIFEST_SHA = "a6038e1f993b13ef953b7818e77fae639b228546cf2d88325a30dcbd8024ff31"
LICENSE_SHA = "b997b4ab95fef146345de8b279c4d6dff41fc4a198d2bfe2488ba0ae0ac071d5"
INSTALLER_COMMIT = "1e2f334083d609986d8c8bc9e31ae8e87c39fab4"
INSTALLER_URL = f"https://raw.githubusercontent.com/ScoopInstaller/Install/{INSTALLER_COMMIT}/install.ps1"
INSTALLER_BYTES, INSTALLER_SHA = 28743, "94f983b190438311e006b957db7c8422709e0ba62a6c2ac04e278164108f2512"


DEFAULT_INPUT = "public-v0.3.4"
PUBLIC_INPUT = {"name": DEFAULT_INPUT, "version": VERSION, "owner_check": False,
    "product": PRODUCT, "source": "9a801df30c3e71a1c61bc04cf3ed15b1f8574ea46b9bb2c75561c3850ff3bf35",
    "zip_bytes": ZIP_BYTES, "zip_sha": ZIP_SHA, "payload_sha": PAYLOAD_SHA, "manifest_sha": MANIFEST_SHA}
OWNER_INPUT_NAME = "candidate-scoop-owner"
# Fill only after independent review of one successful 0.3.5 Windows candidate.
# No caller-supplied URLs, commits or hashes can activate this unavailable slot.
OWNER_INPUT = None
OWNER_KEYS = {"name", "version", "owner_check", "run", "artifact", "artifact_bytes", "artifact_name",
    "workflow", "artifact_sha", "product", "source", "zip_sha", "zip_bytes", "payload_sha", "manifest_sha",
    "nuspec_sha", "script_sha", "nupkg_sha", "receipt_sha", "source_entries", "artifact_entries"}
OWNER_GUIDANCE = ("Local Scoop: This companion is installed by Scoop. Use Scoop with the next reviewed Flere "
    "manifest/package to upgrade, or remove it with Scoop, then reopen the companion. In-app Apply is disabled.")


def selection(name=DEFAULT_INPUT):
    if name == DEFAULT_INPUT:
        return copy.deepcopy(PUBLIC_INPUT)
    require(name == OWNER_INPUT_NAME and isinstance(OWNER_INPUT, dict), "reviewed Scoop owner candidate pins are not configured")
    value = copy.deepcopy(OWNER_INPUT)
    require(set(value) == OWNER_KEYS and value["name"] == name and value["version"] == "0.3.5"
            and value["owner_check"] is True, "Scoop owner candidate identity differs")
    for key in ("workflow", "product"):
        require(isinstance(value[key], str) and re.fullmatch(r"[0-9a-f]{40}", value[key]), "candidate commit differs")
    for key in ("artifact_sha", "source", "zip_sha", "payload_sha", "manifest_sha", "nuspec_sha", "script_sha", "nupkg_sha", "receipt_sha"):
        require(isinstance(value[key], str) and re.fullmatch(r"[0-9a-f]{64}", value[key]), "candidate digest differs")
    for key, maximum in (("run", 10**15), ("artifact", 10**15), ("artifact_bytes", 16*1024*1024),
                         ("zip_bytes", 8*1024*1024), ("source_entries", 2048), ("artifact_entries", 128)):
        require(type(value[key]) is int and 0 < value[key] <= maximum, "candidate bound differs")
    require(value["artifact_name"] == f"windows-candidate-{value['run']}-1"
            and value["product"] != PRODUCT and value["payload_sha"] != PAYLOAD_SHA, "candidate must be a distinct reviewed035 build")
    return value


def public_url(selected):
    return f"https://github.com/robert-cronin/flere/releases/download/v{base.input_version(selected)}/{base.zip_name(selected)}"


def hosted(environment):
    candidate.hosted(environment)
    require(environment.get("CI") == "true", "requires inherited GitHub CI")
    require(not any(environment.get(key) for key in ("SCOOP", "SCOOP_GLOBAL", "SCOOP_CACHE", "XDG_CONFIG_HOME")),
            "custom Scoop/config roots are outside this fixture")


def recipe(selected=PUBLIC_INPUT, url=None):
    version = base.input_version(selected)
    url = public_url(selected) if url is None else url
    value = packager.scoop_manifest(version, url, selected["zip_sha"])
    require(value["pre_install"] == 'Rename-Item -LiteralPath "$dir\\manifest.json" -NewName \'flere-release.manifest.json\' -ErrorAction Stop',
            "only the fixed release-manifest rename is reviewed")
    require(set(value) == {"version", "description", "homepage", "license", "architecture", "bin", "pre_install", "notes"}
            and value["architecture"] == {"64bit": {"url": url, "hash": selected["zip_sha"]}}
            and value["version"] == version and value["license"] == "MIT"
            and value["bin"] == ["flere.exe", "flere-connect.exe"], "Scoop recipe contract differs")
    return value


def verify_bytes(path, size, digest):
    data = packager.read_regular(path, size)
    require(len(data) == size and sha(data) == digest, "downloaded bytes differ: " + path.name)
    return data


def payload(path, selected=PUBLIC_INPUT):
    verify_bytes(path, selected["zip_bytes"], selected["zip_sha"])
    with zipfile.ZipFile(path) as archive:
        require(set(archive.namelist()) == {"flere.exe", "flere-connect.exe", "manifest.json", "LICENSE"}, "ZIP inventory differs")
        require(all(item.file_size <= 3 * 1024 * 1024 for item in archive.infolist()), "ZIP expanded bound")
        raw, license_bytes = archive.read("manifest.json"), archive.read("LICENSE")
    require(sha(raw) == selected["manifest_sha"] and sha(license_bytes) == LICENSE_SHA, "public release records differ")
    manifest = json.loads(raw)
    require(manifest["source"]["git_commit"] == selected["product"] and manifest["source"]["dirty"] is False
            and manifest["build"]["package_version"] == base.input_version(selected) and manifest["build"]["target"] == TARGET
            and manifest["payload"]["sha256"] == selected["payload_sha"]
            and manifest["source"]["source_sha256"] == selected["source"], "public payload provenance differs")
    candidate.verify_zip(path, manifest, license_bytes)
    return manifest, raw


class Run(common.Run):
    # Reuse the normal bounded command/JSON runner without a Scoop prompt adapter.
    def download(self, name, url, size, digest):
        path = self.work / name
        self.command("download-" + name, [self.curl, "--fail", "--silent", "--show-error", "--location",
            "--proto", "=https", "--proto-redir", "=https", "--max-time", "90", "--max-filesize", str(size),
            "--output", path, url], seconds=100)
        verify_bytes(path, size, digest)
        self.record(name + "-pin", {"url": url, "bytes": size, "sha256": digest})
        return path

    def scoop(self, name, *arguments, **options):
        return self.command(name, [self.pwsh, "-NoLogo", "-NoProfile", "-File", self.entry, *arguments], **options)


def verify_cli(flag, data, manifest):
    if flag == "build-info":
        require(json.loads(data) == manifest["build"], "alias full build-info differs")
    elif flag == "version":
        expected = f"flere-connect {manifest['build']['package_version']} ({TARGET}; build {manifest['build']['build_id']})"
        require(data.decode().strip() == expected, "alias version/build differs")
    else:
        require(b"--build-info" in data and b"ssh" in data, "alias help missing")


def installed_record(value, install, expected, manifest_path):
    require(value == expected, "installed Scoop manifest differs")
    require(install == {"architecture": "64bit", "url": str(manifest_path)}, "installed Scoop source/architecture differs")


def verify_removal(value):
    require(value == {"application_absent": True, "aliases_absent": True, "list_absent": True,
                      "unrelated_inventory_preserved": True, "path_preserved": True, "state_preserved": True},
            "normal Scoop removal or preservation failed")


def manager_revisions(run, root, label):
    result = {}
    for name, path in (("Scoop", root/"apps/scoop/current"), ("Main", root/"buckets/main")):
        require(path.is_dir(), "normal Git bootstrap did not produce " + name)
        head = run.command(label+"-"+name+"-head", [run.git, "-C", path, "rev-parse", "HEAD"]).decode().strip()
        origin = run.command(label+"-"+name+"-origin", [run.git, "-C", path, "remote", "get-url", "origin"]).decode().strip()
        dirty = run.command(label+"-"+name+"-status", [run.git, "-C", path, "status", "--porcelain", "--untracked-files=no"])
        require(re.fullmatch(r"[0-9a-f]{40}", head) and origin == f"https://github.com/ScoopInstaller/{name}.git"
                and not dirty.strip(), "normal upstream manager identity differs")
        result[name] = {"commit": head, "origin": origin}
    run.record(label+"-manager-revisions", result)
    return result


def app_names(root):
    apps = root/"apps"
    if not apps.exists():
        return []
    names = sorted(item.name for item in apps.iterdir())
    require(len(names) <= 8 and all(name in ("scoop", "flere") for name in names), "unexpected app in fixture manager")
    return [name for name in names if name != "scoop"]


def aliases_absent(root):
    return all(not any(os.path.lexists(root/"shims"/(name+suffix)) for suffix in (".exe", ".shim", ".cmd", ".ps1", ""))
               and shutil.which(name, path=common.normal_path()) is None for name in ("flere", "flere-connect"))


def inspect_installed(run, root, manifest, manifest_path, expected_recipe, selected, environment, prefix=""):
    version = base.input_version(selected)
    listing = run.scoop(prefix+"installed-list", "list", "^flere$")
    require(re.search((rb"(?m)^\s*flere\s+"+re.escape(version.encode())+rb"(?:\s|$)"), common.clean_console(listing)), "normal installed list lacks exact package/version")
    version_dir, current = root/"apps/flere"/version, root/"apps/flere/current"
    require(app_names(root) == ["flere"] and current.is_junction() and current.resolve() == version_dir.resolve(), "current version junction differs")
    expected_names = {"flere.exe", "flere-connect.exe", "LICENSE", "manifest.json", "flere-release.manifest.json", "install.json"}
    require({item.name for item in version_dir.iterdir()} == expected_names, "installed payload/record inventory differs")
    records = {item.name: base.file_record(item) for item in version_dir.iterdir()}
    run.record(prefix+"installed-files", records)
    for name in ("manifest.json", "flere-release.manifest.json", "install.json"):
        (run.proof/"records"/(prefix+"installed-"+name)).write_bytes((version_dir/name).read_bytes())
    installed_record(json.loads((version_dir/"manifest.json").read_text(encoding="utf-8-sig")),
                     json.loads((version_dir/"install.json").read_text(encoding="utf-8-sig")), expected_recipe, manifest_path)
    require(records["flere-release.manifest.json"]["sha256"] == selected["manifest_sha"]
            and records["LICENSE"]["sha256"] == LICENSE_SHA, "release manifest/license bytes changed")
    run.record(prefix+"current-junction", {"path": str(current), "target": str(current.resolve()), "version": version})
    normal_path = common.normal_path(); environment["PATH"] = normal_path
    aliases = {}
    for name in ("flere", "flere-connect"):
        alias = shutil.which(name+".exe", path=normal_path)
        require(alias is not None and Path(alias) == root/"shims"/(name+".exe"), "PATH alias is not the Scoop shim")
        require(records[name+".exe"]["sha256"] == selected["payload_sha"], "installed executable differs")
        shim = root/"shims"/(name+".shim")
        aliases[name] = {"alias": base.file_record(Path(alias)), "definition": base.file_record(shim)}
        (run.proof/"records"/(prefix+name+".shim.bin")).write_bytes(shim.read_bytes())
        for flag in ("version", "build-info", "help"):
            data = run.command(prefix+name+"-"+flag, [alias, "--"+flag], env=environment, seconds=30)
            verify_cli(flag, data, manifest)
    run.record(prefix+"aliases", aliases)
    return {"version": version, "build": manifest["build"], "payload_sha": selected["payload_sha"],
            "current": str(current.resolve()), "files": records, "aliases": aliases}


def verify_upgrade(before, after):
    require(before["version"] == "0.3.4" and after["version"] == "0.3.5"
            and before["current"] != after["current"] and before["build"] != after["build"]
            and before["payload_sha"] != after["payload_sha"], "normal Scoop update was not an actual034-to-035 upgrade")


def verify_installed_owner(value, manifest, executable):
    require(isinstance(value, dict) and {key: item for key, item in value.items() if key != "ownership"} == {
        "schema_version": 1, "running_build": manifest["build"], "installation": None,
        "other_frontends": "untracked"}, "installed status fields/full build differ")
    owner = value.get("ownership")
    require(isinstance(owner, dict) and common.windows_path(owner.get("executable")) == common.windows_path(str(executable)),
            "owner executable is not the selected installed alias payload")
    require(owner == {"kind":"manager", "executable":owner["executable"], "sha256":manifest["payload"]["sha256"],
                     "attempt":None, "guidance":OWNER_GUIDANCE}, "verified Scoop owner/guidance differs")
    return owner


def verify_synthetic_override(environment, root):
    home, installed_root = common.windows_path(environment["HOME"]), common.windows_path(str(root))
    require(common.windows_path(environment["USERPROFILE"]) == home and home != installed_root.parent,
            "synthetic profile must differ from actual installed profile")
    keys = ("LOCALAPPDATA", "APPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME", "TEMP", "TMP", "FLERE_STATE_DIR", "SCOOP")
    for key in keys:
        require(common.windows_path(environment[key]).is_relative_to(home), "synthetic path escaped: "+key)
    require(common.windows_path(environment["SCOOP"]) != installed_root, "synthetic SCOOP must not identify actual manager")
    return {key:environment[key] for key in ("HOME", "USERPROFILE", *keys)}


def candidate_input(run, selected):
    gh = shutil.which("gh.exe"); require(gh, "existing GitHub CLI missing")
    api = f"repos/{base.REPO}/actions/artifacts/{selected['artifact']}"
    value = json.loads(run.command("artifact-api", [gh,"api",api], env=os.environ.copy(), maximum=65536))
    base.verify_api(value, selected); run.record("artifact-api", value)
    archive = run.command("artifact-download", [gh,"api",api+"/zip"], env=os.environ.copy(),
                          destination=run.work/"candidate-artifact.zip", maximum=selected["artifact_bytes"])
    portable, _, _, manifest, members, receipt = base.inputs(archive, run.work, selected)
    require(sha(members["manifest.json"]) == selected["manifest_sha"], "candidate manifest bytes differ")
    payload(run.work/base.zip_name(selected), selected)
    with zipfile.ZipFile(io.BytesIO(archive)) as packed:
        generated = json.loads(packed.read("windows/scoop/bucket/flere.json"))
    require(generated == recipe(selected), "reviewed candidate Scoop recipe differs from maintained generator")
    run.record("candidate-receipt", receipt)
    run.record("candidate-public-manifest", manifest)
    return portable, manifest


def main(output, selected=PUBLIC_INPUT):
    hosted(os.environ)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64") and sys.version_info >= (3, 12), "native Windows/Python3.12+ required")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=candidate.PROJECT, text=True, timeout=15).strip()
            == os.environ["FLERE_WORKFLOW_SHA"], "workflow checkout differs")
    require(output.parent.resolve() == Path.home().resolve()/".cache/flere/tmp" and not output.exists(), "fresh home-cache output required")
    output.mkdir(); work, proof = output/"work", output/"proof"
    for path in (work, proof, proof/"logs", proof/"records"):
        path.mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    receipt = {"schema": "flere-scoop-lifecycle-v1", "status": "running", "checks": [],
        "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"], "run_id": os.environ["GITHUB_RUN_ID"],
        "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "product_commit": selected["product"], "zip_sha256": selected["zip_sha"],
        "input_selection": selected["name"], "installed_owner_check_required": selected["owner_check"],
        "limits": ["Per-user installation on an elevated hosted CI VM; not unelevated desktop acceptance.",
                   ("Public034 install then normal candidate035 upgrade and installed owner JSON; candidate download uses private loopback, no UI refusal proof." if selected["owner_check"] else
                    "First public-ZIP install/remove and record capture; no upgrade or Scoop owner detector proof."),
                   "Normal bootstrap fetches mutable official Scoop/Main revisions, recorded separately from the pinned installer.",
                   "No UI, clipboard, external SSH, signing, catalogue or release publication.",
                   "Bootstrapped Scoop/config/cache and manager-ready PATH remain until disposable VM teardown."],
        "machine": {"platform": platform.platform(), "image_os": os.environ.get("ImageOS"),
                    "image_version": os.environ.get("ImageVersion"), "python": platform.python_version()}}
    run = Run(work, proof, receipt)
    root = Path(os.environ["USERPROFILE"])/"scoop"
    config = Path(os.environ["USERPROFILE"])/".config/scoop"
    global_root = Path(os.environ["ProgramData"])/"scoop"
    run.entry = root/"apps/scoop/current/bin/scoop.ps1"
    boot_attempted = package_attempted = removed = False
    mirror = None; candidate_data = candidate_manifest = None
    initial_paths = initial_inventory = during_paths = state_before = None
    fixture = work/"synthetic-home"; fixture.mkdir()
    for name in (".cache/flere/state", "AppData/Local", "AppData/Roaming", "temp", "scoop"):
        (fixture/name).mkdir(parents=True, exist_ok=True)
    (fixture/".cache/flere/state/sentinel").write_bytes(b"synthetic Scoop lifecycle state\n")
    state_before = candidate.tree(fixture)
    binary_env = dict(run.env, HOME=str(fixture), USERPROFILE=str(fixture), APPDATA=str(fixture/"AppData/Roaming"),
                      LOCALAPPDATA=str(fixture/"AppData/Local"), TEMP=str(fixture/"temp"), TMP=str(fixture/"temp"),
                      FLERE_STATE_DIR=str(fixture/".cache/flere/state"), XDG_CACHE_HOME=str(fixture/".cache"))
    try:
        for key in ("pwsh", "git", "curl"):
            executable = shutil.which(key + ".exe")
            require(executable is not None, "required hosted tool absent: " + key)
            setattr(run, key, executable)
        machine = run.ps("preflight", "[ordered]@{elevated=([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator);policy=(Get-ExecutionPolicy).ToString();policies=@(Get-ExecutionPolicy -List | Select-Object Scope,ExecutionPolicy);language=$ExecutionContext.SessionState.LanguageMode.ToString();powershell=$PSVersionTable.PSVersion.ToString();ci=$env:CI}|ConvertTo-Json -Depth 5")
        run.record("preflight", machine)
        require(machine["elevated"] is True and machine["ci"] == "true" and machine["language"] == "FullLanguage"
                and machine["policy"] in ("RemoteSigned", "Unrestricted", "Bypass"), "normal hosted Scoop prerequisites differ")
        require(all(not os.path.lexists(path) for path in (root, config, global_root))
                and all(shutil.which(name) is None for name in ("scoop", "flere", "flere-connect")), "pre-existing manager/root/alias")
        initial_paths, initial_inventory = base.path_hashes(), common.registry_inventory()
        run.record("initial-path", initial_paths); run.record("initial-programs", initial_inventory)
        if selected["owner_check"]:
            binary_env.update(XDG_CONFIG_HOME=str(fixture/"AppData/Roaming"), XDG_DATA_HOME=str(fixture/"AppData/Local"),
                              XDG_STATE_HOME=str(fixture/".cache/flere/state"), SCOOP=str(fixture/"scoop"))
            run.record("owner-synthetic-environment", verify_synthetic_override(binary_env, root))
            cold_state = state_before
            shell = shutil.which("powershell.exe", path=binary_env["PATH"])
            require(shell is not None, "normal Windows PowerShell dependency unavailable")
            require(run.command("initialize-powershell", [shell,"-NoProfile","-NonInteractive","-Command",
                    "[System.Console]::Out.Write('flere-tool-baseline')"], env=binary_env,
                    seconds=30, maximum=4096) == b"flere-tool-baseline", "PowerShell initialization output differs")
            state_before = candidate.tree(fixture)
            run.record("tool-initialization-state", base.preservation_snapshot(cold_state, state_before, initial_paths, base.path_hashes()))
            common.verify_powershell_initialization(cold_state, state_before)
            require(base.path_hashes() == initial_paths, "tool initialization changed PATH")
            receipt["limits"].append("Synthetic profile preservation begins after recorded normal PowerShell initialization; cold-profile no-write behavior is not claimed.")
            candidate_data, candidate_manifest = candidate_input(run, selected)
        run.record("state-before", state_before)
        installer = run.download("scoop-install.ps1", INSTALLER_URL, INSTALLER_BYTES, INSTALLER_SHA)
        archive = run.download(ZIP_NAME, PUBLIC_URL, ZIP_BYTES, ZIP_SHA)
        manifest, raw = payload(archive)
        (proof/"records/public-release-manifest.json").write_bytes(raw)
        manifest_path = work/"flere.json"; manifest_path.write_text(json.dumps(recipe(), indent=2)+"\n", encoding="utf-8")
        run.record("install-recipe", recipe())
        boot_attempted = True
        run.command("bootstrap", [run.pwsh, "-NoLogo", "-NoProfile", "-File", installer], seconds=300, maximum=1024*1024)
        require(run.env["CI"] == os.environ["CI"] == "true", "inherited CI changed")
        require(run.entry.is_file() and not app_names(root), "bootstrap app inventory differs")
        manager_revisions(run, root, "before")
        run.scoop("version", "--version")
        during_paths = base.path_hashes()
        run.record("manager-ready-path", during_paths)
        require(during_paths["machine"] == initial_paths["machine"] and common.registry_inventory() == initial_inventory,
                "bootstrap changed system PATH or unrelated packages")
        package_attempted = True
        run.scoop("install", "install", manifest_path, seconds=240, maximum=1024*1024)
        initial = inspect_installed(run, root, manifest, manifest_path, recipe(), PUBLIC_INPUT, binary_env)
        if selected["owner_check"]:
            mirror = base.Mirror(candidate_data, selected)
            local_url = f"http://127.0.0.1:{mirror.server_port}/{base.zip_name(selected)}"
            upgraded_recipe = recipe(selected, local_url)
            require(dict(upgraded_recipe, architecture=recipe(selected)["architecture"]) == recipe(selected), "candidate recipe changed beyond one URL")
            manifest_path.write_text(json.dumps(upgraded_recipe, indent=2)+"\n", encoding="utf-8")
            run.record("upgrade-recipe", upgraded_recipe)
            run.scoop("upgrade", "update", "flere", seconds=240, maximum=1024*1024)
            upgraded = inspect_installed(run, root, candidate_manifest, manifest_path, upgraded_recipe, selected, binary_env, "upgraded-")
            verify_upgrade(initial, upgraded); run.record("actual-version-upgrade", {"before":initial["version"], "after":upgraded["version"],
                "before_build":initial["build"], "after_build":upgraded["build"], "before_payload_sha256":initial["payload_sha"], "after_payload_sha256":upgraded["payload_sha"]})
            version_dir = root/"apps/flere"/selected["version"]
            before_owner_paths = base.path_hashes(); before_owner_inventory = common.registry_inventory(); owners = {}
            for name in ("flere", "flere-connect"):
                value = json.loads(run.command(name+"-update-status", [root/"shims"/(name+".exe"), "update-status"],
                                               env=binary_env, seconds=60, maximum=32768))
                owners[name] = verify_installed_owner(value, candidate_manifest, (version_dir/(name+".exe")).resolve())
                run.record(name+"-update-status", value)
            after_files = {path.name:base.file_record(path) for path in version_dir.iterdir()}
            after_shims = {name:{"alias":base.file_record(root/"shims"/(name+".exe")), "definition":base.file_record(root/"shims"/(name+".shim"))} for name in ("flere", "flere-connect")}
            after_state, after_paths = candidate.tree(fixture), base.path_hashes()
            run.record("owner-state-preservation", base.preservation_snapshot(state_before, after_state, before_owner_paths, after_paths))
            run.record("owner-installed-after", {"files":after_files,"aliases":after_shims})
            require(after_files == upgraded["files"] and after_shims == upgraded["aliases"]
                    and str((root/"apps/flere/current").resolve()) == upgraded["current"]
                    and after_state == state_before and after_paths == before_owner_paths
                    and common.registry_inventory() == before_owner_inventory, "owner queries changed records, active version, files, state or PATH")
            run.record("installed-ownership", owners)
            receipt["installed_owner_aliases_checked"] = 2
            receipt["actual_upgrade"] = {"from":"0.3.4", "to":"0.3.5"}
        manager_revisions(run, root, "after")
        require(common.registry_inventory() == initial_inventory and base.path_hashes() == during_paths
                and candidate.tree(fixture) == state_before, "install/CLI changed unrelated packages, PATH or synthetic state")
        run.scoop("uninstall", "uninstall", "flere")
        removed = True
        listing = run.scoop("removed-list", "list", "^flere$", accepted=(0, 1))
        list_absent = b"There aren't any apps installed." in common.clean_console(listing)
        value = {"application_absent": not os.path.lexists(root/"apps/flere"),
                 "aliases_absent": aliases_absent(root),
                 "list_absent": list_absent, "unrelated_inventory_preserved": common.registry_inventory() == initial_inventory,
                 "path_preserved": base.path_hashes() == during_paths, "state_preserved": candidate.tree(fixture) == state_before}
        run.record("removal", value); verify_removal(value)
        receipt["package_lifecycle_passed"] = True
    except BaseException as error:
        receipt["error"] = str(error); receipt["traceback"] = traceback.format_exc(limit=8)
    finally:
        run.deadline = time.monotonic() + 180
        try:
            if package_attempted and not removed and run.entry.is_file() and os.path.lexists(root/"apps/flere"):
                run.scoop("cleanup-package", "uninstall", "flere")
            if boot_attempted:
                require(run.entry.is_file() and not app_names(root) and aliases_absent(root), "normal Flere removal is incomplete")
                retained = {"disposition": "Retained until disposable hosted VM teardown",
                            "manager_entry": base.file_record(run.entry),
                            "root_entries": sorted(path.name for path in root.iterdir()),
                            "config": base.file_record(config/"config.json") if (config/"config.json").is_file() else None,
                            "cache": [base.file_record(path) for path in base.owned_paths(root/"cache")] if (root/"cache").exists() else []}
                run.record("retained-manager", retained)
                require(during_paths is not None and base.path_hashes() == during_paths,
                        "manager-ready PATH changed during package lifecycle")
                receipt["manager_retained_for_vm_teardown"] = True
            if initial_inventory is not None:
                require(common.registry_inventory() == initial_inventory and candidate.tree(fixture) == state_before,
                        "final unrelated inventory or synthetic state changed")
            receipt["cleanup_passed"] = True
        except BaseException as error:
            receipt["cleanup_error"] = str(error)
        if mirror is not None:
            try:
                mirror.close_owned(); receipt["loopback_closed"] = True
                base.verify_mirror(mirror.requests,mirror.attempts,mirror.rejections,mirror.rejections_dropped,mirror.internal_errors,selected)
                receipt["loopback_verified"] = True
            except BaseException as error:
                receipt["loopback_error"] = str(error); receipt["cleanup_passed"] = False
            receipt.update(loopback_requests=mirror.requests,loopback_response_attempts=mirror.attempts,
                loopback_rejections=mirror.rejections,loopback_rejections_dropped=mirror.rejections_dropped,loopback_internal_errors=mirror.internal_errors)
        receipt["status"] = "passed" if receipt.get("package_lifecycle_passed") and receipt.get("cleanup_passed") and "error" not in receipt else "failed"
        run.save()
        files = [path for path in proof.rglob("*") if path.is_file()]
        require(len(files) <= 128 and sum(path.stat().st_size for path in files) <= 16*1024*1024, "proof inventory bound")
        (proof/"hashes.json").write_text(json.dumps({str(path.relative_to(proof)): {"bytes": path.stat().st_size, "sha256": sha(path.read_bytes())} for path in files}, indent=2)+"\n")
    require(receipt["status"] == "passed", "Scoop lifecycle failed; inspect retained proof")


def self_test():
    import copy
    import tempfile
    import unittest
    from unittest import mock

    class Guards(unittest.TestCase):
        def test_candidate_slot_is_closed_until_exact_reviewed035_pins_exist(self):
            public = selection(); public["version"] = "changed"
            self.assertEqual(selection()["version"], "0.3.4")
            with mock.patch.dict(globals(), OWNER_INPUT=None), self.assertRaises(ValueError):
                selection(OWNER_INPUT_NAME)
            value = {key:"a"*64 for key in OWNER_KEYS}
            value.update(name=OWNER_INPUT_NAME, version="0.3.5", owner_check=True,
                workflow="b"*40, product="c"*40, run=12, artifact=34, artifact_bytes=500,
                zip_bytes=400, source_entries=30, artifact_entries=33, artifact_name="windows-candidate-12-1")
            with mock.patch.dict(globals(), OWNER_INPUT=value):
                self.assertEqual(selection(OWNER_INPUT_NAME), value)
            for change in ({"version":"0.3.4"}, {"run":True}, {"product":PRODUCT}, {"payload_sha":PAYLOAD_SHA},
                           {"workflow":"moving-main"}, {"artifact_name":"windows-candidate-12-2"},
                           {"artifact_bytes":17*1024*1024}, {"extra":"not allowed"}):
                with mock.patch.dict(globals(), OWNER_INPUT=dict(value, **change)), self.assertRaises(ValueError):
                    selection(OWNER_INPUT_NAME)

        def test_candidate_recipe_keeps_hash_and_hook_for_one_loopback_url(self):
            selected = dict(PUBLIC_INPUT, version="0.3.5", zip_sha="a"*64)
            expected, local = recipe(selected), recipe(selected, "http://127.0.0.1:12345/"+base.zip_name(selected))
            self.assertEqual(dict(local, architecture=expected["architecture"]), expected)
            self.assertEqual(local["architecture"]["64bit"]["hash"], "a"*64)
            self.assertEqual(expected["version"], "0.3.5")

        def test_upgrade_requires_actual_version_current_build_and_payload_change(self):
            before = {"version":"0.3.4", "current":"old", "build":{"build_id":"old"}, "payload_sha":"a"*64}
            after = {"version":"0.3.5", "current":"new", "build":{"build_id":"new"}, "payload_sha":"b"*64}
            verify_upgrade(before, after)
            for key in after:
                with self.assertRaises(ValueError):
                    verify_upgrade(before, dict(after, **{key:before[key]}))

        def test_owner_requires_full_new_build_exact_payload_and_fixed_scoop_guidance(self):
            manifest = {"build":{"package_version":"0.3.5", "build_id":"new"}, "payload":{"sha256":"b"*64}}
            for alias in ("flere", "flere-connect"):
                exe = rf"C:\Users\fixture\scoop\apps\flere\0.3.5\{alias}.exe"
                owner = {"kind":"manager", "executable":"\\\\?\\"+exe, "sha256":"b"*64, "attempt":None, "guidance":OWNER_GUIDANCE}
                value = {"schema_version":1, "running_build":manifest["build"], "installation":None,
                         "other_frontends":"untracked", "ownership":owner}
                verify_installed_owner(value, manifest, exe)
                for change in ({"kind":"unknown"}, {"kind":"local"}, {"sha256":"c"*64}, {"attempt":"pending"},
                               {"guidance":"Local WinGet"}, {"executable":exe.replace("0.3.5", "0.3.4")}, {"extra":True}):
                    with self.assertRaises(ValueError):
                        verify_installed_owner(dict(value, ownership=dict(owner, **change)), manifest, exe)
                with self.assertRaises(ValueError):
                    verify_installed_owner(dict(value, running_build={"package_version":"0.3.5", "build_id":"old"}), manifest, exe)

        def test_synthetic_roots_cannot_substitute_the_actual_manager_profile(self):
            home = r"C:\private\synthetic"
            env = {key:home+"\\"+key for key in ("LOCALAPPDATA", "APPDATA", "XDG_CONFIG_HOME", "XDG_DATA_HOME",
                "XDG_STATE_HOME", "XDG_CACHE_HOME", "TEMP", "TMP", "FLERE_STATE_DIR", "SCOOP")}
            env.update(HOME=home, USERPROFILE=home)
            root = r"C:\Users\fixture\scoop"
            verify_synthetic_override(env, root)
            for key in env:
                with self.assertRaises(ValueError):
                    verify_synthetic_override(dict(env, **{key:root}), root)

        def test_installed_phase_retains_records_and_runs_both_aliases_for_selected_version(self):
            from types import SimpleNamespace
            cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home()/".cache"))/"flere/tests"
            cache.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=cache) as folder:
                root = Path(folder); version = root/"apps/flere/0.3.5"; version.mkdir(parents=True)
                current = root/"apps/flere/current"; current.mkdir()
                (root/"shims").mkdir(); (root/"records").mkdir()
                source = root/"flere.json"; data = b"synthetic executable"
                manifest = {"build":{"package_version":"0.3.5", "target":TARGET, "build_id":"fixture"}}
                selected = dict(PUBLIC_INPUT, version="0.3.5", payload_sha=sha(data), manifest_sha=sha(b"release record"))
                expected = recipe(selected)
                for name, contents in {"manifest.json":json.dumps(expected).encode(),
                    "install.json":json.dumps({"architecture":"64bit", "url":str(source)}).encode(),
                    "flere-release.manifest.json":b"release record", "LICENSE":b"fixture license",
                    "flere.exe":data, "flere-connect.exe":data}.items():
                    (version/name).write_bytes(contents)
                for alias in ("flere", "flere-connect"):
                    (root/"shims"/(alias+".exe")).write_bytes(b"shim")
                    (root/"shims"/(alias+".shim")).write_bytes(b"shim definition")
                commands=[]; records={}
                def command(name, argv, **unused):
                    commands.append(argv)
                    return (json.dumps(manifest["build"]).encode() if argv[1] == "--build-info" else
                        f"flere-connect 0.3.5 ({TARGET}; build fixture)".encode() if argv[1] == "--version" else b"--build-info ssh")
                run = SimpleNamespace(proof=root, record=lambda key,value:records.update({key:value}),
                    scoop=lambda *args:b"flere 0.3.5 fixture\r\n", command=command)
                original_resolve = type(root).resolve
                def resolve(path, *args, **kwargs):
                    return original_resolve(version if path == current else path, *args, **kwargs)
                with mock.patch.object(type(root), "resolve", resolve), \
                     mock.patch.object(Path, "is_junction", return_value=True, create=True), \
                     mock.patch.object(shutil, "which", side_effect=lambda name,**unused:str(root/"shims"/name)), \
                     mock.patch.object(common, "normal_path", return_value="synthetic"), \
                     mock.patch.dict(globals(), LICENSE_SHA=sha(b"fixture license")):
                    result = inspect_installed(run, root, manifest, source, expected, selected, {}, "upgraded-")
                self.assertEqual(len(commands),6)
                self.assertEqual(result["version"],"0.3.5")
                self.assertEqual((root/"records/upgraded-installed-flere-release.manifest.json").read_bytes(),b"release record")
                self.assertEqual(set(records["upgraded-aliases"]),{"flere","flere-connect"})

        def test_hosted_ci_is_observed_not_created(self):
            env = {"GITHUB_ACTIONS":"true", "FLERE_RUNNER_ENVIRONMENT":"github-hosted", "GITHUB_EVENT_NAME":"workflow_dispatch",
                   "GITHUB_REPOSITORY":"robert-cronin/flere", "GITHUB_REF":"refs/heads/main", "RUNNER_OS":"Windows", "RUNNER_ARCH":"X64",
                   "FLERE_WORKFLOW_SHA":"a"*40, "GITHUB_RUN_ID":"1", "GITHUB_RUN_ATTEMPT":"1", "CI":"true"}
            before = dict(env); hosted(env); self.assertEqual(env, before)
            hosted(dict(env, ImageOS="win25-vs2026"))  # Image labels are recorded, not a support gate.
            for key in env:
                with self.assertRaises(ValueError):
                    hosted(dict(env, **{key:"wrong"}))
            for key in ("SCOOP", "SCOOP_GLOBAL", "SCOOP_CACHE", "XDG_CONFIG_HOME"):
                with self.assertRaises(ValueError):
                    hosted(dict(env, **{key:"private-other"}))

        def test_exact_recipe_keeps_public_url_hash_and_only_fixed_hook(self):
            good = recipe()
            for key, value in (("pre_install", "Remove-Item anything"), ("bin", ["other.exe"]), ("version", "0.3.3"), ("installer", "unexpected")):
                changed = copy.deepcopy(good); changed[key] = value
                with mock.patch.object(packager, "scoop_manifest", return_value=changed), self.assertRaises(ValueError):
                    recipe()

        def test_exact_scoop_records_distinct_from_original_manifest(self):
            value = recipe(); source = Path("fixture")/"flere.json"
            install = {"architecture":"64bit", "url":str(source)}
            installed_record(value, install, value, source)
            for bad in ({}, dict(install, architecture="32bit"), dict(install, bucket="main"), dict(install, url="other")):
                with self.assertRaises(ValueError):
                    installed_record(value, bad, value, source)
            with self.assertRaises(ValueError):
                installed_record({"build":{}}, install, value, source)

        def test_alias_version_contains_the_exact_build_not_only_semver(self):
            manifest = {"build": {"build_id":"fixture-build", "target":TARGET, "package_version":VERSION}}
            exact = f"flere-connect {VERSION} ({TARGET}; build fixture-build)\r\n".encode()
            verify_cli("version", exact, manifest)
            verify_cli("build-info", json.dumps(manifest["build"]).encode(), manifest)
            for bad in (b"flere-connect 0.3.4", exact.replace(b"fixture-build",b"other"), exact.replace(TARGET.encode(),b"wrong")):
                with self.assertRaises(ValueError):
                    verify_cli("version", bad, manifest)

        def test_zero_exit_is_not_removal_or_preservation(self):
            good = {key:True for key in ("application_absent", "aliases_absent", "list_absent", "unrelated_inventory_preserved", "path_preserved", "state_preserved")}
            verify_removal(good)
            for key in good:
                with self.assertRaises(ValueError):
                    verify_removal(dict(good, **{key:False}))

        def test_retained_manager_does_not_relax_package_alias_removal(self):
            cache = Path(os.environ.get("XDG_CACHE_HOME", Path.home()/".cache"))/"flere/tests"
            cache.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=cache) as folder, mock.patch.object(common, "normal_path", return_value=""), mock.patch.object(shutil, "which", return_value=None):
                root = Path(folder)
                (root/"apps/scoop").mkdir(parents=True)
                (root/"cache").mkdir(); (root/"cache/public-zip").write_bytes(b"retained cache")
                (root/"shims").mkdir()
                self.assertEqual(app_names(root), [])
                self.assertTrue(aliases_absent(root))
                (root/"shims/flere.shim").write_bytes(b"residual")
                self.assertFalse(aliases_absent(root))
                (root/"shims/flere.shim").unlink()
                with mock.patch.object(shutil, "which", return_value="unexpected-alias"):
                    self.assertFalse(aliases_absent(root))
                (root/"apps/flere").mkdir()
                self.assertEqual(app_names(root), ["flere"])

        def test_exact_download_hash_and_size_are_both_required(self):
            root = Path(os.environ.get("XDG_CACHE_HOME", Path.home()/".cache"))/"flere/tests"
            root.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(dir=root) as folder:
                path=Path(folder)/"input"; path.write_bytes(b"reviewed")
                verify_bytes(path, 8, sha(b"reviewed"))
                for length, digest in ((7,sha(b"reviewed")),(9,sha(b"reviewed")),(8,"0"*64)):
                    with self.assertRaises(ValueError):
                        verify_bytes(path,length,digest)

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(Guards))
    require(result.wasSuccessful(), "Scoop guard checks failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", nargs="?", type=Path)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--input", choices=(DEFAULT_INPUT, OWNER_INPUT_NAME), default=DEFAULT_INPUT)
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        require(args.output is not None, "output required")
        main(args.output, selection(args.input))
