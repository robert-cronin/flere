[Documentation](README.md) · [CLI reference](reference/cli.md)

# Troubleshooting

## Ctrl+Space does nothing

The outer terminal or OS may consume the key. On macOS, inspect the input-source
switching shortcut in Keyboard settings. Release that binding or configure your
terminal to send NUL for the navigation shortcut. Once navigation is active, the
footer changes and **q** detaches the UI. Command is not Flere's modifier.

## My shell is still running after I closed the UI

That is the detach behavior. Run the same command with the same state path to
reattach. To end a specific tab, use the close confirmation in the UI. To end an
entire explicitly selected instance, `flere --state DIR stop --terminate`
hangs up its terminals and retains workspace metadata.

## Reopening after a reboot shows stopped cards

Flere retains the project metadata and recorded conversation IDs. Shell
processes, unsent input, and scrollback do not survive supervisor or machine
failure. Explicitly start a shell or select the saved native conversation to resume.

## I rebuilt it, but my running instance looks unchanged

Rebuilding does not replace an already-running process. Install the new executable,
then use **Ctrl+Space → Space → Shift+R** or `flere refresh` for the selected
instance. Inspect `flere refresh-status` if the operation is deferred or rejected.
An incompatible image leaves the old supervisor running.

## An agent or message looks stuck

First inspect the native terminal for a draft, approval prompt, or active tool.
Those states can intentionally defer automatic delivery. Read
[message activation and receipts](guides/agents.md#activate-agent-message-delivery).
Refresh cannot add launch-time hooks to an existing native process; any restart or
resume must be explicit and use the correct saved conversation.

## An image does not paste or preview on my Mac

The optional companion's native bitmap workflow currently targets Windows.
Ordinary SSH and the macOS companion do not acquire Windows clipboard/decoder
support. Text input is separate. See [remote compatibility](guides/remote.md).

## Finding diagnostic logs

Connection, resize, preview, refresh, exit, and error events have private diagnostic
logs. They omit terminal output, typed text, clipboard contents, image bytes, and
panic payloads. Each component keeps up to eight files capped at 256 KiB each;
logging failures do not prevent attachment.

| Component | Location |
| --- | --- |
| UI, bridge, supervisor | `STATE/diagnostics/` |
| Windows companion | `%LOCALAPPDATA%\Flere\logs\` |
| Linux/macOS companion | `$XDG_STATE_HOME/flere-connect/diagnostics/` or `~/.local/state/flere-connect/diagnostics/` |

The companion prints its exact log path after disconnect. Note the version, time,
terminal, and action alongside the relevant log when investigating an exit.

## Something feels slow

Separate UI response from the model, network, Git command, or program running in
a tab. The [performance guide](performance.md) explains the measured pipeline,
raw data, and reproduction commands. A light supervisor does not make a remote
model generate tokens faster. Check the active workload and terminal dimensions
when comparing a new sample.

## Folder loading is interrupted or unavailable

Files reads directories in the background and retries interrupted OS calls up to
three times. Focus Files and press **r** to retry manually; **-** still opens the
parent directory. If Documents fails repeatedly in Ghostty, check the terminal's
macOS Files and Folders access. Reopen Ghostty after changing its permissions.
Repeated failures from a plain directory listing outside Flere indicate an
OS or terminal access problem as well as any UI error handling. Detaching
Flere preserves its supervisor-owned sessions.

### Stopped chats after a Mac or supervisor restart

Flere saves every open tab, its order, the selected tab and the selected workspace. Opening the UI after a supervisor or machine restart reopens those tabs: agents use their own recorded harness and exact conversation ID; shells start in their last saved directory; editor tabs reopen their file. Three open chats remain three separate tabs. Supervisor-only startup launches nothing. Reattaching to live tabs reuses them.

**S** starts a new agent with a harness selector only. Flere has no conversation-ID field or saved-conversation chooser. Switch conversations inside the harness. Codex discovers the current ID from a unique rollout descriptor in its owned child tree, including after a conversation switch. Automatic ID discovery for Claude and Copilot is not yet implemented: recorded IDs resume exactly; an unknown ID opens that harness's native resume picker. Flere never selects the latest chat automatically or adds approval-bypass flags.

Shell directories are sampled once per second and before an explicit supervisor stop. Restart does not replay commands, drafts, scrollback or unsaved editor buffers. An explicitly closed tab stays closed. A failed reopen leaves the saved tab available while other tabs continue; **Ctrl+Space, W** retries saved tabs for the selected card. Opening that card also retries. Hover and keyboard preview do not launch anything.

### A project logo disappears when Ghostty redraws

Ghostty can discard uploaded image data when the screen is cleared. Flere reuploads cached project icons after a full redraw; ordinary frames reuse the existing image. A missing logo does not imply a missing GitHub URL: repository-local logos are discovered directly from the checkout.
