#!/usr/bin/env python3
"""Opt-in native038 receipt-reader fixture; no UI, SSH, channel lookup or publication."""
import argparse
import copy
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import zipfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("candidate", Path(__file__).with_name("hosted-candidate.py"))
candidate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(candidate)
require, sha = candidate.require, candidate.sha
VERSION = "0.3.8"
OLD_VERSION = "0.3.7"
OLD_COMMIT = "48e209ba5ee1716d8a7e2a110b1ed15efe7fc8b1"
OLD_ZIP = "flere-connect-0.3.7-x86_64-pc-windows-msvc.zip"
OLD_URL = "https://github.com/robert-cronin/flere/releases/download/v0.3.7/" + OLD_ZIP
OLD_BYTES = 1900739
OLD_SHA = "1f579d6c3c0fd78b7d085dd28cbe1a4e6c6988b4eaea66e9e5113fa5c98d7389"
OLD_PAYLOAD = "14bcdf024d2df200b906220d8dec0dd2fb99bccf6d92f6d841c82d79d9990f08"
OLD_MANIFEST = "b7fe3c0a74327cc4a111dcdafdb6c12cd57d553d344a918337b7f31c6cda79d0"
OLD_MANIFEST_URL = "https://github.com/robert-cronin/flere/releases/download/v0.3.7/flere-connect-x86_64-pc-windows-msvc.manifest.json"
LICENSE_SHA = "b997b4ab95fef146345de8b279c4d6dff41fc4a198d2bfe2488ba0ae0ac071d5"
CAPABILITY_ARGS = ["--build-info", "--default-channel-info"]
CAPABILITY = {"schema_version": 1, "source_policy": "default_channel_v1"}
REFUSAL = b"Candidate or retained Windows launcher cannot retain default-channel intent"
MAX_LOG = 256 * 1024


def read(path, maximum=8 * 1024 * 1024):
    meta = path.lstat()
    require(stat.S_ISREG(meta.st_mode) and not getattr(meta, "st_file_attributes", 0) & 0x400
            and meta.st_size <= maximum, "nonregular/reparse/oversized input: " + str(path))
    data = path.read_bytes()
    require(len(data) == meta.st_size, "input changed while reading")
    return data


def snapshot(root):
    """Bounded exact file identities/bytes; directory mtimes and read atimes are irrelevant."""
    result = {}
    pending = list(sorted(root.iterdir()))
    while pending:
        path = pending.pop(0)
        meta = path.lstat()
        require(not getattr(meta, "st_file_attributes", 0) & 0x400 and not path.is_symlink(), "fixture reparse point")
        require(len(result) < 128, "fixture inventory exceeds bound")
        name = path.relative_to(root).as_posix()
        if stat.S_ISDIR(meta.st_mode):
            result[name] = "directory"
            pending.extend(sorted(path.iterdir()))
        else:
            data = read(path)
            result[name] = {"bytes": len(data), "sha256": sha(data), "device": meta.st_dev,
                            "file_id": meta.st_ino, "mtime_ns": meta.st_mtime_ns}
    return result


def bind_producer(producer, manifest, source, env, commit, version):
    candidate.selection(commit, version)
    require(version == VERSION, "default-channel launcher fixture requires exactly0.3.8")
    require(producer.get("schema") == "flere-windows-candidate-v1"
            and producer.get("status") == "prepared_not_published", "candidate did not pass")
    for key, expected in {"commit": commit, "version": version, "workflow_sha": env["FLERE_WORKFLOW_SHA"],
                          "run_id": env["GITHUB_RUN_ID"], "run_attempt": env["GITHUB_RUN_ATTEMPT"]}.items():
        require(producer.get(key) == expected, "producer identity differs: " + key)
    require(producer.get("source_unchanged") is True and producer.get("stateless_state_unchanged") is True
            and producer.get("stateless_checks") == 6 and producer.get("checks")
            and all(row.get("status") == "passed" for row in producer["checks"]), "candidate checks incomplete")
    require(manifest["source"] == source and source["git_commit"] == commit and source["dirty"] is False
            and re.fullmatch("[0-9a-f]{64}", source["source_sha256"])
            and source["source_sha256"] == producer["source_sha256"], "source receipt differs")
    require(manifest["build"] == producer["build"] and manifest["build"]["package_version"] == version
            and manifest["build"]["target"] == candidate.TARGET and manifest["build"]["component"] == "flere-connect"
            and manifest["build"]["profile"] == "release" and manifest["payload"]["file_name"] == "flere-connect"
            and manifest["payload"]["sha256"] == producer["payload_sha256"], "candidate package differs")


def unpack(path, work, expected_sha, expected_bytes, *, old=False):
    data = read(path)
    require(len(data) == expected_bytes and sha(data) == expected_sha, "ZIP pin differs")
    with zipfile.ZipFile(path) as archive:
        # The maintained validator checks exact inventory, regular modes and sizes before extraction.
        require(all(item.file_size <= 8 * 1024 * 1024 for item in archive.infolist()), "fixture ZIP exceeds bound")
        raw_manifest = archive.read("manifest.json")
        manifest = json.loads(raw_manifest)
        license_bytes = archive.read("LICENSE")
        require(sha(license_bytes) == LICENSE_SHA and len(license_bytes) == 1075, "license differs")
        candidate.verify_zip(path, manifest, license_bytes)
        if old:
            require(len(raw_manifest) == 1423 and sha(raw_manifest) == OLD_MANIFEST
                    and manifest["build"]["package_version"] == OLD_VERSION
                    and manifest["source"]["git_commit"] == OLD_COMMIT
                    and manifest["payload"]["bytes"] == 2276864
                    and manifest["payload"]["sha256"] == OLD_PAYLOAD, "public037 manifest/payload differs")
        work.mkdir()
        package = work / "package"; package.mkdir()
        (package / "manifest.json").write_bytes(raw_manifest)
        (package / "flere-connect").write_bytes(archive.read("flere-connect.exe"))
        for name in ("flere.exe", "flere-connect.exe"):
            (work / name).write_bytes(archive.read(name))
    return manifest, package


def fixture_env(root):
    root.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith(("FLERE_", "CARGO_"))
           and k not in ("GH_TOKEN", "GITHUB_TOKEN")}
    for key, name in {"HOME": "home", "USERPROFILE": "home", "LOCALAPPDATA": "local", "APPDATA": "roaming",
                      "XDG_CACHE_HOME": "cache", "XDG_CONFIG_HOME": "config", "XDG_DATA_HOME": "data",
                      "XDG_STATE_HOME": "state", "TEMP": "temp", "TMP": "temp"}.items():
        path = root / name; path.mkdir(exist_ok=True); env[key] = str(path)
    (root / "unrelated-sentinel").write_bytes(b"owned synthetic state; retain exactly\n")
    return env


def check_refusal(code, stdout, stderr, before, after):
    require(code != 0 and stdout == b"" and REFUSAL in stderr, "wrong default-source refusal")
    require(before == after, "refused default source mutated fixture files/identity")



def check_transition(prior, final, before, after):
    require(prior["source"] == {"kind": "local"} and final["source"] == {"kind": "default_channel"}
            and final["current"] == prior["current"] and final["previous"] == prior["previous"]
            and final["attempt"] != prior["attempt"], "supporting transition lost policy/package/attempt")
    path = "local/Flere/install/flere-connect.json"
    require(path in before and path in after and before[path] != after[path], "receipt did not change")
    require({k: v for k, v in before.items() if k != path} == {k: v for k, v in after.items() if k != path},
            "supporting transition changed other state")


def check_status(value, receipt, manifest):
    require(value.get("schema_version") == 1 and value.get("running_build") == manifest["build"]
            and value.get("installation") == receipt, "actual worker/status receipt differs")


def run(producer_proof, output, commit, version):
    candidate.hosted(os.environ)
    candidate.selection(commit, version)
    require(version == VERSION and os.name == "nt" and platform.machine().lower() in ("amd64", "x86_64"),
            "native Windows038 required")
    require(subprocess.check_output(["git", "-C", str(candidate.PROJECT), "rev-parse", "HEAD"], text=True).strip()
            == os.environ["FLERE_WORKFLOW_SHA"], "automation checkout differs")
    cache = Path.home().resolve() / ".cache/flere/tmp"
    require(output.parent.resolve() == cache and not output.exists(), "fresh private HOME-cache output required")
    require(producer_proof.name == "proof" and producer_proof.parent.parent.resolve() == cache,
            "same-job private candidate proof required")
    output.mkdir(); work = output / "work"; work.mkdir(); proof = output / "proof"; proof.mkdir()
    (proof / "logs").mkdir(); (proof / "records").mkdir()
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        stream.write("evidence=" + str(proof) + "\n")
    result = {"schema": "flere-windows-default-launcher-v1", "status": "running", "checks": [],
              "commit": commit, "version": version, "workflow_sha": os.environ["FLERE_WORKFLOW_SHA"],
              "run_id": os.environ["GITHUB_RUN_ID"], "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"],
              "machine": {"platform": platform.platform(), "image_os": os.environ.get("ImageOS"),
                          "image_version": os.environ.get("ImageVersion"), "python": platform.python_version()},
              "old_public_zip": {"url": OLD_URL, "bytes": OLD_BYTES, "sha256": OLD_SHA},
              "limits": ["Native Windows managed companion/receipt-reader check only; no core, UI, SSH or sessions.",
                         "No live default-channel resolution, rollback/recovery interruption or network update proof.",
                         "Only fixed public037 ZIP is downloaded;038 is the exact same-run candidate, not a release."]}
    deadline = time.monotonic() + 300
    owned_running = set()

    def save(name, value):
        (proof / name).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    def command(name, argv, env, *, rejected=False):
        require(time.monotonic() < deadline, "fixture deadline exceeded")
        paths = [proof / "logs" / (name + suffix + ".log") for suffix in ("", "-stderr")]
        row = {"name": name, "argv": list(map(str, argv)), "status": "running", "expected_rejection": rejected}
        result["checks"].append(row); save("receipt.json", result)
        started = time.monotonic(); failure = None
        with paths[0].open("xb") as stdout, paths[1].open("xb") as stderr:
            process = subprocess.Popen(row["argv"], env=env, cwd=work, stdin=subprocess.DEVNULL,
                                       stdout=stdout, stderr=stderr)
            row["pid"] = process.pid; owned_running.add(process.pid)
            try:
                while process.poll() is None:
                    require(time.monotonic() < min(deadline, started + 90), name + " timed out")
                    require(all(p.stat().st_size <= MAX_LOG for p in paths), name + " output exceeds bound")
                    time.sleep(0.05)
            except BaseException as error:
                failure = error
            finally:
                row["forced_stop"] = process.poll() is None
                if row["forced_stop"]:
                    subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/taskkill.exe"),
                                    "/PID", str(process.pid), "/T", "/F"], stdin=subprocess.DEVNULL,
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=15, check=True)
                process.wait(timeout=15); owned_running.remove(process.pid)
        for path in paths:
            if path.stat().st_size > MAX_LOG:
                row["output_overflow"] = True
                with path.open("r+b") as stream:
                    stream.truncate(MAX_LOG)
                failure = failure or ValueError(name + " output exceeds bound")
        data, errors = [p.read_bytes() for p in paths]
        row.update(exit=process.returncode, seconds=round(time.monotonic() - started, 3),
                   stdout_bytes=len(data), stdout_sha256=sha(data), stderr_bytes=len(errors), stderr_sha256=sha(errors),
                   status="passed" if failure is None and ((process.returncode != 0) if rejected else
                                                           (process.returncode == 0 and errors == b"")) else "failed")
        save("receipt.json", result)
        if failure is not None:
            raise failure
        require(row["status"] == "passed", name + " failed; retained separate stdout/stderr")
        return process.returncode, data, errors

    def installed(env, manifest, launcher_sha, source):
        base = Path(env["LOCALAPPDATA"]) / "Flere"
        receipt = json.loads(read(base / "install/flere-connect.json"))
        require(receipt["schema_version"] == 1 and receipt["owner"] == "flere"
                and receipt["component"] == "flere-connect" and receipt["source"] == source
                and receipt["current"]["manifest"] == manifest and receipt["launcher_sha256"] == launcher_sha,
                "installed receipt identity/source differs")
        launcher = base / "bin/flere.exe"
        require(Path(receipt["destination"]) == launcher
                and list(map(Path, receipt["aliases"])) == [base / "bin/flere-connect.exe"], "installed aliases differ")
        for name in ("flere.exe", "flere-connect.exe"):
            require(sha(read(base / "bin" / name)) == launcher_sha, "stable launcher bytes changed")
        worker = Path(receipt["current"]["executable"])
        require(worker == base / "install/packages" / receipt["current"]["id"] / "flere-connect.exe"
                and sha(read(worker)) == manifest["payload"]["sha256"], "installed worker identity differs")
        require(not (base / "install/flere-connect.pending.json").exists(), "pending receipt remained")
        return receipt, launcher, worker

    try:
        producer = json.loads(read(producer_proof / "candidate.json"))
        zip_pin = producer["zip"]
        require(zip_pin["name"] == "flere-connect-0.3.8-" + candidate.TARGET + ".zip", "candidate ZIP name differs")
        new, new_package = unpack(producer_proof / "windows" / zip_pin["name"], work / "new",
                                  zip_pin["sha256"], zip_pin["bytes"])
        source = json.loads(read(producer_proof / "source-receipt.json"))
        bind_producer(producer, new, source, os.environ, commit, version)
        save("records/candidate.json", producer); save("records/source-receipt.json", source)
        save("records/new-manifest.json", new)
        old_archive = work / OLD_ZIP
        with old_archive.open("xb") as stream:
            _, count, digest = candidate.module("install").fetch(OLD_URL, OLD_BYTES, stream)
        require((count, digest) == (OLD_BYTES, OLD_SHA), "public037 download pin differs")
        old, old_package = unpack(old_archive, work / "old", OLD_SHA, OLD_BYTES, old=True)
        save("records/old-manifest.json", old)
        old_env = fixture_env(work / "retained-old")
        new_env = fixture_env(work / "supporting-new")
        initial = snapshot(work / "retained-old")
        for alias in ("flere", "flere-connect"):
            exe = work / "new" / (alias + ".exe")
            require(json.loads(command(alias + "-build", [exe, "--build-info"], old_env)[1]) == new["build"],
                    "actual038 build differs")
            require(json.loads(command(alias + "-capability", [exe, *CAPABILITY_ARGS], old_env)[1]) == CAPABILITY,
                    "actual038 capability differs")
        require(snapshot(work / "retained-old") == initial, "stateless038 probe wrote state")
        old_exe, new_exe = work / "old/flere.exe", work / "new/flere.exe"
        command("install-public037", [old_exe, "install", old_package, "--source-url", OLD_MANIFEST_URL], old_env)
        prior, launcher, _ = installed(old_env, old, OLD_PAYLOAD, {"kind": "public", "manifest_url": OLD_MANIFEST_URL})
        save("records/old-initial-receipt.json", prior)
        command("install038-local-retain037", [new_exe, "install", new_package], old_env)
        before_receipt, launcher, worker = installed(old_env, new, OLD_PAYLOAD, {"kind": "local"})
        require(before_receipt["previous"] == prior["current"], "public037 previous package not retained")
        require(json.loads(command("retained037-build", [launcher, "--build-info"], old_env)[1]) == old["build"],
                "retained launcher is not actual037")
        check_status(json.loads(command("retained037-forwards038", [launcher, "update-status"], old_env)[1]), before_receipt, new)
        require(json.loads(command("worker038-capability", [worker, *CAPABILITY_ARGS], old_env)[1]) == CAPABILITY,
                "installed038 worker capability differs")
        before = snapshot(work / "retained-old")
        save("records/refusal-before-tree.json", before); save("records/refusal-before-receipt.json", before_receipt)
        _, data, errors = command("retained037-rejects-pair", [launcher, *CAPABILITY_ARGS], old_env, rejected=True)
        require(data == b"" and b"stateless inspection flags must be used alone" in errors and snapshot(work / "retained-old") == before,
                "old launcher forwarded capability or mutated state")
        code, data, errors = command("refuse-default-with037-launcher", [worker, "install", new_package, "--default-channel"],
                                     old_env, rejected=True)
        after = snapshot(work / "retained-old")
        save("records/refusal-after-tree.json", after)
        after_receipt, _, _ = installed(old_env, new, OLD_PAYLOAD, {"kind": "local"})
        save("records/refusal-after-receipt.json", after_receipt)
        check_refusal(code, data, errors, before, after)
        require(after_receipt == before_receipt, "refused receipt changed")
        result["old_launcher_refusal_without_mutation"] = True
        command("install-supporting038-local", [new_exe, "install", new_package], new_env)
        prior, launcher, worker = installed(new_env, new, new["payload"]["sha256"], {"kind": "local"})
        # Warm normal installed status (including the system hash helper) before the policy-only baseline.
        check_status(json.loads(command("supporting-local-status", [launcher, "update-status"], new_env)[1]), prior, new)
        before = snapshot(work / "supporting-new")
        save("records/support-before-tree.json", before); save("records/support-before-receipt.json", prior)
        command("supporting038-default", [worker, "install", new_package, "--default-channel"], new_env)
        final, launcher, _ = installed(new_env, new, new["payload"]["sha256"], {"kind": "default_channel"})
        for alias in ("flere", "flere-connect"):
            exe = launcher.with_name(alias + ".exe")
            require(json.loads(command("supported-" + alias + "-capability", [exe, *CAPABILITY_ARGS], new_env)[1]) == CAPABILITY,
                    "supporting launcher capability differs")
            check_status(json.loads(command("supported-" + alias + "-status", [exe, "update-status"], new_env)[1]), final, new)
        after = snapshot(work / "supporting-new")
        save("records/support-after-tree.json", after); save("records/support-after-receipt.json", final)
        check_transition(prior, final, before, after)
        result.update(status="passed", supporting_launcher_default_receipt=True,
                      preserved_old_public_package=True, stateless038_alias_checks=4)
    except BaseException as error:
        result.update(status="failed", error=str(error))
        raise
    finally:
        result["owned_processes_remaining"] = sorted(owned_running)
        try:
            require(not owned_running, "owned process cleanup incomplete; retain private fixture")
            # Only this newly allocated work tree; the VM is the final boundary on any failed cleanup.
            snapshot(work)
            shutil.rmtree(work)
            result["private_fixture_removed"] = not work.exists()
        except BaseException as error:
            result.update(status="failed", cleanup_error=str(error), private_fixture_removed=False)
        save("receipt.json", result)
        files = {p.relative_to(proof).as_posix(): {"bytes": p.stat().st_size, "sha256": sha(read(p))}
                 for p in sorted(proof.rglob("*")) if p.is_file()}
        require(len(files) <= 80 and sum(v["bytes"] for v in files.values()) <= 8 * 1024 * 1024, "proof exceeds bound")
        save("hashes.json", files)
    require(result["status"] == "passed", "fixture failed; retain original proof")


def self_test():
    class Checks(unittest.TestCase):
        def test_exact_known_prefix_pair(self):
            self.assertEqual(CAPABILITY_ARGS, ["--build-info", "--default-channel-info"])
            self.assertEqual(candidate.selection(), (candidate.COMMIT, "0.3.4"))
        def test_refusal_requires_specific_failure_and_no_stdout(self):
            for code, out, err in [(0, b"", REFUSAL), (1, b"{}", REFUSAL), (1, b"", b"usage")]:
                with self.assertRaises(ValueError): check_refusal(code, out, err, {}, {})
            check_refusal(1, b"", REFUSAL, {"receipt": "old"}, {"receipt": "old"})
        def test_refusal_rejects_any_state_change(self):
            with self.assertRaises(ValueError): check_refusal(1, b"", REFUSAL, {"receipt": "old"}, {"receipt": "new"})
        def test_support_changes_only_policy_receipt_and_attempt(self):
            prior = {"source": {"kind": "local"}, "current": "same", "previous": None, "attempt": "old"}
            final = dict(prior, source={"kind": "default_channel"}, attempt="new")
            path = "local/Flere/install/flere-connect.json"
            before, after = {path: "old", "sentinel": "same"}, {path: "new", "sentinel": "same"}
            check_transition(prior, final, before, after)
            for changed in [dict(final, attempt="old"), dict(final, current="different"), dict(final, source={"kind": "local"})]:
                with self.assertRaises(ValueError): check_transition(prior, changed, before, after)
            with self.assertRaises(ValueError): check_transition(prior, final, before, dict(after, sentinel="changed"))
        def test_status_binds_running_worker_and_receipt(self):
            check_status({"schema_version": 1, "running_build": {"v": 8}, "installation": {"source": "local"}},
                         {"source": "local"}, {"build": {"v": 8}})
            with self.assertRaises(ValueError):
                check_status({"schema_version": 1, "running_build": {"v": 7}, "installation": {}}, {}, {"build": {"v": 8}})
        def test_bound_producer_identity_and_version(self):
            env = {"FLERE_WORKFLOW_SHA": "a" * 40, "GITHUB_RUN_ID": "1", "GITHUB_RUN_ATTEMPT": "2"}
            source = {"git_commit": "b" * 40, "dirty": False, "source_sha256": "c" * 64}
            manifest = {"source": source, "build": {"package_version": VERSION, "target": candidate.TARGET,
                        "component": "flere-connect", "profile": "release"},
                        "payload": {"file_name": "flere-connect", "sha256": "d" * 64}}
            producer = {"schema": "flere-windows-candidate-v1", "status": "prepared_not_published",
                        "commit": "b" * 40, "version": VERSION, "workflow_sha": "a" * 40, "run_id": "1", "run_attempt": "2",
                        "source_unchanged": True, "stateless_state_unchanged": True, "stateless_checks": 6,
                        "checks": [{"status": "passed"}], "source_sha256": "c" * 64, "build": manifest["build"], "payload_sha256": "d" * 64}
            bind_producer(producer, manifest, source, env, "b" * 40, VERSION)
            for key, value in [("run_id", "3"), ("run_attempt", "1"), ("commit", "e" * 40), ("workflow_sha", "e" * 40),
                               ("status", "failed"), ("source_unchanged", False), ("checks", [{"status": "failed"}])]:
                wrong = copy.deepcopy(producer); wrong[key] = value
                with self.subTest(key=key), self.assertRaises(ValueError):
                    bind_producer(wrong, manifest, source, env, "b" * 40, VERSION)
            with self.assertRaises(ValueError): bind_producer(producer, manifest, source, env, "b" * 40, "0.3.7")
        def test_snapshot_detects_identity_and_bytes(self):
            cache = Path.home() / ".cache/flere/tmp"; cache.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(prefix="launcher-pure-", dir=cache) as directory:
                root = Path(directory); path = root / "sentinel"; path.write_bytes(b"old")
                before = snapshot(root); path.write_bytes(b"new")
                self.assertNotEqual(before, snapshot(root))
                path.unlink(); path.symlink_to(root)
                with self.assertRaises(ValueError): snapshot(root)
    suite = unittest.defaultTestLoader.loadTestsFromTestCase(Checks)
    require(unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful(), "pure fixture checks failed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--candidate-proof", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--commit")
    parser.add_argument("--version")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        require(all((args.candidate_proof, args.output, args.commit, args.version)), "explicit candidate/output/commit/version required")
        run(args.candidate_proof.resolve(), args.output.resolve(), args.commit, args.version)
