# Ghostline sidebar: read-only layout audit

Historical geometry analysis before the selected workspace-card implementation. Source line references describe that earlier layout. See the [current sidebar design](README.md) and live tests for the implemented behavior.

## Exact current geometry

The attached PNG is 3540×1760 and explicitly shows a 236×55 terminal grid: 15×32 pixels per cell. The sidebar ends at x=735 px (49 columns), its separator occupies column 48, column 49 is the pane gutter, and terminal content starts at column 50. Thus the current screenshot has a **49-column sidebar**, not the 26-column default. All following columns are zero-based.

- Layout source: `src/ui.rs:74`. Width <56 hides the normal sidebar and Cards focus uses a full-width overlay. Width 56–99 constrains the sidebar to 16–30 cells; width ≥100 allows 16–70, further bounded by `(screen_width - 42) / 2`. Default left width is 26 (`src/workspace.rs:134`). A 48-column wide sidebar is feasible from screen width 138 upward.
- Row 0: app bar; row 1: rule; row 2: Workspaces + total; groups start row 3. Content has `height - 5` rows. At 55 rows that is 50 scrollable rows. Bottom rule is row 53; app notice is row 54.
- Normal workspace: 3 text rows. Compact: 2. Each expanded child adds 1 row. Every workspace then adds one inert Gap. A heading adds one row; there is no explicit group-end row. Source: `src/ui/workflows.rs:187`, `src/ui/polish.rs:5`.
- Heading disclosure column 2, heading text column 4. One-digit group count column W−4, while the top total and card `?` end at W−3. They are actually misaligned by a cell.
- Workspace selection marker column 1; disclosure column 2; logo/initials column 3. With default two-cell initials title begins column 6. A square image at this screenshot's cell aspect usually requires three cells and moves the title to column 7. Wide logos can move it farther. Source: `src/ui/chrome.rs:203` and `src/avatar.rs:62`.
- Branch/project metadata begins column 3. A wrapped second title line begins at the title column and replaces metadata entirely. Runtime glyph column 3, runtime text column 5. Child connectors columns 2–3, current-child `*` column 4, child label column 6. These are separate alignment systems.
- At W=49, normal two-cell-logo title width is 38; three-cell-logo title width is 37. At default W=26 they are 15 and 14. Compact removes four more title cells (11 or 10 at W=26).
- Child labels reserve the full trailing word `running`, `working` or `stopped`. At W=26 the label has just 10 cells, versus 33 at W=49. The quiet but repeated `running` column competes with useful tab names. Source: `src/ui/cards.rs:305`.

## Alignment and grouping issues in the earlier layout

1. Title position is logo-dependent, and metadata, status, child text and heading text start at different columns. Some hierarchy is appropriate; this many unrelated anchors is not.
2. Every inactive card and every group body uses the same PANEL (#09121B). Group headings only change text color, with no bold weight, background or lower closure. `tint()` already defines status background colors, but the card renderer explicitly discards that second result. See `src/ui/chrome.rs:157`, `src/ui/chrome.rs:457`, `src/ui/workflows.rs:96`.
3. The active workspace gets only a slight SURFACE (#0C1B25), versus PANEL (#09121B). Strong ACTIVE_BG (#0E2A37) and cyan rail appear only while Cards has keyboard focus. In the screenshot the native terminal owns focus, so the active card is weakly highlighted.
4. Children return to PANEL regardless of the active parent, and the current child only gains a small cyan `*`; the parent highlight visually stops before its own children. See `src/ui/cards.rs:266`.
5. The one blank row between sibling cards is also the only space at a section end. In a group with multiple cards, there is no stronger cue showing which gap ends the group.
6. The activity glyph prioritizes Working over Needs me, while the runtime line substitutes Working and agent text for workflow context. Pinned cards are grouped outside their workflow, so a working pinned attention card needs an independent attention cue; activity must not erase it.
7. The `?` details control and title-row right edge are always present but visually tiny. Current mouse hit areas are larger than their marks: disclosure is x≤2, details x≥W−4, whole heading toggles a group. A redesigned gutter must update these together.
8. A row offset can scroll a heading away while keeping its children visible. Explicit group identity/continuation treatment would preserve meaning on a partial section.
9. The Workspaces number currently counts cards emitted by `side_rows()`, so collapsing a group changes that total. A redesign should decide whether the top number means all filtered workspaces or visible expanded rows; all filtered workspaces is the clearer workspace count.

## Shared implementable constraints

- Design in whole terminal cells, monospaced type, solid RGB cell fills, existing Unicode or ASCII fallback glyphs, and ordinary box-drawing rules. No rounded CSS pills, proportional type, subcell decorative gradients, floating controls or graphical UI dependencies.
- Keep current Ghostline colors: PANEL #09121B, text #D8E2EE, muted #7A8CAF, cyan #43E3F7, working blue #68ACFF, amber #FFCD6E. Status backgrounds can reuse the existing palette in `tint()`.
- Fix the logo slot for every workspace within a given layout. A three-cell × one-row slot supports the existing square-logo geometry at the screenshot's 15×32 metric. Fit all images proportionally within that slot, no stretch/crop, and show two initials centered in the same slot when unsupported/missing. A very wide logo becomes smaller, not a reason to shift one title.
- Titles and metadata share a single text anchor. Child text is intentionally two cells deeper; connectors occupy the icon gutter. Counts and details controls share one right edge.
- Group membership, visible rectangles, selection following, scroll continuation, keyboard navigation and pointer targets must come from one layout description. Headers, section-end rules and spacers stay inert except the explicit collapse header.
- Preserve exact epoch/workspace/tab/run targeting; navigation never forwards input to a child. Preserve retained stopped and failed tabs, native picker rules, manual statuses, peer equality, and pinning as presentation. The existing screenshot's user-given name `Lead` should not become a role in a mockup; use a neutral workspace name in synthetic examples.
- Separate persisted active workspace/tab from temporary keyboard cursor. Keep the active context visible when focus returns to the native terminal; make keyboard focus a brighter caret/rail on the cursor target.
- Working: a blue animated glyph plus Working text where space permits; reduced motion uses a static glyph. Needs me: stationary amber diamond and/or amber section, retained independently when working. Open but not working: quiet live marker and current `running` label in wide mode; stopped: muted hollow circle plus stopped. Selection remains cyan and never changes workflow meaning.
- Pinned needs its own workflow marker because membership there does not imply workflow. Main checkout remains a directory/context label, never a privileged role.

## Direction A — section bands with connected workspace blocks (recommended)

Full-width, bold status header on a subtle tinted background; continue a quieter version of that surface through the group; close it with an explicit horizontal rule and one outer blank row between groups. Keep three-row workspaces. Use one blank row only between siblings. This gives unambiguous section start/end with just two more rows than the current screenshot's three-group layout, because a closing rule replaces each group's final Gap.

Geometry for both W=26 and W=48:

| Slot | Column(s) | W=26 | W=48 |
| --- | --- | --- | --- |
| Outer separator | W−1 | 25 | 47 |
| Highlight / keyboard-cursor gutter | 1 | 1 | 1 |
| Workspace disclosure | 2 | 2 | 2 |
| Logo slot | 4–6 | 3 cells | 3 cells |
| Title and metadata anchor | 8 | 8 | 8 |
| Title ending before details | W−5 | 21 (14 title cells) | 43 (36 title cells) |
| Details and count right edge | W−3 | 23 | 45 |
| Child connector | 4–6 | same | same |
| Child type marker | 8 | same | same |
| Child text | 10 | 13 cells through 22, runtime glyph at 23 | 27 cells through 36, 8-cell runtime field at 38–45 |

Header fold at 2, label at 4, count such as `[1]` ending at W−3. Metadata uses up to W−10 cells (16 narrow / 38 wide). Use one-line ellipsis in the narrow mockup so every card has stable name/context/runtime rows; show full text in existing details. A row may wrap only under an explicit shared policy, without shifting the text anchor.

Active workspace: subtle cyan block across its own three rows **and its children**, with the left cyan rail continuous through that block. The active tab gets a stronger cyan row fill. Keyboard focus adds a brighter caret at the specific cursor row. Needs me continues to show amber in header/attention glyph even when that workspace is selected or working.

At 49-wide screenshot scale the same structure gains one cell of title/metadata space. A useful full-screen comparison is 160×48 with W=48 (right inspector 40, terminal 70 columns), versus 80×30 with W=26 (no inspector, terminal 52 columns). A narrow 16–22-cell layout needs a compact reduction: omit logo/kind descriptions, retain disclosure, title, workflow/runtime glyph and details; do not simply shrink the full layout until labels disappear.

Benefits: strongest answer to ambiguous Needs me boundaries, preserves current card density, gives ordinary cards one obvious silhouette and nested tabs one obvious owner. Main tradeoff: calm tint across a large attention group needs restraint so selection remains more prominent.

## Direction B — framed workspace cards inside ruled sections

Give each workspace one cell border on all four sides; put its children inside the same boundary. Section header is a strong filled row with count, and an explicit section-end rule. Keep the same title anchor across every card; group status lives in the header and a small card status marker, while cyan selection changes card surface/left border.

At W=26, a card spanning columns 1–24 has 22 interior cells; three-cell logo + fixed gutters + details leaves roughly 12–14 title cells. At W=48 it has 44 interior cells and roughly 34–36 title cells. Each workspace costs 5+n rows (three content + top/bottom + children) before inter-card spacing, versus current 3+n, so four cards cost eight extra rows. Header text can replace part of the top rule only if that same geometry is used for hit tests.

Benefits: clearest object boundaries and ownership of child tabs; strong reviewable visual distinction. Tradeoff: consumes vertical space quickly in a 24–30-row terminal and risks too much box-drawing noise. Appropriate if the user values explicit card edges more than density.

## Direction C — compact ledger with status bars

Use two-row workspace entries and one-row children, with full-width status header bars and explicit bottom rules. Workspace row 1 is name plus one compact state marker; row 2 aligns project/branch and tab count under the name. In wide mode retain a fixed right runtime field; in narrow mode replace the repeated runtime word with a glyph, preserving useful names. Apply full-row cyan fill only to the active workspace/current child and a subtle separator between siblings.

At W=26, fixed title anchor 8 and details at 23 gives 14 title cells, metadata 16; children use 13 name cells and a one-cell state marker. At W=48, use a 27-cell title field (8–34), a gap, runtime 36–43, details 45; child indent at 10 with the same field boundaries. Two-row parents save one row per card and retain all attention grouping. Narrow 16–22 can reduce icon slot to initials/no-logo mode.

Benefits: most visible workspaces and low visual noise. Tradeoff: the user already dislikes weak element distinction; this direction depends on precise borders and highlighting, and has less room to display workflow and observed activity separately on pinned cards. Prefer A for the first design image; C is the density comparison, B the more explicit boundary comparison.

## Read-only validation

Inspected the supplied PNG, exact PNG dimensions and pixel boundary around the sidebar. Inspected source geometry, palette, image slot sizing and pointer/keyboard selection code. No builds/tests are needed for this read-only design audit, and none were run.

## Final mockup widths: 46 / 28 cells

The design comparison used 46-column and 28-column sidebars. Both are supported by current layout rules without changing the pane model: W=28 is feasible at total screen width 62 or more; W=46 is feasible from total screen width 134. Recommended full-screen comparisons are 160×48 with left 46 / right 40 / native terminal 72 columns, and 80×30 with left 28 / right 0 / native terminal 50 columns. The 26/48 examples above remain edge examples, not the chosen image-sheet dimensions.

Authoritative shared slot contract for the generated comparisons:

| Element | 28-column sidebar | 46-column sidebar |
| --- | --- | --- |
| Content band | 1–26 | 1–44 |
| Pane separator | 27 | 45 |
| Selection / keyboard marker | 1 | 1 |
| Workspace fold control | 2 | 2 |
| Fixed icon/fallback slot | 4–6 | 4–6 |
| Title + metadata start | 8 | 8 |
| Title span | 8–23 (16 cells) | 8–41 (34 cells) |
| Right-aligned details / group count edge | 25 | 43 |
| Metadata span | 8–25 (18 cells) | 8–43 (36 cells) |
| Child connector | 4–6 | 4–6 |
| Child type marker | 8 | 8 |
| Child label span | 10–24 (15 cells) | 10–34 (25 cells) |
| Child runtime | glyph at 25 | 8-cell field at 36–43 |

Two-letter fallback initials occupy columns 4–5 with column 6 padding, or use a one-cell leading pad consistently; reserve all three cells either way. The important invariant is that image presence/aspect/arrival never moves the title. One-row image fit remains aspect-preserving within three cells. Missing graphics still yields the same label geometry.

## Minimal implementation and verification plan after design selection

1. Keep the change UI-only. Add explicit section-end/continuation geometry and centralized sidebar slots beside `SideRow` in `src/ui/workflows.rs`; consume it in `draw_sidebar`, `draw_workspace_card`, `draw_terminal_row`, `card_at`, and the sidebar click/wheel paths. If shared board drawing cannot use the new sidebar geometry cleanly, leave board rendering as its own caller with explicit metrics instead of silently changing board density.
2. Render header/body/footer bands and three-row workspace blocks; determine active workspace + active child from existing snapshot, temporary cursor from Cards. Apply parent active surface to its visible child rows. Keep independent workflow and observed-working glyphs. Reuse existing `tint` colors and reduced-motion behavior.
3. Clamp icon slots at the sidebar layout layer while preserving existing badge dimensions/transport contracts. Update text clipping and right-edge reservations for both sizes and the 16–22 minimum-width reduction. No new state, protocol, subprocess, schema, role, or application dependency.
4. Verify static snapshots at 46 and 28, plus minimum 16 and full-width Cards overlay: long Unicode names, missing and very wide image icons, 10+ counts, folded groups, selected parent/current child, pinned Needs me + Working, stopped/failed children, and reduced motion. Assert useful title/name cells remain visible and badges never overlap text or exceed the pane.
5. Extend existing actual-UI coverage around `tests/live.rs:1505`, `:1557`, `:6506`: section headings/closure/gaps never select a workspace accidentally; disclosure and details columns match displayed targets; wheel/resize/fold keep hit testing matched to visible rows; a partially hidden card is inert; selecting the displayed child preserves exact tab/run; current highlights persist when the native terminal owns focus. Existing draft-preservation coverage is the important regression test, not adding tests that merely repeat every coordinate expression.
6. After implementation, run the repository-required offline checks once: `cargo test --offline`, `cargo clippy --offline --all-targets -- -D warnings`, `cargo build --offline --release`, `cargo fmt --check`. Capture static native-grid narrow/wide evidence. Use disposable fixtures without real native chats.

