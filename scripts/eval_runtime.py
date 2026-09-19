#!/usr/bin/env python3
"""Linux supervisor CPU/RSS and request latency under a controlled output workload."""
import argparse
import os
import re
from pathlib import Path
import shlex
import struct
import threading
import time

from eval_arrivals import sample as sample_arrivals
from eval_support import Fixture, distribution, frame, read_frame, save_report


def resources(pid):
    # /proc stat comm may contain spaces; field 3 begins after the last ')'.
    fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    ticks = int(fields[11]) + int(fields[12])
    rss = int(fields[21]) * os.sysconf("SC_PAGE_SIZE")
    return ticks / os.sysconf("SC_CLK_TCK"), rss


class Watcher:
    def __init__(self, fixture, command):
        self.stream = fixture.connect()
        # Quiet watchers may have no new frame indefinitely. close() shuts down
        # this owned socket to wake the reader, including a partial-frame read.
        self.stream.settimeout(None)
        self.stream.sendall(frame(command.encode()))
        self.version = {"watch": 4, "watch-panes": 5, "watch-links": 6}[command]
        self.frames = self.bytes = 0
        self.error = None
        self.generation = 0
        self.stopped = threading.Event()
        self.thread = threading.Thread(target=self.run)
        self.thread.start()

    def run(self):
        try:
            while not self.stopped.is_set():
                data = read_frame(self.stream)
                if data[0] != self.version:
                    raise ValueError("wrong snapshot wire version")
                epoch_bytes, = struct.unpack_from(">Q", data, 1)
                generation, = struct.unpack_from(">Q", data, 9 + epoch_bytes)
                if generation < self.generation:
                    raise ValueError("watch generation moved backwards")
                self.generation = generation
                self.frames += 1
                self.bytes += len(data) + 4
        except (OSError, EOFError, ValueError, struct.error) as error:
            if not self.stopped.is_set():
                self.error = str(error)

    def close(self):
        self.stopped.set()
        import socket
        self.stream.shutdown(socket.SHUT_RDWR)
        self.thread.join(timeout=6)
        self.stream.close()
        if self.thread.is_alive():
            raise RuntimeError("watcher did not stop")
        if self.error:
            raise RuntimeError(self.error)


def run(binary, cards, notes, seconds, repeats, arrival_rate=0, arrival_seed=0, max_inflight=16):
    if not Path("/proc/self/stat").exists():
        raise RuntimeError("this resource probe requires Linux /proc")
    with Fixture(binary) as fixture:
        owner = fixture.card("output fixture", shell=True)
        tab = fixture.tab(owner)
        for number in range(cards):
            fixture.metadata(fixture.card(f"fixture {number}"), "n" * notes)
        fixture.request("focus", owner, tab["id"])
        producer = fixture.root / "output.py"
        producer.write_text("import time\nfor i in range(60000):\n print(f'OUTPUT_{i:06d}', flush=True)\n time.sleep(.01)\n")
        fixture.input(tab, f"/usr/bin/python3 {shlex.quote(str(producer))}\n".encode())
        time.sleep(.5)
        fixture.request("save-tabs")
        time.sleep(1.1)  # Settle directory/layout observation before measuring.
        inventory = fixture.json("list")
        store = fixture.state / "workspaces.v2.json"
        saved, inode = store.read_bytes(), store.stat().st_ino
        cases = []
        for repeat in range(repeats):
            # Reverse order on alternate rounds to expose time/order effects.
            scenarios = [("detached", []), ("legacy", ["watch"]),
                         ("panes", ["watch-panes"]), ("links", ["watch-links"]),
                         ("mixed", ["watch", "watch-panes", "watch-links"])]
            if repeat % 2:
                scenarios.reverse()
            for label, commands in scenarios:
                watchers = [Watcher(fixture, command) for command in commands]
                try:
                    time.sleep(.25)
                    for watcher in watchers:
                        watcher.frames = watcher.bytes = 0
                    def output_index():
                        text = fixture.json("capture", tab["id"], tab["run"], 20)["text"]
                        return max(map(int, re.findall(r"OUTPUT_(\d+)", text)), default=-1)
                    first_generations = [w.generation for w in watchers]
                    first_output = output_index()
                    expected_ping = fixture.request("ping") if arrival_rate else None
                    cpu_before, _ = resources(fixture.server.pid)
                    begin = time.monotonic()
                    latency, rss = [], []
                    arrivals = None
                    if arrival_rate:
                        rss.append(resources(fixture.server.pid)[1])
                        arrivals = sample_arrivals(fixture.state / "control.sock", expected_ping,
                            seconds=seconds, rate=arrival_rate, seed=arrival_seed,
                            max_inflight=max_inflight)
                        rss.append(resources(fixture.server.pid)[1])
                        latency = [s["completion_ms"] - s["scheduled_ms"]
                                   for s in arrivals["samples"] if s["status"] == "ok"]
                    else:
                        while time.monotonic() - begin < seconds:
                            start = time.monotonic()
                            fixture.request("ping")
                            latency.append((time.monotonic() - start) * 1000)
                            rss.append(resources(fixture.server.pid)[1])
                            time.sleep(.02)
                    elapsed = time.monotonic() - begin
                    cpu_after, _ = resources(fixture.server.pid)
                    output_lines = output_index() - first_output
                    if output_lines < elapsed * 50:
                        raise RuntimeError("output fixture did not sustain at least 50 lines/sec")
                    if any(w.frames < 2 or w.generation <= first
                           for w, first in zip(watchers, first_generations)):
                        raise RuntimeError("a watcher stopped receiving current output")
                    if fixture.json("list") != inventory:
                        raise RuntimeError("fixture identity or metadata changed")
                    if store.read_bytes() != saved or store.stat().st_ino != inode:
                        raise RuntimeError("saved layout or store changed")
                    case = {"scenario": label, "repeat": repeat, "seconds": elapsed,
                            "observed_output_lines": output_lines,
                            "supervisor_cpu_percent_one_core": (cpu_after - cpu_before) / elapsed * 100,
                            "supervisor_max_sampled_rss_bytes": max(rss),
                            "ping": distribution(latency) if latency else None,
                            "arrival_probe": arrivals,
                            "watch_frames": sum(w.frames for w in watchers),
                            "watch_bytes": sum(w.bytes for w in watchers),
                            "session_identities_and_metadata_unchanged": True,
                            "saved_layout_and_store_unchanged": True}
                    cases.append(case)
                    p95 = f"{case['ping']['p95_ms']:.2f}" if latency else "none"
                    print(f"{label}: cpu={case['supervisor_cpu_percent_one_core']:.2f}% "
                          f"ping_p95={p95}ms", flush=True)
                finally:
                    for watcher in watchers:
                        watcher.close()
        return {"schema": 2, "provenance": fixture.provenance(),
                "fixture": {"stopped_cards": cards, "notes_bytes_per_card": notes,
                            "shells": 1, "output_hz_target": 100,
                            "arrival_rate_per_second": arrival_rate, "arrival_seed": arrival_seed,
                            "max_inflight": max_inflight},
                "method": "Supervisor only: /proc CPU ticks and RSS; ping round trip; fully drained wire watchers. "
                          "Checks exact session identities, metadata, saved file bytes and inode after each case. "
                          "Optional independent arrivals retain every failure/missed request; quantiles cover successes only. "
                          "Arrival-mode RSS uses endpoint samples; default sequential mode samples after each ping. "
                          "Excludes shells, observer, real UI, SSH, Windows and model inference. No user sessions.",
                "cases": cases}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    parser.add_argument("output")
    parser.add_argument("--cards", type=int, default=64)
    parser.add_argument("--notes", type=int, default=24000)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--arrival-rate", type=float, default=0)
    parser.add_argument("--arrival-seed", type=int, default=0)
    parser.add_argument("--max-inflight", type=int, default=16)
    args = parser.parse_args()
    if not ((args.arrival_rate == 0 or 1 <= args.arrival_rate <= 500)
            and 1 <= args.max_inflight <= 64):
        parser.error("arrival-rate must be 0 or 1..500; max-inflight must be 1..64")
    if not (0 <= args.cards <= 100 and 0 <= args.notes <= 30000
            and 1 <= args.seconds <= 10 and 1 <= args.repeats <= 3):
        parser.error("use cards 0..100, notes 0..30000, seconds 1..10, repeats 1..3")
    result = run(args.binary, args.cards, args.notes, args.seconds, args.repeats,
                 args.arrival_rate, args.arrival_seed, args.max_inflight)
    save_report(args.output, result)
    if any(c["arrival_probe"] and not c["arrival_probe"]["all_arrivals_succeeded"] for c in result["cases"]):
        raise SystemExit("arrival probe recorded failures or missed arrivals; inspect the saved report")
