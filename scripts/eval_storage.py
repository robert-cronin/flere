#!/usr/bin/env python3
"""Measure retained-history capacity without deleting evidence or launching chats."""
import argparse
import copy
import math
import subprocess
import time

from eval_support import Fixture, encode, save_report


STORE_LIMIT = 8 * 1024 * 1024
BODY_LIMIT = 16384
CAPACITY_ERROR = "workspace metadata exceeds 8 MiB"


def human(fixture, workspace, operation, args=None):
    return fixture.json("coordinate", workspace, operation, encode(args or {}).hex())


def stop(fixture):
    fixture.request("stop")
    fixture.server.wait(timeout=10)
    assert fixture.server.returncode == 0, "owned supervisor failed to stop"


def start(fixture):
    fixture.server = subprocess.Popen(
        [fixture.binary, "--state", str(fixture.state), "serve"],
        env=fixture.env, stdin=subprocess.DEVNULL, stdout=fixture.log, stderr=fixture.log)
    deadline = time.monotonic() + 10
    while True:
        try:
            fixture.request("ping")
            return
        except (OSError, EOFError):
            if fixture.server.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("owned supervisor did not restart")
            time.sleep(.01)


def saved(fixture):
    import json
    return json.loads((fixture.state / "workspaces.v2.json").read_bytes())


def seed_history(document, pending, target_bytes):
    """Fill with individually bounded synthetic records, not ignored JSON padding."""
    document = copy.deepcopy(document)
    template = dict(pending, acknowledged=1700000000, surfaced=1700000000,
                    body="", saved=1699999999)
    document["coordination"]["messages"] = [pending]
    # IDs have fixed width. Adding each empty record costs its bytes plus a comma.
    overhead = len(encode(dict(template, id="0" * 32))) + 1
    count = math.ceil((target_bytes - len(encode(document))) / (BODY_LIMIT + overhead))
    assert count > 0, "fixture target is too small"
    history = [dict(template, id=f"{number + 1:032x}") for number in range(count)]
    document["coordination"]["messages"] = history + [pending]
    remaining = target_bytes - len(encode(document))
    assert count <= remaining <= count * BODY_LIMIT, "fixture cannot fit bounded records"
    for number, record in enumerate(history):
        length = min(BODY_LIMIT, remaining - (count - number - 1))
        record["body"] = "s" * length
        remaining -= length
    assert remaining == 0 and len(encode(document)) == target_bytes
    assert len({m["id"] for m in document["coordination"]["messages"]}) == count + 1
    return document


def verify_records(fixture, workspace, expected):
    """Recover through public pages and compare every record with an independent oracle."""
    messages, cursor, calls = [], None, 0
    while True:
        args = {"include_acknowledged": True, "limit": 8}
        if cursor is not None:
            args["after"] = cursor
        page = human(fixture, workspace, "inbox", args)
        calls += 1
        assert page["acknowledged"] == [], "history read claims new acknowledgements"
        assert page["total"] == len(expected["messages"]), "history count differs"
        assert page["pending"] == sum(m["acknowledged"] is None for m in expected["messages"])
        messages.extend(page["messages"])
        assert page["remaining"] == len(expected["messages"]) - len(messages)
        if page["next_after"] is None:
            break
        assert page["next_after"] != cursor and calls <= len(expected["messages"]), "paging did not converge"
        cursor = page["next_after"]
    assert messages == expected["messages"], "retrieved history differs from exact oracle"
    assert human(fixture, workspace, "decisions")["decisions"] == expected["decisions"]
    context = human(fixture, workspace, "context")
    assert context["checkpoints"] == list(reversed(expected["checkpoints"]))
    assert saved(fixture)["coordination"] == expected, "durable evidence differs from exact oracle"
    return calls


def run_case(binary, headroom):
    with Fixture(binary) as fixture:
        workspace = fixture.card("storage fixture")
        pending = human(fixture, workspace, "send_message", {
            "to": "user", "body": "Synthetic pending correction: retain evidence; keep work local."})["message"]
        # Fixed synthetic ID cannot collide with the short retained-history sequence.
        pending["id"] = f"{1 << 120:032x}"
        human(fixture, workspace, "request_decision", {
            "question": "Synthetic storage decision", "recommendation": "Preserve pending work.",
            "evidence": "Keep the exact original records."})
        human(fixture, workspace, "checkpoint", {"body": "Synthetic checkpoint: publication is not authorized."})
        stop(fixture)
        document = seed_history(saved(fixture), pending, STORE_LIMIT - headroom)
        store = fixture.state / "workspaces.v2.json"
        store.write_bytes(encode(document))
        expected = copy.deepcopy(document["coordination"])
        start(fixture)
        # Cold startup has no remembered layout checkpoint. Normalize the seeded
        # JSON's field order and establish that checkpoint before measuring reads.
        fixture.request("save-tabs")
        assert store.stat().st_size == STORE_LIMIT - headroom, "seed size changed on normalization"
        inventory = fixture.json("list")
        assert all(not w["tabs"] for w in inventory["workspaces"]), "capacity probe launched a terminal"
        initial = store.read_bytes(), store.stat().st_ino
        begin = time.monotonic()
        reads = verify_records(fixture, workspace, expected)
        history_ms = (time.monotonic() - begin) * 1000
        assert (store.read_bytes(), store.stat().st_ino) == initial, "history read rewrote saved state"
        operations = []
        for operation, args in [
            ("checkpoint", {"body": "Synthetic capacity progress checkpoint."}),
            ("read_message", {"id": pending["id"]}),
            ("inbox", {"ack_ids": [pending["id"]]}),
        ]:
            before, inode = store.read_bytes(), store.stat().st_ino
            begin = time.monotonic()
            try:
                reply = human(fixture, workspace, operation, args)
            except RuntimeError as error:
                assert str(error) == CAPACITY_ERROR, f"unexpected mutation failure: {error}"
                accepted = False
                assert (store.read_bytes(), store.stat().st_ino) == (before, inode), "failed mutation changed store"
            else:
                accepted = True
                if operation == "checkpoint":
                    record = reply["saved"]
                    assert record["body"] == args["body"] and record["workspace"] == workspace
                    assert record["kind"] == "checkpoint" and isinstance(record["time"], int)
                    expected["checkpoints"].append(record)
                elif operation == "read_message":
                    record = reply["message"]
                    assert isinstance(record["surfaced"], int)
                    expected["messages"][-1]["surfaced"] = record["surfaced"]
                    assert record == expected["messages"][-1]
                else:
                    assert reply["acknowledged"] == [pending["id"]] and reply["pending"] == 0
                    acknowledged = saved(fixture)["coordination"]["messages"][-1]["acknowledged"]
                    assert isinstance(acknowledged, int), "ACK receipt lacks durable acknowledgement"
                    expected["messages"][-1]["acknowledged"] = acknowledged
            elapsed_ms = (time.monotonic() - begin) * 1000
            verify_records(fixture, workspace, expected)
            assert fixture.json("list") == inventory, "mutation changed workspace metadata or tabs"
            operations.append({"operation": operation, "accepted": accepted,
                               "error": None if accepted else CAPACITY_ERROR, "ms": elapsed_ms,
                               "store_growth_bytes": store.stat().st_size - len(before)})
        # Cold restart proves retained evidence is loadable, not merely in memory.
        durable = store.read_bytes()
        stop(fixture)
        start(fixture)
        verify_records(fixture, workspace, expected)
        restarted = fixture.json("list")
        assert restarted["workspaces"] == inventory["workspaces"], "cold restart changed cards"
        assert store.read_bytes() == durable, "cold restart changed durable evidence"
        return {"configured_headroom_bytes": headroom, "initial_store_bytes": len(initial[0]),
                "retained_messages": len(expected["messages"]), "history_calls": reads,
                "history_ms": history_ms, "operations": operations,
                "records_and_provenance_preserved": True, "cold_restart_preserved": True,
                "all_completion_operations_available": all(o["accepted"] for o in operations[1:]),
                "provenance": fixture.provenance()}


def run(binary, headrooms):
    cases = []
    for headroom in headrooms:
        case = run_case(binary, headroom)
        cases.append(case)
        print(f"headroom={headroom} messages={case['retained_messages']} "
              f"operations={[(o['operation'], o['accepted']) for o in case['operations']]}", flush=True)
    return {"schema": 1, "store_limit_bytes": STORE_LIMIT, "cases": cases,
            "method": "Synthetic bounded records, owned supervisors and stopped cards only. "
                      "Measures human coordination reads, checkpoints, preview surfacing and ACK at the "
                      "actual file cap. Exact paged records, decisions, checkpoints, failure rollback "
                      "and cold restart checked. No native agents, live sessions or retention deletion. "
                      "Valid evidence can report unavailable completion operations; this is not a "
                      "claim that capacity behavior is acceptable."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("output")
    parser.add_argument("--headroom", type=int, action="append",
                        help="unused bytes before mutations; repeat for several cases")
    parser.add_argument("--require-completion", action="store_true",
                        help="fail after writing evidence if any read/ACK is blocked")
    args = parser.parse_args()
    headrooms = args.headroom if args.headroom is not None else [STORE_LIMIT - 65536, STORE_LIMIT // 2, 4096, 0]
    if not (1 <= len(headrooms) <= 8 and all(0 <= n <= STORE_LIMIT - 65536 for n in headrooms)):
        parser.error("use 1..8 headrooms between 0 and the limit minus 65536 bytes")
    result = run(args.binary, headrooms)
    save_report(args.output, result)
    if args.require_completion and not all(c["all_completion_operations_available"] for c in result["cases"]):
        parser.exit(1, "Storage pressure blocked a completion operation; evidence saved.\n")
