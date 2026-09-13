# Platform and native acceptance

This checklist separates implemented behavior and retained evidence from checks
that require the user's terminal or native agent. It does not authorize opening
new chats, approving hooks, changing trust, or sending messages automatically.

## Evidence and remaining limits

| Area | Evidence available | Still needs a real environment |
| --- | --- | --- |
| macOS arm64 core | Real shell/UI attach, detach, refresh and draft retention; actual split-pane PTY tests and Flere-rendered frames. See [MACOS.md](../MACOS.md). | Exact appearance/key behavior in the user's terminal; native-agent acceptance below. |
| Linux x86_64 core | GNU runtime suites and isolated SSH bootstrap checks passed during Flere 0.3.0 validation; see [release scope](releases/0.3.0.md). The [validation record](releases/0.3.0-validation.md) gives the exact scope. | Desktop clipboard ownership on the chosen X11/Wayland session and visible terminal/native-agent checks below. Historical package counts are not current-build results. |
| Intel/older macOS | Intel cross-compilation, target-specific Darwin bindings. | Runtime on an Intel Mac and the oldest macOS version the owner intends to support. |
| PNG/JPEG rendering | Bounded portable decoder tests cover alpha, PNG/JPEG, fitting, truncation, excessive bytes and pixel dimensions. Preview protocol tests cover ownership, resize and cleanup. | Actual font/cell sizing and visible cleanup in each chosen terminal. |
| macOS companion clipboard | A real, unique private pasteboard test reads PNG and TIFF, preserves text, and never accesses the general clipboard. | The user's copy/paste shortcut, source application and existing native chat. |
| Windows companion | Windows compilation and the [historical acceptance report](windows-acceptance-20260913.md). | The complete current-build Windows/SSH checks below. Earlier limited physical evidence did not establish draft retention, every shortcut, resize or cleanup. |
| Native delivery | Exact-target, DND, queue and receipt tests use harmless stand-ins. | Human hook/MCP trust, actual message handling and explicit recipient acknowledgement. Claude/Copilot automatic delivery remains unimplemented. |

Direct local Flere uses Kitty graphics, including Ghostty. The SSH companion
uses Sixel on Windows/Linux/macOS and requires both Sixel and cell-geometry
replies; a Kitty-only terminal is not sufficient for companion images. Linux
incoming clipboard images use an already-installed `wl-paste` or `xclip` helper.
macOS reads native PNG/TIFF on demand. Screenshot export always renders Flere's
own composed terminal view; no screen-recording or desktop capture is involved.

The [current validation record](releases/0.3.0-validation.md) separates full
baselines from final focused checks. Test coverage includes actual PTY updates,
exact process/draft retention, paired installation, isolated SSH bootstrap and
headless X11 clipboard ownership. These fixtures do not establish every physical
terminal, native harness trust prompt or public download path. Historical local
installation records and personal session inventories are not distributed here.

## Before checking

Record `flere --build-info`, `flere --state STATE build-status`, and, when
used, `flere-connect --build-info`. Also record OS, terminal version, font and
size, local/SSH route and the chosen display protocol. Build stamps identify
embedded metadata; a matching version string alone does not identify a build.
Use a known harmless PNG/JPEG and an existing chat the user explicitly chooses.

## Image paste into an existing chat

1. Leave a harmless draft unsubmitted in that chat. Copy a known image locally,
   then use the normal paste shortcut through the companion.
2. Confirm exactly one native image attachment appears in the same conversation,
   the draft remains, and no Enter or message submission occurs. Test Ctrl+V and
   the terminal's own paste shortcut separately when both are used.
3. Repeat with plain text and confirm ordinary text paste. During a separate
   image transfer, change the target or detach; the cancelled image must not
   appear in the new target after reconnecting.
4. On macOS, check both a source that supplies PNG and one that supplies TIFF.
   On Linux, record the desktop session and the installed clipboard helper.
5. With the current pair, drop one harmless PNG, JPEG and ordinary file path.
   Record whether the terminal sends bracketed paste and its actual path quoting.
   Check the LOCAL Attach / Paste as text / Cancel prompt, default Cancel, and
   fresh choice. Attach must preserve the draft and insert the completed remote
   path once without Enter; only supported image types should show an image badge.
   Text must preserve the original path text. Change focus while the prompt or
   transfer is pending, then return: neither choice may reach the old or new chat.

## Terminal images and cleanup

1. Open the PNG and JPEG from Explorer and from visible terminal paths. Check
   filename/index, previous/next navigation, aspect ratio and alpha edges.
2. Resize wide/narrow and change font size. Images and repository badges must fit
   their cells, stay aligned with text and leave the surrounding content readable.
3. Close the preview with Esc, q and Ctrl+Space in separate checks. Open/close
   menus, hover details, switch tabs, detach and reconnect. Old images must not
   remain over the restored terminal or another workspace.
4. Confirm the idle intro covers old graphics and retains its original wordmark.
   Flere drifts below it; mouse movement/clicks keep it open, petting changes its
   fields, and grabbing/throwing moves it. A keyboard press dismisses it without
   typing into the child. Check reduced motion, a narrow window and the Duck choice.
5. If a refresh/update is already authorized, observe it with a harmless draft in
   each split pane; verify the same processes/conversations and drafts survive.

## Native-agent delivery

Follow the [activation guide](guides/agents.md#activate-agent-message-delivery) in
the selected existing native conversation. The user reviews its exact native
hook/MCP definitions and trust prompts. UI refresh cannot add launch flags to an
existing process; any required exit/resume is an explicit user action.

1. Inspect messaging activation and exact native session/run ownership. Resolve
   missing configuration or ambiguous recipients before sending anything.
2. Send one harmless, user-approved message to that recipient. Check saved,
   queued/surfaced and explicitly acknowledged states separately; inspect the
   recipient's actual response before recording handling as passed.
3. Check delivery at an ordinary tool boundary and during a verified idle period.
   A draft, DND or native approval dialog must defer automatic attention without
   losing the saved message or typing a fallback into the composer.

For each result record the build identities, environment, gesture, expected and
observed behavior, and pass/fail. A cropped terminal image or Flere diagnostic
identifier is enough when useful; chat contents and clipboard payloads are not
needed in the acceptance report.
