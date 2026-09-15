#!/usr/bin/env python3
"""Validate recipes for one retained Windows candidate; never build or execute it."""
import importlib.util
import io
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
spec = importlib.util.spec_from_file_location("candidate", Path(__file__).with_name("hosted-candidate.py"))
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)
require, sha = candidate.require, candidate.sha
REPOSITORY = "robert-cronin/flere"
RUN = 34911230794
WORKFLOW = "4a20c5dc9910771e9162b1d0d193bb47da69e645"
ARTIFACT = 10374423428
ARTIFACT_BYTES = 1808132
ARTIFACT_SHA = "669c36d41fd155c62d2f1556b3984132b14e63730fc790313f1200ed384c881a"
PRODUCT = "422058c0fa4dda3cff7693a32953fea1b2c5404e"
SOURCE = "ef49b87e4b4ce6975c22ca9ce7c9f00e061495ff5658137655be60ff08834562"
ZIP_NAME = "flere-connect-0.3.4-x86_64-pc-windows-msvc.zip"
ZIP_SHA = "e562f99168a34202ecaa2dc163db265e9df3c0007699fbb7fea6b24c6c8e7392"
PAYLOAD_SHA = "9622d73775ce583009a05d0ce6ee57e1ff1ec84e6d83f12ff04dd0f6968ce21f"


def verify_api(metadata):
    require(metadata["id"] == ARTIFACT and metadata["name"] == f"windows-candidate-{RUN}-1"
            and metadata["size_in_bytes"] == ARTIFACT_BYTES
            and metadata["digest"] == "sha256:" + ARTIFACT_SHA and not metadata["expired"]
            and metadata["workflow_run"]["id"] == RUN
            and metadata["workflow_run"]["head_sha"] == WORKFLOW, "original artifact API identity differs")


def evidence(data):
    require(len(data) == ARTIFACT_BYTES and sha(data) == ARTIFACT_SHA, "original artifact hash/size differs")
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        names = archive.namelist()
        require(len(names) == 30 and len(names) == len(set(names)), "original artifact inventory differs")
        total = 0
        for item in archive.infolist():
            name = item.filename
            require(not name.startswith("/") and "\\" not in name
                    and all(part not in ("", ".", "..") for part in name.split("/"))
                    and not item.is_dir() and item.file_size <= 16 * 1024 * 1024
                    and ((item.external_attr >> 16) & 0o170000) in (0, 0o100000), "unsafe original member")
            total += item.file_size
        require(total <= 64 * 1024 * 1024, "original artifact contents exceed bound")
        receipt = json.loads(archive.read("candidate.json"))
        source = json.loads(archive.read("source.json"))
        source_receipt = json.loads(archive.read("source-receipt.json"))
        require(receipt["schema"] == "flere-windows-candidate-v1" and receipt["status"] == "failed"
                and receipt["commit"] == PRODUCT and receipt["workflow_sha"] == WORKFLOW
                and receipt["run_id"] == str(RUN) and receipt["run_attempt"] == "1", "original candidate identity differs")
        checks = receipt["checks"]
        require(len(checks) == 18 and all(row["status"] == "passed" and row["exit"] == 0 for row in checks[:-1])
                and checks[-1]["name"] == "winget-validate" and checks[-1]["status"] == "failed"
                and checks[-1]["exit"] == 2316632104, "original candidate outcome differs")
        for row in checks:
            require(re.fullmatch("[A-Za-z0-9-]+", row["name"]), "invalid original log name")
            log = archive.read("logs/" + row["name"] + ".log")
            require(len(log) == row["log_bytes"] and sha(log) == row["log_sha256"], "original log differs")
        require(source["commit"] == PRODUCT and source["version"] == "0.3.4"
                and source["fingerprint_algorithm"] == "flere-source-v1"
                and source["source_sha256"] == SOURCE and len(source["entries"]) == 365
                and candidate.module("release-source").fingerprint(source["entries"]) == SOURCE,
                "original source inventory differs")
        require(source_receipt == {"git_commit": PRODUCT, "dirty": False, "source_sha256": SOURCE,
                                  "checks": [row["name"] for row in checks[:8]]}, "source receipt differs")
        payload_zip = archive.read("windows/" + ZIP_NAME)
        require(sha(payload_zip) == ZIP_SHA, "portable ZIP differs")
    with zipfile.ZipFile(io.BytesIO(payload_zip)) as archive:
        manifest = json.loads(archive.read("manifest.json"))
        require(manifest["source"] == source_receipt and manifest["payload"]["sha256"] == PAYLOAD_SHA,
                "portable source or payload differs")
    return receipt, source, source_receipt, payload_zip


def main(output):
    candidate.hosted(os.environ)
    require(os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64"), "native Windows AMD64 required")
    require(candidate.COMMIT == PRODUCT and candidate.VERSION == "0.3.4", "candidate helper pin changed")
    require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=candidate.PROJECT, text=True).strip()
            == os.environ["FLERE_WORKFLOW_SHA"], "automation checkout differs")
    require(output.parent.resolve() == Path.home().resolve() / ".cache/flere/tmp" and not output.exists(), "fresh HOME-cache output required")
    output.mkdir(); proof = output / "proof"; work = output / "work"
    proof.mkdir(); work.mkdir(); (proof / "logs").mkdir(); (proof / "input").mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    receipt = {"schema": "flere-windows-recipes-only-v1", "status": "running", "product_commit": PRODUCT,
               "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"], "run_id": os.environ["GITHUB_RUN_ID"],
               "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"], "input_run": RUN, "input_artifact": ARTIFACT,
               "input_artifact_sha256": ARTIFACT_SHA, "zip_sha256": ZIP_SHA, "checks": [],
               "machine": {"system": platform.platform(), "image_os": os.environ.get("ImageOS"),
                           "image_version": os.environ.get("ImageVersion")},
               "limits": ["No Rust build, payload execution, installation, manager lifecycle or publication.",
                          "Original candidate failed at WinGet warnings; its59 tests and6 stateless flags remain that run's evidence.",
                          "Physical Windows acceptance is separate; Arcade is optional."]}

    def save():
        (proof / "receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def command(name, argv, env, destination=None, maximum=1024 * 1024, seconds=120, cwd=work):
        destination = destination or proof / "logs" / (name + ".log")
        row = {"name": name, "argv": list(map(str, argv)), "status": "running"}
        receipt["checks"].append(row); save(); started = time.monotonic()
        error_path = proof / "logs" / (name + "-stderr.log")
        failure = None
        with destination.open("xb") as stream, error_path.open("xb") as errors:
            proc = subprocess.Popen(list(map(str, argv)), cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=errors)
            try:
                while proc.poll() is None:
                    require(time.monotonic() - started < seconds, name + " timed out")
                    require(destination.stat().st_size <= maximum and error_path.stat().st_size <= 1024 * 1024,
                            name + " output exceeds bound")
                    time.sleep(0.05)
            except BaseException as error:
                failure = error
            finally:
                if proc.poll() is None:
                    subprocess.run(["taskkill.exe", "/PID", str(proc.pid), "/T", "/F"],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15)
                proc.wait(timeout=15)
        for path, limit in ((destination, maximum), (error_path, 1024 * 1024)):
            if path.stat().st_size > limit:
                failure = failure or ValueError(name + " output exceeds bound")
                with path.open("r+b") as stream:
                    stream.truncate(limit)
        data = destination.read_bytes(); stderr = error_path.read_bytes()
        row.update(exit=proc.returncode, seconds=round(time.monotonic() - started, 3),
                   stdout_bytes=len(data), stdout_sha256=sha(data), stderr_bytes=len(stderr), stderr_sha256=sha(stderr),
                   status="passed" if proc.returncode == 0 and failure is None else "failed")
        save()
        if failure is not None:
            raise failure
        require(len(data) <= maximum and len(stderr) <= 1024 * 1024 and proc.returncode == 0,
                name + " failed; see retained log")
        return data

    try:
        gh = shutil.which("gh.exe"); require(gh, "GitHub CLI unavailable")
        api = f"repos/{REPOSITORY}/actions/artifacts/{ARTIFACT}"
        metadata = json.loads(command("artifact-api", [gh, "api", api], os.environ.copy(), maximum=65536))
        verify_api(metadata)
        (proof / "input/artifact-api.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
        data = command("artifact-download", [gh, "api", api + "/zip"], os.environ.copy(),
                       destination=work / "original-artifact.zip", maximum=ARTIFACT_BYTES)
        original, source, source_receipt, portable = evidence(data)
        for name, value in [("candidate", original), ("source", source), ("source-receipt", source_receipt)]:
            (proof / "input" / (name + ".json")).write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")
        assets = work / "assets"; assets.mkdir(); license_path = work / "LICENSE"
        with zipfile.ZipFile(io.BytesIO(portable)) as archive:
            manifest = json.loads(archive.read("manifest.json"))
            name = "flere-connect-x86_64-pc-windows-msvc"
            (assets / name).write_bytes(archive.read("flere-connect.exe"))
            (assets / (name + ".manifest.json")).write_bytes(archive.read("manifest.json"))
            license_path.write_bytes(archive.read("LICENSE"))
            require(sha(license_path.read_bytes()) == source["entries"]["LICENSE"]["sha256"], "source license differs")
        portable_path = work / ZIP_NAME; portable_path.write_bytes(portable)
        candidate.verify_zip(portable_path, manifest, license_path.read_bytes())
        windows = proof / "windows"
        generated = candidate.module("windows-manifests").prepare(assets, windows, candidate.TARGET, license_path)
        require(generated["sha256"] == ZIP_SHA and (windows / ZIP_NAME).read_bytes() == portable,
                "recipe generation changed the validated portable ZIP")
        env = {key: value for key, value in os.environ.items() if key not in ("GH_TOKEN", "GITHUB_TOKEN")}
        winget = shutil.which("winget.exe"); require(winget, "WinGet unavailable; no install attempted")
        receipt["winget_version"] = command("winget-version", [winget, "--version"], env).decode().strip()
        folder = windows / "winget/manifests/r/RobertCronin/FlereConnect/0.3.4"
        command("winget-validate", [winget, "validate", folder, "--disable-interactivity"], env)
        choco = shutil.which("choco.exe"); require(choco, "Chocolatey unavailable; no install attempted")
        receipt["chocolatey_version"] = command("chocolatey-version", [choco, "--version"], env).decode().strip()
        recipe = windows / "chocolatey/flere-connect"; packed = windows / "chocolatey-package"; packed.mkdir()
        command("chocolatey-pack", [choco, "pack", recipe / "flere-connect.nuspec", "--outputdirectory", packed], env, cwd=recipe)
        package = packed / "flere-connect.0.3.4.nupkg"
        require(list(packed.iterdir()) == [package], "Chocolatey package inventory differs")
        receipt["chocolatey_inventory"] = candidate.verify_nupkg(package, (recipe / "tools/chocolateyInstall.ps1").read_bytes())
        receipt.update(status="recipes_validated_not_published", source_sha256=SOURCE, payload_sha256=PAYLOAD_SHA,
                       original_artifact_unchanged=sha((work / "original-artifact.zip").read_bytes()) == ARTIFACT_SHA,
                       files={p.relative_to(windows).as_posix(): {"bytes": p.stat().st_size, "sha256": sha(p.read_bytes())}
                              for p in sorted(windows.rglob("*")) if p.is_file()})
        require(receipt["original_artifact_unchanged"], "original artifact changed")
    except BaseException as error:
        receipt.update(status="failed", error=str(error), traceback=traceback.format_exc())
        raise
    finally:
        save()


if __name__ == "__main__":
    require(len(sys.argv) == 2, "expected fresh output directory")
    main(Path(sys.argv[1]).resolve())
