//! End-to-end sidebar contracts: real disposable supervisors, UI PTYs and mouse input.
use super::*;
use flere::workspace::Workflow;
use serde_json::{Value, json};

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
    left: usize,
}
impl Ui {
    fn attach(f: &Fixture, width: u16, height: u16, left: usize, expanded: &[u64]) -> Self {
        fs::write(
            f.state.join("ui.json"),
            serde_json::to_vec(&json!({
                "left": left, "right": 30, "expanded_cards": expanded,
                "reduced_motion": true, "pet": false, "grouping": "status"
            }))
            .unwrap(),
        )
        .unwrap();
        let mut command = outer_ui_command();
        command
            .arg("--state")
            .arg(&f.state)
            .arg("attach")
            .env("HOME", &f.root)
            .env_remove("XDG_CACHE_HOME");
        let (master, child) = os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut ui = Self {
            master,
            child,
            screen: Terminal::new(width as usize, height as usize),
            left,
        };
        ui.wait(|s| s.capture(100).contains("FLERE"));
        ui.pump();
        ui
    }
    fn wait(&mut self, pred: impl Fn(&Terminal) -> bool) {
        wait_current_ui(&mut self.master, &mut self.screen, pred);
    }
    fn pump(&mut self) {
        pump_ui_bytes(&mut self.master, &mut self.screen, 120);
    }
    fn key(&mut self, bytes: &[u8]) {
        write_ui_bytes(&mut self.master, &mut self.screen, bytes);
        self.pump();
    }
    fn click(&mut self, x: usize, y: usize) {
        self.key(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes());
    }
    fn wheel(&mut self, down: bool, times: usize) {
        self.key(
            format!("\x1b[<{};8;6M", if down { 65 } else { 64 })
                .repeat(times)
                .as_bytes(),
        );
    }
    fn resize(&mut self, width: u16, height: u16, left: usize) {
        os::resize(self.master.as_raw_fd(), width, height).unwrap();
        self.screen.resize(width as usize, height as usize);
        self.left = left;
        pump_ui_bytes(&mut self.master, &mut self.screen, 350);
    }
    fn line(&self, y: usize) -> String {
        side_line(&self.screen, self.left, y)
    }
    fn find(&self, text: &str) -> (usize, usize) {
        find_side(&self.screen, self.left, text)
            .unwrap_or_else(|| panic!("missing sidebar {text:?}: {}", self.screen.capture(100)))
    }
    fn heading(&self, text: &str) -> usize {
        (3..self.screen.grid.rows - 2)
            .find(|&y| {
                self.cell(1, y).text == " "
                    && matches!(self.cell(2, y).text.as_str(), "▾" | "▸")
                    && self.line(y).contains(text)
            })
            .unwrap_or_else(|| panic!("missing heading {text:?}: {}", self.screen.capture(100)))
    }
    fn cell(&self, x: usize, y: usize) -> &flere::terminal::Cell {
        &self.screen.grid.cells[y * self.screen.grid.cols + x]
    }
    fn artifact(&self, name: &str) {
        let Some(path) = std::env::var_os("FLERE_TEST_SIDEBAR_ARTIFACTS") else {
            return;
        };
        let path = PathBuf::from(path);
        fs::create_dir_all(&path).unwrap();
        let frame = flere::screenshot::Frame {
            width: self.screen.grid.cols,
            height: self.screen.grid.rows,
            cells: self.screen.grid.cells.clone(),
            layers: Vec::new(),
            cursor: None,
        };
        fs::write(path.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
        fs::write(path.join(format!("{name}.txt")), self.screen.capture(100)).unwrap();
    }
    fn finish(mut self) {
        finish_ui(&mut self.master, &mut self.screen, &mut self.child);
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            os::hangup(self.master.as_raw_fd(), &mut self.child);
        }
    }
}

fn side_line(screen: &Terminal, width: usize, y: usize) -> String {
    screen.grid.cells[y * screen.grid.cols..y * screen.grid.cols + width]
        .iter()
        .map(|cell| cell.text.as_str())
        .collect()
}
fn find_side(screen: &Terminal, width: usize, text: &str) -> Option<(usize, usize)> {
    (3..screen.grid.rows - 2).find_map(|y| {
        let line = side_line(screen, width, y);
        line.find(text).map(|at| {
            (
                line[..at]
                    .chars()
                    .map(|c| flere::terminal::char_width(c) as usize)
                    .sum(),
                y,
            )
        })
    })
}
fn preferences(f: &Fixture) -> Value {
    super::preferences::durable(f)
}
fn metadata(f: &Fixture, wid: u64, project: &str, status: Workflow, pinned: bool) {
    let mut meta = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == wid)
        .unwrap()
        .meta
        .clone();
    meta.project = project.into();
    meta.status = status;
    meta.pinned = pinned;
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
}
struct Scene {
    first: u64,
    first_tabs: Vec<TabView>,
    review: u64,
    review_tab: TabView,
    restore: u64,
    expanded: Vec<u64>,
}
fn scene(f: &Fixture) -> Scene {
    f.new_workspace("Flere");
    let first = f.snapshot().active;
    f.req(&["tab", &first.to_string()]);
    fs::write(f.root.join("TODO-ACTIVE.md"), "Sidebar fixture document\n").unwrap();
    f.req(&["open", &first.to_string(), &wire::hex(b"TODO-ACTIVE.md")]);
    let first_tabs = f.snapshot().workspace().unwrap().tabs.clone();
    metadata(f, first, "flere", Workflow::Todo, true);
    f.new_workspace("Review");
    let review = f.snapshot().active;
    f.req(&["tab", &review.to_string()]);
    let review_tab = f.snapshot().session().unwrap().clone();
    metadata(
        f,
        review,
        "sample-project-fixture",
        Workflow::NeedsMe,
        false,
    );
    f.new_workspace("Restore");
    let restore = f.snapshot().active;
    metadata(f, restore, "sample-project-fixture", Workflow::Todo, false);
    f.req(&[
        "new-stopped",
        &wire::hex(b"Stopped"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]);
    let stopped = f.snapshot().active;
    metadata(f, stopped, "exampleco", Workflow::Todo, false);
    f.req(&["focus", &first.to_string(), &first_tabs[0].id.to_string()]);
    Scene {
        first,
        first_tabs,
        review,
        review_tab,
        restore,
        expanded: vec![first, review, restore],
    }
}
fn assert_identity(f: &Fixture, epoch: &str, tabs: &[TabView]) {
    let snapshot = f.snapshot();
    assert_eq!(snapshot.epoch, epoch);
    for before in tabs {
        let after = snapshot
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|t| t.id == before.id)
            .unwrap();
        assert_eq!(
            (after.pid, &after.run, after.alive),
            (before.pid, &before.run, true)
        );
    }
}
fn child_rows(ui: &Ui, start: usize, end: usize) -> Vec<usize> {
    (start..end)
        .filter(|&y| {
            let row = ui.line(y);
            let child = if ui.left < 24 {
                matches!(ui.cell(3, y).text.as_str(), "$" | "≡")
            } else {
                row.contains('├') || row.contains('└')
            };
            child && (row.contains("sh") || row.contains("TODO"))
        })
        .collect()
}
fn bottom_border(ui: &Ui, after: usize) -> usize {
    (after..ui.screen.grid.rows - 2)
        .find(|&y| {
            let row = ui.line(y);
            (row.contains('╰') || row.contains('└')) && row.matches('─').count() > ui.left / 2
        })
        .expect("visible card bottom border")
}

#[test]
fn sidebar_boxes_keep_group_counts_gutters_and_selected_surfaces_at_all_widths() {
    let f = Fixture::new();
    let scene = scene(&f);
    let epoch = f.snapshot().epoch;
    for (width, left) in [(160, 46), (90, 28), (56, 16)] {
        let mut ui = Ui::attach(&f, width, 46, left, &scene.expanded);
        let (title_x, title_y) = ui.find("Flere");
        let (_, review_y) = ui.find("Review");
        let needs_y = ui.heading("Needs me");
        let children = child_rows(&ui, title_y + 1, needs_y);
        assert_eq!(
            children.len(),
            3,
            "all expanded tabs remain individually readable at {left} columns"
        );
        assert!(ui.line(title_y - 1).contains('╭') || ui.line(title_y - 1).contains('┌'));
        let selected = ui.cell(title_x, title_y).style.bg;
        assert_ne!(selected, ui.cell(title_x, review_y).style.bg);
        let current = ui.cell(title_x, children[0]).style.bg;
        assert_ne!(
            current, selected,
            "current child remains distinct while native terminal has focus"
        );
        for y in title_y..=children[2] {
            let expected = if y == children[0] { current } else { selected };
            assert_eq!(
                ui.cell(left - 5, y).style.bg,
                expected,
                "selected card surface broke at row {y}"
            );
        }
        assert_eq!(
            ui.find("Review").0,
            title_x,
            "workspace titles share a fixed gutter"
        );
        assert_eq!(ui.find("Restore").0, title_x);
        for label in ["Pinned", "Needs me", "Todo"] {
            let y = ui.heading(label);
            let tint = ui.cell(4, y).style.bg;
            assert_ne!(tint, ui.cell(4, y + 1).style.bg);
            assert!(
                (3..left - 3).all(|x| ui.cell(x, y).style.bg == tint),
                "heading is a full-width surface"
            );
        }
        ui.artifact(&format!("sidebar-boxes-{left}"));
        if left == 46 {
            ui.click(8, 2);
            // PTY pumping does not acknowledge the click. The grouping header
            // is redrawn after its preference has been saved.
            ui.wait(|s| side_line(s, left, 2).contains("Project"));
            assert_eq!(preferences(&f)["grouping"], "project");
            ui.artifact("sidebar-project-expanded");
            ui.click(8, 2);
            ui.wait(|s| side_line(s, left, 2).contains("Status"));
        }
        let heading = ui.heading("Todo");
        let count_before = ui.line(heading);
        let top_before = ui.cell(left - 3, 2).text.clone();
        assert_eq!(top_before, "4");
        ui.click(5, heading);
        ui.wait(|s| {
            (3..s.grid.rows - 2).any(|y| {
                s.grid.cells[y * s.grid.cols + 1].text == " "
                    && s.grid.cells[y * s.grid.cols + 2].text == "▸"
                    && side_line(s, left, y).contains("Todo")
            })
        });
        assert!(
            preferences(&f)["folds"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "Todo")
        );
        let heading = ui.heading("Todo");
        assert_eq!(
            ui.line(heading)
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>(),
            count_before
                .chars()
                .filter(char::is_ascii_digit)
                .collect::<String>()
        );
        assert_eq!(
            ui.cell(left - 3, 2).text,
            top_before,
            "folding must not change total card count"
        );
        assert!(find_side(&ui.screen, left, "Restore").is_none());
        assert_identity(&f, &epoch, &scene.first_tabs);
        ui.finish();
    }
}

#[test]
fn sidebar_clicks_follow_disclosures_details_children_and_inert_box_edges() {
    for (width, left) in [(160, 46), (90, 28), (56, 16)] {
        let f = Fixture::new();
        let scene = scene(&f);
        f.send(&scene.first_tabs[0], b"FIRST_DRAFT");
        f.send(&scene.first_tabs[1], b"SECOND_DRAFT");
        let before = f.snapshot();
        let mut ui = Ui::attach(&f, width, 46, left, &scene.expanded);
        let (_, title_y) = ui.find("Flere");
        let needs_y = ui.heading("Needs me");
        let children = child_rows(&ui, title_y + 1, needs_y);
        ui.click(if left >= 24 { 10 } else { 5 }, children[1]);
        assert_eq!(
            f.snapshot().tab,
            scene.first_tabs[1].id,
            "duplicate shell titles must retain exact targets"
        );
        ui.key(b"_UI");
        f.wait_text(&scene.first_tabs[1], "SECOND_DRAFT_UI");
        assert!(!f.capture(&scene.first_tabs[0]).contains("_UI"));
        let (_, review_y) = ui.find("Review");
        let bottom = bottom_border(&ui, review_y + 1);
        for (x, y) in [
            (7, review_y - 1),
            (1, review_y),
            (left - 2, review_y),
            (7, bottom),
        ] {
            ui.click(x, y);
            assert_eq!(
                (f.snapshot().active, f.snapshot().tab),
                (scene.first, scene.first_tabs[1].id),
                "inert border/gutter ({x},{y}) changed focus"
            );
        }
        let (_, title_y) = ui.find("Flere");
        ui.click(2, title_y);
        assert!(
            !preferences(&f)["expanded_cards"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == scene.first)
        );
        assert_eq!(f.snapshot().tab, scene.first_tabs[1].id);
        let (_, title_y) = ui.find("Flere");
        ui.click(2, title_y);
        assert!(
            preferences(&f)["expanded_cards"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == scene.first)
        );
        let (_, title_y) = ui.find("Flere");
        ui.click(left - 3, title_y);
        ui.wait(|s| s.capture(100).contains("Details · Flere"));
        assert_eq!(f.snapshot().tab, scene.first_tabs[1].id);
        ui.key(b"\x1b");
        assert!(!ui.screen.capture(100).contains("Details · Flere"));
        assert_identity(&f, &before.epoch, &scene.first_tabs);
        assert!(f.capture(&scene.first_tabs[1]).contains("SECOND_DRAFT_UI"));
        ui.finish();
    }
}

#[test]
fn sidebar_scroll_and_resize_leave_partial_parents_inert_and_keep_exact_drafts() {
    let f = Fixture::new();
    let scene = scene(&f);
    f.send(&scene.first_tabs[0], b"FIRST_SCROLL_DRAFT");
    f.send(&scene.review_tab, b"REVIEW_SCROLL_DRAFT");
    f.req(&[
        "focus",
        &scene.review.to_string(),
        &scene.review_tab.id.to_string(),
    ]);
    let before = f.snapshot();
    let mut ui = Ui::attach(&f, 160, 20, 46, &scene.expanded);
    ui.wheel(false, 20);
    assert!(find_side(&ui.screen, ui.left, "Flere").is_some());
    ui.wheel(true, 1);
    assert!(
        find_side(&ui.screen, ui.left, "Flere").is_none(),
        "root title should be above the clipped viewport"
    );
    assert_eq!(
        ui.cell(1, 3).text,
        "│",
        "partial parent keeps its enclosure"
    );
    assert!(
        (2..ui.left - 2).all(|x| ui.cell(x, 3).text == " "),
        "partial parent content is hidden rather than exposing an actionable fragment"
    );
    ui.click(8, 3);
    assert_eq!(
        (f.snapshot().active, f.snapshot().tab),
        (scene.review, scene.review_tab.id)
    );
    let children = child_rows(&ui, 3, ui.screen.grid.rows - 2);
    assert!(!children.is_empty());
    ui.click(10, children[0]);
    assert_eq!(f.snapshot().tab, scene.first_tabs[0].id);
    ui.resize(62, 18, 28);
    assert_eq!(f.snapshot().tab, scene.first_tabs[0].id);
    ui.key(b"_RESIZED");
    f.wait_text(&scene.first_tabs[0], "FIRST_SCROLL_DRAFT_RESIZED");
    assert!(!f.capture(&scene.review_tab).contains("_RESIZED"));
    ui.wheel(true, 2);
    assert_eq!(f.snapshot().tab, scene.first_tabs[0].id);
    ui.artifact("sidebar-clipped-resized");
    let mut preserved = scene.first_tabs.clone();
    preserved.push(scene.review_tab);
    assert_identity(&f, &before.epoch, &preserved);
    assert!(f.capture(&preserved[3]).contains("REVIEW_SCROLL_DRAFT"));
    ui.finish();
}

#[test]
fn sidebar_grouping_keeps_pinned_counts_and_separate_project_status_folds() {
    let f = Fixture::new();
    let scene = scene(&f);
    // Identical project/status labels must not share a fold key.
    metadata(&f, scene.review, "Needs me", Workflow::NeedsMe, false);
    metadata(&f, scene.restore, "Needs me", Workflow::Todo, false);
    f.send(&scene.first_tabs[0], b"GROUPING_DRAFT");
    let before = f.snapshot();
    let mut ui = Ui::attach(&f, 160, 46, 46, &scene.expanded);
    let needs = ui.heading("Needs me");
    ui.click(5, needs);
    assert!(
        preferences(&f)["folds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "Needs me")
    );
    assert!(find_side(&ui.screen, ui.left, "Review").is_none());
    let header_count = ui.cell(ui.left - 3, 2).text.clone();
    assert_eq!(header_count, "4");
    ui.click(8, 2);
    assert_eq!(preferences(&f)["grouping"], "project");
    assert_eq!(ui.cell(ui.left - 3, 2).text, header_count);
    assert!(ui.heading("Pinned") < ui.heading("Needs me"));
    assert!(find_side(&ui.screen, ui.left, "Review").is_some());
    assert!(find_side(&ui.screen, ui.left, "Restore").is_some());
    let needs = ui.heading("Needs me");
    let count = ui
        .line(needs)
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>();
    assert_eq!(count, "2");
    ui.click(5, needs);
    assert!(
        preferences(&f)["folds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "project:Needs me")
    );
    assert!(find_side(&ui.screen, ui.left, "Review").is_none());
    assert!(find_side(&ui.screen, ui.left, "Restore").is_none());
    let needs = ui.heading("Needs me");
    assert_eq!(
        ui.line(needs)
            .chars()
            .filter(char::is_ascii_digit)
            .collect::<String>(),
        count
    );
    ui.click(8, 2);
    assert_eq!(preferences(&f)["grouping"], "status");
    assert!(find_side(&ui.screen, ui.left, "Review").is_none());
    assert!(find_side(&ui.screen, ui.left, "Restore").is_some());
    ui.key(b"\0L");
    assert_eq!(preferences(&f)["grouping"], "project");
    ui.artifact("sidebar-project-grouping");
    ui.key(b"\x1b");
    ui.key(b"_AFTER");
    f.wait_text(&scene.first_tabs[0], "GROUPING_DRAFT_AFTER");
    assert_identity(&f, &before.epoch, &scene.first_tabs);
    ui.finish();
}

#[test]
fn sidebar_selected_pinned_attention_and_working_signals_remain_independent() {
    let f = Fixture::new();
    let busy = mailbox_native(&f, "Busy", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    metadata(&f, busy.wid, "flere", Workflow::NeedsMe, true);
    mailbox_event(
        &busy,
        1,
        json!({"event":"UserPromptSubmit","screen":"compact-active"}),
    );
    let before = f.snapshot();
    for (width, left) in [(160, 46), (90, 28), (56, 16)] {
        let mut ui = Ui::attach(&f, width, 32, left, &[busy.wid]);
        ui.wait(|_| f.snapshot().session().is_some_and(|t| t.working));
        ui.pump();
        let pinned_y = ui.heading("Pinned");
        let (title_x, title_y) = ui.find("Busy");
        assert!(pinned_y < title_y);
        let top = ui.line(title_y - 1);
        assert!(top.contains('◆'), "Needs me keeps its own glyph: {top}");
        assert!(
            top.contains('●'),
            "working keeps a separate reduced-motion glyph: {top}"
        );
        let fg = |needle: &str| {
            ui.screen.grid.cells
                [(title_y - 1) * ui.screen.grid.cols..(title_y - 1) * ui.screen.grid.cols + left]
                .iter()
                .find(|c| c.text == needle)
                .unwrap()
                .style
                .fg
        };
        assert_ne!(
            fg("◆"),
            fg("●"),
            "attention and working must not share one ambiguous signal"
        );
        assert_eq!(
            ui.cell(title_x, title_y).style.bg,
            ui.cell(title_x, title_y + 1).style.bg
        );
        ui.artifact(&format!("sidebar-attention-working-{left}"));
        assert_identity(&f, &before.epoch, std::slice::from_ref(&busy.tab));
        ui.finish();
    }
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
}

#[test]
fn sidebar_default_navigation_skips_children_until_explicit_terminal_entry() {
    let f = Fixture::new();
    let scene = scene(&f);
    let before = f.snapshot();
    let mut ui = Ui::attach(&f, 160, 42, 38, &scene.expanded);
    ui.key(b"\0hj");
    ui.wait(|_| f.snapshot().active == scene.review);
    assert_eq!(
        f.snapshot().active,
        scene.review,
        "expanded children add no card stops"
    );
    ui.key(b"\x1b[A");
    ui.wait(|_| f.snapshot().active == scene.first);
    assert_eq!(f.snapshot().active, scene.first);
    ui.key(b"\x1b[B");
    ui.wait(|_| f.snapshot().active == scene.review);
    assert_eq!(f.snapshot().active, scene.review);
    ui.key(b"k\t");
    ui.wait(|s| s.grid.line(41).contains("NAV Terminals"));
    let current = f.snapshot().tab;
    assert_eq!(current, scene.first_tabs[0].id);
    ui.key(b"j");
    assert_eq!(f.snapshot().tab, current, "child navigation is passive");
    ui.key(b"\x1b[B\x1b[A\r");
    ui.wait(|s| {
        !s.grid.line(41).trim_start().starts_with("NAV")
            && f.snapshot().tab == scene.first_tabs[1].id
    });
    assert_eq!(f.snapshot().active, scene.first);
    assert_eq!(
        f.snapshot().tab,
        scene.first_tabs[1].id,
        "Enter activates the exact duplicate shell"
    );
    for back in [b"h".as_slice(), b"\x1b[D", b"\x1b"] {
        ui.key(b"\0h\t");
        ui.wait(|s| s.grid.line(41).contains("NAV Terminals"));
        ui.key(back);
        ui.wait(|s| s.grid.line(41).contains("NAV Cards"));
        assert!(
            ui.screen.grid.line(41).contains("NAV Cards"),
            "return key restores card scope: {back:?}"
        );
        ui.key(b"j");
        ui.wait(|_| f.snapshot().active == scene.review);
        assert_eq!(
            f.snapshot().active,
            scene.review,
            "return key restores card traversal: {back:?}"
        );
        ui.key(b"k\r");
        ui.wait(|s| {
            !s.grid.line(41).trim_start().starts_with("NAV") && f.snapshot().active == scene.first
        });
        assert_eq!(f.snapshot().tab, scene.first_tabs[1].id);
    }
    assert_eq!(
        preferences(&f)["expanded_cards"],
        serde_json::json!(scene.expanded)
    );
    // Mouse expansion is independent of the keyboard scope. Tab reopens a folded
    // card and selects its current terminal, then Escape returns to its header.
    ui.key(b"\0h");
    ui.wait(|s| s.grid.line(41).contains("NAV Cards"));
    let (_, y) = ui.find("Flere");
    ui.click(2, y);
    assert!(
        !preferences(&f)["expanded_cards"]
            .as_array()
            .unwrap()
            .contains(&json!(scene.first))
    );
    ui.key(b"\t");
    ui.wait(|s| s.grid.line(41).contains("NAV Terminals"));
    ui.key(b"\x1b");
    ui.wait(|s| s.grid.line(41).contains("NAV Cards"));
    assert!(
        preferences(&f)["expanded_cards"]
            .as_array()
            .unwrap()
            .contains(&json!(scene.first))
    );
    ui.key(b"j");
    ui.wait(|_| f.snapshot().active == scene.review);
    assert_eq!(f.snapshot().active, scene.review);
    ui.key(b"\t");
    ui.wait(|s| s.grid.line(41).contains("NAV Terminals"));
    f.req(&[
        "focus",
        &scene.first.to_string(),
        &scene.first_tabs[1].id.to_string(),
    ]);
    ui.wait(|s| s.grid.line(41).contains("NAV Cards"));
    ui.key(b"j");
    ui.wait(|_| f.snapshot().active == scene.review);
    assert_eq!(
        f.snapshot().active,
        scene.review,
        "an external workspace focus leaves the old child list"
    );
    assert_identity(&f, &before.epoch, &scene.first_tabs);
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
    ui.finish();
}

#[test]
fn sidebar_primary_badge_arrives_asynchronously_and_survives_narrow_details() {
    let f = Fixture::new();
    let main = f.root.join("main-checkout");
    let linked = f.root.join("linked-checkout");
    fs::create_dir(&main).unwrap();
    history_git(&main, &["init", "-q", "-b", "main"]);
    fs::write(
        main.join("README.md"),
        "Disposable checkout identity fixture\n",
    )
    .unwrap();
    history_git(&main, &["add", "README.md"]);
    history_git(
        &main,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "fixture",
        ],
    );
    history_git(
        &main,
        &[
            "worktree",
            "add",
            "--detach",
            linked.to_str().unwrap(),
            "HEAD",
        ],
    );
    f.req(&[
        "new",
        &wire::hex(b"Original"),
        &wire::hex(main.to_str().unwrap().as_bytes()),
    ]);
    let before = f.snapshot();
    let original = before.active;
    let shell = before.session().unwrap().clone();
    metadata(&f, original, "checkout-fixture", Workflow::Todo, false);
    f.req(&[
        "new-stopped",
        &wire::hex(b"Linked"),
        &wire::hex(linked.to_str().unwrap().as_bytes()),
    ]);
    let linked_id = f.snapshot().active;
    metadata(&f, linked_id, "checkout-fixture", Workflow::Todo, false);
    f.req(&["focus", &original.to_string(), &shell.id.to_string()]);
    for (width, left) in [(160, 46), (90, 28), (56, 16)] {
        let mut ui = Ui::attach(&f, width, 32, left, &[]);
        let (x, y) = ui.find("Original");
        let badge_y = if left == 46 { y } else { y + 3 };
        ui.wait(|s| side_line(s, left, badge_y).contains("[primary]"));
        let (_, linked_y) = ui.find("Linked");
        assert!((linked_y..=linked_y + 3).all(|row| !ui.line(row).contains("primary")));
        assert!(!ui.line(y + 1).contains("primary"));
        assert!(!ui.screen.capture(100).contains("Main checkout"));
        let (badge_x, _) = ui.find("[primary]");
        assert_ne!(
            ui.cell(badge_x, badge_y).style.bg,
            ui.cell(x, y).style.bg,
            "the directory label has its own badge surface"
        );
        ui.artifact(&format!("sidebar-primary-{left}"));
        ui.click(left - 3, y);
        ui.wait(|s| s.capture(100).contains("Checkout: primary"));
        ui.artifact(&format!("sidebar-primary-details-{left}"));
        ui.key(b"\x1b");
        assert_identity(&f, &before.epoch, std::slice::from_ref(&shell));
        ui.finish();
    }
}
