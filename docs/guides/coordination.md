[Documentation](../README.md) · [Quick start](../getting-started.md) · [Keyboard](../reference/keyboard.md)

# Coordinate work across projects

## Coordination and diagnostics

New supported Codex launches register a task-scoped Flere stdio MCP server through ordinary native configuration. It exposes context, inbox, message, checkpoint, decision request, result submission, workflow status, bounded own-terminal reads and explicitly user-requested focus. Existing chats do not acquire a new MCP catalog just by refreshing the UI. Native repository and MCP trust remain yours to review.

Messages have separate saved, surfaced and acknowledged timestamps. Opening a human message reader or reading the native inbox surfaces returned messages; handling requires explicit acknowledgement. Pending messages/decisions come before old retired records. Agents cannot answer human decisions or set Done; result submission requests review. Codex delivery now uses optional native hooks and an idle queue, with separate transport receipts. Human previews do not count as delivery to an agent. See the [agent-message activation guide](agents.md#activate-agent-message-delivery). MCP registration and automatic delivery for Claude/Copilot remain pending.

The private Unix socket supports bounded live inspection while attached or detached:

```sh
flere --state ~/.local/state/flere list
flere --state ~/.local/state/flere capture --session ID --run TOKEN --lines 60
flere --state ~/.local/state/flere focus WORKSPACE_ID TAB_ID
```

`list` reports exact workspace/session/run/process identities. Capture emits passive JSON and at most 200 recent lines. Explicit diagnostic input separates text from keys, rejects stale/mismatched targets and records target/run/byte count without recording typed text:

```sh
flere --state ~/.local/state/flere send --session ID --run TOKEN --text 'literal text'
flere --state ~/.local/state/flere send --session ID --run TOKEN --key Enter
```

Literal text rejects embedded control keys. Multiline diagnostic text is allowed only for a child advertising bracketed paste. Nothing automatically submits native approvals. The same-user socket is an ownership boundary, not a sandbox against other programs running as your OS user.

`coordinate WORKSPACE_ID OPERATION JSON` provides the human CLI path for local coordination. `close --session ID --run TOKEN --terminate` and `stop --terminate` explicitly hang up the targeted terminal or this instance; detach to preserve sessions. Programs deliberately ignoring hangup can outlive termination.

State defaults to `$XDG_STATE_HOME/flere` or `~/.local/state/flere` in a private directory. Metadata and coordination share an atomic versioned store; UI preferences, run specifications, socket, lock, action receipts and supervisor log are also local. Child temporary files use the Flere home/XDG cache, not system `/tmp`. No Switchyard data is imported or modified, and Flere has no GitHub publication feature.

## Bounded agent context and inbox pages

Agent `get_context` defaults to the current workspace/run identity, complete
assignment, up to 32 decisions (pending first, then recent answers), the latest checkpoint and up to 32 pending message
summaries. Summaries contain IDs and byte counts, not message bodies, and do not
mark bodies as surfaced. `pending_messages` reports all pending messages, including
those outside that summary page; `messages_total` also includes acknowledged
received messages visible to this conversation. Use `inbox` for bodies or
`message_status` with an exact ID. `get_context({"detail":true})` restores the fuller view with card notes,
up to eight checkpoints, board summary and activation diagnostics. Human
`coordinate ... context` retains the full view. Assignments, decisions and the
latest checkpoint remain complete, so context has no universal byte cap.

### Retrieving older decisions and checkpoints

`decisions_total`, `decisions_remaining` and `pending_decisions` make the context
window explicit. `decisions` reads complete decision records in creation order,
eight per page by default (`limit:1..32`) with the same soft 32 KiB record budget
as the inbox. Follow `next_after` using `after`, or pass `id` for one decision,
including an older answer omitted from context. `total` and `pending` describe
the whole workspace; `remaining` describes records beyond this page. An answer
can change between reads, so re-read an exact ID when its current answer matters.
Human `coordinate ... decisions {}` keeps its complete unpaged history view.

`checkpoints_total` and `latest_checkpoint_id` accompany the recent context view.
`checkpoints` retrieves all checkpoint and submitted-result history, oldest first,
with the same page limits. Each entry contains an opaque `id` and the complete
original `checkpoint`. Use that ID with `id` for an exact read or `after` for the
next page. IDs remain stable across appends and supervisor refresh; callers must
not construct or interpret them. These references depend on the preserved,
append-only history, so future archival must retain their meaning.

Both history tools are scoped to the caller's workspace and perform no durable
writes, acknowledgements, acceptance or status changes. A single oversized
record remains whole. `id` cannot be combined with pagination. Pages are live
views; new records can arrive between calls. Reading history still works when
persistence is unavailable. Original records are retained; no pruning is added.

### Optional incremental reads

A compact `get_context` response includes `context_mode:"full"` and an opaque
`context_revision`. If the previous compact view is still in your working
context, pass that revision as `since` on the next call. A matching cached base
returns `context_mode:"delta"`, `base_revision`, a new `context_revision`, exact
changed top-level fields in `changes`, and deleted field names in `removed`.
Replace each changed field in full (including arrays); keep omitted fields from
the matching base. An empty `changes`/`removed` pair means that view is unchanged.
Each delta includes the current epoch, workspace, session, run and conversation
in `identity`. Revisions are retrieval hints, not authorization credentials.

Verified native `PreCompact` and `PostCompact` hooks discard only that exact
run's optional base. The next read returns the complete compact view even if an
old revision is supplied. A changed conversation, workspace or epoch also
requires a full base; a native process can switch chats while keeping its run.

**After lost context or uncertainty about the base, omit `since`.** This remains
necessary when compaction hooks are unavailable. It always returns the complete
compact view. Unknown, superseded, foreign-run or evicted revisions also return
a full view. `detail:true` always returns full
detail and does not use the delta cache. A delta remains a summary read: it does
not surface message bodies or acknowledge handling. Read pending bodies through
`inbox` and acknowledge exact handled IDs as usual.

The supervisor retains at most 64 cached compact views, with a total encoded
payload budget of 2 MiB and at most 128 KiB per view. Oversized views are returned
whole with `context_revision:null`; no extra copy is cached. Cache eviction and
supervisor refresh require a full response next time; they never delete durable
records. Exact content comparison determines changes, without model summaries
or hash-based omission. See the [evaluation guide](../evaluations.md) for the
measured scope and limits.

`list_workspaces` returns up to 32 compact cards by default (maximum `limit:64`),
without notes or saved conversation history. Exact live tab/run identities and
worktree preparation state remain available. Follow `next_after` using `after`
until it is null; `remaining` counts records beyond the returned page. To fetch
one card's complete metadata and dispatch receipts, use
`list_workspaces({"workspace":43,"detail":true})`. Full detail requires a target
workspace; `workspace` and `after` cannot be combined. Compact metadata is not a
valid replacement for complete `expected.meta` in a guarded update.

`inbox` returns up to eight pending messages by default (`limit` accepts 1–32),
with a 32 KiB serialized-record target. Inventory pages use the same byte target.
Records are never cut in half: one oversized message/card is returned whole, so
this is a soft page target plus a small response envelope, not a hard wire limit.
Use `next_after` as `after` to continue; only returned bodies are surfaced by the
inbox read. The cursor remains valid after its message is acknowledged and is
scoped to the recipient conversation. `pending` counts all unacknowledged
messages, regardless of cursor or history selection. `total` counts all records
selected by the call, and `remaining` counts selected records after this page.
Read again without a cursor to revisit older unhandled messages. Paging is a
live view, not a frozen snapshot.

To recover a received message after losing its ID, request
`inbox({"include_acknowledged":true})` and retain that option on each page. This
includes handled records in creation order, using the same count/byte bounds
and exact recipient-conversation scope. The original bodies, provenance and
acknowledgement timestamps remain intact. Reading history does not reopen or
acknowledge work; returned bodies acquire a surfaced timestamp only if needed.
Other conversations' private messages and sent-only history are not included.
The default inbox still returns only unacknowledged messages. Inbox resolves
its current native conversation once at the paging boundary; ownership is
proved again on each call, never retained as an authorization cache.

`inbox({"ack_ids":["HANDLED_ID"]})` returns an acknowledgement receipt and pending
counts without reading another page. Include `limit` or `after` explicitly to
combine acknowledgement with a page read. Acknowledgement is not deletion.
Unchanged reads and repeated acknowledgements do not rewrite the durable store.
Sender responses and checkpoints return compact receipts instead of echoing
bodies. `message_status` includes a body for an unacknowledged recipient read;
use `detail:true` for other permitted bodies and full activation diagnostics.
Exact conversation authorization and request deduplication are unchanged.

Existing chats retain their native tool catalog until refreshed by the harness.
After updating the supervisor, an older catalog can use the same scoped CLI:

```sh
flere --state "$FLERE_STATE" agent-call list_workspaces '{"workspace":43,"detail":true}'
flere --state "$FLERE_STATE" agent-call inbox '{"after":"LAST_RETURNED_ID","limit":8}'
flere --state "$FLERE_STATE" agent-call inbox '{"include_acknowledged":true,"limit":8}'
flere --state "$FLERE_STATE" agent-call message_status '{"id":"MESSAGE_ID","detail":true}'
```

Hook and tool-response notices carry compact identity/ID reminders. Native queue
submissions retain their existing canonical wording so older hook binaries in
already-running chats can still recognize submission receipts. Message bodies
and permission grants never come from a notice. Routine tool replies first check
whether any saved message could need a notice in that workspace. If none does,
no native ownership inspection is needed; otherwise current conversation scope
is proved before selecting IDs. This check is repeated per call, so newly arrived
messages remain discoverable and already surfaced mail is not re-announced.

Records stay in the atomic store, now serialized without formatting whitespace.
This reduces overhead; it adds no automatic record deletion, archival or garbage
collection. See the [research and design rationale](../design/context-engineering.md)
for the separation between durable history and working context.

## Assignments and agent startup

A shell card is preparation. Issue or saved-chat cards with no hosted native tab say
**no agent**; a hosted agent and observed working animation remain distinct.
For a fresh implementation worker, any agent uses this sequence through Flere MCP:

1. `list_workspaces` obtains exact current workspace IDs, directories and epoch.
2. Use `add_project` with `cwd` and optional `name` to create/reuse the main checkout card without launching a shell or moving human focus. For a **new agent card**, use `prepare_workspace` with `name` and `cwd`, or
   `name`, `repository`, `branch`, `base` for a Git worktree. Either accepts an
   optional `project`, saved with the new card.
   Within Git, the directory form reuses project topology: an unregistered main checkout becomes its primary card; subsequent cards get linked worktrees. Non-Git directories retain the low-level loose-card behavior. Preparation preserves human focus and creates a stopped card without an extra shell.
   Retain the returned workspace ID and wait for Git preparation to finish.
   Existing cards keep their tabs.
3. `prepare_worker` saves a bounded assignment and the actual user request, with
   `request_id`, `workspace`, `expected_epoch` and `expected_cwd`. It starts nothing.
4. `start_worker` launches that exact `dispatch_id` when authorized. Repeating the
   ID returns its existing attempt. It preserves the current selected tab/draft.
5. `worker_status` distinguishes host/native startup, assignment surfaced and
   assignment acknowledged. The requesting agent checks these before reporting dispatch ready;
   actual progress still requires a worker checkpoint/result.

Human `new` and `worktree` commands still open the default shell. For agent
preparation from the CLI, add `--no-shell`; then use the normal worker dispatch
steps. The sole agent tab returns to the configured default shell on native exit.
Creating a workspace is not retry-idempotent: if a creation reply is lost, inspect
`list_workspaces` before repeating it. No existing shells are closed automatically.
Older supervisors reject these new preparation commands; refresh first.

Fresh Codex workers get a short instruction to call `get_context`, read their
assignment/inbox and call `ack_assignment`. The brief stays in the private state
store. Dispatch refuses workspaces with existing native work or saved conversations,
so it cannot silently replace a chat already underway.

Reuse an existing live chat through `send_chat_message` when its conversation is
verified. A newly opened blank chat may not have a conversation ID until its first
turn: inspect that exact terminal with `inspect_terminal`, then use authorized
`send_terminal_input` to submit its initial assignment. Preserve drafts and native
trust/approval prompts. This uses the existing chat; it does not require a guessed
UUID, another launch or resume. Input being queued does not prove the assignment
was handled; verify the recipient's context and progress. These routes do not
permit retrying a native approval refusal.

Resume stopped conversations by exact native UUID with no appended context dump.
If the ID is unknown, use the harness picker.

A native approval refusal must be recorded with `report_worker_block` and reported
to the user. The blocked ID never launches. Do not retry through terminal input,
raw sockets, another tool or changed permission settings. A later explicit human
clarification may support a newly reviewed attempt; stored request text does not
grant authority. `cancel_worker` cancels only unlaunched preparation, retaining
its record. An uncertain attempt must be inspected rather than automatically retried.

After **Ctrl+Space → Space → Shift+R**, an existing agent with an older MCP catalog
can use the supported CLI without restarting its chat:

```sh
flere --state "$FLERE_STATE" agent-call get_context
flere --state "$FLERE_STATE" agent-call list_workspaces
```

`agent-call OP JSON` uses the caller's existing `FLERE_SESSION` and
`FLERE_RUN`; never copy another run's identity or substitute human `coordinate`
for a refused agent operation. New native sessions receive the expanded MCP
catalog. Actual Codex assignment handling still needs native acceptance with
ordinary repository/MCP trust; process fixtures do not prove that acceptance.
Once dispatch records exist, older binaries reject refresh/store loading rather
than discarding them. Existing cards can use this flow; recreation is unnecessary.

## Card administration

Any live agent can use `update_workspace` to edit an existing card's `name`, `project`,
`pinned`, `status`, `notes`, `issue` and `pr`. Read targeted
`list_workspaces({"workspace":ID,"detail":true})` first and
supply its `epoch` as `expected_epoch` and the exact target workspace ID.

Updates containing `status`, `notes`, `issue` or `pr` also require
`expected: {"name": CARD_NAME, "meta": COMPLETE_CARD_META}` copied from that
listing. The supervisor compares the full name/metadata before saving. A worker
status change, human note edit, rename or any other metadata difference rejects
the update even within the same epoch. Re-read and reconcile on a conflict;
never automatically replace the expectation and repeat an old decision. This
compares values, so an identical restored value still matches. Terminal output
and focus do not invalidate an otherwise current expectation.

At least one editable field is required. Omitted fields stay unchanged; explicit
empty strings clear `project`, `notes`, `issue` or `pr`. Only `todo`, `in-progress`,
`needs-me` and `waiting` are writable statuses. A user-deferred card can use
`waiting` with a note explaining the settled decision and when to revisit it.
No new workflow state is needed. Legacy name/project/pin-only calls may omit
`expected`; when supplied, it is always checked, including on those calls.

The combined edit validates and saves atomically with rollback on save failure.
Unknown fields, nulls and wrong types are rejected. Name is 1–256 bytes without
controls; project/issue/pr are at most 2048 bytes without controls; notes support
multiline text up to the existing 64 KiB metadata limit. The existing transport
also bounds the entire JSON argument (including `expected`) to 64 KiB; large
combined requests can hit that bound before a field's own limit. Links remain
inert text and are not fetched or used as publication authority.

Any card can be pinned or unpinned. This operation cannot mark Done/accept work, archive, change
role, cwd, branch, base or conversations, operate terminals or alter focus.
Native session/run checks remain mandatory. Existing agents can use their own
scoped CLI after the supervisor is refreshed, without restarting their chat to
acquire a newer MCP catalog. For example, from the live agent's own terminal:

```sh
flere --state "$FLERE_STATE" agent-call list_workspaces
flere --state "$FLERE_STATE" agent-call update_workspace '{"workspace":43,"expected_epoch":"EPOCH_FROM_LIST","name":"Builder","project":"flere","pinned":true}'
```

For a guarded reconciliation, this example selects card 43 from a fresh listing,
builds the exact expectation, then submits a waiting status and next-action note.
Change the target ID and note to the intended, authorized decision:

```sh
update=$(flere --state "$FLERE_STATE" agent-call list_workspaces |
  python3 -c 'import json,sys
listing = json.load(sys.stdin)
card = next(w for w in listing["workspaces"] if w["id"] == int(sys.argv[1]))
print(json.dumps({"workspace":card["id"], "expected_epoch":listing["epoch"],
  "expected":{"name":card["name"], "meta":card["meta"]},
  "status":"waiting", "notes":"Deferred by user; retain candidate until revisited."}))' 43)
flere --state "$FLERE_STATE" agent-call update_workspace "$update"
```

The response includes `epoch` and the updated `workspace` (`id`, `name`, `cwd`,
complete `meta`) for readback. The same operation is available to a human through
`coordinate WORKSPACE_ID update_workspace JSON`; an agent must keep using its
own `agent-call` identity. `coordinate` is not a fallback for a rejected native
request. Supervisors before 0.2.13 reject the new fields explicitly.
