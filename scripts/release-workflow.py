#!/usr/bin/env python3
"""Small Actions adapter for the maintained producers; never publishes a release."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
if sys.version_info < (3, 11):
    raise SystemExit("Release orchestration requires Python 3.11 or newer")
ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("workflow_macos", ROOT / "scripts/release-macos.py")
mac = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(mac)
SUBMIT_STEP = "Submit once and retain checkpoint"
PROFILES = {"linux-windows": 3, "complete": 4}


def outputs(values):
    allowed = {"directory", "state", "build_receipt_sha256", "signed_receipt_sha256", "notary_receipt_sha256",
               "accepted_receipt_sha256", "windows_receipt_sha256", "checkpoint_status", "profile", "schema"}
    mac.require(set(values) <= allowed, "unsupported workflow output")
    for key, value in values.items():
        mac.require(isinstance(value, (str, int)) and not any(c in str(value) for c in "\r\n\0"), "unsafe workflow output")
        if key.endswith("sha256"):
            mac.require(re.fullmatch(r"[0-9a-f]{64}", str(value)), "invalid receipt output digest")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
        for key, value in values.items():
            stream.write(f"{key}={value}\n")


def prior_submission_guard(job_sets):
    """A lost runner response is ambiguous; never retry a started submit step."""
    for value in job_sets:
        mac.require(isinstance(value, dict) and isinstance(value.get("jobs"), list)
                    and value.get("total_count") == len(value["jobs"]) <= 50, "incomplete prior job inventory")
        jobs = [job for job in value["jobs"] if job.get("name") == "macos-submit"]
        mac.require(len(jobs) <= 1, "ambiguous prior submission job")
        for job in jobs:
            steps = job.get("steps")
            mac.require(isinstance(steps, list), "prior submission steps unavailable")
            attempts = [step for step in steps if step.get("name") == SUBMIT_STEP]
            mac.require(len(attempts) <= 1, "ambiguous prior submission step")
            if attempts:
                mac.require(attempts[0].get("conclusion") == "skipped", "submission step previously started; recover its checkpoint, do not resubmit")
            else:
                mac.require(job.get("conclusion") == "skipped", "prior submission history is incomplete")


def guard_submission(selected):
    mac.native(selected)
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "")
    mac.require(re.fullmatch(r"[1-9][0-9]*", attempt) and int(attempt) <= 20, "run-attempt history exceeds supported bound")
    if int(attempt) == 1:
        return
    api = mac.release.GitHub(os.environ.get("GH_TOKEN"))
    run = api.get("actions/runs/" + selected["run_id"])
    mac.require(str(run["id"]) == selected["run_id"] and run["head_sha"] == selected["workflow_sha"]
                and run["event"] == "workflow_dispatch" and run["head_branch"] == "main", "prior run identity differs")
    history = []
    for number in range(1, int(attempt)):
        value = api.get(f"actions/runs/{selected['run_id']}/attempts/{number}/jobs?per_page=100")
        mac.require(len(mac.release.json_bytes(value)) <= 512 * 1024, "prior job response exceeds bound")
        history.append(value)
    prior_submission_guard(history)


def checkpoint(directory, state, selected, signed_pin):
    """Pin even an ambiguous/terminal intent so a failed polling job cannot resubmit."""
    signed = mac.validate_signed(directory, signed_pin, selected)
    raw = mac.read(state / "notary.json", 65536)
    value = json.loads(raw)
    mac.require(value["schema"] == "flere-macos-notary-v1" and value["identity"] == selected
                and value["signed_receipt_sha256"] == signed_pin
                and value["zip_sha256"] == signed["files"][mac.zip_name(selected["version"])]["sha256"]
                and value["status"] in ("submitting", "submitted", "pending", "Accepted", "Invalid", "Rejected"),
                "notary checkpoint identity differs")
    if value["submission_id"] is not None:
        mac.uuid(value["submission_id"])
    else:
        mac.require(value["status"] == "submitting", "notary checkpoint lacks a submission ID")
    names = {path.name for path in state.iterdir()}
    mac.require(names <= {"notary.json", "notary-log.json"}, "unexpected checkpoint file")
    if "notary-log.json" in names:
        mac.read(state / "notary-log.json", 1024 * 1024)
    pin = mac.sha(raw)
    if value["status"] == "Accepted":
        mac.validate_notary(state, pin, selected, signed_pin, value["zip_sha256"])
    return {"state": str(state), "notary_receipt_sha256": pin, "checkpoint_status": value["status"]}


def copy_checkpoint(source, destination, pin):
    raw = mac.read(source / "notary.json", 65536)
    mac.require(mac.sha(raw) == pin, "notary checkpoint differs from the successful producer output")
    names = {path.name for path in source.iterdir()}
    mac.require(names <= {"notary.json", "notary-log.json"}, "unexpected checkpoint member")
    mac.private_output(destination)
    for name in sorted(names):
        (destination / name).write_bytes(mac.read(source / name, 65536 if name == "notary.json" else 1024 * 1024))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("profile", "preflight", "build", "sign", "notary-guard", "notary-submit", "notary-resume", "verify", "windows-pin"))
    parser.add_argument("--profile", choices=tuple(PROFILES))
    for name in ("version", "commit", "run-id", "workflow-sha", "build-receipt-sha256", "signed-receipt-sha256", "notary-receipt-sha256"):
        parser.add_argument("--" + name)
    for name in ("checkout", "directory", "notary-state"):
        parser.add_argument("--" + name, type=Path)
    args = parser.parse_args(argv)
    if args.phase == "profile":
        mac.require(args.profile in PROFILES, "explicit supported release profile required")
        outputs({"profile": args.profile, "schema": PROFILES[args.profile]}); return 0
    selected = mac.identity(args.version, args.commit, args.run_id, args.workflow_sha)
    if args.phase == "windows-pin":
        mac.windows.hosted(os.environ)
        raw = mac.read(args.directory / "candidate.json", 1024 * 1024)
        receipt = json.loads(raw)
        mac.require(receipt["status"] == "prepared_not_published" and receipt["commit"] == args.commit
                    and receipt["version"] == args.version and receipt["run_id"] == args.run_id
                    and receipt["workflow_sha"] == args.workflow_sha, "Windows candidate identity differs")
        outputs({"windows_receipt_sha256": mac.sha(raw)}); return 0
    if args.phase == "notary-guard":
        guard_submission(selected); return 0
    if args.phase == "preflight":
        mac.native(selected, credentials=True); mac.configuration(os.environ, "preflight"); return 0
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "")
    mac.require(re.fullmatch(r"[1-9][0-9]*", attempt), "run attempt missing")
    output = Path.home() / ".cache/flere/tmp" / f"release-macos-{args.run_id}-{attempt}-{args.phase}"
    output.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    if args.phase == "build":
        result = mac.build(args.checkout.resolve(), output, selected)
    elif args.phase == "sign":
        result = mac.sign(args.directory, output, selected, args.build_receipt_sha256)
    elif args.phase == "notary-submit":
        # The independent guard step must precede this step in the trusted
        # workflow. No candidate code or GitHub API token runs in this phase.
        try:
            mac.notarize(args.directory, output, selected, args.signed_receipt_sha256)
        except (OSError, ValueError, KeyError, TypeError, mac.subprocess.SubprocessError):
            pass  # Preserve only validated checkpoint data; no secret tool output.
        result = checkpoint(args.directory, output, selected, args.signed_receipt_sha256)
    elif args.phase == "notary-resume":
        copy_checkpoint(args.notary_state, output, args.notary_receipt_sha256)
        result = mac.notarize(args.directory, output, selected, args.signed_receipt_sha256,
                              resume_pin=args.notary_receipt_sha256)
        outputs({key: value for key, value in result.items() if key != "status"})
        mac.require(result["status"] == "Accepted", "notary submission is not accepted; rerun this polling job to use the same ID")
        return 0
    else:
        result = mac.verify(args.directory, args.notary_state, output, selected,
                            args.signed_receipt_sha256, args.notary_receipt_sha256)
    outputs(result)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except mac.PhaseError as error:
        print("Release workflow stopped: " + str(error), file=sys.stderr)
        sys.exit(1)
    except (OSError, ValueError, KeyError, TypeError, AttributeError, mac.subprocess.SubprocessError):
        print("Release workflow failed; inspect the retained phase state. No publication was attempted.", file=sys.stderr)
        sys.exit(1)
