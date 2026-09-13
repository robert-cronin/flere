# Flere design

The target is Switchyard's useful workflow with a deliberately small, independently owned stack. This prototype does not import or control Switchyard state.

## Process ownership

One executable has UI, supervisor and diagnostic CLI modes. The supervisor is the sole owner of PTY master descriptors and direct shell children. UI clients can disconnect without transferring or losing that ownership. A held file lock prevents two supervisors from owning one state directory. A stale socket may be removed only while holding that lock and after checking its file type and owner.

Each terminal has an in-memory numeric ID plus a random 128-bit run token. Every capture, input and close request resolves both; session IDs alone are insufficient after restart. Workspace IDs, names and directories are persisted using a versioned file written to a new 0600 file, synced, atomically renamed and followed by directory fsync. Workspaces reload in a stopped state. Supervisor-only startup creates no replacement processes. User UI opening restores the saved open-tab layout, order and selection one tab per request. Each agent uses its own exact recorded harness/UUID, or the native picker if unknown; shells use sampled directories and editors use file paths. Epoch checks, exact live-conversation reuse and prior process start identities prevent duplicates. Failed tabs remain pending for explicit retry. No drafts or commands are replayed. Store version 7 accepts 2–7; handoff version 6 accepts 1–6. Downgrade requires the pre-upgrade binary and store backup after supervisor shutdown.

PTY children retain a Flere marker. UI entry points reject nested attachments before any connection or state/bootstrap side effect, while noninteractive diagnostics remain usable. This prevents recursive canvases without treating the marker as authentication.

The native programs own their prompts and approvals. Flere hosts them in an ordinary shell. The socket is available to the same OS user and is not a substitute for human approval. No permission-bypass arguments, synthetic terminal input for message delivery, conversation copying or inherited Switchyard/Orca credentials are added. Optional native activity/inbox hooks retain native trust review.

## UI and rendering

One UI process draws one shared canvas with cards, workspace-local tabs, a terminal and file explorer. There are no replicated sidebar programs per workspace. The supervisor drains all PTYs, even when their workspace is hidden; background output is retained in that terminal's bounded emulator state.

Child output passes through a bounded VT state machine. Only sanitized cells and whitelisted style attributes reach the outer terminal. OSC clipboard commands and arbitrary control strings are consumed rather than replayed. The parser owns primary/alternate grids and bounded history. Unknown sequences are ignored; terminal conformance is a validation milestone, not an assumption. The terminal area has a neutral virtual default foreground/background (#cccccc/#0c0c0c), separate from UI chrome. Read-only OSC 10/11 queries report these exact colors; Flere does not proxy arbitrary child strings to the outer terminal. This palette is stable across attachment changes, so background-aware native apps do not depend on which client attaches first. Outer-terminal theme discovery/custom palette settings remain future work. Dim, italic and strikethrough attributes survive snapshots and refresh; older handoffs default missing attributes to false. Unused snapshot flag bits extend the existing layout without breaking old attached decoders (old UIs omit the new styles until refreshed).

The supervisor polls its Unix listener, connected clients and PTYs. Watch clients receive coalesced state changes, with bounded queues and deadlines; slow readers cannot hold up PTY draining. An idle state does not stream frames. The UI redraws changed rows and batches ordinary input. Every changed display frame hides the cursor before painting and restores it only at the final native location, inside DEC 2026 synchronized output. Unchanged cells/cursor emit nothing; resize clearing uses the same frame. Cleanup ends synchronization and restores the outer terminal. Synchronous navigation refreshes its exact target before processing subsequent typed bytes; older queued watch generations cannot undo that selection. No language-level performance claim substitutes for measurements.

## Platform and dependencies

All unsafe code is isolated in `src/os.rs`. Linux x86_64 and macOS arm64/x86_64 have platform backends. Apple Silicon runtime validation and Intel compile-check evidence are recorded in MACOS.md. Platform bindings use libc where it replaces duplicated ABI definitions. The system calls are exposed through the C runtime provided by the OS or by Rust's musl target. The static build includes that runtime and Rust std in a single executable. Small infrastructure crates provide serialization, Unicode widths, argv parsing and audited OS definitions. Flere owns its UI and explorer; no tmux, UI toolkit or Oil. Git and the configured editor remain external tools.

External shells and explicitly invoked coding harnesses remain external. Rebuilding a shell, model client, network stack, font renderer or OS is outside the desired feature set.

## Current implementation and next milestones

Workspace-local editor and native tabs, shared layout/workflow state, project board, Git status/worktrees and scoped local coordination are implemented. See the [validation record](docs/releases/0.3.0-validation.md) and [acceptance checklist](docs/acceptance.md) for evidence and gaps.

Next acceptance milestones are native harness trials, physical Windows/SSH input and image workflow, and terminal conformance against the actual applications used. Codex message wake-up, optional native hooks and DND use the delivery lifecycle below; actual native acceptance and other harness transports remain pending. Flere does not import or control Switchyard state.

## Workspace and refresh lifecycle

Workflow metadata uses a versioned JSON store with atomic replacement and directory sync; v1 metadata remains readable. Status, pin/project/issue/PR fields are independent of process liveness. New Git worktrees are explicit local operations, performed outside the PTY loop against a resolved commit. Failed/interrupted checkout paths remain inspectable and never trigger an automatic replacement or native launch.

File tabs launch the configured editor with literal argv and canonical paths. Reopening an existing file selects its live editor buffer. Tabs disappear only after the child is reaped and the PTY reaches EOF. Native tabs use a small host that runs the exact native argv then execs the default interactive shell on exit. Native trust is unchanged. UI attachment may reopen saved tabs; explicit Start agent opens a fresh native chat. Native resume uses exact recorded IDs, with a native picker when unknown and no prompt dump. Codex UUID discovery examines metadata on open rollout descriptors in the owned descendant tree; it never chooses the newest global session.

New supervisors can refresh through a same-PID exec. The private handoff contains descriptor numbers, verified child process starts/statuses, PTY/emulator/input state and live socket/client buffers. The candidate binary parses and validates the exact image before descriptor inheritance/exec. On preflight or exec rejection the old supervisor continues. On success the replacement adopts each descriptor exactly once, restores CLOEXEC, retains the lock, reaps the same direct children via waitpid and removes the handoff. Unsupported older supervisor formats fail safely before adoption. A handoff format change must preserve compatibility or reject preflight safely.

Working-card animation is a display observation: a bounded live Codex status/composer pattern and a matching owned native process are both required. Idle/approval/unknown observations do not animate. Animation never authorizes input, workflow transitions or message acknowledgement.

## Coordination and visible context

First opening creates one ordinary shell workspace. All live agents have the same coordination tools; pinning controls presentation only. Existing names, directories, conversation IDs, pins and native runs are preserved. The serialized `lead` boolean is retained as inert compatibility data (`legacy_lead` internally), with no bootstrap, unique identity or privilege. Old `lead` commands return migration guidance. Messages use exact workspace IDs or `user`, not a role alias.

Workspace metadata and local messages, decisions and checkpoints share one atomic versioned JSON image. Saved messages are distinct from exact returned/surfaced messages and explicit acknowledgements. Human preview opening records only the selected message, native inbox records only the bounded returned batch, and neither treats a read as handling. Bounded context prioritizes pending work before acknowledged or answered history, and includes only the workspace's recent checkpoints.

Human decisions are shown as wrapped, scrollable text before the answer form. Question/recommendation fields are read-only. Agents request review rather than setting Done or answering human decisions. Native MCP resolves exact live session/run scope at the supervisor; this is an ownership contract, not an OS sandbox against programs sharing the account. No saved message is equated with a native wake-up.

## Matching selection and presentation

Board rendering, mouse hits and keyboard movement use the same project-filtered list and selected-card viewport offset. Narrow pickers always keep their selected result visible, including a compact form on eight-row terminals. Each saved tab retains its harness, conversation and directory together. The UI has no conversation-ID field; Start agent selects only the harness. Stored history alone does not imply an open tab, and preview navigation does not launch programs.

## Text selection and copying

Selection belongs to the shared UI, not a child process or the supervisor. A
left-button drag inside the terminal viewport captures the displayed sanitized
cells and highlights a linear range. Wide/combining characters are preserved.
While dragging, the UI displays those original cells; native output is still
drained normally. Workspace/epoch/run or viewport changes cancel the selection.
A plain click does not copy. Mouse release generates one bounded (64 KiB) OSC 52
clipboard write containing base64-encoded text, never a child-provided escape
sequence. This explicit user action is the only clipboard write path; child OSC
52 remains discarded. The UI never queries the clipboard, submits native input
or assumes the outer terminal accepted the write. Escape clears a selection
locally. The base64 crate is infrastructure only; selection/rendering stay custom.

## Terminal scrollback

Wheel decoding is separate from native key input. Primary-screen regions whose
top is row zero retain the lines they scroll away, including Codex's partial
region above its composer. Lower subregions and alternate-screen output do not
enter global history. Up to 100,000 rows or 32 MiB of compact row payload are
retained per terminal, whichever limit is reached first. Default blank padding is
omitted. Each immutable row packs adjacent equal-style cells into runs; cells
retain their exact UTF-8 text, widths and styles. Only requested pages decode back
to cells. Refresh clones share row storage through Arc and serialize packed rows
as bounded base64 values. Old plain-string and cell-array history images convert
on load; counters are recomputed from validated data. Old binaries reject packed
history during preflight. None of this adds a disk history journal or process
restart recovery. The row byte budget excludes allocation metadata and live grids.

Recent capture examines only the current grid plus at most 200 history rows,
regardless of the total retention size. Shift+Home from the terminal, or Home in
history, reads the oldest retained page directly instead of issuing thousands of
wheel requests. The existing wire page format stays unchanged.

Codex resume can initially load just recent conversation items; older pages are
retrieved by its own transcript UI (default Ctrl+T, then Home). Flere neither
manufactures missing rows nor automatically sends native keys when terminal
history ends. Retention expansion cannot restore output an earlier build discarded
or that Codex has not emitted.

The UI fetches one bounded styled page on demand using exact session/run tokens;
watch snapshots and ordinary capture framing remain unchanged. Logical row
anchors prevent incoming output from shifting the requested history. The
currently displayed page stays fixed until scrolling again or returning live;
evicted anchors clamp to retained history on the next request. Width changes
clip/pad old rows and repair wide-cell boundaries; there is no history reflow.
Selection uses this displayed page, while the supervisor continues draining PTYs.
Geometry, epoch or selected workspace/tab/run changes discard the view.

Unmodified wheel input can be translated to arrows only when the child explicitly
enables DEC 1007 and uses the alternate screen (Codex's transcript view). This
separate audited operation rechecks selected tab, exact run, dimensions, process
liveness and input capacity. It preserves application-cursor mode. Primary-screen
scrolling, Shift-wheel and keyboard history paging never synthesize native keys.
Adjacent wheel events coalesce with a bounded delta; scrollback is requested on
demand and is not added to every live watch frame. General native mouse reporting,
physical Windows/SSH forwarding and history reflow remain unverified/unimplemented
as tracked in the [acceptance checklist](docs/acceptance.md).

## Explicit agent dispatch

Card creation, worktree preparation and shell creation are distinct from worker
dispatch. For an implementation request, an agent prepares an immutable assignment,
starts its exact dispatch when authorized, and checks native startup and assignment
handling. Requests for cards alone stop at preparation. Do not invent another
permission gate for work already authorized; a native approval refusal remains
binding and must be recorded and reported without retrying through another path.

`prepare_worker` records the exact epoch, workspace, canonical directory and its
device/inode, issue/branch/base metadata, native argv and the user's actual request.
That request is context, not an approval credential. `start_worker` rechecks these
values and saves a reservation before spawning. The supervisor serializes these
operations; retries of any attempted ID return the recorded outcome, never another
spawn. An uncertain reservation survives restart and prevents automatic replacement.
Failures to persist reservation prevent launch. Configuration or target changes
require inspecting and cancelling the unlaunched preparation before preparing anew.

The host reports native spawn success/exit separately from PTY creation. Assignment
surfacing is recorded only when context is returned to its exact live session/run;
acknowledgement requires that same run and a prior surfacing receipt. Other chats
in the workspace can read retained context, including after manual exact-UUID
resume, without acknowledging another run's delivery. A receipt does not establish
implementation progress or acceptance; checkpoints and review remain separate.
Fresh dispatched Codex receives a short instruction to fetch the assignment and
inbox. Manual native start/resume continues to have no appended prompt. Dispatch
never changes human focus or an existing shell draft.

The assignment and dispatch lifecycle share the atomic workspace store. Version 4
is emitted only after dispatch records exist; older stores remain readable. Such
states require refresh handoff v2, so a downgrade that would discard dispatch
identity is rejected during preflight while the current supervisor keeps running.
Nothing launches on restart or navigation. `agent-call` provides the same exact-run
operations for older loaded MCP catalogs; it cannot fall back to human scope or
replace missing identity. Same-user socket access remains an ownership contract,
not a security boundary against other processes under the same OS account.


## Native mailbox delivery

The supervisor owns a fair bounded delivery loop independent of agent tool calls.
Messages retain saved, any-reader surfacing, native surfacing and acknowledgement
separately. A human preview cannot suppress attention to the intended agent.
Transport receipts record waiting/activation, hook leases, queue reservation and
queue success/unknown independently from handling. Returned context is not proof
of model action. Metadata v5 and refresh handoff v3 are required only once delivery
records exist; earlier formats load, incompatible downgrades reject before exec.

Optional non-managed Codex hooks observe the main turn and emit bounded exact-ID
notices after tools. Stop may continue once for interrupt-intent mail, guarded by
stop_hook_active. Interrupt observes idle state but never prevents interruption.
Compaction preserves main-turn identity. Hooks carry no message bodies or tool
output and make no permission decisions. The helper confirms an emission lease
after flushing JSON; failure leaves unconfirmed preparation, not acknowledgement.
Ordinary Flere tool responses can include notices for older loaded MCP clients.
Native hook configuration is additive through Codex's normal layer merge/trust
flow. Neither installing nor refreshing retrofits an existing native launch.

Automatic idle delivery requires one live native tab in the recipient workspace,
exact owned host/native PID/start, executable/cwd, inherited run identity and one
open CLI rollout matching the UUID. Bounded process/descriptor scans reject
incomplete or ambiguous proof. A recent observed idle lifecycle boundary must
settle, and the composer must remain empty. The guard recognizes native dim
placeholder styling with trailing single-dot Braille idle animation; literal and
multiline drafts, active/dialog/alternate-screen shapes defer delivery. This is
conservative and version-specific, with uncertain shapes left for human attention.
Run-bound DND also defers hooks, ordinary response notices and idle queueing.

Before spawning a native queue helper, exact proof, pending acknowledgement and
DND are rechecked and a reservation is durably persisted. Four helper processes
maximum are polled and timed out without blocking PTY draining; this bounds
transport subprocesses, not workers. Fair scheduling prevents inactive recipients
from starving others. The helper uses target-native configuration paths and an
owned/private cache/run/tmp tree, rejects symlinks and receives no inherited
Flere/Switchyard/Orca run identity. No PTY input is synthesized. Native queue
acceptance is distinct from surfacing, and uncertain/attempted handoffs never
replay automatically, including across restart. Refresh defers while helpers are
in flight; shutdown reaps only its own helpers. Stopped chats are never launched.

## Worker preparation and activity presentation

Any agent’s `prepare_workspace` creates a stopped workspace or asynchronous Git worktree,
preserving human focus. Dedicated `new-stopped`/`worktree-stopped` wire commands
also back CLI `--no-shell`, so old supervisors fail explicitly. Ordinary human
creation still starts a shell. Successful worker dispatch creates the single
native-host terminal, which returns to the default shell on exit. Existing
terminals are never silently reused or removed; workspace creation is not
idempotent and callers inspect retained IDs after an uncertain reply.

The same observed working count drives sidebar cards, board cards and native tab
icons. A blue badge and a bright moving trail occupy fixed existing cells, with
no moving card geometry or added service calls per frame. Idle cards and workflow
headings are static. Reduced motion retains a static activity badge. This
observation has no effect on workflow status or delivery authorization.

## Agent card metadata updates (0.2.13)

`update_workspace` extends the existing exact-run path for name/project/pin
with status/notes/issue/pr. It requires the exact live caller session/run, an
unarchived native caller, an unarchived target and the current supervisor epoch.
The new reconciliation fields require `expected: {name, meta}` copied from a
fresh `list_workspaces` entry. The name and complete serialized metadata must
still equal the current target before mutation. Raw JSON comparison avoids
CardMeta's deserialization defaults allowing a partial expectation to match.
Any metadata difference (including worker status, human notes, branch or saved
conversations) rejects an obsolete read in the same epoch. Output/focus is not
part of the comparison. Identical values match even after intervening edits;
this is a value comparison, not a historical revision counter.

For compatibility, name/project/pin-only calls may omit the comparison, retaining
their previous partial-update behavior. Every supplied expectation is validated.
A mixed edit uses the same guard and cannot use a legacy field to bypass it.
Only the four existing agent-writable statuses are accepted; Done is rejected
for this operation even through the human wrapper. Empty project/notes/issue/pr
strings clear those fields; omission preserves them. Nulls, unknown fields,
invalid types and malformed expectations fail before saving. Existing metadata
limits apply; the unchanged 64 KiB decoded argument bound also includes the
expectation and therefore bounds large combined edits.

The supervisor prepares the full candidate before mutation and writes it through
one existing atomic workspace-store save. Failure restores the old name and
metadata in memory; notification follows successful persistence. Audit records
identify a request, not an accepted candidate. No schema/version migration or
workflow enum was introduced. Focus, tabs, drafts, roles, archive state,
directories, Git identity, conversations and dispatches are preserved. Pinning has no effect on agent permissions. Archived targets require separate human restoration first.
A deferred/waiting note records a settled opt-out without inventing acceptance,
publication authority or a recurring approval gate.

Existing-directory `prepare_workspace` accepts a project as Git worktree
preparation already does. Creation validates and saves that project together with
the stopped card, so an invalid project cannot leave a partially prepared card.
The exact-run `agent-call` CLI supports older loaded native tool catalogs.

## Git history inspection (0.2.10)

The existing Git inspector owns its commit graph, selection and expansion. It
retains one 50-commit page, one expanded commit and one message preview per
workspace. Installed Git supplies topology and decorations from captured exact
local tip IDs; only inert graph glyphs and sanitized text reach the custom canvas.
Paging does not mutate the checkout or index. Explicit refresh captures new tips;
working status refreshes independently. The repository root is retained so paths
work when a workspace starts in a subdirectory.

A separate bounded history worker serializes commands and keeps one pending
request. Hover never replaces an explicit request. Git commands retain the
three-second/512 KiB bounds, disable lazy fetching, use literal pathspecs and
disable external diff/textconv. Full messages and diffs preserve lines/tabs while
neutralizing other terminal controls and bidi controls. Merge changes use the
first parent; root changes use the empty tree. Rename diffs include both paths.

UI actions carry the originating supervisor epoch, workspace, cwd and selection
key. Editor/preview results apply only while that target and Git focus still
match; editor-open also checks the epoch on the supervisor. This adds no persisted
schema or restart behavior. Historical diffs use the normal configured editor
tab path with private read-only content-addressed documents. Reuse verifies
ownership, privacy and exact contents and rejects symlinks or collisions. Cache
documents are individually bounded and retained without eviction for live tabs.

Mouse motion is enabled only while Git inspection is available, using SGR
no-button reports for an optional delayed message preview. Keyboard `?` provides
the full message regardless of mouse support. UI detach restores mouse modes;
physical Windows/SSH behavior remains a separate acceptance task.


## Native comparisons and chrome (0.2.11)

Selected historical/working files use bounded exact before/after snapshots from
Git trees/index and the working file. Unstaged changes take precedence over staged
changes. Root/add/delete cases have an empty side; renames retain the old path;
symlinks read link text rather than following the target. Non-UTF-8/binary,
submodule and unmerged entries direct the user to the unified patch preview.

Pairs have private owned directories and immutable files under the existing diff
cache, with exact content checks on reuse and no eviction of open-tab material.
Configured Vim/Neovim runs with native vertical diff mode, read-only/nonmodifiable
buffers, no swap, disabled modelines, line numbers and unfolded text. Installed
editor configuration supplies syntax colors. The existing editor PTY lifecycle
is reused by an epoch-checked comparison command; no persisted schema changes.
Async workspace/cwd/selection checks remain in force and successful opens focus
the editor. Unified preview remains explicit on `u`.

Chrome uses a dark navy/cyan/magenta palette, slim status strips, square borders
and underlined active tabs. Native terminal colors and hit-target geometry are
unchanged. Real outer UI dimensions are measured read-only for diagnostic captures;
fixture supervisors and editors use isolated home-cache state.


## First-use intro and compact cards (0.2.12)

The ordinary open command shows an intro only when its existing initial snapshot
contains no workspaces, before creating the first shell. After the accepted key
it reads a fresh snapshot, so a window that waited while another completed first
use does not create another initial shell. Cancel/hangup leaves the empty state
available for a retry. Populated reopen, explicit attach, supervisor-only start,
refresh and native lifecycle paths retain their existing behavior. No onboarding
marker, metadata migration or new startup authority is introduced.

The intro is a separate presentation loop over the existing canvas, renderer,
input decoder and terminal guards. It assembles the wordmark then animates at
10 frames per second; reduced motion draws one static ready frame. Start input
is consumed locally; bracketed paste and pointer/focus reports do not dismiss it.
Resize redraws within the existing UI bounds. The standalone intro command is a
preview with no socket commands, state writes or child launches. The existing
nested-UI guard applies to it too.

Sidebar cards now occupy three rows: title, detail, one spacing/activity row.
Rendering, scroll offset and mouse hit testing use that same footprint. Board
cards already use the same spacing.

## Keyboard terminal scroll in navigation (0.2.14)

Terminal-focused NAV consumes Ctrl+U/D (half viewport), Ctrl+Y/E (one row),
PageUp/Down (viewport minus one row), and Home/End (oldest/live). It uses the
existing per-attachment scrollback request with exact session/run identities;
it never calls native wheel translation. Bounds and stable history anchoring
remain owned by the existing scrollback implementation. NAV remains active at
the bottom; leaving NAV with Enter/Escape/Ctrl+Space discards the old page and
consumes that key before native input resumes. Existing pane/tab movement,
preview/form/menu/board handling and ordinary native control keys are preserved.
No supervisor, protocol, persistence or native lifecycle changes are involved.


## Optional SSH companion and image attachments

The foreground companion owns one installed OpenSSH child, local console modes
and deliberate local clipboard reads. `_bridge` owns a remote Flere UI over
SSH stdio. A separate bounded `flere-remote-v2` protocol carries rendered UI,
keys/resize and explicit image transfers. The shared frame is a big-endian u32
length followed by tag/u64 request ID/payload, maximum 65,536 bytes. A fresh nonce
challenge and dimensions bind the handshake; on refresh a bounded stale input
tail is discarded before the new reply. No uncertain input/image is replayed.
The remote process renders emulator cells; native OSC cannot invoke local tools
or cross this boundary as executable terminal output. Host authentication and
trust prompts stay in OpenSSH before local raw-console setup.

Each remote UI owns at most one pending upload. Image request IDs increase within
that connection. The UI obtains a 120-second, single-use supervisor ticket bound
to epoch/workspace/session/run, active selection and live Codex bracketed paste.
Selection changes invalidate tickets, including away-and-back transitions.
Tickets are ephemeral and absent from refresh/restart persistence. The UI also
checks its local focus/forms/previews and cancels at 110 seconds while processing
input. Completion consumes the ticket, validates the private completed PNG path
and atomically queues exactly one bracketed path paste, without Enter. Native
attachment rendering is owned by Codex and requires physical acceptance.

Uploads use a private owned cache, no-follow/create-new 0600 files, an exclusive
nonblocking cache lock, exact offsets, logical size reservation and bounded
aggregate capacity. Finalization fsyncs and links the nonce partial to a unique
PNG, validates signature/IHDR/pixel bounds, unlinks the partial and syncs the
directory. This checks the PNG envelope, not a complete image decode. Cancellation
cleans before sending RESULT; exec explicitly drops pending uploads. A subsequent
exclusive upload removes verified private nonce partials left by abrupt death.
Completed files are retained even after an uncertain native response; never
silently evict a file that a draft/history may reference. No schema migration,
native restart, global DISPLAY, listener or SSH configuration mutation is needed.

Linux clipboard helpers have bounded output, a two-second deadline and owned
child cleanup. Windows borrows clipboard HANDLEs under OpenClipboard, converts
CF_BITMAP/DIB through built-in GDI+, and owns/releases console modes, GDI objects
and the COM stream through RAII. Platform FFI remains in each crate's src/os.rs.
The companion's Windows executable is not a Windows port of the remote workbench.


## Smooth keyboard scroll follow-up (0.2.15)

Terminal NAV uses Ctrl+U/D (also Ctrl+Up/Down) as exactly layout.rows, with no overlap, and plain
u/d as three-row moves. Multirow NAV moves take a first row immediately and then
advance through bounded intermediate frames on a 20ms UI tick. Repeats extend
the remaining destination (bounded by the history row budget); reversal retargets
from the displayed page. Position requests retain absolute history anchors.
The animation owns only transient per-attachment state bound to epoch, workspace,
session/run and layout. Navigation exit, changed focus/target/layout, selections,
forms/menus or bounds cancel pending motion. Reduced motion performs immediate
moves. Ctrl+U/D supersedes the 0.2.14 half-page step. Ctrl+Y/E single rows and PageUp/Down one-row
overlap remain available. The workbench only animates its own retained output;
native input, wheel delegation and the supervisor/persisted schema are unchanged.


## Explicit image display (0.2.16)

Explorer p is a read-only explicit transfer into a full-screen modal owned by
one UI connection. It binds to epoch/workspace/session/run, consumes all modal
input, and streams at most four 48 KiB offset-checked chunks per loop turn after
painting the modal. Source bytes are limited to 20 MiB and PNG/JPEG headers to
20 million pixels before the OS decoder runs. OpenOptions rejects non-regular
files, final symlinks and blocking special-file reads. No preview cache is saved.

The v2 companion advertises this extension only on Windows after primary DA
reports Sixel and CSI 16t supplies bounded virtual cell pixels. Its parser removes
these response forms outside bracketed paste and preserves literal pasted bytes.
Lone Escape retains the existing ambiguity timeout. Windows GDI+ objects, stream
memory and bitmap locks stay in companion/src/os.rs with reverse-order RAII.
The bounded thumbnail is flattened to RGB and encoded by a std-only Sixel writer
with fixed palette, ordered dithering and run compression. Nothing from a child
program is interpreted as a graphics command or copied as a terminal escape.

Text repaint of the modal clears and redraws the full frame before reapplying
cached graphics; unchanged frames emit nothing. Resize invalidates local raster
placement. Close/target change sends a typed CLEAR before restoring the regular
UI. Connection Drop sends CLEAR before DisplayGuard leaves the alternate screen,
including refresh/error paths. The companion clears graphics on its own errors
and disconnect too. A new handshake discards input as before and resets preview
IDs while retaining detected terminal capabilities. Old protocol pairings fail
clearly rather than guessing extension compatibility. No persisted schema changes.


## Card and inspector refinement (0.2.17)

A shared UI-only card renderer keeps the three-row sidebar/board geometry while
wrapping titles, showing project/branch context and separating observed activity
from workflow status. Hidden partial sidebar cards have no mouse target; all
three rows of a rendered board card select that card. No metadata/launch/schema
semantics changed. Status and work signals continue to come from existing
snapshots, with stationary reduced-motion presentation.

Inspector tab labels and hit regions share one layout. Files derive visible
rows from the same geometry used by keyboard paging and mouse selection; the
selected-file panel is inert. Type markers use ordinary Unicode, with no icon
font dependency. Details wrap passive metadata into bounded lines and retain a
transient, clamped per-workspace scroll offset in the attachment; epoch changes
clear it. Drawing performs no new filesystem or socket work. Pane wheel/page
input is consumed locally; the native draft and exact session/run remain intact.

The source stays on flere-remote-v2. The existing 0.2.16 Windows companion is
compatible with this remote UI update; it does not need replacement for these
changes. The user's screenshot attachment is positive evidence for that submitted
Windows/SSH image path, not proof of every graphics/clipboard lifecycle case.


## Card spacing correction (0.2.18)

User screenshot feedback supersedes the compact-card preference: sidebar and board
now leave one blank row between three-row cards. Each card uses a continuous
surface; titles stay prominent and contextual/runtime text is muted, with observed
Working still bright. Shared height/pitch constants define rendering and board
scrolling. Sidebar rows explicitly identify their card and content-row index; a
separate Gap has no target. Clipped cards and spacers remain inert, while all three
rows of fully visible cards select the exact workspace. Keyboard navigation keeps
the active card fully visible, including at the end of a filtered list. A quiet
count in the sidebar border marks cards above or below the visible area.


## Renderer, diagnostics and image opening (0.2.19)

Render non-ASCII cells and the following run at absolute canvas coordinates.
ASCII runs remain batched. This bounds client width disagreement locally instead
of carrying accumulated drift into later panes; no native escape sequences are
passed through. A regression models narrow/wide/zero-width disagreement.

File Enter previews recognized image extensions; Shift+E preserves explicit
editor opening. Unmoved terminal selection releases inspect only frozen visible
cells from the exact epoch/workspace/session/run and layout. At most two bounded
forms reconstruct raw/indented line wraps; candidates are local PNG/JPEG paths,
not hidden OSC targets or executable links. File reads retain O_NONBLOCK,
O_NOFOLLOW, regular-file/type/20-MiB limits and existing modal preview ownership.
Dragging, stale targets and unsupported capabilities do not send native input.

A shared dependency-free logger records lifecycle/error metadata in private
per-process files; no terminal/paste/image payloads or panic payloads. Files are
limited to 256 KiB and eight per component; failures are nonfatal. UI re-exec
records its intent before the existing handshake discards uncertain input.
Companion EOF distinguishes local input from remote output and retains refresh
receipts. No protocol, native lifecycle or persisted workspace schema change.


## Repository image badges (0.2.20)

Flere owns card geometry and repository image selection. A bounded background
worker resolves the Git top-level and checks a fixed list of local PNG/JPEG logo
paths, with `.flere/icon.png` / `.jpg` first. Paths must remain within that
repository; final symlinks, special files, excessive bytes and image dimensions
are rejected. No URL discovery, author lookup, HTTP or external decoder runs in
the UI. Results are read once per attachment, bounded to 128 directories and 64
distinct images (256 KiB and 1024×1024 each). Identical bytes share one image ID.
Project initials remain underneath the two-cell badge; status signals stay separate.

Protocol v3 sends data-only placements after each complete text frame. One visible
image per UI loop is sent in ordered 48 KiB chunks, once per attachment. IDs are
attachment-local numeric `repo-N` keys, never paths or URLs. The companion bounds
one pending image, requires exact offsets/lengths and validates image headers
before native decoding; malformed image data retains initials. Images/encoded
tiles are memory-only and cleared on a new handshake, avoiding stale IDs across
refresh and leaving private repository images off client disk.

Layout changes force repaint to remove old Sixel pixels. Canvas writes invalidate
covered placements; menus/modals hide badges. The companion holds late image data
until a complete current frame, rejects overlapping/offscreen slots, and renders
only current positions. Resize/font changes request a full repaint even at the
same row/column count. Cleanup precedes alternate-screen restoration; badge
painting preserves native cursor visibility. New companions accept v2 without
badge capabilities. No persisted workspace or native lifecycle changes.


## Height-driven icons and sidebar wheel (0.2.21)

Protocol v4 retains bounded project-image transfer and adds capability
`sixel-project-icons-v2` with validated pixel-cell width/height (1–128 each).
Sized layout tag 26 adds each badge's column span (1–32), rejecting overlapping
or offscreen spans. The UI uses source dimensions and cell height to reserve the
logo width plus one column before the title, retaining at least eight title
columns. The companion fits the source into this slot without stretching,
cropping or upscaling. Raster and DCS attributes both specify square pixels.
Same-grid font changes renegotiate geometry and request a complete frame.
The new companion accepts v3 with old fixed slots and v2 without badges; v4
requires a new companion. No persistent state or native lifecycle change.

The sidebar has an optional attachment-local wheel offset. Rendering and click
hit testing use the same clamped offset, including the narrow Cards overlay.
Wheel events are consumed locally without selecting a card, changing focus or
sending native input. Explicit card selection, epoch changes and project filter
changes restore selection-following; ordinary terminal snapshots do not. Fold
and resize bounds apply before drawing or hit testing. Board, modals, previews,
selection dragging and the inspector retain their existing input ownership.


## Interactive inspector mascots (0.2.25)

The user endorsed compact pixel duck, robot and cat prototypes and requested
interactions. The former silhouette is replaced by a checked-in 37-colour,
64x64 palette/RLE asset with 24 frames per action, eight actions per character.
Decoding is bounded by the embedded index; complete frame and mirror tests cover
the asset. Normal builds and runtime need no image library or asset file access.
An optional Python/Pillow authoring script regenerates it deterministically.

The existing eight-row inspector reservation, capability negotiation, output
chunking, synchronization and pixel-region cleanup are retained. A fixed palette
encoder replaces the single-hue encoder and keeps the 640x160 raster and 24 KiB
frame caps. Every frame paints its entire private background. Plain SSH uses
area samples for downscaling into coloured half-blocks; larger pixels remain
crisp. Untrusted native output still passes through the emulator.

NAV Shift+O or a header/sprite click opens attachment-local pet controls. They remember
the prior navigation flag, preserve the existing pane selection, clear UI paste
ownership and consume keyboard/paste/pointer events while focused. Esc, Enter,
q or Ctrl+Space leaves controls and restores the flag. An outside click closes
them before normal pointer routing. Hidden/tiny/overlay layouts dismiss controls.
Selection uses h/l, arrows or 1–3; it is saved through a defaulted pet_kind enum in
existing UI preferences. Unknown future kind names fall back to Duck without
resetting layout. No workspace schema, session/run identity or protocol changes.

Pet, trick and nap reactions have bounded attachment-local duration. Explicit
play takes priority over activity metadata until it finishes. The toy's ball
persists until a workspace/kind change; pursuit ends when it settles. A damped
second-order follower reconnects automatic travel at the current position.
Workspace/kind changes cancel prior reactions and physics without touching cards.

Pointer picking shares the exact raster geometry. Only a cell containing painted
sprite pixels starts its gesture; transparent margins and empty dock cells are
inert. Quick release pets; a 240 ms hold lifts the sprite. The held body uses a
compliant pendulum with gravity and tangential momentum, a smoothed moving
attachment point and a damped ballistic landing. An elastic grip allows contact
constraints to keep the rotated opaque sprite inside the dock. Released sprites
retain linear and angular momentum; ground contact damps them back upright. Rotation is sampled about its
body centre in the same clipped opaque raster. Ball picking takes precedence
where the drawn objects overlap. A recent fast drag supplies throw velocity;
pull-and-hold supplies bounded spring recoil. A dotted tether shows the anchor.
Gravity, restitution, air drag and floor friction settle the ball. Physics uses
height-relative units for isotropic motion and substeps no larger than 1/240 s;
the 40 ms rendering cadence and bounded elapsed time remain unchanged.

A capture owns drag/release outside the dock and consumes unrelated keys, paste
and wheel input. Exact workspace and raster/cell geometry invalidate stale grabs.
Escape cancels and restores prior focus. Chat selections no longer hide the dock;
real overlays, pane resizing and tiny layouts still do. Reduced motion allows
static positioning but freezes simulation and suppresses launch velocity.
No schema/protocol, native input, task execution, extra process or runtime
dependency is added. These are bounded 2D toy dynamics, not a general rigid-body
engine; mouse sampling remains cell-granular and perception depends on the link.


### Terminal interaction refinement (0.2.26)

Selection attaches logical row IDs only after a read-only, exact-session/run
scrollback response matches the pressed visible cells. It freezes cached row
contents and the initial history end, so incoming output does not move the
selection or substitute text. Edge ticks request bounded pages; retention loss,
alternate screens, size/identity changes and cache limits cannot emit native
scroll/input commands. Frozen selections remain subject to the clipboard bound.

Literal path candidates map visible bytes to clicked cells, with bounded scan
lengths and no shell evaluation. Files use the existing current-workspace editor
flow; directories require a default-cancel UI confirmation before existing
new-stopped/metadata commands. Canonical path reuse avoids duplicate cards;
origin checks and native lifecycle/approval rules remain intact. No persisted
schema or new wire message was added. Pixel cell serialization now has its own
validated 1–128 range instead of the terminal-grid dimension clamp.
