[Documentation](../README.md) · [Quick start](../getting-started.md) · [Keyboard](../reference/keyboard.md)

# Compatibility and current limits

## Remaining acceptance and compatibility work

Linux x86_64 is the first supported target; macOS arm64/x86_64 has a native port.
[MACOS.md](../../MACOS.md) records Apple Silicon runtime evidence and Intel compile
checks. The [validation record](../releases/0.3.0-validation.md) records Linux runtime
and Windows companion compile evidence. These checks do not establish complete VT compatibility or
all real native Codex/Claude/Copilot workflows.

The optional companion implements [image attachment, preview, and project badges](../guides/remote.md).
The [historical Windows report](../windows-acceptance-20260913.md) distinguishes
limited physical observations from automated checks. Comprehensive current-build
Windows rendering, clipboard, resize and cleanup acceptance remains open.
The current remote protocol is v6. Companion previews and badges use Sixel on
Windows/Linux/macOS after capability and cell-geometry replies; a Kitty-only
terminal does not provide that path. The local core uses Kitty, including Ghostty.
The Unix companion decodes PNG/JPEG portably; macOS reads native PNG/TIFF clipboard
images and Linux uses optional installed desktop helpers. Automatic Codex delivery
is tested with harmless native stand-ins; actual handling and acknowledgement
after hook trust still need acceptance. The [acceptance checklist](../acceptance.md)
records the precise remaining environment checks and retained evidence.

Terminal emulation covers common cursor/erase/insert/delete operations, scroll regions, primary/alternate grids, indexed/RGB colors, dim/italic/strikethrough, UTF-8 widths/combining characters, bracketed paste and basic device replies. The terminal area uses neutral #cccccc text on #0c0c0c; sidebars keep the workbench palette. Bounded OSC 10/11 replies report these exact terminal defaults so Codex can shade messages and the composer. OSC 133 records bounded command boundaries. Current `main` builds also retain bounded OSC 8 HTTP(S) hyperlinks; other OSC/DCS commands stay inert. Font size is controlled by your outer terminal. Codex caches color detection at startup: refresh updates Flere, but an already-running Codex may need your explicit exit/resume of its saved conversation to acquire the new shading. Outer-theme detection/custom terminal palettes are not yet implemented. Full grapheme shaping, history reflow, extended keyboard and native mouse protocols remain incomplete. The active workspace and viewport are shared across attachments.

The explorer currently reads at most 512 entries per directory. History remains in memory, bounded to 100,000 rows or 32 MiB of compact row data per terminal. Refresh retains it; a supervisor crash or machine restart does not. Workspace/session/client buffers are bounded; Git output is limited to 512 KiB. Diagnostic logs retain eight files per component, capped at 256 KiB each; other retained state and caches have their own cleanup policies. These limits and compatibility gaps are tracked rather than hidden behind a full-parity claim.

## Cursor redraws

Flere hides the physical cursor while painting changed rows and restores the
native cursor only at its final position. Frames use synchronized output (DEC
2026) where the outer terminal supports it. Terminals that ignore that mode still
receive cursor hiding before paint; unchanged frames emit no cursor traffic.
This fixes Flere's visible paint-position jumps. Physical Windows/SSH behavior
still needs checking on the user's terminal; it does not claim emulator parity.

## Terminal hyperlinks

Current `main` builds preserve HTTP(S) links through pane rendering, retained
history and supervisor refresh. This is newer than the published v0.3.0 assets.
The outer terminal handles opening a link after its normal explicit gesture;
Flere does not open a browser from child output. With Ghostty, use Command-click
on macOS or Control-click on Linux. Its [link settings](https://ghostty.org/docs/config/reference#link-url)
control URL matching and previews.

Older clients still receive their existing text/style snapshots. Links that an
older emulator already discarded cannot be recovered from their display labels;
the child must emit or redraw the link. File links and custom URL schemes are
outside this OSC 8 support; existing Flere file-selection actions remain available.

Targets are limited to 2 KiB each. Distinct live target storage and per-grid
encoded link tables each have a 64 KiB budget; history annotations count toward
the existing 32 MiB retention limit. The renderer caps hyperlink sequence output
at 256 KiB per paint. When a limit is reached, readable text remains without a
clickable target. Invalid targets and literal control characters are discarded.
