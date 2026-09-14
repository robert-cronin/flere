[Documentation](README.md) · [Architecture](architecture.md) · [Raw data](benchmarks/macos-m5-2026-09-11.json)

# Historical benchmark: Railhand 0.2.20

At 120×32 cells, this run measured **18.7 ms median** and
**20.3 ms p95** from writing an eight-byte marker into Railhand's
outer PTY to observing that marker in a completed rendered frame. Idle Railhand
used **6.0 MiB resident memory** with one shell, excluding that shell.

![Measured latency and resident memory](assets/performance.svg)

## Environment and provenance

| Property | Recorded value |
| --- | --- |
| Date | 2026-09-11T12:10:06Z |
| Hardware | Apple M5, 10 logical CPUs, 32 GiB RAM |
| OS | macOS 26.5.1, arm64 |
| Build | Release, Railhand 0.2.20, rustc 1.98.0 (88d9e12ae 2026-08-18) |
| Runtime source | `cc19456` (private historical revision; excluded from this public snapshot) |
| Scope | One local run; ordinary desktop applications remained running |

[Machine-readable results](benchmarks/macos-m5-2026-09-11.json) include the release
binary SHA-256, benchmark source hashes, every measured latency value (sorted),
and time-stamped resident-memory samples. The run did not use CPU affinity,
system tuning, a coding agent, or an existing user workspace.

## Input-to-frame latency

Each case uses an owned `/bin/sh -i` with a private home and predictable prompt.
After five unreported warmups, the probe performs 100 measurements:

1. Start a monotonic timer and write a unique eight-byte printable marker.
2. Let the real UI, control socket, supervisor, child PTY, and tty echo process it.
3. Drain the outer PTY into a terminal observer. Stop only when the marker is
   visible and the corresponding DEC 2026 synchronized frame is complete.
4. Clear the unsubmitted line with Ctrl+U and wait for that update before proceeding.

The baseline writes the same marker directly into a separate shell PTY, using the
same observer without a Railhand UI/supervisor round trip. No command is submitted.
Percentiles use the nearest-rank definition. All table values below are **milliseconds**.

| Outer viewport | Direct PTY median | Railhand median | Railhand p95 | Railhand p99 | Railhand max |
| --- | ---: | ---: | ---: | ---: | ---: |
| 80×24 | 0.055 | 18.127 | 19.683 | 23.445 | 25.690 |
| 120×32 | 0.074 | 18.680 | 20.319 | 22.593 | 36.538 |
| 160×42 | 0.108 | 18.962 | 20.325 | 22.350 | 23.983 |

This is a closed-loop shell-echo probe. It includes local scheduling, IPC, emulator
processing, row rendering, and observation overhead. The direct baseline helps
make those boundaries visible; it is not a competitor comparison.

The supervisor [coalesces watch frames on a 16 ms interval](../src/server.rs).
That implementation is consistent with this run's roughly 18–20 ms medians;
this is an explanation from source, not a separately measured decomposition.

**It does not measure physical key-to-photon latency**, monitor refresh, terminal
font rendering, SSH, model inference, tool execution, or sustained output load.
Results for a quiet shell do not establish a worst-case bound for active agents.

## CPU and resident memory

Three scenarios are sampled for ten seconds each at a 120×32 attached viewport.
Resource sampling follows the latency cases, so these are warmed processes, not
cold-start minimums. Railhand means the supervisor plus its UI when attached.
Shell processes are reported separately. No unrelated desktop process is counted.

| Scenario | Railhand mean RSS (MiB) | Railhand max sampled RSS (MiB) | Shell mean RSS (MiB) | Railhand CPU / one core |
| --- | ---: | ---: | ---: | ---: |
| detached / 1 shell | 3.27 | 3.33 | 1.77 | 0.00% |
| attached / 1 shell | 6.03 | 6.86 | 1.65 | 0.00% |
| attached / 8 shells | 7.04 | 7.23 | 15.41 | 0.19% |

RSS comes from `ps`, sampled about once a second. The maximum is the maximum of
those samples, not a continuous peak. RSS may count shared pages in multiple
processes and excludes compressed/nonresident memory; it is not identical to
Activity Monitor's memory footprint. The earlier live-session estimate is not
used as this benchmark's dataset.

CPU is the delta in process CPU time divided by wall time. **100% means one CPU
core**, not the whole machine. The counter has finite resolution, so a displayed
0.00% means no measurable increment during that sample. Shells in these scenarios
were idle; models, MCP servers, editors, and compilers add their own resource use.
The observer and `ps` sampler are excluded from the application totals but can
still affect scheduling on this shared machine.

## Reproduce the run

The commands below record a fresh Flere build. The published 0.2.20 results above remain historical Railhand evidence.


From the repository checkout, with the locked dependencies cached:

```sh
cargo build --offline --locked --release --bin flere --example latency_bench
mkdir -p "$HOME/.cache/flere/docs-results"
./target/release/examples/latency_bench \
  ./target/release/flere \
  "$HOME/.cache/flere/docs-results/local.json"
```

The probe creates a disposable home-cache state and stops only its own supervisor
and child sessions on exit. It never connects to your default Flere state.
It runs 600 measured echo trials across the direct and Flere paths, plus
warmups and three ten-second resource windows. Run it again under your intended
workload; keep each run as a separate result instead of selecting the fastest.

To render the charts, install the documentation-only Python dependencies in a
virtual environment and run:

```sh
python3 -m venv "$HOME/.cache/flere/docs-tools"
"$HOME/.cache/flere/docs-tools/bin/python" -m pip install -r docs/tools/requirements.txt
"$HOME/.cache/flere/docs-tools/bin/python" docs/tools/render_performance.py \
  docs/benchmarks/macos-m5-2026-09-11.json docs/assets
```

The benchmark has portable Unix primitives; this published dataset and the
hardware-identification fields are macOS-specific. A Linux run needs its own
recorded machine/OS context. [Benchmark source](../examples/latency_bench.rs) ·
[Owned-session helper](../examples/support/mod.rs) ·
[Chart renderer](tools/render_performance.py)
