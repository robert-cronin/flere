[Documentation](../README.md) · [Quick start](../getting-started.md) · [Keyboard](../reference/keyboard.md)

# Native agents and message delivery

## Native sessions and refresh

First opening creates one ordinary shell workspace in your current directory. Every agent has the same coordination tools. Pin any workspace with **Ctrl+Space, p** to keep it prominent; names and pins do not grant privileges. You can name a pinned card “Lead” if that suits your workflow.

Existing Lead cards keep their names, directories, chats and pins, and can be unpinned normally. Flere no longer creates or assigns a Lead role. Address messages to an exact workspace ID (from `list_workspaces`) or `user`.

Messages have no broadcast destination, topic categories or channel subscriptions.
The `quiet` and `interrupt` intents control when a notice can draw attention; they
do not change its recipient. Both can wake a verified idle Codex through the
native queue described below.

Start agents explicitly through `S` or the CLI:

```sh
flere --state ~/.local/state/flere list
flere --state ~/.local/state/flere agent WORKSPACE_ID codex EXACT_UUID
```

The optional CLI UUID is an advanced integration interface; omit it for a fresh conversation. Exact resume maps to `codex resume UUID`, `claude --resume UUID`, or `copilot --resume=UUID`.

Flere saves every open tab, its order, the selected tab and the selected workspace. Opening the UI after a supervisor or machine restart reopens those tabs: agents use their own recorded harness and exact conversation ID; shells start in their last saved directory; editor tabs reopen their file. Three open chats remain three separate tabs. Supervisor-only startup launches nothing. Reattaching to live tabs reuses them.

**S** opens a harness selector: use Up/Down, j/k or Tab/Shift+Tab to choose Codex, Claude Code or GitHub Copilot, then Enter; a click also starts the selected harness. Escape cancels. Flere has no conversation-ID field or saved-conversation chooser. Switch conversations inside the harness. Codex discovers the current ID from a unique rollout descriptor in its owned child tree, including after a conversation switch. Automatic ID discovery for Claude and Copilot is not yet implemented: recorded IDs resume exactly; an unknown ID opens that harness's native resume picker. Flere never guesses the globally latest chat or adds approval-bypass flags.

Inside newly opened Flere terminals, plain `codex` and `codex resume` automatically
connect the interactive chat to the same card, including Flere MCP tools and
native paste support. A private executable on that terminal's PATH forwards your
arguments to the real Codex. Each launch gets a fresh run identity; exiting
returns to the existing shell, preserving its variables and jobs. Recognized
management commands such as `codex --version`, `codex exec` and `codex mcp list`
pass through without registering a chat. Your global shell and Codex settings
are unchanged. User aliases are preserved; aliases or commands that name an
absolute Codex executable can bypass this integration. Nested shells inherit
the launcher unless their startup configuration replaces PATH.

When every tab on an existing card has been closed, **Enter** or explicit card
activation reopens its last recorded native conversation. A legacy card with one
saved conversation resumes that exact ID; ambiguous older history opens the
native harness picker. A closed chat with an unknown ID also reopens its recorded harness picker. Passive browsing and merely attaching the UI do not
reopen closed chats. Existing live tabs and pending saved-tab restoration take
precedence. Failed starts retain the card's conversation history for retry.

The launcher becomes available in new terminals, including the shell opened
after a newly launched native chat exits. Refresh preserves existing processes;
it cannot change an already-running shell's PATH or add flags to a running Codex.
Saved-state and refresh handoff version 9 retain the native run and shell's
separate lifecycle identity. Older binaries reject this state instead of
silently dropping its recovery information.


Shell directories are sampled once per second and before an explicit supervisor stop. Restart does not replay commands, drafts, scrollback or unsaved editor buffers. An explicitly closed tab stays closed. A failed reopen leaves the saved tab available while other tabs continue; **Ctrl+Space, W** retries saved tabs for the selected card. Opening that card also retries. Hover and keyboard preview do not launch anything.

After a native process exits, its host execs your default interactive shell in the same PTY. Exiting that shell closes the tab. You may also run a native CLI yourself from any ordinary shell tab.

`Ctrl+Space`, Space, `R` refreshes supervisors and re-execs the UI while preserving child PIDs, exact run identities, PTYs, input queues, terminal/alternate-screen state and active sockets. The supervisor preflights a private handoff before same-PID exec; incompatible images are rejected while the old supervisor keeps running. Pending worktree creation and in-flight native message handoffs must finish first. Refresh is an explicit action and is separate from reboot recovery.

## Activate agent-message delivery

First use **Ctrl+Space → Space → Shift+R** to activate the installed Flere
binary. This preserves every terminal, native PID, conversation and draft.
An existing chat's ordinary Flere tool replies can then contain bounded
`mailbox_notice` message IDs; the current MCP adapter need not be restarted for
those responses. The agent reads its inbox and acknowledges only handled IDs.

For delivery after other tools, or while idle, Codex needs Flere's optional
native hooks. Inspect activation from inside the existing native session:

```sh
flere --state "$FLERE_STATE" agent-call messaging_activation
```

If hooks are configured, review them yourself in Codex's `/hooks`. If the chat
was launched before this feature, explicitly `/exit`, then use Flere's
native harness resume picker to reopen the conversation, and review the
native repository/MCP/hook prompts. New launches include the optional hooks.
Refresh cannot add launch flags to an already-running Codex. No automatic
restart, prompt replay, conversation copy or trust approval occurs.

A normal PostToolUse boundary surfaces up to eight IDs. An interrupt-intent
message may also ask a Stop hook to continue once; it never cancels work. Quiet
messages wait for a normal tool boundary or verified idle state. The supervisor
can deliver to an idle Codex without another tool call, using its native
`codex queue --thread UUID --message NOTICE` path. It requires exactly one live
native recipient, matching process/rollout identity, observed idle hooks and an
empty composer. Drafts, active work, native approval dialogs, ambiguous targets
and DND defer it. No keystroke fallback is automatic.

`set_focus` defers automatic attention for the calling run for up to 1800 seconds,
with a reason; zero clears it. Messages remain saved. `message_status` reports
activation/waiting reasons, queue reservation/result, native surfacing and explicit
acknowledgement independently. `deliver_message` lets the sender or recipient
request the same guarded check with a fresh session/run identity. It cannot
restart a chat, bypass DND or repeat an uncertain native handoff.

A saved message is durable; **queued** means the native queue helper accepted it;
**queue-submitted** means an exact native prompt hook observed that notice as input;
**surfaced to agent** means a notice/context was returned to that agent's tool or
emitted hook; **acknowledged** means the recipient explicitly recorded handling.
A human preview has its own distinction and cannot suppress agent delivery.
Prompt submission does not prove model consumption: another native hook may block
it, or subsequent inbox tools may fail. Submission does not set surfacing or
acknowledgment; later notices still use the normal idle and focus guards.
Neither queued, queue-submitted nor surfaced proves the model understood or acted
on a message. Unknown queue outcomes require inspection and are never automatically
replayed. Manually submitting the complete exact notice counts as the same input
observation; it does not remove an already queued native item or prove queue
consumption. Flere never resubmits that attempt. Old hooks without prompt data
retain the conservative receipt behavior.
The UI message picker shows these states. Native queue helpers are bounded and
run outside the PTY loop, using the recipient's native configuration and private
home-state caches.

Hook definitions follow [official Codex hooks](https://developers.openai.com/codex/hooks/).
Codex merges hook sources and requires human review of each non-managed hook's
exact definition. These hooks never answer permissions. Once delivery state is
saved, older binaries reject a downgrade before replacing the supervisor, so
receipts and focus state cannot be silently lost. Physical/native acceptance is
tracked in the [acceptance checklist](../acceptance.md).

## Send to an existing conversation

Use `send_chat_message` for ordinary agent-to-agent conversation. Read
`list_workspaces` first and supply the recipient's fresh workspace, session and
run identities, a nonempty `request_id` (at most 256 bytes), and `body` (at most
16 KiB). Optional `user_request_ref` (at most 4 KiB) records a source reference as
context; it does not authenticate human permission. The service records the
actual sending agent's workspace, session, run and native conversation.

```json
{
  "workspace": 12,
  "session": 34,
  "run": "<fresh run from list_workspaces>",
  "request_id": "followup-1",
  "body": "Read your inbox and continue the work within your existing assignment.",
  "user_request_ref": "<original request reference, when relevant>"
}
```

The message belongs to the recipient's workspace and verified native conversation.
Each delivery attempt proves the current exact process/run. If that same chat
resumes, pending mail follows it automatically. A different conversation on the
card cannot read, receive or acknowledge it. Multiple live instances of the same
conversation defer delivery. Stopped chats retain mail without being launched by
messaging. The initial session/run remains in the receipt for inspection.

Reuse the same `request_id` when retrying a lost reply. An identical retry from the
same sending workspace/conversation returns the original message and its current status,
including after the sender resumes. A changed body, recipient conversation or
source reference is rejected. A retry may retain the original target arguments
or supply the fresh run of the same recipient conversation. A new request with a
stale target is rejected before saving. Existing `send_message` remains card mail;
its workspace routing and lack of send deduplication are unchanged.

Busy recipients receive notices at a trusted boundary; idle recipients use the
native queue. Only a fixed inbox notice and IDs go into that queue. The body and
agent/source provenance are read through Flere's inbox, so slash commands and
quoted permissions are conversation data. Drafts, DND and native permission
requests remain protected. `message_status` distinguishes saved, queued,
queue-submitted, surfaced and acknowledged, and reports the specific idle
predicate that blocks delivery.
Completed prose mentioning permission/trust and decorative dots around a dim
empty placeholder no longer falsely imply a draft or approval dialog.

A queued or uncertain handoff is never requeued just because a chat restarts.
Inspect its original message ID; the same conversation can still read and
acknowledge it through `inbox`. Recipients must handle an ID only once and retain
operation/checkpoint records where their work has side effects. An acknowledgment
records handling; it does not prove successful work or grant native approval.

Install the new core and explicitly refresh Flere to activate this behavior.
Existing native processes and drafts survive a coordinated supervisor refresh.
The new MCP tool requires a tool-catalog reload through the harness's supported
MCP controls. If it is not yet listed, the existing scoped CLI fallback works
from inside the sending native session:

```sh
flere --state "$FLERE_STATE" agent-call send_chat_message '<JSON arguments>'
```

Native hook activation is still required for automatic attention. Message storage
and refresh handoffs use version 8 so older binaries cannot silently discard the
conversation binding during downgrade. The acceptance tests exercise harmless
native stand-ins; actual native-model handling remains a separate human-run
acceptance check.
