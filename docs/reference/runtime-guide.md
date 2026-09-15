[Handbook](../README.md) · [Workspace details](../workspace-details.md)

# Runtime guide

A native Linux and macOS terminal workbench inspired by Switchyard. One Rust executable provides a custom UI, shared side panes, workspace-local PTY tabs, a private control socket and a supervisor that survives UI detach. Flere owns its renderer, navigation and file explorer. There is no tmux, UI framework or Oil.

Small infrastructure crates provide JSON, Unicode widths, argv parsing and OS bindings. Installed Git, your configured editor, shells and native coding agents remain external programs. The musl build is a static Linux x86_64 executable.

Flere implements the main workspace/UI workflows and remains experimental. See the [acceptance checklist](../acceptance.md) for native harness, Windows/SSH and terminal limits, and the [validation record](../releases/0.3.0-validation.md) for measured evidence.

## Native macOS build and install

Apple Silicon and Intel Macs build directly with Rust 1.98+ and Xcode Command
Line Tools (`xcode-select --install`). Flere runs in an ordinary terminal and
uses your configured shell, including zsh. Git and Vim/Neovim remain external tools.

```sh
./scripts/dev package --with-companion
./scripts/dev update --with-companion --state "$HOME/.local/state/flere"
"$HOME/.local/bin/flere"
```

Add `~/.local/bin` to PATH to launch with `flere`. State defaults to
`~/.local/state/flere`; `--state DIR` selects a separate instance. Detach with
**Ctrl+Space, then q** and run the same command to reconnect. If macOS reserves
Ctrl+Space for input-source switching, release that binding in System Settings →
Keyboard → Keyboard Shortcuts → Input Sources or configure your terminal to send
NUL for the navigation key.

The [developer script](../../scripts/dev) runs offline validation and retains
verified packages and receipts in private home cache. Dependencies must already
be cached. `package` does not install; `update` installs and refreshes the selected
running supervisor while keeping sessions, and registers this checkout as an
explicit development source. It starts no supervisor when none is running.
`--with-companion` also validates/packages the Unix companion and installs it first
during update. Use `--adopt` only to adopt a known manual installation.
See [setup and updates](../getting-started.md#update-from-flere) for the managed
UI flow and one-time legacy-supervisor refresh requirement.

The first-use [Unix bootstrap](../../scripts/install.py) and
[Windows companion bootstrap](../../scripts/install-companion.ps1) require an
published manifests from the configured GitHub Releases channel or an explicit
HTTPS manifest URL. The default channel works only when matching release assets
are published. They need no Rust/source checkout and do not start chats.

The Darwin backend uses native PTYs, Unix peer credentials and bounded kernel
process metadata. It checks process start identities across supervisor refresh
and verifies the inode of an owned open transcript before reading its path.
It does not require `/proc`, Rosetta, Docker, or root access at runtime. macOS
process inspection failures reject native target proofs; they do not trigger a
transcript search or weaker identity fallback. See [MACOS.md](../../MACOS.md) for the
platform checks and remaining acceptance limits.

## Try the candidate separately

From an ordinary terminal, outside an existing Flere UI:

```sh
flere --state "$HOME/.local/state/flere-trial"
```

First use creates one ordinary default-shell workspace in your current directory. Later openings reconnect to the same selected supervisor. A separate state keeps this trial independent of other Flere instances.

A fresh state opens with a cyberpunk intro: the block-letter Flere wordmark
assembles over flowing data and perspective rails, then keeps shimmering above
**Press any key to start**. Any key skips the opening sequence and starts the
workbench; Ctrl+C exits before a shell is created. The start key, paste and mouse
reports are consumed by the intro. A populated state reopens directly. The intro
honors the existing reduced-motion preference and adapts to terminal resizing.

To preview it separately from an ordinary outer terminal, run
`flere intro`. This preview creates no state or supervisor.

Opening or attaching another Flere UI inside a Flere terminal is refused before creating state or starting a supervisor. Use the existing navigation/actions instead; diagnostic and coordination CLI commands still work inside a workspace.

`Ctrl+Space`, then `q` detaches the UI while its processes continue. Open the same command to reconnect. If the supervisor exits or the machine reboots, workspace metadata, coordination and recorded conversation IDs remain. Processes, drafts and terminal scrollback do not survive a machine/supervisor failure. Supervisor-only startup creates no terminals. Opening the UI restores saved open tabs in their recorded order, using exact native conversation IDs where known; failed restores remain available for explicit retry.

## Workspaces and tabs

Each card points to a directory and owns its shell, agent and named editor tabs. The left sidebar and right inspector share one UI layout across all cards. Widths, folds, grouping, project filter, inspector choice and reduced-motion preference persist. Drag pane borders or use the actions menu to reset or zoom the layout.

The Ghostline sidebar encloses each workspace and its expanded tabs in one square
card. Filled group headings separate the sections; the current terminal has its
own row highlight. **Ctrl+Space, Shift+L**, the grouping action, or a click on the
sidebar header switches between **Status** and **Project**. **Pinned** stays
first in both modes. Project grouping retains each card's workflow and observed
activity cues. Folding a section keeps its count visible; folding by project
does not change the saved status folds.

Use **Ctrl+Space, G** for **Add project**. Type or paste a directory, including a relative path or `~/`, and browse its folder suggestions with **↑/↓** and **Tab**. **←** opens the parent directory; **Ctrl+U** clears the path and **Ctrl+R** retries a failed folder read. **Enter adds the displayed path**, while **Esc** cancels. Paths with spaces need no shell quotes. Flere takes the name from the local Git `origin` repository URL, falling back to the main checkout's directory name; no network connection or GitHub sign-in is needed. It finds the main checkout even when you select a subdirectory or linked worktree, then creates or reuses its card. The sidebar marks the original Git worktree with a small **Primary** badge beside its workspace name. In narrow cards, the badge moves to the bottom border to keep the name readable. The Details overlay also shows **Primary**. Flere checks the actual Git directory identity in the background; the current branch name, card order and pinning do not determine this badge. It describes the directory and grants no agent privilege.

Use **n** (or **N**) for another card in a new local Git branch and linked worktree under `STATE/worktrees/ID`, from the exact locally resolved base commit. Use **t** for another terminal in the current card. Git runs outside the PTY/UI loop; completion preserves your current focus and the source checkout. An interrupted or failed worktree remains inspectable and is never replaced automatically. Existing cards are not moved or deleted during upgrade.

The CLI equivalent is `flere add-project --cwd /path/to/repo --name Project`; add `--no-shell` for a stopped primary card. The low-level `new` CLI remains available for loose directories outside this project flow.

Right-click an item to open a context menu without switching the active terminal.
Choose with the mouse or **j/k**, **Up/Down** and **Enter**. **Home/End** select
the first/last item; **Escape** or a click outside dismisses the menu.

| Right-click target | Available actions |
| --- | --- |
| Workspace card | Focus workspace, expand/collapse tabs, pin/unpin, details |
| Sidebar child or centre tab | Activate tab, close tab, workspace details |
| File | Open in editor, copy path; PNG/JPEG also offers preview |
| Directory | Open directory, copy path |
| Git row | Open/expand, preview diff where available, copy commit SHA where available, refresh |
| Terminal area | Copy Flere screenshot, open Flere actions |

The menu retains the item you clicked. A changed target or resized layout
dismisses it. Menu keys and paste events stay in Flere; Close tab uses the
same checks and confirmation as its keyboard action. Path and commit copies use
the outer terminal's existing OSC 52 clipboard support.

The Files inspector includes `..`; `-` or Backspace goes up. Enter opens a directory or a named editor tab while keeping the explorer selected. Reopening the same canonical file selects its existing live editor buffer. Flere uses `$VISUAL`, then `$EDITOR`, then installed `nvim`, `vim` or `vi`; editor settings are parsed as argv without a shell. Git view shows the current branch, working changes and a commit graph. Enter on a working change edits that file; `d` opens a side-by-side Vim/Neovim diff; `u` previews the patch. Diff inspection disables external diff and textconv.

Tabs have close buttons. `x` checks the exact current tab and run before closing.
An integrated Zsh shell at an empty prompt with no jobs, or a clean Vim/Neovim
editor opened by Flere, can quit gracefully without a dialog. The program
checks again before quitting; a clean observation never triggers an automatic
hangup. Unsaved buffers (including hidden buffers), unsubmitted commands,
foreground/background/stopped jobs, chats, and unknown state require confirmation
with a reason. Enter defaults to Cancel in the dialog. Escape cancels a pending
check while the program has not begun quitting.

These private integrations load only for newly started Zsh shells and configured
`vim`, `nvim`, `vimdiff`, or `nvimdiff` editor commands. Your startup files are not
edited. Existing processes, manually launched editors inside shells, other
shells/editors, missing integration support, and unavailable process information
retain confirmation. Neovim language servers can exit with a clean editor when
their registered commands and child processes match; unfamiliar jobs, wrappers,
and special plugin buffers remain conservative. This does not prove the state of
arbitrary plugins or attribute detached daemons to a tab. Ordinary modified-buffer
checks and a non-forced quit protect file edits; Flere does not automatically
save them or force an editor that refuses to quit.

Exited tabs disappear after child reaping and PTY EOF. Archiving requires stopped
terminals and retains the directory and metadata; restoring starts no process.

**Ctrl+Space, s** or **Actions → Copy Flere screenshot** copies the complete
Flere view: workspace list, tabs, visible buffer or scrollback, inspector,
status bar and displayed graphics. In a full-screen image preview, **s** captures
the preview. The action closes its launcher and returns to buffer input without
sending child input. Flere exports its composed frame inside the terminal;
there is no desktop capture or native window picker. A bundled JetBrains Mono
Nerd Font makes rendering portable. Host font settings, color emoji, terminal
antialiasing and cursor shape/blink may differ from the displayed pixels.

Image rendering runs off the UI thread. Local macOS and Linux (X11 or Wayland
with clipboard data-control support) publish PNG plus its saved path. The Linux
selection owner outlives UI detach and exits after another copy replaces it.
With a matching **protocol v6 flere-connect**, Windows, macOS and Linux receive
the PNG on the local clipboard over SSH. Install the updated companion before
reconnecting; older companions cannot attach to the current protocol. New companions
accept supported older servers with only their available capabilities; screenshot
export needs v5 or newer. Plain SSH cannot export
images to a local clipboard.

Wait for “Flere screenshot copied,” then use your terminal's paste shortcut
(**⌘V** in Ghostty). Image-aware apps read the PNG; terminals paste its retained
path. The companion recognizes its most recently copied screenshot path and
uploads that exact local image, so a local path is never mistaken for a remote
file. Copies do not submit the chat. A copy succeeds only after the local
clipboard confirms both formats; transfer receipts use exact connection IDs.

Screenshots are bounded to 20 MiB. The local attachment cache and companion
screenshot cache each refuse new images at 256 MiB and retain successful files
for clipboard/draft reuse; failed copies remove their unused file. Unix caches
are private (0700 directory, 0600 images); the Windows companion uses
`%LOCALAPPDATA%/Flere/screenshots`. Incomplete remote images remain in bounded
memory and cannot reach the clipboard. Detaching before rendering completes
drops the pending copy. Clipboard availability still depends on the local
system's graphical session; rendering does not.

Cards have independent workflow status, project colors, issue/PR symbols and ordinary pinning. The board uses the same project filter and status colors. Native completion requests Needs me review; it does not accept the work. Working-card animation requires both a matching owned Codex process and observed working/composer text. It is a display heuristic, never a basis for sending input or acknowledging messages. Needs me itself does not continuously animate. A blue Working badge and moving border highlight mark busy cards; background chats count even when a shell or another card is selected. Tabs and the board use the same observation. Actions → Shift+M toggles reduced motion, keeping a static badge.

### Git history

Open Git with **Ctrl+Space → g**. Working files are grouped into **Staged**,
**Unstaged**, **Untracked** and, when needed, **Conflicts**, with collapsible
folder trees. A path changed in both the index and working tree appears in both
groups; its selected group determines the comparison. Enter folds a folder or
section, and stays in navigation mode.

**Committed on branch** shows the branch's net committed file changes from its
merge base with the locally known target/default branch. The section is omitted
when that comparison has no files, including equal trees, fully reverted changes
and a branch that is only behind its target. Dirty working files stay in Working
tree. Branch-file comparisons retain the captured base/head identities even if
refs move afterward.

Flere first honors `branch.<current-name>.gh-merge-base` when configured. It
then uses the branch remote's local HEAD reference, origin's HEAD, or the sole
other remote HEAD. Without those refs it tries `init.defaultBranch`, `main` and
`master` locally. It does not contact GitHub or fetch refs. A configured target
that is missing locally, unrelated histories or ambiguous merge bases produce a
comparison error while working files remain accessible. For example, set a local
PR target with `git config branch.YOUR_BRANCH.gh-merge-base TARGET_BRANCH`.

The graph below shows local commits with branch, remote-tracking and tag labels,
in pages of 50. Expand a commit with Enter or a
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
empty side. **Unstaged** comparisons use **index → working file** and **Staged**
comparisons use **HEAD → index**, including when both groups contain the same path.
An unborn repository
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

### Appearance

The Ghostline workbench uses near-black navy surfaces, a slim cyan focus rail,
muted lilac/blue metadata and thin pane separators. The current workspace retains
a subtle selection and chevron while another pane owns keyboard focus. Native
programs keep their own terminal rendering and input; Files retains the existing
directory-at-a-time explorer. No UI framework, editor plugin or runtime dependency
is added.

New layouts give the inspector 40 columns for Git folder trees and commit history.
Saved pane widths remain intact; **Ctrl+Space, r** restores the default layout.
Narrow windows use the existing focused-pane layout with shorter Git hints.

### Refined cards and inspector (0.2.18)

Cards have three content rows with a full blank row between them. Continuous
card surfaces, prominent titles and muted metadata make each workspace distinct;
workflow color stays on the rail and status symbol. Long task names wrap across
two lines; short names show project or branch context. The final row separates the
runtime from the workflow: a live agent is distinct from observed **Working**,
and **no agent** stays explicit. Sidebar and board use the same card presentation.

The inspector has directly clickable **Files / Git / Details** tabs; `i` still
cycles them. Files use ordinary Unicode type markers, readable truncation, a
selected-file panel and workspace context when space permits. With Files focused,
**Home/End** and **PgUp/PgDn** select entries; `j/k`, the wheel, Enter and `-` retain
their navigation roles. Clicking the selected-file panel does not select another
file. PNG/JPEG preview remains on `p`.

**Ctrl+Space, v** opens Details: wrapped task notes, workflow, runtime, project,
branch, issue/PR links, directory, terminal list and connection capabilities.
Use `j/k`, the wheel, Home/End or PgUp/PgDn to scroll; **Ctrl+Space, e** edits the
card. View scrolling belongs to this UI attachment and is not persisted. Tiny
inspectors abbreviate the tabs to F/G/D. Image-preview notices now distinguish a
missing local companion from unavailable terminal graphics support.

This UI update works with the existing **0.2.16 Windows companion**; the remote
protocol is unchanged. The user has supplied a screenshot through the SSH image
attachment flow. Physical preview rendering, resize and cleanup remain separate
acceptance checks.

## Workspace terminal trees and details

In Cards navigation (**Ctrl+Space, h**), **j/k** and **Up/Down** select whole cards. **Tab** enters the selected card's terminal list; those same keys then select a terminal and **Enter** activates it. **Left**, **Esc** or **Tab** returns to the card. Expanded terminal rows do not add stops to ordinary card navigation. **?** opens focused details. Hover shows a bounded tooltip next to the pointer/card; it preserves the logo when space permits and never steals native input or starts a request. Expansion is remembered.

**i/p** explicitly opens a linked issue/PR locally, or copies its URL over SSH. **e** edits notes without overwriting newer card metadata. **Tab** selects an action and **Esc** returns. GitHub loading is optional, bounded, and uses installed `gh`; offline/local work remains usable. See the [workspace details guide](../../docs/workspace-details.md) for keys, mouse behavior, limits and notes conflict recovery.

## Split centre panes

Use **Ctrl+Space**, then **Space**, and search for **Split**. Split right/below
can move the current tab (when another tab remains) or create a new shell.
Each pane owns its tabs. **Tab / Shift+Tab** in navigation cycles only the focused
pane's tabs; new tabs and the adjacent plus button also belong to that pane.
Click a pane, or use **h/l / Left/Right** for side-by-side panes and
**j/k / Up/Down** for stacked panes in navigation. The wheel focuses its pane
before scrolling it. Drag the divider to resize.

**Shift+Z** zooms the focused pane and restores the split. **~** moves its selected
tab to the other pane; **Shift+X** removes the split while keeping every tab and
process. A window too small for both panes temporarily shows the focused pane;
resizing restores the split. Lowercase **z** still toggles the outer sidebars.

All windows attached to the same state share the workspace, split layout, pane
focus and each pane's selected tab. The last actual attachment resize sets their
shared viewport. Refresh preserves exact processes, drafts and pane membership.
Reopening saved tabs restores their groups and order; failed tabs remain available
for explicit retry. Screenshots include both visible panes and their displayed
scrollback. Text previews and retained scrollback belong to their individual tabs.

## Intro screensaver

After **five minutes without user input**, Flere opens its original neon intro
and wordmark with a small wandering mascot. New preferences default to floating
Flere; existing saved Duck choices are respected. **Ctrl+Space, Shift+U** starts it immediately; Actions also has
**Start screensaver**. **Ctrl+Space, Shift+V** cycles the idle setting between five
minutes, fifteen minutes and manual only. The setting is saved with UI preferences.

A keyboard press or paste wakes the workbench. Flere consumes that gesture so
it cannot type into a chat, activate a tab or submit a draft. Mouse movement,
clicks and wheel keep the saver open; grab and throw the mascot to play.
Terminal output and capability/focus reports do not reset the idle timer.
Unfinished forms, selections, transfers and other active UI operations delay entry.
Sessions keep draining output, and each attachment keeps its screensaver local;
its shared workspace/panes remain available in other windows. Entry itself does
not resize terminals or restart anything. Reduced motion keeps the intro and mascot
still except for explicit dragging. The intro and screensaver use Flere's terminal renderer on local and
remote attachments.

Actions offers **Mascot: Duck (screensaver + pet)** and **Mascot: Flere (screensaver + pet)**.
In NAV, `%` selects Duck and `^` selects Flere. Each choice updates the screensaver
and dock mascot, saves the preference and immediately opens the original intro
screensaver. It preserves whether the dock pet is enabled. Flere has a white disc,
revolving rings and yellow-green
fields. Click gently to pet it: it may glow warmly with satisfaction or show
white irritation flecks, then return to normal. Drag to grab and throw it.
Hover does not pet or wake it. Reduced motion keeps feedback static and disables
throw inertia. The [mascot design](../design/flere-mascot.md) records which
appearance details come from the book and which are visual interpretations.

## Navigation and actions

`Ctrl+Space` enters Flere navigation without sending Escape to a child. After a brief pause, the footer reveals navigation hints. `Space` opens Actions; use j/k and Enter, or a displayed shortcut. Press `/` inside Actions to search by words; arrow keys select and Enter runs the result. Escape clears search first, then closes Actions. Pasted text is only a search query. **Shift+C** in NAV toggles compact two-line workspace cards; **Shift+J** cycles workspaces needing attention in creation order, within the project filter. The header counts workspaces needing review and workspaces with observed working terminals. Mouse actions briefly show their keyboard equivalent. From a side pane, Ctrl+Space keeps navigation active so h/l return toward the centre. Enter returns to terminal input, except that Enter in Files opens its selection and Enter in Cards activates its selected terminal.

| Where | Keys | Action |
| --- | --- | --- |
| Navigation | h / l | Move between cards, terminal and inspector |
| Navigation | j / k | Select card/terminal/file, or move between terminal and tabs |
| Cards in navigation | j/k or Up/Down | Select cards, skipping child terminals |
| Cards in navigation | Tab / ? | Enter terminal list / open focused details |
| Card terminal list | j/k or Up/Down; Enter; Left/Esc/Tab | Select terminal; activate; return to card |
| Cards in navigation | Enter | Activate selected exact terminal |
| Tabs in navigation | h / l | Change tab in the focused pane |
| Terminal or tabs in navigation | Tab / Shift+Tab | Next / previous tab in the focused pane, wrapping at either end |
| Navigation | `\|` / `_` | Split right / below, moving the current tab (requires another tab) |
| Navigation | `\` / `=` | Split right / below with a new shell |
| Split terminal in navigation | h/l or Left/Right; j/k or Up/Down | Switch side-by-side panes; switch stacked panes |
| Navigation | `~` / Shift+Z / Shift+X | Move tab to other pane / zoom pane / remove split and keep tabs |
| Navigation | Space | Actions menu (`/` searches) |
| Navigation | Shift+U | Start the intro screensaver |
| Navigation | `%` / `^` | Select Duck / Flere for the pet and open the intro screensaver |
| Navigation | Shift+V | Cycle screensaver idle time: 5 minutes / 15 minutes / manual only |
| Navigation | Shift+C | Toggle compact workspace rows |
| Navigation | Shift+J | Next workspace needing attention |
| Navigation | G / n (or N) / t | Add project / new worktree card / shell tab |
| Navigation | / / P | Fuzzy workspace search / project filter |
| Navigation | Q / H | Quick Open / Find in Files |
| Terminal in navigation / Actions | ? | Find retained terminal output; Cards/Git use ? for details |
| Navigation | ( / ) / Y | Previous / next marked command / copy complete command output |
| Navigation | ! / ; | Run Task in a new owned terminal / Problems |
| Navigation | K | Update Flere; local apply or companion prepare/review/apply |
| Navigation with companion | 6 / 7 / 8 / 9 | Upload / download selected file / open selected file locally / Ports |
| Navigation | T / w | Workspace tabs / all live terminals |
| Navigation | b / f, or < / > | Back/forward through exact workspace, tab and run history |
| Navigation | i / g / v | Cycle inspector / Git / Details |
| Navigation | e / p | Edit workspace metadata / pin |
| Navigation | B | Board; h/l selects column, j/k selects card, Enter returns |
| Navigation | 1 / 2 / 3 / 4 / 5 | Needs me / In progress / Todo / Waiting / Done |
| Navigation | S | Start agent: choose a harness (no conversation-ID field) |
| Navigation | W | Retry saved tabs for the selected workspace |
| Navigation | x | Confirm closing exact terminal |
| Navigation | A / a | Archive stopped workspace / restore archived workspace |
| Navigation | z / r / R | Zoom / reset layout / refresh keeping sessions |
| Navigation | [ / ], { / } | Narrow/widen left and right panes |
| Navigation | M / F | Reduced motion / refresh files and Git |
| Navigation | o | Toggle the pet |
| Terminal in navigation | Ctrl+U / Ctrl+D | Scroll exactly one visible terminal page, with smooth motion |
| Terminal in navigation | u / d | Smooth scroll up/down three rows; hold to continue |
| Terminal in navigation | Ctrl+↑ / Ctrl+↓ | Full-page aliases |
| Terminal in navigation | Ctrl+Y / Ctrl+E | Scroll up/down one line |
| Terminal in navigation | Page Up / Page Down | Scroll a full page with one-row overlap |
| Terminal in navigation | Home / End | Oldest retained output / live bottom; stay in navigation |
| Navigation | c | Page into terminal scrollback and leave navigation |
| Navigation | m / I / D | Send message / read messages / review decisions |
| Navigation | q | Detach, preserving processes |
| Terminal | Wheel, Shift+Page Up/Down | Browse retained terminal history |
| Terminal / scrollback | Shift+Home / Home | Jump to the oldest retained output |
| Scrollback | Wheel, Page Up/Down, arrows | Scroll without sending keys to the chat |
| Scrollback | Esc, End, Enter | Return to live output without sending that key |
| Files | j/k, Enter, -, Backspace | Select, open, parent directory |
| Git inspector | j/k, arrows, PgUp/PgDn, Home/End, wheel | Navigate working changes and history |
| Git inspector | Enter / click | Fold group/folder/commit; open working file or branch/historical comparison |
| Git inspector | d / u / ? | Side-by-side file diff / patch preview / full message |
| Git inspector | G / F | Commits heading / refresh |
| Preview | j/k, Page Up/Down, q | Scroll wrapped text; return without acknowledgement |
| Message / decision review | a | Acknowledge this message / open decision answer form |
| Form/picker | Tab, arrows, Ctrl+U, Enter, Esc | Field/result, clear field, apply, cancel |

**Keyboard-only scrolling:** press **Ctrl+Space**, then **Ctrl+U / Ctrl+D**
to move exactly one full visible terminal page, without overlap. Plain **u / d**
moves up/down three rows; hold the key to keep scrolling. Movement animates through
intermediate terminal rows. Repeated keys extend the destination; reversing
changes direction from the current view. **Shift+M** in NAV toggles reduced motion
for immediate moves. A terminal renders whole rows, so this is animated row
movement rather than pixel scrolling.

Ctrl+↑ / Ctrl+↓ also moves a full page, Ctrl+Y / Ctrl+E moves one line,
and Page Up/Down retains a page with one-row overlap. Home jumps to the oldest
retained output and End returns live. Navigation stays active at either end.
**Enter**, Escape or Ctrl+Space leaves NAV and returns live, consuming that exit
key and cancelling pending motion. These bindings apply with Terminal focused;
Directional keys first switch between split panes; at an outer edge they access the sidebars or tab bar. Outside NAV the child's
own keys remain unchanged. NAV scrolling stays local in native alternate screens.

The wheel over the terminal and Shift+Page Up also enter scrollback. Outside
navigation, Page Up/Down and arrows move while browsing; wheel down to the bottom,
or press Escape, End or Enter to return live. Ordinary typing/pasting outside
navigation returns live and goes to the same native session. New output keeps
draining while your displayed page stays fixed. Each exact tab retains its displayed scrollback while switching tabs or panes; resizing that pane returns it to live output. Selection/copy works on the history you are viewing. Hold a text selection at
or beyond the terminal's top/bottom edge to extend it through history. Scrolling
starts after 240 ms and moves 1–4 rows every 40 ms; releasing copies the full
selection, while Esc cancels. The original anchor and already selected rows stay
frozen as new output arrives. A selection caches at most 4,096 rows / 4 MiB,
with the existing 64 KiB clipboard limit. If history is unavailable, evicted, or
changes before its initial snapshot, the visible selection remains copyable and
an on-screen notice explains why auto-scroll stopped. Scrollback
retains up to 100,000 rows within a 32 MiB compact-data budget per terminal
(whichever fills first); it preserves styles and does not reflow old lines when
width changes. Lines an older version already discarded cannot
be recovered. Wheel over a text preview scrolls that preview. Wheel over the left sidebar scrolls its cards without switching the active
workspace or sending input to a chat. Keyboard card selection reveals the selected
card again. Files, Git and Details keep their own wheel handling. Files and Git retain their visible range when reversing direction: moving up from the bottom moves the selected row up inside that range before scrolling it. Extra wheel events at either end do not accumulate.

A resumed Codex conversation can have older messages that it has not written to
the terminal at all. **Ctrl+T, then Home** opens Codex's own transcript and loads
its beginning with the default Codex keymap. Wheel/Page Up there loads earlier
messages too. This is separate from Flere's retained terminal output, and does
not require restarting or resuming the agent again. Increasing terminal retention
cannot reconstruct never-emitted or already-discarded rows.

Codex's full-screen transcript explicitly enables DEC alternate-scroll mode.
While that alternate screen and mode are both active, unmodified wheel events
become its requested up/down keys, checked against the exact current tab, run
and viewport. Shift-wheel stays local. A normal prompt never receives synthesized
arrow keys. This does not implement general application mouse reporting.

Mouse clicks select cards, tabs, files and board cards; pane borders drag. **Click and drag inside the terminal or its scrollback preview to select text; release to copy automatically.** Flere highlights the selection and uses OSC 52 to send text to the outer terminal's clipboard over SSH. A plain click leaves the clipboard alone. Escape clears the selection without reaching the native chat; typing clears it and continues normally. Text stays stable while you drag, while the process continues running. Resizing or changing the exact workspace/session cancels the pending selection. Copies are bounded to 64 KiB and use visible line breaks; dragging at or beyond the top/bottom edge auto-scrolls through bounded, frozen history after a short dwell. Joining soft-wrapped lines in copied text is not yet supported. The outer terminal must allow OSC 52 clipboard writes; Flere reports sending the selection but cannot confirm the OS clipboard accepted it. Holding the outer terminal's text-selection modifier (usually Shift) still uses its own selection behavior. Native child mouse protocols are not forwarded yet. Ctrl+Space is reserved; outer tmux/terminal bindings may consume it before Flere sees it.

## Search, Tasks and remote actions

**Ctrl+Space, Q** finds file names in the selected workspace; **H** searches file
contents. Up/Down selects a result, Enter opens its file/line in the originating
pane, Ctrl+C copies a result and Esc cancels. **?** searches retained terminal
output and jumps to the selected row. Cards and Git keep their contextual ? key;
**Find Terminal Output** is also available through Actions. Search does not send
typed queries to a child and closes if its exact workspace/pane/run changes.

**(** and **)** move between marked shell commands; **Y** copies the complete
selected command's output. New zsh shells mark complete commands; Bash supplies
prompt boundaries without replacing user DEBUG traps. Missing/incomplete markers
are reported rather than guessed. These controls operate on retained output.

**!** opens **Run Task**. Enter starts the supplied command once in a new
supervisor-owned PTY, leaving existing chats and shell input untouched. **;** opens
**Problems**: Tab cycles tasks, j/k selects a diagnostic and Enter opens its
file/line through the same exact-pane editor action. Completed task tabs retain
their output and locations until explicitly closed. Refresh preserves live and
completed task reports; restoring saved work after a full restart opens ordinary
shells in their directories and never replays task commands.

With a current companion, **6** uploads a local file into the selected remote
directory, **7** downloads the selected file, and **8** downloads and explicitly
opens a supported image, HTML or PDF locally. The local companion asks for the
local path or confirmation; existing files are kept and transfer bytes never
become terminal input. **9** opens Ports, with explicit local loopback forwarding,
browser opening and close controls. Nothing starts from terminal output alone.
Saved connection aliases and reconnect retain the selected OpenSSH settings;
reconnection attaches to retained work without automatically launching native chats.
See [remote setup](../guides/remote.md).

**K** opens **Update Flere**. Locally, Enter applies a selected package/HTTPS
manifest, or the explicitly registered checkout when the source is empty. Register
one with `flere dev-source /absolute/path/to/flere`; a workspace directory
never implicitly becomes executable update code. Over SSH, the companion owns
the local source form: Enter prepares both components, then a separate review
shows their identities and requires Enter to apply or Esc to cancel. Exact PTYs,
sessions and drafts survive; results distinguish file installation, supervisor,
frontend and companion activation. Other attachments are reported separately.
An older supervisor lacking guarded update support requires one explicit legacy
refresh from the newly built executable first. See [update setup](../getting-started.md#update-from-flere).

## Native sessions and refresh

First opening creates one ordinary shell workspace in your current directory. Every agent has the same coordination tools. Pin any workspace with **Ctrl+Space, p** to keep it prominent; names and pins do not grant privileges. You can name a pinned card “Lead” if that suits your workflow.

Existing Lead cards keep their names, directories, chats and pins, and can be unpinned normally. Flere no longer creates or assigns a Lead role. Address messages to an exact workspace ID (from `list_workspaces`) or `user`.

Start agents explicitly through `S` or the CLI:

```sh
flere --state ~/.local/state/flere-parity list
flere --state ~/.local/state/flere-parity agent WORKSPACE_ID codex EXACT_UUID
```

The optional CLI UUID is an advanced integration interface; omit it for a fresh conversation. Exact resume maps to `codex resume UUID`, `claude --resume UUID`, or `copilot --resume=UUID`.

Flere saves every open tab, its order, the selected tab and the selected workspace. Opening the UI after a supervisor or machine restart reopens those tabs: agents use their own recorded harness and exact conversation ID; shells start in their last saved directory; editor tabs reopen their file. Three open chats remain three separate tabs. Supervisor-only startup launches nothing. Reattaching to live tabs reuses them.

**S** starts a new agent with a harness selector only. Flere has no conversation-ID field or saved-conversation chooser. Switch conversations inside the harness. Codex discovers the current ID from a unique rollout descriptor in its owned child tree, including after a conversation switch. Automatic ID discovery for Claude and Copilot is not yet implemented: recorded IDs resume exactly; an unknown ID opens that harness's native resume picker. Flere never selects the latest chat automatically or adds approval-bypass flags.

Shell directories are sampled once per second and before an explicit supervisor stop. Restart does not replay commands, drafts, scrollback or unsaved editor buffers. An explicitly closed tab stays closed. A failed reopen leaves the saved tab available while other tabs continue; **Ctrl+Space, W** retries saved tabs for the selected card. Opening that card also retries. Hover and keyboard preview do not launch anything.

After a native process exits, its host execs your default interactive shell in the same PTY. Exiting that shell closes the tab. You may also run a native CLI yourself from any ordinary shell tab.

`Ctrl+Space`, Space, `R` refreshes candidate supervisors and re-execs the UI while preserving child PIDs, exact run identities, PTYs, input queues, terminal/alternate-screen state and active sockets. The candidate preflights a private handoff before same-PID exec; incompatible images are rejected while the old supervisor keeps running. Pending worktree creation and in-flight native message handoffs must finish first. Refresh is an explicit action and is separate from reboot recovery.

## Coordination and diagnostics

New supported Codex launches register a task-scoped Flere stdio MCP server through ordinary native configuration. It exposes context, inbox, message, checkpoint, decision request, result submission, workflow status, bounded own-terminal reads and explicitly user-requested focus. Existing chats do not acquire a new MCP catalog just by refreshing the UI. Native repository and MCP trust remain yours to review.

Messages have separate saved, surfaced and acknowledged timestamps. Opening a human message reader or reading the native inbox surfaces returned messages; handling requires explicit acknowledgement. Pending messages/decisions come before old retired records. Agents cannot answer human decisions or set Done; result submission requests review. Codex delivery now uses optional native hooks and an idle queue, with separate transport receipts. Human previews do not count as delivery to an agent. See activation below. MCP registration and automatic delivery for Claude/Copilot remain pending.

The private Unix socket supports bounded live inspection while attached or detached:

```sh
flere --state ~/.local/state/flere-parity list
flere --state ~/.local/state/flere-parity capture --session ID --run TOKEN --lines 60
flere --state ~/.local/state/flere-parity focus WORKSPACE_ID TAB_ID
```

`list` reports exact workspace/session/run/process identities. Capture emits passive JSON and at most 200 recent lines. Explicit diagnostic input separates text from keys, rejects stale/mismatched targets and records target/run/byte count without recording typed text:

```sh
flere --state ~/.local/state/flere-parity send --session ID --run TOKEN --text 'literal text'
flere --state ~/.local/state/flere-parity send --session ID --run TOKEN --key Enter
```

Literal text rejects embedded control keys. Multiline diagnostic text is allowed only for a child advertising bracketed paste. Nothing automatically submits native approvals. The same-user socket is an ownership boundary, not a sandbox against other programs running as your OS user.

`coordinate WORKSPACE_ID OPERATION JSON` provides the human CLI path for local coordination. `close --session ID --run TOKEN --terminate` and `stop --terminate` explicitly hang up the targeted terminal or this instance; detach to preserve sessions. Programs deliberately ignoring hangup can outlive termination.

State defaults to `$XDG_STATE_HOME/flere` or `~/.local/state/flere` in a private directory. Metadata and coordination share an atomic versioned store; UI preferences, run specifications, socket, lock, action receipts and supervisor log are also local. Child temporary files use the Flere home/XDG cache, not system `/tmp`. No Switchyard data is imported or modified, and Flere has no GitHub publication feature.

## Assignments and agent startup

A shell card is preparation. Issue or saved-chat cards with no hosted native tab say
**no agent**; a hosted agent and observed working animation remain distinct.
For authorized implementation work, any agent uses this sequence through Flere MCP:

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
store. Existing conversations continue through explicit exact-UUID resume with no
appended context dump. Dispatch refuses workspaces with existing native work or
saved conversations, so it cannot silently replace a chat already underway.

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
`pinned`, `status`, `notes`, `issue` and `pr`. Read `list_workspaces` first and
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

## Activate agent-message delivery (0.2.8)

First use **Ctrl+Space → Space → Shift+R** to activate the installed Flere
candidate. This preserves every terminal, native PID, conversation and draft.
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
requests remain protected. `message_status` distinguishes saved, queued, surfaced
and acknowledged and reports the specific idle predicate that blocks delivery.
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

## Build and validate

Rust 1.98+ and a system C linker are required for a source build (Xcode Command Line Tools on macOS). `Cargo.lock` pins the small infrastructure dependency tree. Offline commands require those crates already cached; a clean developer environment must fetch them once.

```sh
export TMPDIR="$HOME/.cache/flere/tmp"
mkdir -p "$TMPDIR"
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
./scripts/dev package --with-companion
```

The developer script also builds release packages and records their source/payload
identities; it retains the historical binaries. Installation is a separate
`scripts/dev update` action as described above. Tests use isolated home-cache states
and Git repositories, real kernel PTYs, sockets and Vim, plus harmless stand-ins for
native launch/lifecycle tests. They start no models or Switchyard sessions.

`examples/capture_ui.rs` captures actual UI cells into SVG/text/ANSI using an owned UI PTY. It is a diagnostic rendering, not a physical Windows Terminal screenshot; use it only on disposable or explicitly selected states because it changes the shared viewport.

## Remaining acceptance and compatibility work

The implementation supports Linux x86_64 and macOS arm64/x86_64; runtime evidence
and its precise limits are in [MACOS.md](../../MACOS.md) and the
[acceptance checklist](../acceptance.md). User confirmation covers one historical
Windows attachment and visible logos; complete current Windows/SSH, Intel/older
macOS and native-agent acceptance remain open. Automatic Codex delivery is tested
with harmless stand-ins; actual handling and acknowledgement after human hook
trust still need acceptance.

Terminal emulation covers common cursor/erase/insert/delete operations, scroll regions, primary/alternate grids, indexed/RGB colors, dim/italic/strikethrough, UTF-8 widths/combining characters, bracketed paste and basic device replies. The terminal area uses neutral #cccccc text on #0c0c0c; sidebars keep the workbench palette. Bounded OSC 10/11 replies report these exact terminal defaults so Codex can shade messages and the composer. OSC 133 records bounded command boundaries; other OSC/DCS commands stay inert. Font size is controlled by your outer terminal. Codex caches color detection at startup: refresh updates Flere, but an already-running Codex may need your explicit exit/resume of its saved conversation to acquire the new shading. Outer-theme detection/custom terminal palettes are not yet implemented. Full grapheme shaping, history reflow, extended keyboard and native mouse protocols remain incomplete. The active workspace and viewport are shared across attachments.

The explorer currently reads at most 512 entries per directory. History remains in memory, bounded to 100,000 rows or 32 MiB of compact row data per terminal. Refresh retains it; a supervisor crash or machine restart does not. Workspace/session/client buffers are bounded; Git output is limited to 512 KiB. Diagnostic logs retain eight files per component, capped at 256 KiB each. These limits and compatibility gaps are tracked rather than hidden behind a full-parity claim.

## Cursor redraws

Flere hides the physical cursor while painting changed rows and restores the
native cursor only at its final position. Frames use synchronized output (DEC
2026) where the outer terminal supports it. Terminals that ignore that mode still
receive cursor hiding before paint; unchanged frames emit no cursor traffic.
This fixes Flere's visible paint-position jumps. Physical Windows/SSH behavior
still needs checking on the user's terminal; it does not claim emulator parity.


## Image paste over SSH (experimental)

`flere-connect` is an optional local companion for an already-running remote
Flere supervisor. Copy a screenshot and paste while the native Codex input is
focused. The companion reads the local image, transfers it privately through
OpenSSH, then queues the completed image path as a native bracketed paste.
Codex's supported composer turns that path into an image attachment. Check the
attachment before sending; Flere never presses Enter for you.

On your local machine, use the same SSH host alias you already use:

```text
flere-connect HOST --remote /path/to/flere-parity --state /path/to/flere-state
```

Windows uses built-in clipboard and PNG encoding APIs; no Xvfb, Python, Node,
image runtime or remote display is required. The local executable needs installed
OpenSSH (`ssh`). The remote workbench can run on Linux x86_64 or a supported Mac. Ordinary SSH clients
without the companion do not acquire bitmap clipboard support.

Ctrl+V works when the terminal forwards it. The terminal's paste shortcut also
works when it emits an empty bracketed paste for an image, as supported by the
inspected Windows Terminal implementation. Terminals that consume the gesture
without emitting input need a key binding that forwards Ctrl+V. Text bracketed
paste remains ordinary text. The macOS companion reads native PNG/TIFF on demand.
The Linux helper can use an already-installed `wl-paste` or `xclip`; neither is
installed automatically. `--image LOCAL.png`
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

Build the Linux companion with
`cargo build --offline --release --manifest-path companion/Cargo.toml`.
Windows cross-build evidence, native compatibility evidence and the authorized
remote-development backlog are in [REMOTE-DEVELOPMENT.md](../../REMOTE-DEVELOPMENT.md).
This implementation has real local companion/supervisor/PTY test coverage with
harmless native stand-ins. Physical Windows clipboard → SSH → Codex remains an
acceptance check; the Windows executable has been cross-built, not run here.


## Local terminal images

Run `flere` normally in **Ghostty** on macOS. The pet, repository icons and
PNG/JPEG previews render directly in the terminal using the Kitty graphics
protocol. **No SSH connection or companion is needed.** Flere probes the
terminal and its pixel cell size; terminals without that support retain text
icons and the compact half-block pet.

Use **Ctrl+Space, l** to focus Files, **j/k** to select an image, and **Enter** or
**p** to preview it. **q**, **Escape** or **Ctrl+Space** closes the preview without
sending that key to the underlying shell. **Ctrl+Space, O** opens pet controls.
Resize, overlays, detach and frontend refresh clear only Flere's own images.
Shells and native agents keep their existing processes and drafts.

Pet frames use full RGB pixels on a canvas up to 640×160, compressed with system
zlib; identical frames are skipped. Repository assets are cached and transmitted
once per attachment. macOS ImageIO decodes previews on a worker, preserving
transparency and aspect ratio, with a 1600×1200 display bound, 20-million-pixel
source limit and 20 MiB file limit. Transfers are inline, in bounded chunks;
paths and native terminal escape sequences are never forwarded as graphics.

The original macOS path uses ImageIO, CoreGraphics and zlib. The current core also
has approved portable PNG/JPEG codecs, including bounded Linux JPEG decoding.
Linux builds need the zlib development library. The Unix companion shares the
portable decoder for Sixel images; its display path still needs Sixel and cell
geometry replies. Bitmap clipboard upload remains a separate companion action.

Protocol references: [Ghostty graphics support](https://ghostty.org/docs/features)
and the [Kitty graphics specification](https://sw.kovidgoyal.net/kitty/graphics-protocol/).

## Real image previews (0.2.16)

With the current **flere-connect** and remote Flere,
use **Ctrl+Space, l** to focus Files, select a PNG/JPEG with **j/k**, and press
**p**. The image opens in a full-screen viewer. **q**, **Escape** or **Ctrl+Space**
closes it and consumes that key. Enter also previews images; Shift+E in Files
explicitly opens the file in the editor.

The first display path targets **Windows Terminal 1.22+** with Sixel support.
The companion detects Sixel and queries its cell pixel grid; a terminal that
cannot report both gets a text notice. Images fit the window without changing
their proportions or upscaling, and resize with it. Transparency uses the dark
Flere background. Display uses a dithered 216-color palette, with a maximum
preview of 1280×720, 20 million source pixels and a 20 MiB file limit.

The original Windows implementation added no decoder dependency: it decodes
PNG/JPEG through built-in GDI+/COM, and Flere's Rust code encodes Sixel. The
current Linux/macOS companion uses approved cached portable codecs. The file travels over the
existing SSH connection only after you press p; preview bytes stay in memory.
Closing, switching the exact remote target, refreshing or disconnecting clears
the preview. Shells and native chats continue running, and modal input is never
submitted to them. Native applications cannot draw arbitrary images through
Flere's emulator.

Sixel previews now work through the companion on Windows/Linux/macOS when the
terminal reports the required capabilities. Plain SSH has no image transport;
direct local attachments use Kitty as described above. The companion has no Kitty
or iTerm-specific image path. The current protocol is v6; unsupported combinations
fail before native input. Physical Windows rendering, clipboard and SSH acceptance
still need a Windows check; cross-builds and protocol/renderer tests do not establish it.


### Image opening and frontend diagnostics (0.2.19)

In Files, **Enter** previews PNG/JPEG images through the local graphics path
or the Windows companion.
**p** remains a preview shortcut; **Shift+E** explicitly opens the selected file
in the editor. Other files and directories retain their normal Enter behavior.
A click without dragging on a visible local PNG/JPEG path in terminal text opens
the same preview; drag-to-copy is unchanged. Absolute, `~/` and `./` paths are
supported, including spaces and visible wrapped paths. Truncated paths and hidden
link targets cannot be reconstructed; URLs/OSC links never launch programs.

While viewing an image, **h/k/Left/Up** selects the previous PNG/JPEG in the same
folder and **l/j/Right/Down** selects the next. The viewer shows the filename and
position, such as **2/3**, and wraps at either end. Navigation keys stay inside
the viewer. Loading runs off the UI loop; closing or changing the exact terminal
target discards stale results. **s** still copies the full Flere view.

The renderer explicitly positions Unicode cells and the runs following them,
preventing cumulative width differences from shifting native text into the
inspector. Windows fonts/grapheme shaping can still differ from the Linux grid.

Diagnostic logs now retain connection, resize, preview, refresh, exit and error
events, plus panic source locations. They contain no terminal output, typed text,
clipboard payloads or image bytes. Logging is best effort and never prevents
attachment. Each component retains eight files, with each file capped at 256 KiB
(old entries within a full file are discarded to keep recent errors). Linux files
are 0600 inside a 0700 directory. Windows logs inherit the user's LocalAppData ACL.

- Remote UI/bridge/supervisor: `STATE/diagnostics/` (this instance: `~/.local/state/flere-parity/diagnostics/`).
- Windows companion: `%LOCALAPPDATA%\Flere\logs\`.
- Linux companion: `$XDG_STATE_HOME/flere-connect/diagnostics/`, or `~/.local/state/flere-connect/diagnostics/`.

The companion prints the exact log path after disconnect; remote errors print
their log path too. The prior unexplained frontend exit cannot be reconstructed
from the old empty supervisor log. All sessions survived it.

At 0.2.19, these image-opening fixes retained protocol v2 and added local diagnostics.
The current combined build uses v6 and requires the current companion.


### Project image badges (0.2.21)

Cards show the repository's own logo at one title row's pixel height, preserving
its original aspect ratio. The title moves over by the width the logo needs. Very
wide logos scale down to fit the available card width; tiny images are not upscaled.
Workflow and activity symbols remain separate. Put a PNG/JPEG at `.flere/icon.png` or
`.flere/icon.jpg` in the repository root to choose its badge. Without an override,
Flere checks common logo paths in order: `logo.png`, `docs/logo.png`,
`assets/logo.png`, `assets/icon.png`, `frontend/public/logo.png`, `public/logo.png`,
`website/static/img/logo.png`, `docs/images/logo.png`, then PNG favicons in
`frontend/public`, `public`, and the root. Project PNG/JPEG logos in those locations work directly. Other names and SVG-only logos need the PNG/JPEG override.
Images must be regular files inside the repository, at most 256 KiB and 1024×1024
pixels. Reconnect to pick up a changed logo. Missing/invalid images and ordinary
SSH use project initials. Issue/PR authors and personal GitHub avatars are not
used for project identity.

Local Kitty-capable terminals use their own pixel cell metrics. Over SSH, use
**the current companion** for height-driven icons, including narrow fonts. Repository images travel over
the existing authenticated SSH connection without GitHub requests or login.
The companion uses native Windows or portable Unix decoding and its Sixel renderer.
At most 64 distinct images are held in memory per
attachment; they are not saved to disk. Repeated cards share the same image,
including across worktrees. Only visible images are transferred. Resize, font
changes, overlays, preview and detach clear/repaint at the current card positions.

Protocol v4 originally added cell dimensions and variable-width icon slots; the
current combined protocol is v6. Use the coordinated update flow to verify the
core/companion pair, then observe the activation result for each component.
Linux/macOS companions can display these images in capable Sixel terminals.
Updates preserve native chats. Reference renders and cross-builds do not replace
physical Windows acceptance.


### Interactive inspector pets (0.2.25)

Press **Ctrl+Space, then Shift+O** to open pet controls, or click its header.
Flere is the default; explicit saved choices stay unchanged. **h/l** or arrow keys
cycle characters; **1** selects Duck, **2** Robot, **3** Cat and **4** Flere.
The choice is remembered. **Ctrl+Space, o** still toggles the
window, which defaults off until enabled or opened explicitly.

While pet controls are focused, use **p** (or Space) to pet it, **t** to toss a toy,
**k** for a trick, and **n** for a nap. Clicking the compact dock first opens controls; a quick click on the expanded sprite pets
it; hold for a quarter-second to pick it up, move to swing it, then release to
let it fall. Empty space does nothing. You can also use
the labelled buttons below it, or the header arrows to change characters. **Esc**
returns to the previous navigation/native-input mode; clicking another pane also
leaves pet controls. Play keys and pasted text in pet controls are consumed by
the UI. They cannot issue shell commands or operate native agents.

Press **t** to create a ball. Grab it and flick to throw; pull it back, pause
briefly, then release for a slingshot launch along the dotted tension line.
Gravity, momentum, wall/floor bounces and friction make it settle naturally.
The pet follows smoothly; the settled ball stays available to pick up again.
Mouse reports have terminal-cell precision, even with detailed pixel graphics.
Dragging outside the dock stays captured until release or Esc; resize, font-size
and workspace changes cancel a grab safely. Chat clicks and text selection keep
the pet visible.

Flere floats with animated fields; petting produces either a satisfied warm glow
or annoyed white flecks. Reduced motion keeps the response static until it expires.
The other pets show hearts when petted, tricks add a hop and sparkles, and naps
show a sleeping pose. A new interaction replaces the previous reaction;
automatic activity reactions resume afterward. Working sessions pace, Needs me
gets an alert pose, and idle pets stretch, groom and sleep. Changing workspaces
cancels the previous card's play reaction. The idle dock reserves six inspector rows and expands to eight while controls
are focused. It uses existing task metadata without reading chat contents.

Local Ghostty/Kitty graphics display full-colour pixel sprites directly. Existing
**0.2.21 Windows companions** also display them through negotiated Sixel support;
no companion upgrade is needed. Flere uses Flere's procedural disc/field renderer.
The yellow duck uses an embedded pixel grid; the robot and cat use an embedded
palette/RLE asset. Plain SSH
uses a colour half-block version. Motion targets a 40 ms cadence; actual smoothness
depends on the terminal and connection. Menus, previews, hidden inspectors and
small layouts hide the pet. Reduced motion freezes animation; explicit reactions
remain static until dismissed from controls or replaced. Reduced motion allows
direct positioning while held, without animated flights or swings.

The standalone workshop runs without a supervisor or shell:

```sh
cargo run --offline --release --example pet_workshop
```

Use **h/l** to choose a character, **1–8** for an action, **Space** to pause,
**s** for quarter speed and **q** to exit. Add `-- --sixel` in a Sixel-capable
outer terminal. `-- --dump ~/.cache/flere/tmp/pet-frames` exports all character
frames. The optional `examples/generate_pet_sprites.py` authoring script uses
Pillow to regenerate the legacy palette asset; the current duck is authored in
`src/pet/duck.rs`. Ordinary Rust builds never run the Python tool.

### Terminal links and confirmation buttons (0.2.26)

Click an existing absolute, `~/`, or relative file path in terminal text to open
it in an editor tab in the current workspace. Relative paths use the workspace
directory. Quoted names, escaped spaces, wrapped paths and command-line
punctuation are recognized without executing the surrounding text. Existing
editor tabs are reused; PNG/JPEG files retain image preview. Dragging selects
text instead. Links come only from painted text, never hidden OSC targets.

Clicking a directory opens **Add project?** with its path. Confirming creates a
stopped workspace card with the directory name as its project, or focuses the
existing card for that directory. It starts no shell or agent. **Cancel** is the
default. **Tab / Left / Right** choose a button; **Enter** activates it and
**Escape** cancels. The close-terminal dialog uses the same controls, a warning
and distinct **Cancel / Close terminal** buttons. A mouse press and release must
land on the same button; dragging away cancels that press. Pasted text and wheel
events stay in the dialog. Close still addresses the original exact terminal/run.

The **0.2.26 companion** fixes pixel-cell widths below 10 being incorrectly
clamped as terminal-grid dimensions. For a 7×19 pixel font, the tested PNG/JPEG badges
now reserve three columns and fit at 19 pixels high with their original aspect,
instead of shrinking into a two-column slot. Unchanged badges survive terminal
row updates. Pixel metrics are recorded in the existing diagnostic logs.
The local companion and remote Flere both need the current implementation for
this sizing fix. That historical change retained v4; screenshot export followed
in v5, and the current coordinated-update/file-tools protocol is v6. **Ctrl+Space,
Shift+K** now prepares and reviews the pair through the companion before apply.

Current development builds check the installation owner of both remote processes
and the local companion before preparing the update. A package-manager copy shows
upgrade instructions; an unknown owner keeps Apply unavailable. A recognized
manual per-user copy can be adopted only through the explicit local review, with
its previous bytes retained. Ownership and the selected connection are checked
again before installation. Both candidate programs must support this flow; older
endpoints or candidates need a manual upgrade first, even when their version
number matches. Native Linux manager and Windows runtime acceptance of these new
checks remain pending.
