#!/usr/bin/env python3
"""Normal public-ZIP Scoop install/remove on one disposable hosted Windows VM.

Capture manager records for later ownership work. This is elevated CI per-user
acceptance, not non-admin desktop, upgrade, UI, SSH or manager-detector acceptance.
"""
import argparse
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
REMOVE_TITLE = b"This will uninstall Scoop and all the programs that have been installed with Scoop!"
REMOVE_PROMPT = b"Are you sure? (yN): "


def hosted(environment):
    candidate.hosted(environment)
    require(environment.get("CI") == "true", "requires inherited GitHub CI")
    require(not any(environment.get(key) for key in ("SCOOP", "SCOOP_GLOBAL", "SCOOP_CACHE", "XDG_CONFIG_HOME")),
            "custom Scoop/config roots are outside this fixture")


def recipe():
    value = packager.scoop_manifest(VERSION, PUBLIC_URL, ZIP_SHA)
    require(value["pre_install"] == 'Rename-Item -LiteralPath "$dir\\manifest.json" -NewName \'flere-release.manifest.json\' -ErrorAction Stop',
            "only the fixed release-manifest rename is reviewed")
    require(set(value) == {"version", "description", "homepage", "license", "architecture", "bin", "pre_install", "notes"}
            and value["architecture"] == {"64bit": {"url": PUBLIC_URL, "hash": ZIP_SHA}}
            and value["version"] == VERSION and value["license"] == "MIT"
            and value["bin"] == ["flere.exe", "flere-connect.exe"], "Scoop recipe contract differs")
    return value


def prompt_ready(data, answered, allow_remove):
    text = common.clean_console(data)
    count = text.count(REMOVE_PROMPT)
    require(count <= 1 and text.count(b"Are you sure?") <= 1
            and b"Continue installation?" not in text and b"Do you want to" not in text,
            "unexpected or repeated Scoop prompt")
    if count:
        require(allow_remove and text.count(REMOVE_TITLE) == 1
                and text.index(REMOVE_TITLE) < text.index(REMOVE_PROMPT)
                and b"and all persisted data" not in text, "unreviewed manager removal prompt")
    return count == 1 and not answered


def verify_bytes(path, size, digest):
    data = packager.read_regular(path, size)
    require(len(data) == size and sha(data) == digest, "downloaded bytes differ: " + path.name)
    return data


def payload(path):
    verify_bytes(path, ZIP_BYTES, ZIP_SHA)
    with zipfile.ZipFile(path) as archive:
        require(set(archive.namelist()) == {"flere.exe", "flere-connect.exe", "manifest.json", "LICENSE"}, "ZIP inventory differs")
        require(all(item.file_size <= 3 * 1024 * 1024 for item in archive.infolist()), "ZIP expanded bound")
        raw, license_bytes = archive.read("manifest.json"), archive.read("LICENSE")
    require(sha(raw) == MANIFEST_SHA and sha(license_bytes) == LICENSE_SHA, "public release records differ")
    manifest = json.loads(raw)
    require(manifest["source"]["git_commit"] == PRODUCT and manifest["source"]["dirty"] is False
            and manifest["build"]["package_version"] == VERSION and manifest["build"]["target"] == TARGET
            and manifest["payload"]["sha256"] == PAYLOAD_SHA, "public payload provenance differs")
    candidate.verify_zip(path, manifest, license_bytes)
    return manifest, raw


class Run(common.Run):
    # Adapt the existing bounded Windows process primitive only for Scoop's one
    # ordinary self-removal prompt. Keep separate streams and owned tree cleanup.
    def command(self, name, argv, *, env=None, seconds=120, maximum=base.MAX_LOG, allow_remove=False, accepted=(0,)):
        seconds = min(seconds, self.deadline - time.monotonic())
        require(seconds > 0, "lifecycle deadline exhausted")
        log, errlog = (self.proof / "logs" / (name + suffix) for suffix in (".log", "-stderr.log"))
        row = {"name": name, "argv": list(map(str, argv)), "status": "running", "confirmed_remove": False}
        self.receipt["checks"].append(row); self.save()
        start, failure = time.monotonic(), None
        with base.command_logs(log, separate_stderr=True) as (out, err):
            proc = subprocess.Popen(row["argv"], cwd=self.work, env=self.env if env is None else env,
                                    stdin=subprocess.PIPE, stdout=out, stderr=err)
            row["pid"] = proc.pid
            try:
                while proc.poll() is None:
                    require(time.monotonic() - start < seconds, name + " timed out")
                    require(log.stat().st_size <= maximum and errlog.stat().st_size <= maximum, name + " output bound")
                    if prompt_ready(log.read_bytes() + errlog.read_bytes(), row["confirmed_remove"], allow_remove):
                        require(not any(item["confirmed_remove"] for item in self.receipt["checks"]), "manager removal confirmation repeated")
                        proc.stdin.write(b"y\r\n"); proc.stdin.flush(); row["confirmed_remove"] = True
                    time.sleep(0.05)
            except BaseException as error:
                failure = error
            finally:
                row["forced_stop"] = proc.poll() is None
                if row["forced_stop"]:
                    killed = subprocess.run([str(Path(os.environ["SystemRoot"])/"System32/taskkill.exe"),
                        "/PID", str(proc.pid), "/T", "/F"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                        stderr=subprocess.DEVNULL, timeout=15)
                    require(killed.returncode == 0, "owned Scoop child tree could not be stopped")
                proc.wait(timeout=15); proc.stdin.close()
        for path in (log, errlog):
            if path.stat().st_size > maximum:
                failure = failure or ValueError(name + " output bound")
                with path.open("r+b") as stream:
                    stream.truncate(maximum)
        data, errors = log.read_bytes(), errlog.read_bytes()
        try:
            require(not prompt_ready(data + errors, row["confirmed_remove"], allow_remove), "prompt exited before response")
            require(not allow_remove or row["confirmed_remove"], "normal manager removal prompt was not observed")
        except ValueError as error:
            failure = failure or error
        row.update(exit=proc.returncode, seconds=round(time.monotonic()-start, 3), stdout_bytes=len(data),
                   stdout_sha256=sha(data), stderr_bytes=len(errors), stderr_sha256=sha(errors),
                   status="passed" if failure is None and proc.returncode in accepted else "failed")
        self.save()
        if failure:
            raise failure
        require(proc.returncode in accepted, name + " failed; inspect retained log")
        return data

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
        expected = f"flere-connect {VERSION} ({TARGET}; build {manifest['build']['build_id']})"
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


def main(output):
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
        "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "product_commit": PRODUCT, "zip_sha256": ZIP_SHA,
        "limits": ["Per-user installation on an elevated hosted CI VM; not unelevated desktop acceptance.",
                   "First public-ZIP install/remove and record capture; no upgrade or Scoop owner detector proof.",
                   "Normal bootstrap fetches mutable official Scoop/Main revisions, recorded separately from the pinned installer.",
                   "No UI, clipboard, external SSH, signing, catalogue or release publication."],
        "machine": {"platform": platform.platform(), "image_os": os.environ.get("ImageOS"),
                    "image_version": os.environ.get("ImageVersion"), "python": platform.python_version()}}
    run = Run(work, proof, receipt)
    root = Path(os.environ["USERPROFILE"])/"scoop"
    config = Path(os.environ["USERPROFILE"])/".config/scoop"
    global_root = Path(os.environ["ProgramData"])/"scoop"
    run.entry = root/"apps/scoop/current/bin/scoop.ps1"
    boot_attempted = package_attempted = removed = manager_removed = False
    initial_paths = initial_inventory = during_paths = state_before = None
    fixture = work/"synthetic-home"; fixture.mkdir()
    for name in (".cache/flere/state", "AppData/Local", "AppData/Roaming", "temp"):
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
        run.record("initial-path", initial_paths); run.record("initial-programs", initial_inventory); run.record("state-before", state_before)
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
        require(during_paths["machine"] == initial_paths["machine"] and common.registry_inventory() == initial_inventory,
                "bootstrap changed system PATH or unrelated packages")
        package_attempted = True
        run.scoop("install", "install", manifest_path, seconds=240, maximum=1024*1024)
        listing = run.scoop("installed-list", "list", "^flere$")
        require(re.search(rb"(?m)^\s*flere\s+0\.3\.4(?:\s|$)", common.clean_console(listing)), "normal installed list lacks exact package/version")
        version_dir, current = root/"apps/flere"/VERSION, root/"apps/flere/current"
        require(app_names(root) == ["flere"] and current.is_junction() and current.resolve() == version_dir.resolve(), "current version junction differs")
        expected_names = {"flere.exe", "flere-connect.exe", "LICENSE", "manifest.json", "flere-release.manifest.json", "install.json"}
        require({item.name for item in version_dir.iterdir()} == expected_names, "installed payload/record inventory differs")
        records = {item.name: base.file_record(item) for item in version_dir.iterdir()}
        run.record("installed-files", records)
        for name in ("manifest.json", "flere-release.manifest.json", "install.json"):
            (proof/"records"/("installed-"+name)).write_bytes((version_dir/name).read_bytes())
        installed_record(json.loads((version_dir/"manifest.json").read_text(encoding="utf-8-sig")),
                         json.loads((version_dir/"install.json").read_text(encoding="utf-8-sig")), recipe(), manifest_path)
        require(records["flere-release.manifest.json"]["sha256"] == MANIFEST_SHA
                and records["LICENSE"]["sha256"] == LICENSE_SHA, "release manifest/license bytes changed")
        run.record("current-junction", {"path": str(current), "target": str(current.resolve()), "version": VERSION})
        normal_path = common.normal_path(); binary_env["PATH"] = normal_path
        aliases = {}
        for name in ("flere", "flere-connect"):
            alias = shutil.which(name+".exe", path=normal_path)
            require(alias is not None and Path(alias) == root/"shims"/(name+".exe"), "PATH alias is not the Scoop shim")
            require(records[name+".exe"]["sha256"] == PAYLOAD_SHA, "installed executable differs")
            shim = root/"shims"/(name+".shim")
            aliases[name] = {"alias": base.file_record(Path(alias)), "definition": base.file_record(shim)}
            (proof/"records"/(name+".shim.bin")).write_bytes(shim.read_bytes())
            for flag in ("version", "build-info", "help"):
                data = run.command(name+"-"+flag, [alias, "--"+flag], env=binary_env, seconds=30)
                verify_cli(flag, data, manifest)
        run.record("aliases", aliases)
        manager_revisions(run, root, "after")
        require(common.registry_inventory() == initial_inventory and base.path_hashes() == during_paths
                and candidate.tree(fixture) == state_before, "install/CLI changed unrelated packages, PATH or synthetic state")
        run.scoop("uninstall", "uninstall", "flere")
        removed = True
        listing = run.scoop("removed-list", "list", "^flere$", accepted=(0, 1))
        list_absent = b"There aren't any apps installed." in common.clean_console(listing)
        value = {"application_absent": not os.path.lexists(root/"apps/flere"),
                 "aliases_absent": all(not any(os.path.lexists(root/"shims"/(name+suffix)) for suffix in (".exe", ".shim", ".cmd", ".ps1", ""))
                                       and shutil.which(name, path=common.normal_path()) is None for name in ("flere", "flere-connect")),
                 "list_absent": list_absent, "unrelated_inventory_preserved": common.registry_inventory() == initial_inventory,
                 "path_preserved": base.path_hashes() == during_paths, "state_preserved": candidate.tree(fixture) == state_before}
        run.record("removal", value); verify_removal(value)
        receipt["package_lifecycle_passed"] = True
    except BaseException as error:
        receipt["error"] = str(error); receipt["traceback"] = traceback.format_exc(limit=8)
    finally:
        run.deadline = time.monotonic() + 180
        try:
            if boot_attempted and run.entry.is_file():
                if package_attempted and not removed and os.path.lexists(root/"apps/flere"):
                    run.scoop("cleanup-package", "uninstall", "flere")
                require(not app_names(root), "manager still contains an application; no whole-manager removal")
                run.scoop("remove-manager", "uninstall", "scoop", allow_remove=True)
                manager_removed = True
            if boot_attempted:
                residual = {str(path.relative_to(root)): base.file_record(path) for path in root.iterdir()} if root.exists() else {}
                run.record("manager-residue", {"root_entries": residual, "config": base.file_record(config/"config.json") if (config/"config.json").is_file() else None})
                require(manager_removed and not run.entry.exists() and base.path_hashes() == initial_paths,
                        "normal bootstrap removal/PATH restoration incomplete")
            if initial_inventory is not None:
                require(common.registry_inventory() == initial_inventory and candidate.tree(fixture) == state_before,
                        "final unrelated inventory or synthetic state changed")
            receipt["cleanup_passed"] = True
        except BaseException as error:
            receipt["cleanup_error"] = str(error)
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

        def test_only_normal_self_removal_prompt_at_all_splits(self):
            data = REMOVE_TITLE+b"\r\n"+REMOVE_PROMPT
            for split in range(len(data)):
                self.assertFalse(prompt_ready(data[:split], False, True))
                self.assertTrue(prompt_ready(data[:split]+data[split:], False, True))
            self.assertFalse(prompt_ready(data, True, True))
            for invalid, allowed in ((data,False),(data+data,True),(REMOVE_PROMPT,True),(b"Continue installation? [Y/n]",False)):
                with self.assertRaises(ValueError):
                    prompt_ready(invalid, False, allowed)

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
            manifest = {"build": {"build_id":"fixture-build", "target":TARGET}}
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
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        require(args.output is not None, "output required")
        main(args.output)
