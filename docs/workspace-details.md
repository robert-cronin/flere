# Workspace trees and details

Press **Ctrl+Space, h** to enter Cards navigation. **j/k** or **Up/Down** selects whole cards, even when their terminals are expanded. **Tab** enters the selected card’s terminal list; **j/k** or **Up/Down** then selects a terminal, and **Enter** activates it. **Left**, **Esc** or **Tab** returns to the card. Expansion is remembered per workspace. Hover never expands or reorders cards.

Selecting another workspace header previews that workspace, preserving Flere's existing navigation behavior. Moving through terminal children does not change the active terminal until Enter. A `>` and cyan rail identify the keyboard selection; `*` identifies the active terminal when its row is not focused. Child rows show the recorded title/kind and running, working or stopped state. “Working” remains Flere's existing observation, not a model identity or a completion guarantee.

Press **?** in Cards to open details for that workspace. The panel stays pinned to its target until you close it. **Esc** returns to the original pane and card selection. Moving the pointer over a card for 450 ms shows a bounded tooltip beside the pointer/card, keeping the card logo and inspector visible when space permits: native typing continues normally and dismisses the preview. Clicking an issue or PR action opens that link immediately; clicking elsewhere inside pins the panel. Move away to dismiss, with a short grace period for entering the preview.

Issue and PR actions appear at the top of the panel, followed by the workspace, project, directory, recorded branch/base, workflow, terminals and notes. Available GitHub issue/PR links get titles and states when `gh` can read them. PR checks include passed, failed, pending and unknown counts; absent checks and reviews are explicitly identified. Missing links have no action. Other URL formats are displayed as unsupported and are never opened.

| In focused details | Action |
| --- | --- |
| j/k, arrows, Page Up/Down, wheel | Scroll content |
| Tab / Shift+Tab | Next / previous action |
| Enter | Activate the highlighted action |
| i / p | Open the linked issue / PR in your browser, including on the local computer for SSH attachments |
| y / Shift+Y | Copy issue / PR URL |
| d | Copy directory path |
| e | Edit notes |
| r | Refresh GitHub details |
| Esc | Close and return |

The issue/PR rows at the top and the highlighted action row are clickable. To open a link entirely from the keyboard, press **Ctrl+Space, h**, select the card with **j/k**, press **?**, then **i** for its issue or **p** for its PR. **Tab** and **Enter** also select and activate an action. The footer's Next and Close labels are clickable; a link or clipboard action requires a matching press and release on its visible action. Outside clicks dismiss without passing through to the terminal. At small sizes the panel occupies the available workbench and retains a readable body row plus reachable actions. Keyboard operation does not depend on pointer-motion support or color.

Notes use **Ctrl+J** for a newline, **Ctrl+U** to clear, **Ctrl+Y** to copy your edit, **Enter** to save and **Esc** to cancel. The Save and Cancel rows are clickable. Multiline paste is supported up to 64 KiB. Saving patches only notes and checks that they have not changed since editing began. If someone else changes them, your edit stays available: copy it, cancel, reopen, and reconcile the two versions. Other metadata changes are preserved.

GitHub loading uses your installed `gh` and its existing authentication. Only explicit opening or refreshing starts requests; hover uses local/cached data. There is at most one worker, with at most two pending links, five seconds per request, 256 KiB per output stream and a 64-entry cache. Successful entries are reused for 60 seconds and failures for 10 seconds. Closing/changing the target cancels its work; stale results cannot replace another target. Missing `gh`, authentication, rate limits, network errors and malformed output leave the local panel usable. No account setup or browser opening occurs automatically.

Only canonical `https://github.com/OWNER/REPO/issues/NUMBER` and `/pull/NUMBER` targets can open. The OS opener receives the validated URL as a direct argument, without a shell. With an updated SSH companion, the browser opens on the computer running the companion, immediately from your click or shortcut. The companion validates the displayed GitHub target and requires fresh local input; remote output, paste and background activity cannot open a browser. View changes, navigation, resize and reconnect invalidate stale targets. Older companions retain the copy-URL fallback. This does not change the local confirmations for file transfers or forwarded ports. Clipboard actions use bounded OSC 52; the outer terminal must allow clipboard writes, and Flere cannot confirm that the OS clipboard accepted them.

The new UI uses additive exact-focus and notes commands. Refresh the installed Flere with **Ctrl+Space, Shift+R** before using it against an older running supervisor. Unsupported commands fail visibly rather than falling back to a stale session. Expansion preferences default empty when loading existing settings; native children are never started by expansion, hover, details or notes.

The Files pane loads directories in the background. A slow mount or macOS folder-access request leaves workspace navigation and native sessions usable. If loading fails, the pane shows the error and retains parent navigation. Focus Files and press **r** to retry, or **-** to move to the parent directory. On macOS, a permission-denied message may require allowing your terminal app access under **System Settings → Privacy & Security → Files and Folders** before retrying.
