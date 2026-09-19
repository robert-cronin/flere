[Documentation](README.md) · [Context design](design/context-engineering.md) · [Performance](performance.md)

# System evaluations

The offline probes use synthetic data, owned supervisors and disposable private homes
under the user's home cache. They never attach to the default Flere state, start
models, use credentials, or send data over the network. Core probes use Python's
standard library and a locally built Flere binary; syscall-delay modes also
require Linux `strace`. Raw reports remain private. The separately authorized
[model evaluation](#opt-in-model-evaluation) is the exception: it uses the configured
Codex model/provider and may send its synthetic workload over the network.

```sh
cargo build --offline --release
python3 scripts/eval_runtime.py target/release/flere \
  "$HOME/.cache/flere/eval-results/runtime.json"
python3 scripts/eval_coordination.py target/release/flere \
  "$HOME/.cache/flere/eval-results/coordination.json"
```

Use distinct filenames for every run. Keep an unchanged baseline binary and run
the same probe against it and the candidate, sequentially. Avoid concurrent builds
or other benchmarks. Compare all repetitions, not only the best one. Reports bind
each run to its binary SHA-256 and build metadata. Keep those reports out of public
commits until their contents have been reviewed.

## Runtime probe

The Linux-only runtime probe creates one output shell and, by default, 64 stopped
cards with 24,000 bytes of synthetic notes each. It generates about 100 lines/sec
while measuring five scenarios: detached, legacy watcher, pane watcher, hyperlink
watcher, and all three wire formats together. The second repetition reverses their
order. Each scenario verifies that output continued at at least 50 lines/sec and
reports the observed count, rather than treating a finished producer as active.

Measurements include supervisor CPU time from `/proc` (100% is one CPU core),
maximum sampled RSS, ping round-trip p50/p95/p99, watcher frame count and wire bytes.
Readers drain framed socket output and check the negotiated version. The probe
excludes child-process CPU/RSS, its own Python observer, the UI renderer, SSH,
Windows terminal rendering, physical display latency and model inference. RSS is
sampled, not a continuous peak. Ping timing is not input-to-visible-character time.

Each scenario also requires exact session identities and metadata, unchanged
saved-file bytes and inode, and advancing output from every connected watcher.
Run `python3 scripts/test_eval_runtime.py target/release/flere` to exercise a valid
fixture plus deliberately changed metadata and store contents. Those failures
must be rejected even when output continues normally.

The default timing loop sends the next ping after the previous reply and a short
pause. To measure independent arrivals, set `--arrival-rate` (requests per second),
`--arrival-seed`, and optionally `--max-inflight`. The arrival report retains every
offered slot, dispatch lateness, timeouts, errors, and missed requests; it never
reschedules them. Latency quantiles cover successful requests only. An arrival
failure makes the CLI exit unsuccessfully after saving the report. RSS in this
mode is sampled at interval endpoints, not after each request. The runtime control
script exercises both timing modes.

Options: `--cards`, `--notes`, `--seconds`, `--repeats`. Use a small board as well as
a note-heavy board when judging overhead. Slow-reader correctness is covered by
Rust partial-frame/coalescing tests; this resource probe uses draining readers.
The existing [PTY-to-rendered-frame probe](performance.md#reproduce-the-run) covers
another local boundary and should not be confused with this one.

## Information-delivery probe

The coordination probe uses real worker assignment storage, exact native run
checks, Unix sockets and the stdio MCP adapter. Its harness executable is a tiny
Python process that prints a readiness marker and waits; it is not an AI model.
Every policy receives equivalent synthetic assignments, messages, background cards
and corrections in a fresh supervisor. Each workflow makes 14 context reads,
including five unchanged reads before handling and five afterward. Savings depend
on that repeated-read workload; this is not a corpus of arbitrary coding tasks.

| Policy | Delivery behavior |
| --- | --- |
| `full` | Repeated `get_context(detail=true)`, followed by explicit acknowledgements and any needed inbox reads |
| `progressive` | Compact context, bounded whole-message inbox pages, exact acknowledgements and explicit history retrieval |
| `delta-simulation` | Progressive calls with an independently checked receiver-side exact-field delta experiment; its hypothetical bytes are reported separately |
| `incremental` | Actual `get_context(since=...)` responses, reconstructed from the retained base, with full reset after simulated context loss |

Checks include a complete assignment with constraints near its beginning, middle
and end; exact assignment acknowledgement; a later human decision correction;
small, large, escaped and Unicode message bodies; provenance and recipient identity;
arrival between pages and acknowledgements; no skipped or duplicate handling;
empty final inbox; retained historical evidence; and a full read after context loss.

Reports count tool calls, request bytes, response-body bytes, complete MCP response
bytes and elapsed tool-call time including MCP process startup. Initialization and
tool-catalog response sizes and the retained store size are reported separately. The scripts fail on
missing evidence or incorrect reconstruction. The `full` policy is an available
API comparison, not a claim about any particular historical release. The simulated
delta is not measured wire traffic; the `incremental` policy is. Sender preparation
and independent fixture setup are outside recipient workflow totals.

A smaller response can cost more calls. Assess total task traffic and latency, not
only one response. Bytes are not tokens: these evaluations currently use no
model-specific tokenizer. They prove protocol behavior, not that a model remembers,
understands or follows the evidence. A subsequent model evaluation should compare
the same tasks and resource budgets, record actual input/output tokens, and score
constraint recall, correct completion, missed messages, duplicate actions,
compactions and unnecessary human interventions. Do not infer those outcomes from
this deterministic runner's successful assertions.

## Opt-in model evaluation

After obtaining separate authority for native model test launches:

```sh
python3 scripts/eval_agent_context.py target/release/flere \
  "$HOME/.cache/flere/eval-results/model-context" --execute-native
```

The Linux runner executes six synthetic cases sequentially: full, progressive and
incremental retrieval, each with corrections and with forgotten working context.
The proxy selects each retrieval policy and retains incremental revision handles;
this does not test whether an unaided agent chooses or remembers that policy.
It uses the installed Codex default model/provider, an ephemeral conversation and
a read-only native sandbox. It sends synthetic assignments and evidence, without
user chats or repository source. Configured MCP servers are disabled for the
invocation; only an allowlisted proxy to an owned fixture is enabled. It does not
edit the user's configuration or access the live Flere mailbox.

Each case has a 180-second deadline and 24-call budget. The proxy limits forwarded
calls across restarts; an event monitor stops the owned process group on timeout,
unexpected native tools or excess calls, and cleans up surviving proxy children.
This monitor is an evaluation guard, not a security sandbox: a native tool may
start before its event becomes observable. The native read-only sandbox remains
active. A failed or timed-out native run stops the remaining cases. Use a fresh
output directory; prompts, traces, synthetic before/after stores and results stay
private under the home cache.

Checks require the corrected value, exact Unicode evidence, the original
publication constraint, all pending bodies read before their exact ACKs, saved
ACK timestamps, unchanged originals/provenance and unchanged session identities.
The history case must retrieve an already handled result without acknowledging
it again. A notice or summary cannot satisfy body retrieval. Rejected tool calls
are counted even if the model recovers and completes successfully.

The report includes complete MCP response bytes, call counts, elapsed time and
native-reported input/cache/output usage. Usage includes the native harness's
instructions and varying retrieval choices; it is not a tokenizer measurement of
Flere output alone. One case per policy/scenario cannot establish statistical
improvements, production success rates or fewer compactions. The history case
starts without earlier working context; it does not trigger real harness
compaction. Report observed compaction events separately.

```sh
python3 scripts/test_eval_agent_context.py
```

These offline controls launch only Python fixtures. They reject corrupted bodies
or provenance, summary-only reads, premature/duplicate/history ACKs, false ACK
receipts and missing durable writes. They also verify tool-scope and deadline
guards, proxy restart accounting and owned-child cleanup. No model is launched.

## Retained-history probe

```sh
python3 scripts/eval_history.py target/release/flere \
  "$HOME/.cache/flere/eval-results/history.json"
```

This fixture creates 160 acknowledged synthetic messages and 40 decisions, then
repeatedly retrieves **all** exact decision records through MCP. It reports the
number of pages/calls, aggregate bytes and time, and observed store replacements.
It also checks whether history can be read when an owned fixture's persistence
path is temporarily unavailable. Every retained message body, recipient, acknowledgement
and decision is checked after the writes. Background delivery receipt counts are
reported because their timing can change store size without removing history.
Ten checkpoint writes are measured separately;
Linux CPU/RSS samples are taken after each call and are not peak allocation data.

Compare the complete retrieval workflow: paging adds calls and envelope bytes.
The previous unpaged implementation rewrote the whole store during each decision
read; the read-only path should produce zero such replacements. Borrowed store
serialization avoids a redundant copy during actual writes while retaining the
same bytes, atomic write path, size guard and failure behavior. Nothing is archived
or deleted. `--messages`, `--decisions` and `--repeats` select bounded fixture sizes.

When the catalog advertises `include_acknowledged`, the probe also discards its
remembered context and discovers all received message history through inbox
pages, without supplying known IDs. A separate oracle checks exact bodies,
provenance, acknowledgement timestamps, order and completeness. It measures
first-time body surfacing separately from unchanged rereads, verifies that
rereads do not write, and checks that the default inbox remains empty. Use
`--require-message-history` to fail rather than skip this check on an older
binary. This proves API recovery, not a model's ability to choose a retrieval.

## Native inbox ownership probe

```sh
python3 scripts/eval_inbox.py target/release/flere \
  "$HOME/.cache/flere/eval-results/inbox.json" --descriptors 128
```

This Linux probe creates two copied-Python native stand-ins with owned rollout
metadata. One sends two conversation-bound messages; one is acknowledged and
one remains unacknowledged. With DND off, it measures repeated pending-inbox,
received-history, message-status, checkpoint-history and targeted card-list reads
after initial body surfacing. The last two operations pass through notice
attachment, where neither already surfaced record needs another notice. Every inbox response must
match the complete retained records, including conversation provenance and
acknowledgement state. Timed reads must not rewrite the store or change the
live session/run/PID identities. Routine reads must not repeat notices, and the
unacknowledged message must remain in the default inbox.

Use zero and 128 extra descriptors as controls, with identical `--calls` and
`--repeats` across baseline and candidate. Operation order reverses each repeat.
Run binaries sequentially with builds/tests stopped. Timing includes MCP process
startup; CPU and sampled RSS include only the supervisor. This is an ownership
inspection workload, not a model, network or visible typing benchmark. A
separate trace of an owned fixture can count process inspections, but traced
latencies must not be compared with ordinary runs.

Private supervisor diagnostics record stages that take at least 250 ms:
`supervisor-command`, `supervisor-drain`, `supervisor-native-poll` and
`supervisor-persist`, plus `native-spec-write` and `pty-spawn` during startup.
`supervisor-native-round` measures an entire observation round across slices.
`store-file-sync` and `store-directory-sync` distinguish the durable write stages. These entries contain only a static stage name and elapsed
time, within the existing bounded log. Nested entries can locate a pause within
a larger stage; durations alone do not distinguish CPU scheduling from I/O.
Command arguments, message bodies, terminal contents and paths are not recorded
by these timing entries.

## Active-terminal probe

```sh
python3 scripts/eval_activity.py target/release/flere \
  "$HOME/.cache/flere/eval-results/activity.json" --agents 32 --messages 128
```

This Linux probe complements the runtime probe's stopped cards. It creates
multiple owned Python terminals, each with an open synthetic rollout file and a
configurable number of extra descriptors. A private copy of the system Python
interpreter is named `codex` to exercise the existing native discovery path; no
model executable, credentials or real conversation is used.

The probe verifies exact live session/run/PID identities, recognized conversation
IDs and busy/idle flags. Busy terminals redraw a synthetic status and counter at
the requested rate; every terminal must continue advancing. It compares detached
and draining hyperlink-watcher modes, reversing their order on alternate rounds,
and optionally issues rotating MCP context reads. Output and identity checks run
outside the timed section; context calls are part of the measured workload.

Reports include supervisor CPU and sampled RSS, ping latency, context-call latency
and response bytes, output progression, watcher traffic and whether the store
changed. These do not measure child-process resource use, model activity, actual
key input, the UI renderer or Windows/SSH latency. A copied interpreter reproduces
bounded descriptor discovery, not every behavior or resource cost of native Codex.

Use `--activity idle` as a control. `--agents`, `--descriptors`, `--messages`,
`--rate`, `--seconds`, `--repeats` and `--context-interval` select bounded workloads;
a context interval of zero disables MCP reads. Run identical configurations
against baseline and candidate sequentially, after builds/tests have stopped.

For native-observation controls, use `--rate .5 --messages 0 --context-interval 0`
and compare busy/idle runs with zero and 128 extra descriptors. Reports include
relative ping/context-call timestamps and each measured interval's wall-clock start for private
correlation with scoped syscall traces. Tracing changes timing; use traced runs to
count operations, and untraced runs to compare CPU and latency. Quiet watchers may
legitimately receive no new frames; the reader remains interruptible through its
owned socket when the probe closes it.

### Independent arrival sampling

The default activity probe sleeps after each reply. This can hide requests that
would have arrived during a stall and can align samples with periodic polling.
Use independent arrivals for tail-latency investigations:

```sh
python3 -m unittest discover -s scripts -p 'test_eval_arrivals.py'
python3 scripts/eval_activity.py target/release/flere \
  "$HOME/.cache/flere/eval-results/arrivals.json" --agents 32 --descriptors 128 \
  --rate .5 --messages 0 --context-interval 0 --arrival-rate 50 --arrival-seed 19
```

The seed fixes one independently jittered request time within each arrival slot.
Requests do not wait for earlier replies. `--max-inflight` bounds simultaneous
sockets (default 16); there is no waiting queue. Arrivals that exceed the bound or
are over 50 ms late remain in the report as misses. Each dispatched request has a
two-second absolute deadline. The command saves its report and exits unsuccessfully
if any arrival is missed or fails; it never retries those requests silently.

Reports include every scheduled, observed, dispatched and completed timestamp,
inflight count, errors/timeouts, offered/completed rates, client scheduling delay,
and arrival-to-response and dispatch-to-response quantiles. Quantiles describe
successful responses only: always inspect failure/miss counts. RSS in this mode
is sampled at interval endpoints, not during each response. Current arrival mode
requires context reads disabled so a blocking MCP subprocess cannot stop the
scheduler. Attached watcher draining and synthetic terminal output still run.

The sampler controls use a paused private socket responder to prove arrivals
continue during a stall, and test saturation, scheduler delay, timeouts, fragmented
frames and wrong/closed responses. Run identical schedules against each binary
with builds/tests stopped. These are local socket measurements; neither sampler
establishes Windows/SSH or visible typing latency.

### Observation convergence under load

```sh
python3 scripts/eval_observation.py target/release/flere \
  "$HOME/.cache/flere/eval-results/observation.json" --agents 16 --descriptors 128
```

Copied-Python stand-ins switch their actual open metadata files between two
conversations, an ambiguous pair, and no open rollout. The evaluator checks exact
session/run/PID identities, working flags and persisted conversation history while
100 independent requests/sec, snapshot reads and terminal output continue. It
records producer-readiness and convergence times plus every traffic outcome.
Successful completion is evidence for this workload, not a universal time bound.

The supervisor targets two milliseconds and at most eight tabs per observation
slice, services clients/PTYs between slices, and starts rounds about once a second.
A single process inspection, command or durable write can exceed that target.
Conversation updates are collected until the round boundary to avoid a durable
write per slice. Validated close/return/refresh boundaries and process retirement
flush already observed history before identities disappear. Applying an observation
checks the same card, session, run, process and native configuration; observations
never replace fresh ownership proof in coordination operations. The pending round
is disposable across refresh; uninspected tabs are discovered again afterward.

## Regression checks

```sh
cargo test --offline --lib server::clients::tests
cargo test --offline --lib context_delta
cargo test --offline --test live context_budget
```

These cover absent/closed/partially blocked watchers, mixed wire projections,
coalescing without extending a stalled reader's deadline, exact context changes,
cache bounds and eviction, unknown or foreign bases, MCP reconnection, supervisor
refresh with preserved native identities, and full resets after verified native
compaction or conversation changes. The compaction fixture includes pending mail,
acknowledged evidence, a checkpoint and an answered decision; rejected hooks and
other chats remain unaffected. Tests also cover pending summaries versus body
reads, opt-in acknowledged history, conversation isolation, pagination and
acknowledgement persistence failures. Run the repository's complete
offline checks before considering a candidate ready for inspection.

## UI settings under slow disk sync

```sh
python3 scripts/eval_preferences.py target/release/flere \
  "$HOME/.cache/flere/eval-results/preferences.json" --delay-ms 100
```

This Linux probe requires `strace` when `--delay-ms` is nonzero. It traces only its
own disposable supervisor and frontend, delaying each `fsync` by the requested
amount. A Python program receives exact bytes through a real UI and child PTY;
no model is launched. Each sample changes a UI setting and immediately types a
marker. A final change followed by detach checks pending-save ownership, and the
probe verifies every received byte, the final saved setting, and unchanged live
session identities. `--delay-ms 0` runs without tracing. Raw traces and fixture
state are removed on normal completion; the aggregate report stays in home cache.

The measured interval ends when the child receives input, not when a character
is rendered. Injected-delay timings demonstrate behavior under a controlled
stall and include tracing overhead; they are not ordinary disk, Windows, SSH or
physical-display benchmarks. Compare an unchanged baseline and the candidate
sequentially with the same delay and sample count. Record all runs and any failed
assertions. Neither low latency alone nor a successful detach proves persistence;
final saved values and exact input must also pass.

New frontends send bounded settings to a supervisor-owned writer. It keeps one
write in progress and the latest pending value, coalesces intermediate changes,
and skips unchanged saves. Accepted settings are immediately readable by another
frontend while file and directory sync complete in the background. Detach does
not wait for them; supervisor refresh and orderly shutdown do. The existing
`ui.json` format is retained. Failures remain available to the current or next
frontend, and a later save retries them. Abrupt supervisor termination or power
loss before sync can lose pending settings. Messages and workspace state do not
use this queue. A new frontend attached to an older supervisor retains synchronous
saving until the supervisor is refreshed.

## Outer-terminal output pressure

```sh
python3 scripts/eval_output.py target/release/flere \
  "$HOME/.cache/flere/eval-results/output-pressure.json"
```

This Linux probe creates an owned Python child that paints changing text while
recording exact input bytes. It first forwards five markers while draining the
outer UI PTY, then pauses that reader for `--pause-ms` (default 900). Another marker
is typed one third of the way through the pause. The probe reports whether it
reaches the child before output reading resumes, the observed forwarding delay,
and whether the UI was sampled writing to stdout during the pause. It verifies
exact input and unchanged session identities after draining and detaching. The
producer must continue advancing during the pause; an idle fixture fails.

The probe also observes the dedicated writer thread when present, samples UI
RSS and CPU time during the pause, and records the queue's peak admitted bytes
from the owned fixture diagnostics. `--pause-ms` accepts 300–10000 ms for longer
backpressure trials. RSS includes snapshots and allocator retention; it is not a
measurement of queue size alone.

The reader pause is deliberate fault injection. This checks whether output
backpressure also blocks input handling; it does not establish the cause of a
particular user's lag. The timed boundary is receipt by the synthetic child,
not rendered characters, network transit, or the Windows terminal. Wait-state
sampling is best effort and reports unavailable data explicitly. No native model,
existing shell, live card or external application is used.

UI diagnostic entries over 250 ms now include static stage durations for
preferences, polling, input handling, watcher updates, background maintenance,
frames and images. `terminal-write` separately measures output emission; with
queued output it measures the writer thread, not a slice of the UI loop. These
entries stay in the existing bounded private diagnostic log and contain no
keystrokes, terminal text or card metadata. A slow stage alone does not distinguish
CPU work from scheduling, I/O or a blocked subprocess; use owned-process timing
and wait-state evidence before attributing a cause.

### Queued frontend output

The local UI and remote bridge use one ordered writer. Frames, protocol replies,
images and cleanup share it. Rendering and bulk transfers yield until the prior
production turn drains; subsequent screen changes remain dirty and are rendered
against the last admitted frame. Already admitted bytes and partial protocol
packets are never replaced. Input and supervisor snapshots continue processing.
A wake socket resumes rendering without busy polling when output drains.

The queue admits at most 16 MiB of ordinary payload and 1024 ordinary batches,
including the batch blocked in the writer. Attachment teardown has a separate
64 KiB / 16-batch reserve, so a completely full ordinary queue cannot discard
keyboard restoration, graphics deletion or alternate-screen cleanup. Both use
the same ordered writer; ordinary traffic cannot consume the reserve. Rejected
keyboard restores and preview cancellations retain their cleanup state until
teardown. Payloads have exact-size allocations; metadata is bounded by the batch
counts. Ordinary frames and transfer chunks are coalesced before
production. Excess control traffic or a broken transport reports an error rather
than silently losing a reply. HELLO remains synchronous before the output owner
starts. Detach and refresh drain admitted output, graphics cleanup and terminal
restoration in order; a stalled reader can still delay completion of that drain.
Children retain their exact session identities.

The `output::` live regressions check input receipt without draining either a
real outer PTY or the remote pipe, then compare every final native cell with the
reconstructed outer screen. They also check an exact remote failure receipt,
cleanup before the next refresh handshake, complete packets at detach and
unchanged child identities. Unit tests cover partial writes, in-flight byte and
batch accounting, surfaced transport failures, and byte/batch saturation through
real cleanup producers. These are shell fixtures;
no native models or user sessions participate.

## Supervisor durability under delayed disk sync

```sh
python3 scripts/eval_durability.py target/release/flere \
  "$HOME/.cache/flere/eval-results/durability.json" --delay-ms 200 --samples 4
python3 scripts/eval_durability.py target/release/flere \
  "$HOME/.cache/flere/eval-results/durability-cli.json" \
  --mutation-client core --delay-ms 1200 --samples 4
```

This Linux probe uses an owned supervisor and raw-byte Python shell child. It
injects a process-scoped delay on each `fsync` with `strace`, changes synthetic
card notes, then sends unrelated ping and exact-identity input requests after
observing the real pending atomic-write file. `--delay-ms 0` is the control;
values through 1500 ms exercise delays beyond the normal request deadline.
The probe verifies injected syscall records, ongoing child output, the saved
notes and unchanged session identities/order. No request or input is retried.

Setup/verification use a 20-second observation budget because fault injection
also delays fixture creation. The default socket mutation uses eight seconds to
observe its durability acknowledgement. `--mutation-client core` instead invokes
the binary's actual CLI `update_workspace` operation, including its request
deadline, with a 35-second outer observation limit. Unrelated ping/input keep
their two-second budget. Each sample records its actual start time, reply/timeout,
and latency. **Timeout latency is censored**, not a completed response time.
Timeouts and command errors remain in the report and have separate counts.

Schema 3 also records the first observation of exact child input while the
mutation is pending, using 1 ms file polling. This is an upper bound that includes
client-reply handling; compare it with the mutation reply time to distinguish
queue acceptance from delivery during the save. After the mutation replies, the
probe watches any still-missing child bytes for four more seconds and records exact, unobserved or unexpected input. The final report
also compares all requested and received bytes. The saved value is checked at
the mutation reply and again afterward: a lost reply does not prove rollback.
That read verifies file visibility; the held-writer tests below establish the
successful-reply ordering after sync completes.
Failed requests or integrity checks are saved and exit with status 2; they are
measured failures, not successful runs.
This does not establish permanent loss beyond the observation window. Invalid
fixtures (stopped producer or unobserved injection) fail instead of producing a
passing report. Raw synthetic diagnostic details stay in the private cache
report; keep them outside the repository.


The supervisor's workspace save barrier retains synchronous mutation completion
and rollback handling. One scoped writer owns the immutable serialized image;
the calling mutation waits for file sync, rename, and directory sync. During that
wait the reactor services exact-run raw input and literal text, PTY output, ping, and existing
watch displays. Display metadata, layout, and generation stay at the last
published view; terminal cells update only for matching runs and dimensions.
Direct snapshot and coordination reads remain serialized with mutations so they
cannot return speculative or older state in place of a completed command.
Disconnected deferred requests are discarded, never replayed. The active
request's socket counts against the client limit even while it is removed from
the service list. Delivery reservations recheck owner/composer state and input
admitted during their save before starting a native queue helper. Literal text
retains its control-character and native bracketed-paste checks.

The core client keeps a two-second reply budget for input, literal text, and
ping. Other commands have one absolute 30-second reply budget after sending;
partial response progress cannot extend it. Queue admission remains bounded.
A complete request that expires before dispatch is discarded and receives a
definite `Request not started` error. The executing command owns its socket
through the save and replies only after completion. A partial send or missing
final reply reports an uncertain outcome and never reconnects or replays the
command. The wire format is unchanged; older clients retain their own deadlines.

`server::reactor::tests` exercises real sockets and an owned shell PTY while a
controlled writer is held: input reaches the child, watch output advances,
staged metadata stays hidden, and the writer cannot finish before release.
Separate tests cover deferred mutation cancellation, failure rollback, replaced
runs, changed dimensions, snapshot generation ordering, bounded admission, expired
queued requests, and literal-paste activity. `wire::reply_tests` covers absolute
response deadlines and missing replies without automatic reconnection.
No native model is launched. Run these alongside the delayed-syscall probe and
the worker, mailbox, navigation, and refresh integration tests.

This barrier does not make every filesystem operation asynchronous. Serialization,
audit writes, native-launch preparation files, and final shutdown/exec-handoff
writes can still block. A frontend awaiting its own command remains synchronous;
the longer completion budget does not make that caller responsive. A timed-out
in-flight mutation can still have committed, so do not automatically retry it.
The writer is joined before returning, including error paths; a stuck filesystem
can still hold mutation completion indefinitely.


## Fragmented remote input across frontend refresh

`remote::handshake::refresh_preserves_a_partial_old_frame_boundary_without_replaying_its_content`
uses the real stdio bridge and a harmless Python harness fixture. One atomic pipe
write includes the refresh key, a fully buffered stale packet, and part of the
next packet. Its remaining bytes are withheld until the replacement frontend
emits HELLO. The test covers splits inside the length prefix, tag, identity, and
payload, then a clean-boundary refresh. Keys trailing the refresh shortcut inside
the same packet are also discarded. A separate detach test checks that boundary
without restarting the frontend. Fresh capabilities must work immediately;
stale input and notices must not reach the child or display. Session identity and
the existing draft remain unchanged, and subsequent fresh input still works.

Refresh discards full buffered packets and passes only a bounded framing
descriptor through exec: either a missing-byte count or at most three length-prefix
bytes. It carries no payload. The replacement frontend sends HELLO, discards
exactly that old tail, then applies the usual fresh-challenge check and bounded
stale-packet discard. The descriptor belongs to the same PID across exec and is
cleared for the next handoff; other processes ignore an inherited descriptor.
Malformed or truncated tails fail rather than scanning arbitrary input for a
plausible packet. Unit tests cover every split, invalid bounds, EOF, and process
ownership. These checks cover the local bridge's refresh; they do not establish
Windows/network latency or repair a handoff begun by an older implementation.


## Ordinary input with retained stopped tabs

```sh
python3 scripts/eval_input.py target/release/flere \
  "$HOME/.cache/flere/eval-results/input-small.json" --cards 1 --tabs 0
python3 scripts/eval_input.py target/release/flere \
  "$HOME/.cache/flere/eval-results/input-board.json" --cards 64 --tabs 4
python3 scripts/test_eval_input.py target/release/flere
```

This Linux probe cold-starts a synthetic saved board in a disposable home and
explicitly restores only one shell running a raw-byte Python consumer. Background
cards retain their complete stopped-tab layouts and never launch programs. It
compares ping, raw input, and literal text, reversing operation order on alternate
repetitions. `--cards`, `--tabs`, and `--title-bytes` vary retained layout size;
`--samples` and `--repeats` control bounded request counts. `--echo` also sends
received bytes back through the child's PTY and verifies the last token in the
supervisor's terminal capture, exercising output-driven maintenance alongside
ordinary input. Use quiet and echo workloads when interpreting a change.

The probe reports successful request latency, every attempted request's status,
supervisor CPU time, and endpoint RSS samples. Sequential requests measure ordinary
command cost; they do not provide independent-arrival tail latency or visible
character latency. CPU has Linux tick granularity. Child, client, UI, network,
and model resources are excluded. Failed replies remain in the report and are
never retried. All attempted child bytes, session identities, metadata-only input
audit records, the complete saved store, and its inode must remain exact. A failure ends the remaining cases, retains
results, and exits with status 2. Compare the same completed workload and inspect
all failure counts before comparing timings. The evaluator controls require a
valid echo workload to pass, retain one deliberately incorrect acknowledgement,
and reject omitted child bytes even when every input acknowledgement succeeds.
A missing audit record must fail even when replies and child bytes are correct.

Raw input and literal text do not change the saved layout, so their ordinary
command wrapper no longer rebuilds that layout. Structural mutations, background
lifecycle/directory observation, explicit save, refresh, and shutdown retain their
checkpoints. The regression
`ordinary_input_remains_usable_when_an_unrelated_layout_checkpoint_cannot_save`
requires exact child input despite an unrelated failed checkpoint, retains run
and literal-text validation, then verifies that explicit saving still recovers
the pending layout.

Ordinary terminal output also advances the display without rebuilding the saved
layout. PTY retirement, child exit, tab pruning and maintenance changes still
checkpoint; periodic observation retains directory sampling and failed-save
retries. Regression coverage checks output visibility without an incidental save,
recovery of pending layout changes, and PTY closure before child exit, including
closure deferred by the durable writer. Native history and editor-cache retention
checks remain applicable. These changes do not remove synchronous audit writes
or make every UI command asynchronous.

Audit records are formatted into one buffer before appending, avoiding the series
of field-sized writes produced by direct formatting into an unbuffered file.
Append completion and errors remain synchronous; this introduces no background
queue, dropped records, or new durability promise. The owned-shell regression
`audit_failure_rejects_input_before_queueing_and_recovery_keeps_metadata_only`
checks failed audit writes before input is queued, exact encoded fields including
Unicode/control characters, recovery, and accepted/rejected child bytes.


## In-memory snapshot stage probe

```sh
cargo run --offline --release --example eval_snapshot -- 64 24000 100 4 plain
cargo run --offline --release --example eval_snapshot -- 64 24000 100 4 escaped
```

The positional arguments are card count, note bytes per card, iterations per
batch, rounds, and text shape. The bounded probe creates synthetic metadata and
an 80×24 grid in memory, launches no processes or models, and writes no application
state. Redirect JSON reports into a private home-cache directory for comparisons.
It checks complete decoded metadata against the existing legacy projection, then
times snapshot copying, metadata copying, metadata JSON, and full v4/v6 encoding
separately. Batch order reverses each round; results pass through `black_box`.

Stage measurements are separate workloads, not additive attribution of a live
frame. They do not count allocations, predict memory peaks, or measure supervisor,
socket, UI, network, or physical-display latency. Compare identical fixture/batch
settings, retain the executable hash, and verify a change with the watcher probe
as well. Plain and escaped text exercise different JSON costs.

Snapshot metadata is borrowed and serialized directly into its length-prefixed
frame slot. The byte-compatibility regression compares every field against the
previous complete `CardMeta` serialization with recency omitted, across all three
wire versions and changed values. No metadata field, wire version, stored record,
or authorization check is removed; this introduces no persistent serialization
cache.


## Retained-history storage pressure

This is the historical JSON-only capacity reproduction. Run it and its controls
against a retained binary from before the record backend; its direct JSON oracle
does not interpret version-11 manifests. For the current backend use the
[indexed storage growth probe](#indexed-storage-growth) and Rust storage tests.

```sh
python3 scripts/eval_storage.py target/release/flere "$HOME/.cache/flere/evals/storage.json"
python3 scripts/test_eval_storage.py target/release/flere
```

This probe seeds bounded synthetic messages in disposable home-cache state, then
uses actual supervisor commands to test 64 KiB, 4 MiB, 8 MiB minus 4 KiB, and 8 MiB
saved files. All cards stay stopped; no native harness or model is launched.
Initial layout serialization is settled before measuring read-only retrieval.
It checks every retained message through pages, exact decisions/checkpoints,
provenance, pending state, failed-write rollback, and cold-restart recovery.

Each case attempts a progress checkpoint, first human message preview, and explicit
ACK. The default records capacity rejections as observed outcomes: a successful
probe means the evidence is internally consistent, not that these operations
remained available. Use `--require-completion` to exit unsuccessfully **after saving
the report** when preview or ACK is blocked. Repeated `--headroom BYTES` options
select remaining space before mutations. Controls reject corrupted returned
bodies, altered durable evidence, and a fabricated ACK receipt without a write.

The legacy 8 MiB guard covers workspace metadata and coordination together.
Reaching it rejects growing writes, including completion timestamps. Read-only
paging and cold restart remain separate checks; neither proves that new work can
progress. This is a storage-capacity diagnosis, not a retention policy or native
agent delivery/comprehension evaluation. No records are archived or deleted.


### Native handling at the storage bound

This is also a historical JSON-only evaluator and requires a pre-backend binary.
Current native storage behavior is covered by the record-mode Rust integration
tests described below.

```sh
python3 scripts/eval_native_storage.py target/release/flere "$HOME/.cache/flere/evals/native-storage.json"
python3 scripts/test_eval_native_storage.py target/release/flere
```

This companion probe runs two copied-Python stand-ins with discoverable synthetic
conversation metadata. It compares 4 KiB headroom and the exact saved-file bound,
with and without an MCP notice delivered before capacity is exhausted. DND stays
on during measured calls. It checks compact pending-mail discovery, a deduplicated
send, exact body retrieval and retries, durable ACK and retry timestamps, and
handled-history discovery after forgetting message IDs. An independent human read
compares original bodies/provenance even when a native surfacing write cannot fit.
Cold restart must preserve all records and leave every native tab stopped.

The existing `native_surfaced` field can be set by an MCP notice before a body read.
The notice case therefore measures different storage work from an unsurfaced inbox
read; it is not evidence that a model read or handled the body. At the full bound,
recording a new ACK remains unavailable in both cases. Controls reject altered
sender provenance, a changed retry ID and a false successful ACK receipt. Reported
latencies are individual observations including MCP process startup, not tail
percentiles. These fixtures establish protocol behavior, not native model recall
or human approval transfer.


## Indexed storage growth

```sh
python3 scripts/eval_record_storage.py target/release/flere \
  "$HOME/.cache/flere/eval-results/record-storage.json"
cargo test --offline record_store
cargo test --offline storage
```

The Linux probe starts below the actual 8 MiB legacy bound and crosses it with a
supervisor send, invoking production activation. It grows only its stopped,
disposable SQLite fixtures to 600, 2,400 and 7,200 records, then cold-starts the
real supervisor for each size. No saved tab may launch during restart. A Python
stand-in supplies the owned native session; no model is launched. `--records`
accepts increasing unique sizes from 550 through 16,000.

Seven sequential samples per size cover context, send, inbox, exact ACK, unchanged
ACK retry, handled history and card metadata. Checks compare original Unicode
bodies, IDs, pending counts and retry receipts. The report includes binary
provenance, retained original bytes, cold-start time, RSS, response bytes, latency
and Linux process `write_bytes`. It is one run per size, not an independent-arrival
or statistically powered latency study. Filesystem accounting can include
incidental maintenance; it is not physical device wear or power-loss proof.

Growing the stopped database isolates scaling and bypasses API admission for
those seeded records. Separate Rust native stand-in tests cross production
activation and exercise conversation isolation, notices, queue-helper timeouts,
dispatch receipts, retries, draft/native-approval preservation, results/decisions,
refresh and cold restart. Unit tests cover atomic updates, injected commit and
rollback errors, operation-witness reconciliation, process-kill recovery, exact
source retention, corrupted records, failed activation and editor-cache safety.
Counts can scan scoped metadata even though returned bodies and resident working
history are bounded. No Windows/SSH or model-performance claim follows from this
probe.
