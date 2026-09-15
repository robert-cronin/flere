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

**S** starts a new agent with a harness selector only. Flere has no conversation-ID field or saved-conversation chooser. Switch conversations inside the harness. Codex discovers the current ID from a unique rollout descriptor in its owned child tree, including after a conversation switch. Automatic ID discovery for Claude and Copilot is not yet implemented: recorded IDs resume exactly; an unknown ID opens that harness's native resume picker. Flere never selects the latest chat automatically or adds approval-bypass flags.

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
**surfaced to agent** means a notice/context was returned to that agent's tool or
emitted hook; **acknowledged** means the recipient explicitly recorded handling.
A human preview has its own distinction and cannot suppress agent delivery.
Neither queued nor surfaced proves the model understood or acted on a message.
Unknown queue outcomes require inspection and are never automatically replayed.
The UI message picker shows these states. Native queue helpers are bounded and
run outside the PTY loop, using the recipient's native configuration and private
home-state caches.

Hook definitions follow [official Codex hooks](https://developers.openai.com/codex/hooks/).
Codex merges hook sources and requires human review of each non-managed hook's
exact definition. These hooks never answer permissions. Once delivery state is
saved, older binaries reject a downgrade before replacing the supervisor, so
receipts and focus state cannot be silently lost. Physical/native acceptance is
tracked in the [acceptance checklist](../acceptance.md).
