[Documentation](../README.md) · [Quick start](../getting-started.md) · [Keyboard](../reference/keyboard.md)

# Workspaces, editors, and Git

Current Ghostline controls: [expand workspace terminals and open details](../workspace-details.md).
See the [runtime guide](../reference/runtime-guide.md#git-history) for grouped Git
file trees and conditional committed-on-branch comparisons.

Each card points to a directory and owns its shell, agent and named editor tabs. The left sidebar and right inspector share one UI layout across all cards. Widths, folds, project filter, inspector choice and reduced-motion preference persist. Drag pane borders or use the actions menu to reset or zoom the layout.

Use **Ctrl+Space, G** for **Add project**. Type or paste a directory, including a relative path or `~/`, and browse its folder suggestions with **↑/↓** and **Tab**. **←** opens the parent directory; **Ctrl+U** clears the path and **Ctrl+R** retries a failed folder read. **Enter adds the displayed path**, while **Esc** cancels. Paths with spaces need no shell quotes. Flere takes the name from the local Git `origin` repository URL, falling back to the main checkout's directory name; no network connection or GitHub sign-in is needed. It finds the main checkout even when you select a subdirectory or linked worktree, then creates or reuses its primary card. The card is marked **main checkout**. Primary means directory ownership only: it has the same agent tools and ordinary pinning as every other card.

Use **n** (or **N**) for another card in a new local Git branch and linked worktree under `STATE/worktrees/ID`, from the exact locally resolved base commit. Use **t** for another terminal in the current card. Git runs outside the PTY/UI loop; completion preserves your current focus and the source checkout. An interrupted or failed worktree remains inspectable and is never replaced automatically. Existing cards are not moved or deleted during upgrade.

The CLI equivalent is `flere add-project --cwd /path/to/repo --name Project`; add `--no-shell` for a stopped primary card. The low-level `new` CLI remains available for loose directories outside this project flow.

The Files inspector includes `..`; `-` or Backspace goes up. Enter opens a directory, previews PNG/JPEG through a capable companion, or opens other files in a named editor tab. Shift+E explicitly opens the selected file in the editor. Reopening the same canonical file selects its existing live editor buffer. Flere uses `$VISUAL`, then `$EDITOR`, then installed `nvim`, `vim` or `vi`; editor settings are parsed as argv without a shell. Git view shows the current branch, working changes and a commit graph. Enter on a working change edits that file; `d` opens a side-by-side Vim/Neovim diff; `u` previews the patch. Diff inspection disables external diff and textconv.

Tabs have close buttons. `x` requests confirmation for the exact current tab and run, including a warning about unsaved edits or an agent. Exited tabs disappear after child reaping and PTY EOF. Archiving requires stopped terminals and retains the directory and metadata; restoring starts no process.

Cards have independent workflow status, project colors, issue/PR symbols and ordinary pinning. The board uses the same project filter and status colors. Native completion requests Needs me review; it does not accept the work. Working-card animation requires both a matching owned Codex process and observed working/composer text. It is a display heuristic, never a basis for sending input or acknowledging messages. Needs me itself does not continuously animate. A blue Working badge and moving border highlight mark busy cards; background chats count even when a shell or another card is selected. Tabs and the board use the same observation. Actions → Shift+M toggles reduced motion, keeping a static badge.

## Cards and the inspector

Cards have three content rows with a blank row between them. Long titles wrap;
short titles leave room for project or branch context. Workflow and runtime are
separate: **Working** is an observation, and **no agent** stays explicit.
Sidebar and board use the same presentation.

Click **Files / Git / Details** to select an inspector, or press **i** in navigation
to cycle. Files supports Home/End and Page Up/Down as well as j/k and the wheel.
The selected-file panel shows context without changing selection when clicked.
**Ctrl+Space → v** opens Details: task notes, workflow, runtime, project, branch,
issue/PR links, directory, terminals, and connection capabilities. Scroll with
j/k, the wheel, Home/End, or Page Up/Down. **Ctrl+Space → e** edits the card;
Details scroll position belongs to the current attachment.

## Project logos

Place `.flere/icon.png` or `.flere/icon.jpg` in the Git repository root to
choose its card badge, then reconnect. Flere also checks common local logo
paths. Images must stay inside the repository, be regular files, and fit within
256 KiB and 1024×1024 pixels. Missing or invalid images retain project initials.
Actual image pixels require the Windows companion; Linux/macOS and ordinary SSH
use text fallbacks. [Logo discovery and remote compatibility](remote.md#project-image-badges).

## Git history

Open Git with **Ctrl+Space → g**. The graph shows local commits with branch,
remote-tracking and tag labels, in pages of 50. Expand a commit with Enter or a
click to see file paths and change status. Enter/click on a historical file, or
`d` on a working/historical file, opens **side-by-side Vim/Neovim diff mode**:
old version on the left, new version on the right, with highlighted changes and
synchronized scrolling. Working-file Enter/click still edits the actual file.
The expanded **Open all changes** row opens the whole commit as a unified patch
in the configured editor. `u` previews the selected patch inside Flere;
`?` shows the full message, author, date, hash and diff base. Press q/Escape to
return from an internal preview. Resting the pointer on a commit shows a short
message after 350 ms on terminals that support mouse motion reporting.

Inside Vim/Neovim, use `]c` / `[c` for the next/previous change, `Ctrl+w` then
h/l to switch sides, and `:qa` to close the comparison. Your editor configuration
provides the syntax colors; Flere installs no plugins or global configuration.
Comparison buffers are read-only and nonmodifiable, have line numbers, and start
unfolded. On narrow terminals, **Ctrl+Space → z** zooms the terminal pane, then
Enter returns to the editor. Other configured editors can still open whole-commit
patch documents; selected-file comparisons require Vim/Neovim. `u` always offers
the internal patch view.

Use j/k, arrows, PageUp/PageDown, Home/End or the wheel to navigate. `G` jumps
to the commits heading; Enter collapses or expands each section. The next/previous
rows load another page. History uses captured exact local tips so later pages
keep the same traversal even if branches move; `F` refreshes the tips and
returns to the first page. Working status continues to update separately.

Merge diffs compare against the first parent; root commits compare against the
empty tree. Renames read the original and new paths; additions/deletions use an
empty side. Working comparisons prefer **index → working file** when unstaged
changes exist, otherwise **HEAD → index** for staged changes. An unborn repository
uses an empty HEAD and shows “No commits yet”. Symlinks compare their link text.
Binary/non-UTF-8 files, submodules and unmerged entries use `u` for the patch view.
Git reads run in the background with a three-second deadline and 512 KiB output
limit per command; history supports at most 2048 local tips. Missing local objects,
oversized output and non-UTF-8 historical paths produce an error. No fetch is started.

Comparison snapshots are private read-only files under
`STATE/git-diffs-v1`, up to 512 KiB
per side. They retain the filename extension for editor syntax detection. Whole
commit patch documents are capped at 1 MiB. Reopening identical content reuses
its editor tab. Managed snapshots expire after 30 days without a reference;
open, saved and pending editor tabs protect their complete comparison pair.
Legacy shared snapshots and uncertain ownership remain retained; see
[Git snapshot retention](../git-cache-retention.md). Comparisons read snapshots without changing the
source files, checkout or index.
