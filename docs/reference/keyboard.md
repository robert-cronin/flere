[Documentation](../README.md) · [Quick start](../getting-started.md) · [Keyboard](../reference/keyboard.md)

# Keyboard and mouse reference

`Ctrl+Space` enters Flere navigation without sending Escape to a child. `Space` then opens the scrollable actions menu; use j/k and Enter, or its displayed shortcut. From a side pane, Ctrl+Space keeps navigation active so h/l return toward the centre. Enter returns to terminal input, except that Enter in Files opens its selection.

| Where | Keys | Action |
| --- | --- | --- |
| Navigation | h / l | Move between cards, terminal and inspector |
| Navigation | j / k | Select card/file, or move between terminal and tabs |
| Tabs in navigation | h / l | Change tab in the focused pane |
| Centre pane in navigation | Tab / Shift+Tab | Next / previous tab in the focused pane |
| Navigation | `\|` / `_` | Split right / below, moving the current tab (requires another tab) |
| Navigation | `\` / `=` | Split right / below with a new shell |
| Split terminal in navigation | h/l or Left/Right; j/k or Up/Down | Switch side-by-side panes; switch stacked panes |
| Navigation | `~` / Shift+Z / Shift+X | Move tab to other pane / zoom pane / remove split and keep tabs |
| Navigation | & | Arcade: Context Ruins; choose a mascot and explore three platforming rooms |
| Navigation | Space | Actions menu; `/` searches actions |
| Cards in navigation | j/k or Up/Down | Select workspace cards, skipping child terminals |
| Cards in navigation | Tab / ? / Enter | Enter terminal list / open details / activate workspace |
| Card terminal list | j/k or Up/Down; Enter; Left/Esc/Tab | Select terminal; activate; return to card |
| Navigation | Shift+U | Start the intro screensaver |
| Navigation | Shift+V | Cycle screensaver idle time: 5 minutes / 15 minutes / manual only |
| Navigation | Shift+C / Shift+J | Compact cards / next workspace needing attention |
| Navigation | G / n (or N) / t | Add project / new worktree card / shell tab |
| Navigation | / / P | Fuzzy workspace search / project filter |
| Navigation | Q / H | Quick Open / Find in Files |
| Terminal in navigation / Actions | ? | Find retained terminal output; Cards/Git use ? for details |
| Navigation | ( / ) / Y | Previous / next marked shell command / copy complete command output |
| Navigation | ! / ; | Run Task in a new owned terminal / inspect Problems |
| Navigation | K | Update Flere; local apply or companion prepare/review/apply |
| Navigation with companion | 6 / 7 / 8 / 9 | Upload local file / download selected file / open selected file locally / Ports |
| Navigation | Shift+L | Group sidebar by Status / Project; Pinned stays first |
| Navigation | T / w | Workspace tabs / all live terminals |
| Navigation | b / f, or < / > | Back/forward through exact workspace, tab and run history |
| Navigation | i / g / v | Cycle inspector / Git / Details |
| Navigation | e / p | Edit workspace metadata / pin |
| Navigation | B | Board; h/l selects column, j/k selects card, Enter returns |
| Navigation | 1 / 2 / 3 / 4 / 5 | Needs me / In progress / Todo / Waiting / Done |
| Navigation | S | Start agent: choose a harness (no conversation-ID field) |
| Navigation | W | Retry saved tabs for the selected workspace |
| Navigation | x | Close exact terminal; ask when work or unknown state needs protection |
| Navigation | s | Copy the full Flere view as a PNG to the local clipboard (SSH: updated companion) |
| Navigation | A / a | Archive stopped workspace / restore archived workspace |
| Navigation | z / r / R | Zoom / reset layout / refresh keeping sessions |
| Navigation | [ / ], { / } | Narrow/widen left and right panes |
| Navigation | M / F | Reduced motion / refresh files and Git |
| Terminal in navigation | Ctrl+U / Ctrl+D | Scroll exactly one visible terminal page, with smooth motion |
| Terminal in navigation | u / d | Smooth scroll up/down three rows; hold to continue |
| Terminal in navigation | Ctrl+↑ / Ctrl+↓ | Full-page aliases |
| Terminal in navigation | Ctrl+Y / Ctrl+E | Scroll up/down one line |
| Terminal in navigation | Page Up / Page Down | Scroll a full page with one-row overlap |
| Terminal in navigation | Home / End | Oldest retained output / live bottom; stay in navigation |
| Navigation | c | Page into terminal scrollback and leave navigation |
| Navigation | m / I / D | Send message / read messages / review decisions |
| Navigation | q | Detach, preserving processes |
| Stopped terminal | S / W / Enter / t / Space | Start agent / retry saved tabs / reopen tabs or offer start / shell / actions, without a navigation prefix |
| Terminal | Wheel, Shift+Page Up/Down | Browse retained terminal history |
| Web links in current `main` builds | Outer terminal hyperlink gesture (Ghostty: Cmd-click on macOS, Ctrl-click on Linux) | Open the target; requires newly emitted or redrawn OSC 8 output |
| Terminal / scrollback | Shift+Home / Home | Jump to the oldest retained output |
| Scrollback | Wheel, Page Up/Down, arrows | Scroll without sending keys to the chat |
| Scrollback | Esc, End, Enter | Return to live output without sending that key |
| Files | j/k, wheel, Home/End, PgUp/PgDn | Select entries |
| Files | Enter / p / Shift+E | Open or preview / image preview / explicitly open in editor |
| Files | - / Backspace | Parent directory |
| Files | r | Retry or refresh directory loading |
| Details | j/k, wheel, Home/End, PgUp/PgDn | Scroll card details |
| Image preview | q / Escape / Ctrl+Space | Close without sending that key to a child |
| Image preview | h / k / Left / Up | Previous PNG/JPEG in the same folder |
| Image preview | l / j / Right / Down | Next PNG/JPEG in the same folder |
| Image preview | s | Copy the full Flere view as a PNG |
| Git inspector | j/k, arrows, PgUp/PgDn, Home/End, wheel | Navigate working changes and history |
| Git inspector | Enter / click | Expand commit or open historical file diff in editor |
| Git inspector | d / u / ? | Side-by-side file diff / patch preview / full message |
| Git inspector | G / F | Commits heading / refresh |
| Preview | j/k, Page Up/Down, q | Scroll wrapped text; return without acknowledgement |
| Message / decision review | a | Acknowledge this message / open decision answer form |
| Form/picker | Tab, arrows, Ctrl+U, Enter, Esc | Field/result, clear field, apply, cancel |
| Search | Up/Down, Enter, Ctrl+C, Esc | Select result, open/jump, copy result, cancel |
| Problems | Tab, j/k, Enter, Esc | Cycle task, select location, open in editor, close |
| Context menu | j/k or Up/Down, Home/End, Enter, Escape | Select action, first/last, apply, dismiss |

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
draining while your displayed page stays fixed. Each exact tab retains its displayed scrollback while switching tabs or panes; resizing that pane returns it to live output. Selection/copy works on the history you are viewing. Scrollback
retains up to 100,000 rows within a 32 MiB compact-data budget per terminal
(whichever fills first); it preserves styles and does not reflow old lines when
width changes. Lines an older version already discarded cannot
be recovered. Wheel over a text preview scrolls that preview. Wheel over other
panes does not scroll or send input to a chat.

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

Mouse clicks select cards, tabs, files and board cards; pane borders drag. **Click and drag inside the terminal or its scrollback preview to select text; release to copy automatically.** Flere highlights the selection and uses OSC 52 to send text to the outer terminal's clipboard over SSH. A plain click leaves the clipboard alone; clicking a visible local PNG/JPEG path opens the image preview when a capable companion is connected. Escape clears the selection without reaching the native chat; typing clears it and continues normally. Text stays stable while you drag, while the process continues running. Resizing or changing the exact workspace/session cancels the pending selection. Copies are bounded to 64 KiB and use visible line breaks; automatic scroll-while-dragging and joining soft-wrapped lines are not yet supported. The outer terminal must allow OSC 52 clipboard writes; Flere reports sending the selection but cannot confirm the OS clipboard accepted it. Holding the outer terminal's text-selection modifier (usually Shift) still uses its own selection behavior. Native child mouse protocols are not forwarded yet. Ctrl+Space is reserved; outer tmux/terminal bindings may consume it before Flere sees it.

**Right-click** a workspace card, terminal tab, file entry, Git row or terminal area to
open its context menu. Opening a menu keeps the current tab selected. Click an
action or use **j/k**, **Up/Down** and **Enter**; **Escape** or a click outside
dismisses it. Menu input stays inside Flere. Close actions retain the existing
checks for unsaved work and running processes.

See [workspace details](../workspace-details.md) for the overlay and notes bindings, and the [runtime guide](runtime-guide.md) for current Git tree and pet controls.

Search, task forms and Problems consume their input. Tasks run only after Enter,
in a new supervisor-owned terminal; restoring saved work never replays a task
command. Command navigation/copy needs shell annotations; unmarked or incomplete
output is not guessed. Remote file/port actions open a local companion prompt and
require explicit local confirmation. See [setup and updates](../getting-started.md#update-from-flere).

## Arcade: Context Ruins

Choose **Actions → Arcade: Context Ruins** or right-click the dock mascot and
choose **Play Context Ruins**. Select an avatar with h/l or arrows and Enter.
Move with Left/Right or h/l, climb ladders with Up/Down or k/j, and jump with
Space. Down away from a ladder drops through a ledge. Collect keys to open doors
across three rooms; nine relics are optional. You have three lives and no timer.

P or Esc pauses; P or Enter resumes. R retries the checkpoint without costing
a life. Q exits from the picker, pause screen or results. S copies the complete
game view as an image. Focus loss and resize pause play; the minimum window is
40×20, with a camera that follows the avatar. Enlargement does not resume it.
The game has no access to real PRs, repository files or native session input.
