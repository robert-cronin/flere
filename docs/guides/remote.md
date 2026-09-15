[Documentation](../README.md) · [Quick start](../getting-started.md) · [Keyboard](../reference/keyboard.md)

# Remote sessions and image workflows

## First connection

With the local companion installed, use an existing OpenSSH alias:

```sh
flere ssh devbox
```

On Unix, the `flere` command delegates to the adjacent installed `flere-connect`.
On Windows, `flere.exe` is the companion launcher; both names select the same
managed worker. The first-use flow identifies the remote platform and reuses a
compatible installation. If Flere is absent, it verifies a prebuilt package,
installs it in `~/.local/bin`, starts the supervisor only, then attaches. SSH
handles its own authentication and host-key prompts. It does not install Rust,
change SSH configuration or open a native chat.

The default public channel must be published before that download can succeed.
For development, provide a verified local **remote-platform core package** with
`--package /path/to/package`, or an explicit HTTPS manifest with `--from-url URL`.
Linux x86_64 GNU and macOS arm64/x86_64 are recognized targets. Windows is the
local companion platform. A custom route remains explicit:

```sh
flere ssh devbox --remote /absolute/path/to/flere --state /absolute/private/state
```

A changed, unknown or incompatible installation is retained and reported for an
explicit update. First installation uses a locked absence guard and cannot turn
into an update if another client installs meanwhile. Reconnect remembers the
resolved absolute command and state. Old Railhand installations remain separate.

## Versions and connection

The current build uses **protocol v6**. Install the current companion before
connecting to a v6 server; older companions cannot attach. The Flere companion
accepts the Flere **v2–v6 wire shapes**, with features limited to that server's capabilities.
Screenshot export needs v5 or newer; coordinated updates and file/port tools need v6.

With the current pair, **Ctrl+Space, Shift+K** opens the companion's local update
form. Enter prepares both packages; review their identities before applying or
cancelling. This requires the guarded Flere update endpoint; an unrecognized peer
is left unchanged. Old Railhand peers require a fresh Flere installation.
See [setup and updates](../getting-started.md#update-from-flere).

## Disconnects under load

A busy terminal or SSH connection can temporarily stop consuming screen updates.
The supervisor allows 30 seconds without output progress before closing a stalled
UI stream. It retains one bounded snapshot per UI and keeps draining the hosted
programs. Losing only the UI connection leaves those programs running; reconnect
with the same command.

The companion waits briefly for SSH's final exit status and reports a failed
connection as an error. An intentional detach remains a successful exit. Its
printed diagnostic path identifies the local log; matching remote logs are under
`STATE/diagnostics/`. A supervisor `client-timeout` entry records whether the
connection was a UI watch and how many bytes were pending, without recording
screen contents or input.

## Copy the whole Flere view

Press **Ctrl+Space, s** to export Flere's full composed frame to the local
clipboard. This includes side panes, tabs, buffer and status bar, using a bundled
font without desktop capture. The Windows, macOS and Linux companions publish
the PNG and a retained local path. Wait for the copied notice, then paste with
your terminal shortcut. A paste of that screenshot's path uploads the exact local
image into the remote chat's composer without submitting it. Plain SSH lacks this
clipboard channel. Linux requires an available X11 or Wayland clipboard; Wayland
requires data-control support. Successfully copied files remain in the companion's
private user cache for later pastes; a 256 MiB bound refuses additional images
instead of deleting files referenced by drafts.

## Image paste over SSH (experimental)

`flere-connect` is an optional local companion for an already-running remote
Flere supervisor. Copy a screenshot and paste while the native Codex input is
focused. The companion reads the local image, transfers it privately through
OpenSSH, then queues the completed image path as a native bracketed paste.
Codex's supported composer turns that path into an image attachment. Check the
attachment before sending; Flere never presses Enter for you.

On your local machine, use the same SSH host alias you already use:

```text
flere-connect HOST --remote /path/to/flere --state /path/to/flere-state
```

Windows uses built-in clipboard and PNG encoding APIs; no Xvfb, Python, Node,
image runtime or remote display is required. The local executable needs installed
OpenSSH (`ssh`). The remote workbench can run on Linux x86_64 or a supported Mac. Ordinary SSH clients
without the companion do not acquire bitmap clipboard support.

Ctrl+V works when the terminal forwards it. The terminal's paste shortcut also
works when it emits an empty bracketed paste for an image, as supported by the
inspected Windows Terminal implementation. Terminals that consume the gesture
without emitting input need a key binding that forwards Ctrl+V. Text bracketed
paste remains ordinary text. The macOS companion reads native PNG/TIFF images
only for a paste gesture. The Linux helper can use an already-installed
`wl-paste` or `xclip`; neither is installed automatically. `--image LOCAL.png`
explicitly imports one local PNG on connection without accessing the clipboard.

Transfers show progress and support Escape cancellation. Changing UI focus, the
selected workspace/session/run or the supervisor epoch cancels delivery. Rejected
or uncertain pastes are never retried automatically. Detach using Ctrl+Space, q;
re-run the same connection command to reattach to retained shells/chats. Remote
refresh remains your explicit action and renegotiates the companion connection.

Images are limited to 20 MiB and 20 million pixels. Remote files are owned/private
under `$XDG_CACHE_HOME/flere/attachments` or `~/.cache/flere/attachments`.
Completed attachments remain for native drafts/history. The 256 MiB cache refuses
new images when full; remove only attachments you know are no longer referenced.
Incomplete files are cleaned on cancellation/disconnect, or by the next upload
following an abrupt process kill. Multiple clients share Flere's existing
selection; one upload holds the cache lock and exact single-use target ticket.

Build the companion natively on Linux or macOS with
`cargo build --offline --release --manifest-path companion/Cargo.toml`.
This implementation has real companion/supervisor/PTY coverage with harmless
native stand-ins, portable decoder checks and a private macOS pasteboard test.
A historical user-supplied screenshot demonstrated one Windows/SSH attachment
path. Complete current-build physical Windows clipboard, preview, sizing, resize
and cleanup acceptance remains open. The [acceptance checklist](../acceptance.md)
separates those checks from compilation and fixture evidence.

## Drop a local image or file into chat

With a current companion and core, dropping or pasting **one absolute local path
as bracketed paste** opens a local choice: **Attach / Paste as text / Cancel**.
Cancel is selected initially. Use arrows or Tab and Enter, or press **a** to attach,
**t** to paste the original text, or **Esc** to cancel. The prompt names the captured
chat and local path. Nothing reads that file before your fresh Attach choice;
queued Enter, pasted button names and mouse reports cannot confirm it.

Terminals often send dropped paths as ordinary text, so Flere asks rather than
assuming every path is a file drop. Unix single/double quotes and backslash escapes,
Windows quoted drive/UNC paths, and PowerShell single-quote/backtick escapes are
supported without shell evaluation. Multiple paths, control characters and
ambiguous syntax cannot authorize an attachment. Multiline/control-bearing pastes
remain ordinary text. Paths must identify one regular file without symlinks.
Shells, editors, forms, unbracketed input and non-path text keep ordinary paste. If the
terminal does not bracket dropped paths, paste a quoted absolute path using its
bracketed-paste shortcut. Physical Windows Terminal drag/drop remains an acceptance
check. Older peers retain literal path paste and do not gain file access.

Attach streams the original file privately, preserving its basename and bytes,
including PNG/JPEG. The completed remote path enters the same live Codex draft as
one bracketed paste **without Enter**. Image appearance and other file handling
depend on Codex and the file type; a generic file is not promised an image badge.
The target is captured when the completed paste opens the named local prompt,
before the fresh Attach choice. Paste as text also targets that captured chat. A changed workspace, session/run,
split, input mode or connection invalidates the choice; switching away and back
does not revive it. Progress appears in Flere and Esc cancels. Retry by dropping
the path again; an uncertain completion is never replayed automatically.

Files are limited to **128 MiB** and retained privately under
`$XDG_CACHE_HOME/flere/chat-attachments` (or the home-cache fallback). A **512 MiB,
256-directory** bound refuses further uploads instead of deleting draft/history
references. Existing files are never replaced. Incomplete transfers are cleaned;
an upload already committed before cancellation may remain, but cannot paste into
a new target. These limits are separate from clipboard-image limits above.

## Image previews

With the **current companion** and remote Flere, use **Ctrl+Space → l**
to focus Files, select a PNG/JPEG, then press **Enter** or **p**. **Shift+E**
explicitly opens the file in the configured editor. Other files and directories
retain their usual Enter behavior. **q**, **Escape**, or **Ctrl+Space** closes the
full-screen image viewer and consumes that key.

Clicking a visible PNG/JPEG path in remote terminal text opens the same preview;
drag-to-copy remains available. Absolute, `~/`, and `./` paths can include spaces
and visible wrapped lines. Truncated paths and hidden link targets cannot be
reconstructed. URLs and OSC links never launch programs.

The companion displays images on **Windows, Linux and macOS** when the outer
terminal reports Sixel support and its pixel cell geometry. Windows Terminal
1.22+ is one display target. Missing capabilities produce a text notice.
Images fit without changing proportions or upscaling,
and resize with the window. Transparency uses the dark Flere background.
The dithered 216-color preview is bounded to 1280×720, with source limits of
20 million pixels and 20 MiB.

Windows decodes PNG/JPEG through built-in GDI+/COM; Linux/macOS use the shared,
bounded portable decoder. Flere's Rust code encodes Sixel. Preview bytes travel
over the existing SSH connection and remain in memory.
Closing, changing the exact target, refreshing, or disconnecting clears the
preview. Modal input is never submitted to a shell or native chat. Native
applications cannot draw arbitrary images through Flere's emulator.

Direct local Flere uses **Kitty graphics**, including Ghostty, with PNG/JPEG
decoding on Linux and macOS; it needs no SSH companion. The companion's display
path is Sixel and does not implement Kitty or iTerm-specific graphics. Plain SSH
has no companion image transport. Cross-builds and local protocol tests do not
establish physical Windows rendering and cleanup behavior.

## Project image badges

Sidebar cards reserve a three-cell logo slot when space permits; board cards use
two cells. Logos fit one text row's height while preserving their aspect ratio,
with workflow and activity symbols kept separate. `.flere/icon.png` or `.flere/icon.jpg`
in the repository root takes priority. Without an override, Flere checks:

```text
logo.png
docs/logo.png
assets/logo.png
assets/icon.png
frontend/public/logo.png
public/logo.png
website/static/img/logo.png
docs/images/logo.png
frontend/public/favicon.png
public/favicon.png
favicon.png
```

SVG-only or differently named logos need the PNG/JPEG override. Files must be
regular files inside the repository, at most 256 KiB and 1024×1024 pixels.
Reconnect after changing a logo. Missing or invalid images retain project initials.

The Windows/Linux/macOS companion renders pixels using its Sixel path after
capability and cell-geometry detection. Images travel
through authenticated SSH; there is no GitHub lookup, login, or image download.
At most 64 distinct images are held in memory per attachment and none are saved
to the client disk. Repeated cards share an image; only visible images transfer.
Resize, font changes, overlays, preview, and detach clear or repaint the current
positions. Terminals without the required Sixel capabilities and ordinary SSH
retain text fallbacks. Direct local Kitty-capable attachments render their own
badges without the companion.
