#!/usr/bin/env python3
"""Offline information-delivery evaluation: real MCP transport, synthetic agent stand-ins.

Measures exact retrieval and wire cost, not model comprehension. The delta-simulation
policy estimates receiver-side traffic; incremental measures actual server replies.
"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import time

from eval_support import Fixture, distribution, encode, save_report


class Agent:
    def __init__(self, fixture, name, sender=None, assignment=None):
        self.fixture = fixture
        self.workspace = fixture.card(name)
        if sender is None:
            fixture.request("native", self.workspace, "codex", "")
        else:
            inventory = fixture.json("list")
            prepared = sender.call("prepare_worker", {
                "workspace": self.workspace, "request_id": "synthetic-assignment",
                "expected_epoch": inventory["epoch"], "expected_cwd": str(fixture.root),
                "assignment": assignment,
                "user_request": "Evaluate this harmless fixture only. No publication or model inference."})
            sender.call("start_worker", {"dispatch_id": prepared["dispatch"]["id"]})
        self.tab = fixture.tab(self.workspace)
        deadline = time.monotonic() + 5
        while "FIXTURE_READY" not in fixture.json("capture", self.tab["id"], self.tab["run"], 20)["text"]:
            if time.monotonic() > deadline:
                raise RuntimeError("harmless native stand-in did not become ready")
            time.sleep(.01)
        self.metrics = []
        self.previous = None
        self.revision = None
        self.reconstructed = None

    def call(self, name, arguments=None):
        request = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                   "params": {"name": name, "arguments": arguments or {}}}
        env = dict(self.fixture.env, FLERE_SESSION=str(self.tab["id"]), FLERE_RUN=self.tab["run"])
        start = time.monotonic()
        out = subprocess.run([self.fixture.binary, "--state", str(self.fixture.state), "mcp"],
                             env=env, input=encode(request) + b"\n", stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, timeout=10, check=True)
        elapsed = (time.monotonic() - start) * 1000
        response = json.loads(out.stdout)
        result = response["result"]
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        body = result["content"][0]["text"]
        value = json.loads(body)
        self.metrics.append({"tool": name, "ms": elapsed, "body_bytes": len(body.encode()),
                             "request_bytes": len(encode(request)) + 1, "mcp_response_bytes": len(out.stdout)})
        return value

    def context(self, policy):
        arguments = {"detail": policy == "full"}
        if policy == "incremental" and self.revision is not None:
            arguments["since"] = self.revision
        response = self.call("get_context", arguments)
        value = copy.deepcopy(response)
        if policy == "incremental":
            if response["context_mode"] == "delta":
                assert response["base_revision"] == self.revision
                assert response["identity"]["workspace"] == self.workspace
                assert response["identity"]["session"] == self.tab["id"]
                assert response["identity"]["run"] == self.tab["run"]
                for key in response["removed"]:
                    self.reconstructed.pop(key)
                self.reconstructed.update(copy.deepcopy(response["changes"]))
                value = copy.deepcopy(self.reconstructed)
            else:
                value.pop("context_mode", None)
                value.pop("context_revision", None)
                self.reconstructed = copy.deepcopy(value)
            self.revision = response["context_revision"]
        else:
            value.pop("context_mode", None)
            value.pop("context_revision", None)
        if policy == "delta-simulation":
            revision = hashlib.sha256(encode(value)).hexdigest()
            if self.previous is None:
                delivery = {"revision": revision, "reset": value}
                self.reconstructed = copy.deepcopy(value)
            else:
                changed = {key: item for key, item in value.items()
                           if key not in self.previous or self.previous[key] != item}
                removed = [key for key in self.previous if key not in value]
                delivery = {"base": self.revision, "revision": revision,
                            "set": changed, "remove": removed}
                for key in removed:
                    self.reconstructed.pop(key)
                self.reconstructed.update(copy.deepcopy(changed))
            assert self.reconstructed == value, "delta lost an exact field"
            self.previous = copy.deepcopy(value)
            self.revision = revision
            # Candidate cost includes its actual JSON text in an MCP-shaped envelope.
            self.metrics[-1]["candidate_body_bytes"] = len(encode(delivery))
            envelope = {"jsonrpc": "2.0", "id": 1, "result": {
                "content": [{"type": "text", "text": encode(delivery).decode()}]}}
            self.metrics[-1]["candidate_mcp_response_bytes"] = len(encode(envelope)) + 1
        return value

    def forget_context(self):
        self.previous = self.reconstructed = self.revision = None


def evaluate(binary, policy):
    with Fixture(binary) as fixture:
        standin = fixture.root / "standin.py"
        standin.write_text("import time\nprint('FIXTURE_READY', flush=True)\ntime.sleep(300)\n")
        (fixture.state / "harnesses.json").write_bytes(encode([
            {"name": "codex", "command": ["/usr/bin/python3", str(standin)]}]))
        sender = Agent(fixture, "sender")
        assignment = ("FIRST: preserve provenance and exact recipients.\n" + "synthetic task evidence " * 200
                      + "\nMIDDLE: keep work local; do not push.\n" + "synthetic task evidence " * 200
                      + "\nLAST: read later corrections before acting.")
        recipient = Agent(fixture, "recipient", sender, assignment)
        for number in range(40):
            fixture.metadata(fixture.card(f"background {number}"), "unrelated notes " * 800)
        fixture.metadata(recipient.workspace, "recipient reference notes " * 500)
        expected = {}
        # Include tiny messages, page-boundary bodies, escaped text and Unicode.
        bodies = [f"Dependency {i}: exact constraint = preserve-{i}." for i in range(12)]
        bodies += ["long evidence " * 1000, "\u0001" * 8000, "界 🔬 exact tail constraint" * 200]
        for body in bodies:
            result = sender.call("send_message", {"to": recipient.workspace, "body": body})
            expected[result["message"]["id"]] = body
        recipient.call("checkpoint", {"body": "Work stays local. Do not push. Evidence remains retrievable."})
        decision = recipient.call("request_decision", {
            "question": "Which synthetic limit?", "recommendation": "Use 8; preserve exact boundaries.",
            "evidence": "synthetic evidence only"})["decision"]["id"]
        initial = recipient.context(policy)
        assert initial["workspace"]["id"] == recipient.workspace
        assert initial["assignment"]["body"] == assignment
        assert initial["assignment"]["acknowledgement_required"] is True
        assert initial["pending_messages"] == len(expected)
        assert initial["decisions"][0]["id"] == decision
        assert initial["checkpoints"][0]["body"].startswith("Work stays local.")
        recipient.call("ack_assignment", {"dispatch_id": initial["assignment"]["dispatch"]["id"]})
        for _ in range(5):
            repeated = recipient.context(policy)
            assert repeated["assignment"]["body"] == assignment
            assert repeated["assignment"]["acknowledgement_required"] is False
        # A later correction must invalidate the candidate delta and remain exact.
        correction = "Correction: use 5, preserve local-only scope; supersedes limit 8."
        fixture.request("coordinate", recipient.workspace, "answer_decision",
                        encode({"id": decision, "answer": correction}).hex())
        changed = recipient.context(policy)
        assert changed["decisions"][0]["answer"] == correction
        seen = {}
        page = changed if policy == "full" else recipient.call("inbox")
        pages = 0
        while True:
            messages = page["messages"]
            ids = []
            for message in messages:
                assert message["id"] not in seen, "duplicate handling"
                assert message["body"] == expected[message["id"]], "body or provenance changed"
                assert message["from"] == sender.workspace and message["to"] == recipient.workspace
                seen[message["id"]] = message["body"]
                ids.append(message["id"])
            if ids:
                ack = recipient.call("inbox", {"ack_ids": ids})
                assert ack["messages"] == [] and ack["acknowledged"] == ids
            pages += 1
            if pages == 1:
                # Arrival after an ACK must not get skipped by the retained cursor.
                late = "Late dependency: do not discard pending evidence."
                sent = sender.call("send_message", {"to": recipient.workspace, "body": late})
                expected[sent["message"]["id"]] = late
            if len(seen) == len(expected):
                break
            args = {"after": page["next_after"]} if page.get("next_after") else {}
            page = recipient.call("inbox", args)
            assert pages < 20, "paging did not converge"
        assert seen == expected
        final = recipient.context(policy)
        assert final["pending_messages"] == 0
        for _ in range(5):
            recipient.context(policy)
        # A receiver that lost state (e.g. compaction) requires a complete reset.
        recipient.forget_context()
        reset = recipient.context(policy)
        assert reset == final
        original = next(iter(expected))
        restored = recipient.call("message_status", {"id": original, "detail": True})
        assert restored["message"]["body"] == expected[original]
        assert restored["message"]["acknowledged"] is not None
        metrics = recipient.metrics
        # Fixed startup/catalog overhead is reported separately from workflow calls.
        catalog = []
        for method in ["initialize", "tools/list"]:
            request = {"jsonrpc": "2.0", "id": 1, "method": method}
            env = dict(fixture.env, FLERE_SESSION=str(recipient.tab["id"]), FLERE_RUN=recipient.tab["run"])
            output = subprocess.run([fixture.binary, "--state", str(fixture.state), "mcp"],
                                    env=env, input=encode(request) + b"\n", stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=10, check=True).stdout
            result = json.loads(output)["result"]
            catalog.append({"method": method, "response_bytes": len(output),
                            "tools": len(result.get("tools", []))})
        summary = {"policy": policy, "calls": len(metrics), "handled_messages": len(seen),
                   "correct": True, "latency_including_mcp_startup": distribution([m["ms"] for m in metrics]),
                   "body_bytes": sum(m["body_bytes"] for m in metrics),
                   "mcp_response_bytes": sum(m["mcp_response_bytes"] for m in metrics),
                   "request_bytes": sum(m["request_bytes"] for m in metrics), "operations": metrics, "startup": catalog,
                   "durable_store_bytes": (fixture.state / "workspaces.v2.json").stat().st_size}
        if policy == "delta-simulation":
            summary["candidate_body_bytes"] = sum(m.get("candidate_body_bytes", m["body_bytes"]) for m in metrics)
            summary["candidate_mcp_response_bytes"] = sum(m.get("candidate_mcp_response_bytes", m["mcp_response_bytes"]) for m in metrics)
        return summary, fixture.provenance()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("output")
    parser.add_argument("--policy", choices=["all", "full", "progressive", "delta-simulation", "incremental"], default="all")
    args = parser.parse_args()
    cases = []
    policies = ["full", "progressive", "delta-simulation", "incremental"] if args.policy == "all" else [args.policy]
    for policy in policies:
        result, provenance = evaluate(args.binary, policy)
        cases.append(result)
        print(f"{policy}: {result['calls']} calls, {result['body_bytes']} body bytes, exact retrieval passed", flush=True)
    save_report(args.output, {"schema": 1, "provenance": provenance, "cases": cases,
        "method": "Real disposable supervisors and MCP calls; harmless process stand-ins. "
                  "Full detail is an available comparison policy, not a historical-binary measurement. "
                  "Delta-simulation estimates exact-field traffic; incremental measures actual server deltas. "
                  "No model inference, tokenizer counts, comprehension, compaction frequency or native approval claims."})
