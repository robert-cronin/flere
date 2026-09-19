# Coordination storage beyond the working context

The indexed backend is implemented in `src/record_store` and integrated with the
supervisor. Small stores retain the legacy JSON format. The first save that would
exceed its 8 MiB bound activates a private SQLite store; coordination history no
longer shares that combined-file limit with card updates or acknowledgements.
This describes the local implementation, not a completed live deployment.

## Durable records and bounded working context

SQLite uses WAL mode and full synchronous commits through bundled `rusqlite`.
Original message bodies and provenance occupy separate rows from lifecycle fields,
so acknowledging a message does not rewrite its body or all retained history.
The supervisor loads layout and delivery observations at startup. It retrieves
historical bodies only for the current operation and releases them afterward.

| Record family | Storage and retrieval |
| --- | --- |
| Message originals | Stable ID/order, full body/provenance; indexed recipient conversation and sender request identity |
| Message lifecycle | Notice/read markers, delivery attempts and ACK timestamps, separately updated in the same transaction |
| Checkpoints and decisions | Exact text/answers, workspace indexes, stable existing references and bounded history pages |
| Dispatches | Original assignment and user-request context; indexed request/run lifecycle and idempotency |
| Workspaces and saved tabs | Complete metadata, order, selection and exact native conversation IDs, loaded without starting chats |
| Observations | Existing epoch/run validation and bounds, flushed before refresh |

Inbox pages retain the existing limits of eight records by default for agents,
32 maximum, and a 32 KiB soft body budget. An oversized first accepted record is
returned whole. Exact-ID access, conversation scope, native ownership validation,
ACK-only receipts and history cursors remain available. Decision history in record
mode is bounded to 32 per page for humans too; `after` retrieves older pages.
Context and dispatch lookups use bounded selections. Exact counts scan scoped
metadata and are not constant-time; no claim is made that every operation is
independent of history length.

New records have a 128 KiB serialized bound, in addition to application-specific
limits. Previously accepted legacy documents retain their original 8 MiB bound
when imported or updated; chunked storage avoids increasing SQLite's per-value
allocation limit. Layout retains its own 8 MiB bound. Import, transaction payloads,
queries and caches are bounded. Original records, retry identities and operation
witnesses are retained without automatic destructive pruning. Disk capacity still
matters; this change removes the combined JSON limit, not physical storage limits.

Database indexes filter retrieval. Fresh native ownership checks still establish
request authority, and returned originals are checked against their indexed
identities. A peer message, saved request or database row is not an approval
credential. Agents remain peers.

## Transactions and responsiveness

- Results and decision requests save evidence and card status together.
- A conversation message and its deduplication identity commit before submission
  succeeds. Repeating an accepted sender request returns the same record;
  conflicting content fails.
- All requested ACKs and newly surfaced page records commit together. Application
  provenance validation is inside the transaction; a corrupt later record cannot
  leave an earlier ACK committed. Unchanged retries keep timestamps without a
  durable write.
- Dispatch updates retain exact request/run ownership. External process launch
  keeps its existing uncertainty handling; SQLite cannot make a process atomic.
- Failed workspace changes restore the last published state. Observation flush
  failure prevents refresh, preserving the current supervisor and its sessions.

One owned database job runs through the supervisor's durable-write barrier.
Permitted exact-run terminal input, PTY output, ping and existing display watches
continue while it completes. Coordination and structural commands wait; they
cannot observe speculative mutations. Historical reads and metadata counting
are still synchronous. This is not a guarantee that every filesystem operation
or frontend interaction is asynchronous.

A commit error is not treated as proof of rollback. An operation witness records
the exact intent in the same transaction. Reconciliation reopens the same store
and file identity, checks the witness and performs a full durability fence. Only
a verified outcome permits success or continued storage operations. If recovery
cannot establish the outcome, storage-dependent commands and maintenance stop;
terminal I/O and the last published display remain usable. Commands are never
automatically replayed. Recovery requires restoring the same usable store and
restarting the supervisor; an unavailable state is not a successful ACK.

## Activation, compatibility and recovery

The activation manifest uses saved-state version **11**; the internal layout
image and live-refresh format remain version **10**. Build compatibility metadata
advertises saved-state reads from 2 through 11. An older binary that cannot read
version 11 must not be used after activation; retaining its executable alone does
not provide a supported downgrade. No installation or live migration is performed
by the development tests.

Activation validates the exact captured legacy bytes using the existing loader,
imports related records atomically, preserves the original JSON as recovery
evidence, and verifies the imported data. It then applies the triggering save and
publishes the small manifest with file and directory synchronization. The source
must still match before publication. A publication error after rename is resolved
only by verifying and synchronizing that exact marker. Uncertain publication puts
storage out of service rather than assuming the old file survived.

Each preparation has a unique private directory beneath the state directory.
Interrupted, unreferenced preparations remain for inspection and are never
adopted or deleted automatically. The manifest alone selects the active database.
Paths, ownership, regular files and permissions are checked; symlink or arbitrary
manifest paths are rejected. SQLite database, WAL and shared-memory files remain
inside that private directory.

Editor-cache retention resolves version-11 layout using a separate read-only
SQLite transaction under its existing lock/reservation ordering. It never creates
a missing database or schema. Missing or corrupt state fails closed, retaining
cached snapshots rather than deleting potentially referenced data.

## Evidence and limits

Offline Rust tests cover the production activation boundary, complete paginated
history, exact native conversation scope, notices, ACKs and retries, dispatches,
results/decisions, cold restart and refresh. Native tests use harmless stand-ins
through the real supervisor, socket, ownership and hook paths; they do not launch
models. Negative controls cover partial transactions, corrupted provenance/indexes,
source changes, failed marker publication, unresolved outcomes and cache safety.
Process-kill recovery and injected database errors do not prove physical power-loss
behavior or hardware durability.

The [growth evaluation](../evaluations.md#indexed-storage-growth) measures cold
startup, RSS, request time, response bytes and process write accounting beyond the
old combined-file limit. Legacy pressure scripts remain historical reproductions
for the JSON-only implementation. Single-machine Linux results do not establish
macOS/Windows runtime acceptance, visible SSH typing latency, model comprehension,
compaction rates or native approval transfer.
