#!/usr/bin/env python3
"""Measure exact decision retrieval and store writes with synthetic retained history."""
import argparse
import json
import os
import subprocess
from pathlib import Path
import time

from eval_coordination import Agent
from eval_support import Fixture, distribution, encode, save_report


def recover_received_history(actor, store, repeats, required):
    """Lose message IDs, discover received history, and verify exact retained facts."""
    fixture = actor.fixture
    request = {"jsonrpc": "2.0", "id": 1, "method": "tools/list"}
    env = dict(fixture.env, FLERE_SESSION=str(actor.tab["id"]), FLERE_RUN=actor.tab["run"])
    output = subprocess.run([fixture.binary, "--state", str(fixture.state), "mcp"],
                            env=env, input=encode(request) + b"\n", stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=10, check=True).stdout
    tools = json.loads(output)["result"]["tools"]
    inbox = next(tool for tool in tools if tool["name"] == "inbox")
    supported = "include_acknowledged" in inbox["inputSchema"]["properties"]
    if required:
        assert supported, "candidate does not advertise acknowledged-message recovery"
    if not supported:
        return {"supported": False}
    expected = json.loads(store.read_bytes())["coordination"]["messages"]
    expected_ids = [message["id"] for message in expected]
    actor.forget_context()
    context = actor.call("get_context")
    assert context["messages_total"] == len(expected)
    assert context["pending_messages"] == 0 and context["messages"] == []
    rounds = []
    for repeat in range(repeats + 1):
        actor.metrics.clear()
        recovered = []
        args = {"include_acknowledged": True}
        replacements = 0
        while True:
            before = store.stat()
            page = actor.call("inbox", args)
            after = store.stat()
            replacements += (before.st_ino, before.st_mtime_ns) != (after.st_ino, after.st_mtime_ns)
            assert page["pending"] == 0 and page["total"] == len(expected)
            assert page["acknowledged"] == [], "history read claims new handling"
            assert len(page["messages"]) <= 8, "default page limit exceeded"
            page_bytes = sum(len(encode(m)) for m in page["messages"])
            assert len(page["messages"]) <= 1 or page_bytes <= 32 * 1024, "whole-record byte budget exceeded"
            recovered.extend(page["messages"])
            assert page["remaining"] == len(expected) - len(recovered)
            if page["next_after"] is None:
                break
            args["after"] = page["next_after"]
            assert len(actor.metrics) <= len(expected), "history paging did not converge"
        assert [message["id"] for message in recovered] == expected_ids, "history lost, reordered or duplicated records"
        # Discovery uses no remembered IDs. The separate oracle checks retained facts;
        # transport receipts may legitimately progress while the probe runs.
        for message, original in zip(recovered, expected):
            for field in ("id", "from", "to", "body", "intent", "saved", "acknowledged", "chat"):
                assert message.get(field) == original.get(field), f"changed retained message {field}"
            assert message["native_surfaced"] is not None
        rounds.append({"phase": "first_read" if repeat == 0 else "reread", "repeat": repeat,
                       "messages": len(recovered), "calls": len(actor.metrics),
                       "body_bytes": sum(m["body_bytes"] for m in actor.metrics),
                       "mcp_response_bytes": sum(m["mcp_response_bytes"] for m in actor.metrics),
                       "store_replacements": replacements,
                       "total_call_ms": sum(m["ms"] for m in actor.metrics)})
        if repeat > 0:
            assert replacements == 0, "unchanged history reread rewrote the store"
    assert actor.call("inbox")["messages"] == [], "history reopened handled work"
    retained = fixture.state / "retained-history-store.json"
    store.rename(retained)
    store.mkdir()
    try:
        page = actor.call("inbox", {"include_acknowledged": True})
        assert [m["id"] for m in page["messages"]] == expected_ids[:len(page["messages"])]
    finally:
        store.rmdir()
        retained.rename(store)
    durable = json.loads(store.read_bytes())["coordination"]["messages"]
    assert durable == recovered, "recovered evidence differs from retained records"
    return {"supported": True, "rounds": rounds, "read_without_writes": True,
            "default_pending": 0, "records_preserved": len(durable)}


def run(binary, messages, decisions, repeats, require_message_history=False):
    with Fixture(binary) as fixture:
        standin = fixture.root / "standin.py"
        standin.write_text("import time\nprint('FIXTURE_READY', flush=True)\ntime.sleep(600)\n")
        (fixture.state / "harnesses.json").write_bytes(encode([
            {"name": "codex", "command": ["/usr/bin/python3", str(standin)]}]))
        actor = Agent(fixture, "history fixture")

        def human(operation, args):
            return fixture.json("coordinate", actor.workspace, operation, encode(args).hex())

        ids = []
        message_bodies = {}
        for number in range(messages):
            sent = human("send_message", {"to": actor.workspace,
                                          "body": f"retained synthetic record {number}: " + "h" * 15000})
            ids.append(sent["message"]["id"])
            message_bodies[ids[-1]] = sent["message"]["body"]
        if ids:
            human("inbox", {"ack_ids": ids})
        expected = {}
        for number in range(decisions):
            decision = human("request_decision", {
                "question": f"Synthetic question {number:04d}",
                "recommendation": "Retain the exact evidence and local-only boundary.",
                "evidence": "synthetic evidence " * 200})["decision"]
            if number % 2 == 0:
                decision = human("answer_decision", {"id": decision["id"],
                    "answer": f"Exact synthetic answer {number}: preserve all pending work."})["decision"]
            expected[decision["id"]] = decision
        # Settle native startup observation before measuring read-only operations.
        actor.call("get_context")
        time.sleep(.3)
        store = fixture.state / "workspaces.v2.json"
        rounds = []
        for repeat in range(repeats):
            actor.metrics.clear()
            seen = {}
            args = {}
            replacements = 0
            while True:
                before = store.stat()
                page = actor.call("decisions", args)
                after = store.stat()
                replacements += (before.st_ino, before.st_mtime_ns) != (after.st_ino, after.st_mtime_ns)
                for decision in page["decisions"]:
                    assert decision["id"] not in seen, "duplicate decision"
                    assert decision == expected[decision["id"]], "changed decision evidence"
                    seen[decision["id"]] = decision
                if page.get("next_after") is None:
                    break
                args = {"after": page["next_after"]}
                assert len(actor.metrics) <= decisions, "decision paging did not converge"
            assert seen == expected, "missing historical decisions"
            rounds.append({"repeat": repeat, "calls": len(actor.metrics),
                           "body_bytes": sum(m["body_bytes"] for m in actor.metrics),
                           "mcp_response_bytes": sum(m["mcp_response_bytes"] for m in actor.metrics),
                           "store_replacements": replacements,
                           "total_call_ms": sum(m["ms"] for m in actor.metrics),
                           "latency": distribution([m["ms"] for m in actor.metrics])})
        # Read-only history must remain usable even when durable writes cannot succeed.
        retained = fixture.state / "retained-store.json"
        store.rename(retained)
        store.mkdir()
        try:
            try:
                result = actor.call("decisions")
                read_without_writes = bool(result["decisions"])
            except RuntimeError:
                read_without_writes = False
        finally:
            store.rmdir()
            retained.rename(store)
        def resources():
            proc = Path(f"/proc/{fixture.server.pid}/stat")
            if not proc.exists():
                return None
            fields = proc.read_text().rsplit(")", 1)[1].split()
            return {"cpu_seconds": (int(fields[11]) + int(fields[12])) / os.sysconf("SC_CLK_TCK"),
                    "rss_bytes": int(fields[21]) * os.sysconf("SC_PAGE_SIZE")}

        # Measure actual durable writes as well as the read-only path.
        actor.metrics.clear()
        samples = [resources()]
        for number in range(10):
            receipt = actor.call("checkpoint", {"body": f"synthetic durability probe {number}"})
            assert receipt["accepted"] is False
            samples.append(resources())
        durable = json.loads(store.read_bytes())
        saved_messages = durable["coordination"]["messages"]
        assert len(saved_messages) == len(message_bodies), "retained message count changed"
        assert {m["id"]: m["body"] for m in saved_messages} == message_bodies, "retained message bodies changed"
        assert all(m["acknowledged"] is not None and m["from"] == 0
                   and m["to"] == actor.workspace for m in saved_messages), "retained message provenance/ACK changed"
        assert {d["id"]: d for d in durable["coordination"]["decisions"]} == expected, "durable decisions changed"
        retained_history = {"messages": len(saved_messages), "acknowledged": len(saved_messages),
                            "message_body_bytes": sum(len(m["body"].encode()) for m in saved_messages),
                            "message_receipts": sum(m.get("delivery") is not None for m in saved_messages),
                            "decisions": len(expected)}
        assert durable["coordination"]["checkpoints"][-1]["body"] == "synthetic durability probe 9"
        writes = {"calls": len(actor.metrics), "total_call_ms": sum(m["ms"] for m in actor.metrics),
                  "latency": distribution([m["ms"] for m in actor.metrics]),
                  "supervisor_samples": samples}
        checkpoint_history = []
        checkpoint_args = {}
        checkpoint_supported = True
        while True:
            try:
                page = actor.call("checkpoints", checkpoint_args)
            except RuntimeError as error:
                if "unknown coordination operation" not in str(error):
                    raise
                checkpoint_supported = False
                break
            checkpoint_history.extend(entry["checkpoint"] for entry in page["checkpoints"])
            if page.get("next_after") is None:
                break
            checkpoint_args = {"after": page["next_after"]}
            assert len(checkpoint_history) <= 10, "checkpoint paging did not converge"
        if checkpoint_supported:
            assert checkpoint_history == durable["coordination"]["checkpoints"], "checkpoint evidence changed"
        message_history = recover_received_history(actor, store, repeats, require_message_history)
        report = {"schema": 1, "provenance": fixture.provenance(),
                  "fixture": {"retained_acknowledged_messages": messages, "decisions": decisions},
                  "store_bytes": store.stat().st_size, "retained_history": retained_history, "rounds": rounds,
                  "read_without_writes": read_without_writes, "writes": writes,
                  "checkpoint_history_supported": checkpoint_supported,
                  "received_message_history": message_history,
                  "method": "Retrieve all exact decisions through real MCP calls. Count observed store inode/mtime "
                            "changes during reads; simulate unavailable persistence with an owned fixture path. "
                            "Ten checkpoint writes measured separately, with post-call Linux CPU/RSS when available; not a peak allocation measure. "
                            "Received-message history, when advertised, is discovered without supplying remembered IDs and compared with a separate exact oracle. "
                            "First body surfacing and unchanged rereads are measured separately. "
                            "Setup excluded. No models, real conversations, storage deletion or live migration."}
        print(f"{messages} retained messages; {decisions} decisions; "
              f"store={report['store_bytes']} bytes; rounds={rounds}; read_without_writes={read_without_writes}", flush=True)
        return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("output")
    parser.add_argument("--messages", type=int, default=160)
    parser.add_argument("--decisions", type=int, default=40)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--require-message-history", action="store_true",
                        help="fail unless the candidate advertises and passes acknowledged-message recovery")
    args = parser.parse_args()
    if not (0 <= args.messages <= 300 and 1 <= args.decisions <= 100 and 1 <= args.repeats <= 20):
        parser.error("use messages 0..300, decisions 1..100, repeats 1..20")
    save_report(args.output, run(args.binary, args.messages, args.decisions, args.repeats,
                                args.require_message_history))
