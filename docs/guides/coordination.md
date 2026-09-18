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
those outside that summary page. Use `inbox` for bodies or `message_status` with an
exact ID. `get_context({"detail":true})` restores the fuller view with card notes,
up to eight checkpoints, board summary and activation diagnostics. Human
`coordinate ... context` retains the full view. Assignments, decisions and the
latest checkpoint remain complete, so context has no universal byte cap.

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
messages; `remaining` counts those after this page. Read again without a cursor
to revisit older unhandled messages. Paging is a live view, not a frozen snapshot.

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
flere --state "$FLERE_STATE" agent-call message_status '{"id":"MESSAGE_ID","detail":true}'
```

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
