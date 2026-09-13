# Remote files, Ports and saved connections

The local companion owns local files, application launches, saved SSH configuration and SSH forwarding children. Flere's remote supervisor owns remote file handles and validates the selected workspace, tab, run and split revision. File bytes never enter a native terminal's input path.

## User flows

- **Upload local file** (Actions, `6`): select a directory in Files. The companion displays one local path form; enter an absolute local file path. Only its basename and size cross SSH before the upload is accepted. The local path is not sent to the remote host.
- **Download selected file** (`7`): select a regular file in Files. The companion shows the local Downloads destination and size; Enter applies and Esc cancels. Existing files are kept.
- **Open selected file locally** (`8`): explicitly download and open an image, HTML report or PDF. The same local prompt names the effect. Executable, shortcut and script extensions are rejected. Ordinary download never opens an application.
- **Ports** (`9`): `a` opens the companion's local port form, accepting `REMOTE [LOCAL]`. The list shows local and remote ports, the workspace label and starting/listening state. Enter explicitly opens an owned listening port in a local browser; `d` stops that companion's SSH forward; `r` refreshes the list. Stopping a forward leaves the remote service running. Detaching closes the companion's forwards.

Local prompts are marked **LOCAL · flere-connect**. They consume fresh local input after the prompt was displayed, suppress remote painting while active, and request a full Flere repaint on dismissal. Pasted Enter, mouse reports, terminal replies and queued input from before the prompt cannot apply an action. Upload paths and forward port numbers are entered locally. A remote packet cannot choose an arbitrary local upload path or launch a local application by itself.

## Saved SSH connections

```text
flere-connect connections save work user@host --remote /home/user/.local/bin/flere --state /home/user/.local/state/flere
flere-connect connections list
flere-connect --connection work
flere-connect reconnect
flere-connect reconnect work
flere-connect connections remove work
```

`--ssh PATH` selects the installed SSH executable. Authentication, host-key checking and SSH aliases remain OpenSSH's responsibility. Saving or listing a connection never starts SSH. Reconnect remembers only the last successfully negotiated connection, not terminal input, drafts, an image paste or a native start command. An attachment reconnects to the retained supervisor sessions.

Unix configuration is `${XDG_CONFIG_HOME:-$HOME/.config}/flere-connect/connections.json`; Windows uses `%LOCALAPPDATA%/flere-connect/connections.json`. The schema is version 1 with at most 64 named entries. Unix directories/files use 0700/0600, writes are locked and atomic, unknown schemas are rejected, and read-only commands do not create missing directories.

## Framing and compatibility

Core emits `flere-remote-v6`. The companion accepts v2 through v6 and advertises `remote-tools-v1` only after a v6 HELLO. Screenshot support remains available with v5. A mixed-version update installs the new companion first, then the new core.

Generic local path attachment is the optional `chat-drop-v1` capability. Old v6
implementations reject unknown capability packets, so a new companion first sends
the existing NOTICE packet with exact printable marker
`[flere capability probe: chat-drop-v1]`. An old core merely displays it. A new
core consumes that marker and replies CAPABILITIES `chat-drop-v1` to that attachment
only. A new companion enables path candidates only after the reply. An old companion
never sends the marker and receives no new packets. Fresh HELLO clears this state;
duplicate acknowledgements are harmless. New endpoints ignore bounded unknown
optional capabilities while retaining strict operation direction/state checks.
Neither discovery message reads a file or authorizes a transfer.

After negotiation, DROP_CONTEXT reports whether the current rendered input context
is eligible for path capture. It is an affordance, not authority, and is sent only
when the boolean changes. Shells, editors and forms keep ordinary path paste.
If a terminal switch races the hint, a separate one-use literal ticket preserves
the original paste only in that captured live terminal. A changed modal context
consumes the raced paste with a notice rather than guessing where to send it.

Every service packet uses the existing bounded length/tag/request-ID framing:

| Tag | Direction | Payload |
| --- | --- | --- |
| 32 REQUEST | UI → companion | Upload destination context, or download basename/size/open intent |
| 33 METADATA | Companion → UI | Locally chosen upload basename and size |
| 34 READY | Both | Empty; upload acceptance and one acknowledgement per completed chunk |
| 35 DATA | Both | Big-endian u64 offset followed by at most 48 KiB |
| 36 END | Both | Empty; declared bytes complete |
| 37 CANCEL | Both | Cancel current request |
| 38 RESULT | Both | Bounded JSON success/message receipt |
| 39 PORTS_REQUEST | UI → companion | list, forward, stop or open intent |
| 40 PORTS_RESULT | Companion → UI | Bounded status and at most 16 owned forwards; ID 0 permits lifecycle updates |
| 49 DROP_OFFER | Companion → UI | Empty; increasing local candidate ID, no local path |
| 50 DROP_TEXT | Companion → UI | Original bounded bracketed paste; bridge transfer ID |
| 51 DROP_RESULT | UI → companion | Refusal for a local candidate ID |
| 52 DROP_CONTEXT | UI → companion | ID 0, exactly one byte 0/1; capture affordance after capability acknowledgement |

After DROP_OFFER the UI acquires a one-use exact native-chat ticket, then sends
REQUEST with `op:attach`, the local offer ID and a passive target label. The local
consent gate accepts it only while holding that matching locally captured path;
the remote host cannot supply a local path. Fresh Attach then uses the existing
METADATA/READY/DATA/END file stream. Paste as text consumes the same ticket through
DROP_TEXT. Cancel is the default. Local input after the captured paste and before
the displayed prompt cannot confirm or submit anything. A source read starts only
after fresh Attach; file bytes never become native terminal escape sequences.

At most one file transfer is active per companion. IDs increase within an attachment and are reset only by a fresh HELLO. Local consent and uncertain file transfers are discarded across refresh. An active v6 companion retains its owned forwards across a core refresh; detach ends them.

## Supervisor worker API

```text
file-begin upload|download EPOCH WID TID RUN REVISION PATH_HEX [SIZE]
file-read TOKEN OFFSET
file-write TOKEN OFFSET DATA_HEX
file-finish TOKEN
file-poll TOKEN
file-cancel TOKEN
attachment-check EPOCH WID TID RUN REVISION
file-begin attach EPOCH WID TID RUN REVISION NAME_HEX SIZE ATTACHMENT_TICKET
attachment-text ATTACHMENT_TICKET BRACKETED_TEXT_HEX
attachment-cancel ATTACHMENT_TICKET
literal-ticket EPOCH WID TID RUN REVISION
literal-paste LITERAL_TICKET BRACKETED_TEXT_HEX
```

Begin and read/write/finish return `{token, pending:true}` after scheduling. Poll returns `{pending:true}` while the worker is occupied, then `{pending:false, data:HEX}` exactly once. The begin result body contains basename and size; read returns a chunk, write acknowledges it, and finish reports completion. Cancel responds immediately and removes the live ticket. Each ticket captures its complete exact origin; a focus-away-and-back or split revision change cannot revive it.

`attachment-check` returns a one-use token without touching the local source or
creating a remote artifact. Both attach-begin and text consume it. This also guards
unsplit focus-away-and-back, where the visible split revision alone is zero.
Attachment workers reserve a private nonce directory in the shared home-cache
`chat-attachments` store: 128 MiB/file, 512 MiB total and 256 retained directories.
Completed artifacts remain for drafts/history; publication never overwrites.
Finalization revalidates a live native Codex session accepting bracketed paste and
queues the retained path once, without Enter. It reports path insertion, not a
promise that the harness recognizes a particular attachment type. No persisted
state or handoff format changes are required.

There are at most four persistent workers, including cancelled workers still waiting for their filesystem. Each has one bounded command and result slot. Opening, stat, reads, writes, flushes, publication and staging cleanup run on the worker, so the supervisor continues draining PTYs. The UI polls jobs from its normal event loop and remains able to cancel. Idle transfer progress expires after 120 seconds.

Uploads are limited to 128 MiB. Directories and files must be real, with no symlink paths. A 0600 staging file is created beside the explicitly selected destination. Ordered chunks cannot exceed the declared length. Completion flushes and validates the staging file, revalidates the exact origin, then starts a no-replace hard-link publication. Unsupported filesystems fail without replacing an existing destination. Downloads retain and recheck source identity, size and modification time throughout transfer.

Commit-start is ordered against supervisor mutations by a short in-memory gate. Workers release that gate before filesystem operations. Cancellation before that point prevents publication. Cancellation after commit-start reports that publication may complete; it does not claim rollback. A cancelled blocked worker keeps its slot until the filesystem responds and cleanup completes. Refresh requests cancel transfers and require cleanup to finish before exec; they report a retryable busy state instead of abandoning staging files across exec.

## Validation and limits

`tests/remote/tools.rs` exercises the real bridge and companion with disposable PTY probes and SSH stand-ins: multi-chunk upload/download, collisions, abort cleanup, changed source/symlink rejection, stale pane ownership, local consent, hostile unsolicited packets, owned loopback forwarding, and reconnect without draft replay. `server::transfers::tests` holds a worker deterministically while the supervisor's real command/drain path exchanges data with a disposable shell, then proves prompt cancellation and eventual cleanup. Companion units cover consent parsing, queued input, ordered chunks, portable names, saved profiles and forwarding lifecycle.

Optional renderer artifacts are written by the live tests when `FLERE_TEST_REMOTE_TOOLS_ARTIFACTS` is set. Tests do not use real models, user clipboard data, browsers, remote hosts or user sessions. Windows compilation is not physical Windows Terminal/OpenSSH runtime acceptance; that acceptance remains a separate recorded limit.
