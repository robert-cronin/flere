# Remote development: image paste first

This design record explains the companion’s clipboard and terminal transport.
Current setup is in the [remote guide](docs/guides/remote.md); file transfers,
local applications and ports are covered in the
[remote tools design](docs/design/remote-tools.md). Versioned sections below
record earlier implementation increments, not additional release claims.

## Desired experience

Copy a screenshot on the local machine, paste in the active remote Flere chat,
and see a native image attachment before sending the message. The intended experience resembles the VS Code terminal. Text paste stays
ordinary text. No manual save/upload/path choreography. The first implementation targets a small Windows local companion → OpenSSH →
Linux Flere → native Codex path paste. A Linux companion permits local fixture
validation and can use installed desktop clipboard tools. The physical Windows
Terminal gesture and native attachment display still require acceptance.


## Recommended Flere direction

Use a small optional local companion that owns the SSH attachment and local
clipboard access. Flere stays responsible for the remote workbench, routing
and exact native identities. Let one companion transport also support explicit
file upload/download and local opening of remote artifacts in later increments.

The original adapter was researched against Codex 0.154.0. Its pinned upstream release
source at commit `6b9826e3aa83b1a5947db50f4332cb9c65f1b340` recognizes a pasted image
path as a native attachment: `handle_paste_image_path` and the upstream
`pasting_filepath_attaches_image` test in
[codex-rs/tui/src/bottom_pane/chat_composer.rs](https://github.com/openai/codex/blob/6b9826e3aa83b1a5947db50f4332cb9c65f1b340/codex-rs/tui/src/bottom_pane/chat_composer.rs#L1254).
Those sources/tests were read, not run. That source version does not establish every running native process version. This evidence supports
the private upload → bracketed image-path paste adapter, avoiding any X11/display
service or native process environment change. Flere only queues the paste;
Codex owns turning it into an attachment. Check that attachment before sending.

Flere owns its UI and session identities. The companion must pair with an exact UI attachment, and a paste must
remain bound to the exact workspace/session/run selected when it begins. A tab
switch during a transfer must never deliver to the new chat. Ambiguous multiple
clients, reconnects and interrupted uploads need clear handling. Clipboard reads
must be on demand; remote output must not itself authorize clipboard access.

Plain SSH carries terminal bytes; a local image-capable component is needed for
bitmap clipboard access. Standard OSC 52 text copy is not image transfer. The companion path does not add an X11 service or change native process
environments.

## Other useful features, in priority order

1. Image paste and image/file drop into the active native chat, with an attachment
   indicator, progress, cancel, retry and useful failures.
2. Download a selected explorer file or generated artifact to the local Downloads
   folder; upload files into the selected remote workspace with collision checks.
3. Open remote images, HTML reports and PDFs locally through an explicit action.
4. A Ports pane for user-started dev servers: forward a selected port over SSH,
   show its owner/workspace, and open the local browser.
5. Workspace search across file names and contents, with keyboard jumps to the
   configured editor; useful independently of a local companion.

These capabilities are implemented as explicit user actions in the current
workbench. Their controls do not automatically expose ports, transfer files or
launch native chats.

## Proof before calling image paste ready

Use an isolated fixture image/companion transport first. Cover empty/text/image
clipboard types, large or malformed images, disconnect/reconnect, two attached
clients and active-card changes during transfer. Keep received data private and
bounded and clean up incomplete files without touching retained attachments.
Then verify a screenshot copied on the actual Windows client appears as an
attachment in the intended existing remote conversation and survives the normal
message flow. A Linux mock or successful Windows cross-build is insufficient.


## Implemented connection increment

The optional companion uses one foreground SSH child and no listener or custom
cryptography. The remote `_bridge` command attaches only to an already-running
Flere supervisor. All native output passes through Flere's emulator before
rendering. Clipboard reads occur only on Ctrl+V or empty bracketed paste; regular
text paste is preserved. `--image LOCAL.png` explicitly imports a PNG at connect.
No native restart or Enter is sent. README.md has the connection command and limits.

Images stream in bounded frames to private 0600 nonce files. Exact target tickets,
UI focus, selection changes, size/offset validation and completion checks reject
stale delivery. Cancellation removes partials before acknowledgement. Completed
files remain for drafts/history, within a 256 MiB cache that refuses new uploads
when full. PNG signature/IHDR and pixel bounds are checked, not full decoding.
A new upload holding the exclusive lock cleans verified crash-left nonce partials.
Refresh cancels pending transfers, uses a fresh handshake challenge and discards
queued old input; it never replays an uncertain paste. Detach leaves remote work
alive; running the same command reconnects without restarting native chats.

Windows runtime dependencies are the built-in console/clipboard/GDI+/COM APIs and
installed OpenSSH. Linux's optional clipboard readers are bounded to 20 MiB and
two seconds; no tool is installed by the companion. Application builds remain
offline. Cross-build tooling was provisioned locally using the official Rust
Windows target and checksum-verified LLVM-MinGW import libraries; no downloaded
LLVM executable was run. The executable's imported DLLs are Windows OS libraries.
No Azure or cloud build/storage service was used.

The [validation summary](docs/releases/0.3.0-validation.md) records current
checks. A cross-build and local stand-in tests do not prove the physical
Windows → SSH → Codex gesture.


## Real terminal images (0.2.16)

The user's dependency constraint is met by the first Explorer preview path:
Windows built-in image decoding, a small Rust Sixel encoder, and the existing
foreground companion/OpenSSH connection. Explicit p transfers a selected PNG/JPEG
in memory and opens a keyboard-controlled modal. This advances remote artifact
viewing; upload/download, local application opening and ports remain separate TODOs.
Windows Terminal 1.22+ was the initial display target. The later portable decoder
uses approved cached PNG/JPEG crates on Linux/macOS too, retaining the encoded and
pixel bounds. Any companion host can now enable Sixel previews and badges after
the terminal reports Sixel support and cell geometry. macOS image paste reads
native PNG/TIFF; Linux retains optional installed clipboard readers. The direct
local core uses Kitty graphics (including Ghostty); the companion does not yet
implement Kitty. Physical Windows/current-terminal appearance and complete native
attachment acceptance remain separate in the [acceptance checklist](docs/acceptance.md).

## User-started Tasks and Problems (0.2.26)

**Run Task** in Actions, or **Ctrl+Space, !**, opens a label and single-line command
form for the selected workspace. Tab changes fields, Enter starts the command once,
and Esc cancels. The supervisor starts the configured shell with `-c` in a new owned
PTY in the focused pane. Existing terminal input is not used to launch tasks. This
works on the supervisor host through the existing terminal transport and needs no
new companion protocol.

**Problems**, or **Ctrl+Space, ;**, shows recognized `file:line[:column]` locations
and Rust arrow-style diagnostics from interpreted task output. Tab cycles tasks,
j/k selects a location, and Enter explicitly opens its file with the existing exact
workspace/pane editor jump. Relative paths use the task's launch directory; absolute
compiler paths remain usable. Locations are passive data and never authorize a shell
command. Mutable partial output rows wait for a newline or actual process/PTY end.
Unsupported or wrapped diagnostic formats remain readable in the task terminal.

Up to 64 live or retained tasks and 128 unique locations per task are kept. Parsing
shares a rotating 4096-row budget per supervisor drain. Actual child exit status is
authoritative; terminal status sequences cannot declare task completion. Completed
tabs keep output and Problems until explicitly closed. Same-PID refresh preserves
task metadata with the owned PTYs using handoff version 6. Ordinary saved tabs retain
shell-directory entries for running tasks; a full supervisor restart never replays a
task command, and completed task reports are not persisted.

Disposable real-PTY regressions in `tests/tasks/mod.rs` cover exact split ownership,
sibling drafts/output draining, live and completed refresh, actual exit codes,
stale requests, failed launches, partial diagnostic rows, explicit close, full
restart without command replay, and the form-to-Problems-to-editor UI flow. These
stand-in scripts do not claim physical Windows/SSH acceptance or compatibility with
every compiler's diagnostics.
