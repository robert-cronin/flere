# Ghostline sidebar

The implemented sidebar uses square workspace cards, filled group headings,
fixed text gutters and a distinct current-tab row. The geometry study below is
historical; the live implementation and its tests are authoritative.

## Selected implementation

The live sidebar implements B with square cell borders around each workspace and
its expanded tabs, filled group headings, a fixed logo/text gutter and a distinct
current terminal row. Titles use one row; metadata and workflow/activity have
their own rows. Compact cards keep two content rows. The board retains its
existing layout.

The header switches between **Status** and **Project** grouping; **Pinned** stays
first. The same choice is available as **Shift+L** in navigation or through the
actions menu. Grouping persists, project/status folds remain independent, and
folded group counts continue to reflect all their cards.

Git's original worktree gets a small shaded **primary** badge beside the workspace
name. Narrow cards place the badge on the bottom border to keep titles readable.
The metadata row stays dedicated to project/branch context. Details also shows
**primary**. This is an asynchronous directory check, independent of branch name,
pinning and card order.

Image previews also support **h/k/Left/Up** and **l/j/Right/Down** to browse
alphabetically through sibling PNG/JPEG files, wrapping at either end. The viewer
shows the filename and position. Screenshot copying includes the selected image.

## Implementation constraints

Flere remains terminal-only and owns its renderer/layout. Flat background
fills, fixed-width text and square cell rules are feasible; native widgets,
rounded pills, blur and shadows are not part of these proposals. Existing
keyboard navigation, scrolling, fold state and mouse hit regions must follow the
same row model. Workspace selection, keyboard focus and child activity are
separate states. Keep hidden/long labels accessible without inventing shortcuts.

The implementation preserves session identities, processes, drafts, workspace
ordering within groups and status semantics. Real PTY regressions cover 46-, 28-
and 16-column sidebars, counts, folding, exact child targets, inert borders,
scrolling, resize and draft preservation. Validation captures can be reproduced
without desktop capture or clipboard access:

```sh
FLERE_TEST_SIDEBAR_ARTIFACTS="$HOME/.cache/flere/tmp/sidebar-validation" cargo test --offline --test live sidebars:: -- --nocapture
```

## Reproduce the layout study

The [plain text grid](terminal-section-bars.txt) is a generated example with
illustrative workspace labels. It demonstrates wide and narrow fixed-column
layout; it is not a screenshot of personal work. Reproduce its cell-rendered
image from the [example source](../../../examples/sidebar_study.rs):

```sh
mkdir -p "$HOME/.cache/flere/tmp/sidebar-study"
cargo run --offline --example sidebar_study -- "$HOME/.cache/flere/tmp/sidebar-study/terminal-section-bars.png"
```

The [historical geometry audit](layout-audit.md) records the cell budgets and
selection/folding constraints used during design. Private conversation excerpts,
user screenshots and the original design-session provenance are not included.
