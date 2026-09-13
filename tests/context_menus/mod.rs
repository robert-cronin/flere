//! Real UI context menus. Shells/files are disposable; clipboard OSC stays in the test PTY.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

mod git;

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
    layout: flere::ui::Layout,
}
impl Ui {
    fn attach(f: &Fixture, scene: &Scene, width: u16, height: u16) -> Self {
        let prefs = flere::workspace::Preferences {
            left: 38,
            right: 40,
            expanded_cards: vec![scene.source_wid, scene.target_wid],
            reduced_motion: true,
            ..Default::default()
        };
        fs::write(f.state.join("ui.json"), serde_json::to_vec(&prefs).unwrap()).unwrap();
        let layout = flere::ui::Layout::with_preferences(width as usize, height as usize, &prefs);
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
            layout,
        };
        ui.wait(|s| s.capture(100).contains("SOURCE_DRAFT"));
        ui.pump();
        ui
    }
    fn wait(&mut self, condition: impl Fn(&Terminal) -> bool) {
        wait_current_ui(&mut self.master, &mut self.screen, condition);
    }
    fn pump(&mut self) -> Vec<u8> {
        pump_ui_bytes(&mut self.master, &mut self.screen, 120)
    }
    fn key(&mut self, bytes: &[u8]) -> Vec<u8> {
        // Inputs are deliberately short. Keep all resulting output for OSC52 assertions.
        self.master.write_all(bytes).unwrap();
        self.pump()
    }
    fn click(&mut self, x: usize, y: usize) -> Vec<u8> {
        self.key(mouse(0, x, y).as_bytes())
    }
    fn context(&mut self, x: usize, y: usize, action: &str) {
        self.key(mouse(2, x, y).as_bytes());
        self.wait(|s| s.capture(100).contains(action));
    }
    fn find(&self, text: &str) -> (usize, usize) {
        find(
            &self.screen,
            0,
            self.screen.grid.cols,
            0,
            self.screen.grid.rows,
            text,
        )
        .unwrap_or_else(|| panic!("missing {text:?}: {}", self.screen.capture(100)))
    }
    fn card(&self, text: &str) -> (usize, usize) {
        find(
            &self.screen,
            0,
            self.layout.left,
            3,
            self.screen.grid.rows - 2,
            text,
        )
        .unwrap_or_else(|| panic!("missing card {text:?}: {}", self.screen.capture(100)))
    }
    fn source_children(&self) -> Vec<(usize, usize)> {
        let start = self.card("Source").1;
        let end = self.card("Target").1;
        (start + 1..end)
            .filter_map(|y| {
                let row = self.screen.grid.cells
                    [y * self.screen.grid.cols..y * self.screen.grid.cols + self.layout.left]
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>();
                (row.contains("sh") && (row.contains('├') || row.contains('└'))).then_some((10, y))
            })
            .collect()
    }
    fn tabs(&self) -> Vec<(usize, usize)> {
        (self.layout.left..self.screen.grid.cols - self.layout.right.saturating_add(2))
            .filter(|&x| {
                let at = 2 * self.screen.grid.cols + x;
                self.screen.grid.cells[at].text == "s" && self.screen.grid.cells[at + 1].text == "h"
            })
            .map(|x| (x, 2))
            .collect()
    }
    fn artifact(&self, name: &str) {
        let Some(root) = std::env::var_os("FLERE_TEST_CONTEXT_ARTIFACTS") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let frame = flere::screenshot::Frame {
            width: self.screen.grid.cols,
            height: self.screen.grid.rows,
            cells: self.screen.grid.cells.clone(),
            layers: Vec::new(),
            cursor: None,
        };
        fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
        fs::write(root.join(format!("{name}.txt")), self.screen.capture(100)).unwrap();
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
fn mouse(button: u8, x: usize, y: usize) -> String {
    format!(
        "\x1b[<{button};{};{}M\x1b[<{button};{};{}m",
        x + 1,
        y + 1,
        x + 1,
        y + 1
    )
}
fn find(
    screen: &Terminal,
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
    text: &str,
) -> Option<(usize, usize)> {
    (top..bottom).find_map(|y| {
        let row = screen.grid.cells[y * screen.grid.cols + left..y * screen.grid.cols + right]
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        row.find(text).map(|at| {
            (
                left + row[..at]
                    .chars()
                    .map(|c| flere::terminal::char_width(c) as usize)
                    .sum::<usize>(),
                y,
            )
        })
    })
}
struct Scene {
    source_wid: u64,
    source: TabView,
    sibling: TabView,
    target_wid: u64,
    target: TabView,
    source_dir: PathBuf,
    file: PathBuf,
}
fn scene(f: &Fixture) -> Scene {
    let source_dir = f.root.join("source");
    let target_dir = f.root.join("target");
    fs::create_dir(&source_dir).unwrap();
    fs::create_dir(&target_dir).unwrap();
    fs::create_dir(source_dir.join("folder")).unwrap();
    fs::write(source_dir.join("folder/nested.txt"), "Nested file\n").unwrap();
    let file = source_dir.join("a & b 'quoted'.txt");
    fs::write(&file, "EXACT_EDITOR_FILE\n").unwrap();
    f.req(&[
        "new",
        &wire::hex(b"Source"),
        &wire::hex(source_dir.to_str().unwrap().as_bytes()),
    ]);
    let source_wid = f.snapshot().active;
    let source = f.snapshot().session().unwrap().clone();
    f.req(&["tab", &source_wid.to_string()]);
    let sibling = f.snapshot().session().unwrap().clone();
    f.req(&[
        "new",
        &wire::hex(b"Target"),
        &wire::hex(target_dir.to_str().unwrap().as_bytes()),
    ]);
    let target_wid = f.snapshot().active;
    let target = f.snapshot().session().unwrap().clone();
    f.send(&source, b"SOURCE_DRAFT");
    f.send(&sibling, b"SIBLING_DRAFT");
    f.send(&target, b"TARGET_DRAFT");
    f.req(&["focus", &source_wid.to_string(), &source.id.to_string()]);
    Scene {
        source_wid,
        source,
        sibling,
        target_wid,
        target,
        source_dir,
        file,
    }
}
fn input_audit(f: &Fixture) -> Vec<String> {
    fs::read_to_string(f.state.join("actions.log"))
        .unwrap()
        .lines()
        .filter(|line| line.contains("\tinput\t"))
        .map(str::to_owned)
        .collect()
}
fn current(f: &Fixture) -> (u64, u64, String, u32) {
    let s = f.snapshot();
    let tab = s.session().unwrap();
    (s.active, tab.id, tab.run.clone(), tab.pid)
}
fn alive(f: &Fixture, before: &TabView) {
    let s = f.snapshot();
    let tab = s
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .find(|t| t.id == before.id)
        .unwrap();
    assert_eq!(
        (tab.pid, &tab.run, tab.alive),
        (before.pid, &before.run, true)
    );
}
fn osc52(raw: &[u8]) -> Vec<Vec<u8>> {
    let prefix = b"\x1b]52;c;";
    raw.windows(prefix.len())
        .enumerate()
        .filter_map(|(at, window)| {
            if window != prefix {
                return None;
            }
            let payload = &raw[at + prefix.len()..];
            let end = payload
                .iter()
                .position(|b| *b == 7)
                .expect("bounded OSC52 terminator");
            Some(STANDARD.decode(&payload[..end]).unwrap())
        })
        .collect()
}

#[test]
fn right_click_surfaces_open_and_dismiss_without_selecting_or_sending_native_input() {
    for (width, height) in [(160, 40), (60, 24)] {
        let f = Fixture::new();
        let scene = scene(&f);
        let mut ui = Ui::attach(&f, &scene, width, height);
        let before = current(&f);
        let audit = input_audit(&f);
        let workspace = ui.card("Target");
        let children = ui.source_children();
        assert_eq!(children.len(), 2);
        let tabs = ui.tabs();
        assert_eq!(tabs.len(), 2);
        let terminal = (ui.layout.terminal_x + 2, ui.layout.terminal_y + 3);
        for (index, (point, action, label)) in [
            (workspace, "Focus workspace", "workspace"),
            (children[1], "Activate tab", "child"),
            (tabs[1], "Activate tab", "tab"),
            (terminal, "Copy Flere screenshot", "terminal"),
        ]
        .into_iter()
        .enumerate()
        {
            ui.context(point.0, point.1, action);
            assert_eq!(
                current(&f),
                before,
                "right click on {label} changed the selected session"
            );
            ui.artifact(&format!("context-{label}-{width}"));
            ui.key(b"\x1b[200~MENU_ONLY\r\x1b[201~");
            ui.key(b"UNRECOGNIZED_MENU_INPUT");
            ui.key(b"\x1b[<34;1;1M\x1b[<2;1;1m");
            assert!(ui.screen.capture(100).contains(action));
            if index == 3 {
                // This would ordinarily select the other workspace if dismissal clicked through.
                let target = ui.card("Target");
                ui.click(target.0, target.1);
            } else if index % 2 == 0 {
                ui.click(0, 0);
            } else {
                ui.key(b"\x1b");
            }
            assert!(!ui.screen.capture(100).contains(action));
            assert_eq!(current(&f), before);
            assert_eq!(
                input_audit(&f),
                audit,
                "menu events must never reach a child"
            );
        }
        for (tab, draft) in [
            (&scene.source, "SOURCE_DRAFT"),
            (&scene.sibling, "SIBLING_DRAFT"),
            (&scene.target, "TARGET_DRAFT"),
        ] {
            alive(&f, tab);
            assert!(f.capture(tab).contains(draft));
            assert!(!f.capture(tab).contains("MENU_ONLY"));
        }
        ui.finish();
    }
}

#[test]
fn context_actions_use_keyboard_and_matching_mouse_clicks_and_existing_close_confirmation() {
    let f = Fixture::new();
    let scene = scene(&f);
    let mut ui = Ui::attach(&f, &scene, 160, 40);
    let target = ui.card("Target");
    ui.context(target.0, target.1, "Focus workspace");
    ui.key(b"jjjj\r"); // Four workspace items wrap back to Focus workspace.
    assert_eq!(
        current(&f),
        (
            scene.target_wid,
            scene.target.id,
            scene.target.run.clone(),
            scene.target.pid
        )
    );
    let child = ui.source_children()[1];
    ui.context(child.0, child.1, "Activate tab");
    let action = ui.find("Activate tab");
    ui.key(format!("\x1b[<0;{};{}m", action.0 + 1, action.1 + 1).as_bytes());
    assert_eq!(
        f.snapshot().tab,
        scene.target.id,
        "release without press cannot activate"
    );
    ui.key(
        format!(
            "\x1b[<0;{};{}M\x1b[<32;1;1M\x1b[<0;{};{}m",
            action.0 + 1,
            action.1 + 1,
            action.0 + 1,
            action.1 + 1
        )
        .as_bytes(),
    );
    assert_eq!(
        f.snapshot().tab,
        scene.target.id,
        "drag cannot activate an item"
    );
    if !ui.screen.capture(100).contains("Activate tab") {
        ui.context(child.0, child.1, "Activate tab");
    }
    let close = ui.find("Close tab");
    ui.key(format!("\x1b[<0;{};{}M", close.0 + 1, close.1 + 1).as_bytes());
    ui.key(b"\x1b[A"); // Keyboard selection cancels the pending mouse press.
    ui.key(format!("\x1b[<0;{};{}m", close.0 + 1, close.1 + 1).as_bytes());
    assert!(ui.screen.capture(100).contains("Activate tab"));
    assert!(!ui.screen.capture(100).contains("Cancel"));
    assert_eq!(f.snapshot().tab, scene.target.id);
    alive(&f, &scene.sibling);
    let action = ui.find("Activate tab");
    ui.click(action.0, action.1);
    assert_eq!(
        current(&f),
        (
            scene.source_wid,
            scene.sibling.id,
            scene.sibling.run.clone(),
            scene.sibling.pid
        )
    );
    ui.key(b"_ACTIVATED");
    f.wait_text(&scene.sibling, "SIBLING_DRAFT_ACTIVATED");
    assert!(!f.capture(&scene.source).contains("_ACTIVATED"));
    let first = ui.tabs()[0];
    ui.key(mouse(10, first.0, first.1).as_bytes()); // Alt+right still opens a context menu.
    ui.wait(|s| s.capture(100).contains("Close tab"));
    ui.key(b"\x1b[B\r");
    ui.wait(|s| s.capture(100).contains("Cancel"));
    assert!(!ui.screen.capture(100).contains("Activate tab"));
    alive(&f, &scene.source);
    alive(&f, &scene.sibling);
    ui.key(b"\r"); // The existing close dialog defaults to Cancel.
    assert!(!ui.screen.capture(100).contains("Cancel"));
    alive(&f, &scene.source);
    assert!(f.capture(&scene.source).contains("SOURCE_DRAFT"));
    f.req(&[
        "focus",
        &scene.source_wid.to_string(),
        &scene.sibling.id.to_string(),
    ]);
    ui.pump();
    let first = ui.tabs()[0];
    ui.context(first.0, first.1, "Close tab");
    let audit = input_audit(&f);
    let close = ui.find("Close tab");
    ui.click(close.0, close.1);
    ui.wait(|s| s.capture(100).contains("Cancel"));
    ui.key(b"\t\r"); // Explicitly confirm the clicked tab's existing close dialog.
    ui.wait(|_| {
        !f.snapshot()
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .any(|t| t.id == scene.source.id)
    });
    assert_eq!(f.snapshot().tab, scene.sibling.id);
    assert_eq!(input_audit(&f), audit);
    alive(&f, &scene.sibling);
    alive(&f, &scene.target);
    assert!(
        f.capture(&scene.sibling)
            .contains("SIBLING_DRAFT_ACTIVATED")
    );
    assert!(f.capture(&scene.target).contains("TARGET_DRAFT"));
    ui.finish();
}

#[test]
fn context_menus_cancel_on_removed_exact_target_origin_change_and_resize() {
    let f = Fixture::new();
    let scene = scene(&f);
    let mut ui = Ui::attach(&f, &scene, 160, 40);
    let audit = input_audit(&f);
    let child = ui.source_children()[1];
    ui.context(child.0, child.1, "Close tab");
    f.req(&["close", &scene.sibling.id.to_string(), &scene.sibling.run]);
    ui.wait(|s| {
        !s.capture(100).contains("Close tab")
            && !f
                .snapshot()
                .workspaces
                .iter()
                .flat_map(|w| &w.tabs)
                .any(|t| t.id == scene.sibling.id)
    });
    assert_eq!(f.snapshot().tab, scene.source.id);
    f.req(&["tab", &scene.source_wid.to_string()]);
    let replacement = f.snapshot().session().unwrap().clone();
    assert_ne!(replacement.run, scene.sibling.run);
    f.req(&[
        "focus",
        &scene.source_wid.to_string(),
        &scene.source.id.to_string(),
    ]);
    ui.pump();
    let terminal = (ui.layout.terminal_x + 2, ui.layout.terminal_y + 3);
    ui.context(terminal.0, terminal.1, "Copy Flere screenshot");
    f.req(&[
        "focus",
        &scene.target_wid.to_string(),
        &scene.target.id.to_string(),
    ]);
    ui.wait(|s| !s.capture(100).contains("Copy Flere screenshot"));
    assert_eq!(f.snapshot().tab, scene.target.id);
    let tab = ui.tabs()[0];
    ui.context(tab.0, tab.1, "Close tab");
    os::resize(ui.master.as_raw_fd(), 90, 28).unwrap();
    ui.screen.resize(90, 28);
    ui.wait(|s| !s.capture(100).contains("Close tab"));
    assert_eq!(f.snapshot().tab, scene.target.id);
    assert_eq!(input_audit(&f), audit);
    alive(&f, &scene.source);
    alive(&f, &scene.target);
    alive(&f, &replacement);
    assert!(f.capture(&scene.source).contains("SOURCE_DRAFT"));
    assert!(f.capture(&scene.target).contains("TARGET_DRAFT"));
    ui.finish();
}

#[test]
fn file_context_actions_copy_and_open_the_clicked_path_and_directory() {
    for width in [160, 60] {
        let f = Fixture::new();
        let scene = scene(&f);
        let mut ui = Ui::attach(&f, &scene, width, 40);
        let before = current(&f);
        let audit = input_audit(&f);
        let name = scene.file.file_name().unwrap().to_str().unwrap();
        let narrow = ui.layout.right == 0;
        if narrow {
            let hidden_tab = ui.tabs()[0];
            ui.key(b"\0l"); // NAV, Files: inspector now owns the full screen.
            ui.wait(|s| s.capture(100).contains(name));
            ui.key(mouse(2, hidden_tab.0, hidden_tab.1).as_bytes());
            let text = ui.screen.capture(100);
            assert!(text.contains(name));
            assert!(!text.contains("Activate tab"));
            assert!(!text.contains("Focus workspace"));
            assert!(!text.contains("Copy path"));
            assert_eq!(current(&f), before);
        }
        ui.wait(|s| s.capture(100).contains(name));
        let inspector_left = if narrow {
            0
        } else {
            ui.screen.grid.cols - ui.layout.right
        };
        let file = find(
            &ui.screen,
            inspector_left,
            ui.screen.grid.cols,
            3,
            ui.screen.grid.rows - 2,
            name,
        )
        .unwrap();
        if narrow {
            assert!(
                file.0 < ui.layout.left,
                "file must overlap the hidden sidebar"
            );
        } else {
            let source = ui.card("Source");
            ui.key(format!("\x1b[<35;{};{}M", source.0 + 1, source.1 + 1).as_bytes());
            ui.wait(|s| s.capture(100).contains("Preview · click to focus"));
            let overlay = ui.find("Preview · click to focus");
            ui.key(mouse(2, overlay.0, overlay.1).as_bytes());
            let text = ui.screen.capture(100);
            assert!(!text.contains("Open in editor"));
            assert!(!text.contains("Copy path"));
            assert_eq!(current(&f), before);
            assert_eq!(input_audit(&f), audit);
            if text.contains("Preview · click to focus") {
                ui.click(0, 0);
            }
            ui.wait(|s| s.capture(100).contains(name));
        }
        ui.context(file.0, file.1, "Open in editor");
        assert_eq!(current(&f), before);
        assert!(!ui.screen.capture(100).contains("Focus workspace"));
        ui.artifact(&format!("context-file-{width}"));
        let copy = ui.find("Copy path");
        let raw = ui.click(copy.0, copy.1);
        assert_eq!(
            osc52(&raw),
            vec![scene.file.to_str().unwrap().as_bytes().to_vec()]
        );
        assert_eq!(current(&f), before);
        assert_eq!(input_audit(&f), audit);
        ui.context(file.0, file.1, "Open in editor");
        ui.key(b"\r");
        ui.wait(|_| {
            f.snapshot()
                .session()
                .is_some_and(|t| t.kind == "editor" && t.path == scene.file.to_str().unwrap())
        });
        f.wait_text(f.snapshot().session().unwrap(), "EXACT_EDITOR_FILE");
        alive(&f, &scene.source);
        assert!(f.capture(&scene.source).contains("SOURCE_DRAFT"));
        if narrow {
            ui.key(b"\0l"); // Reopen the full-screen inspector after editor activation.
            ui.wait(|s| s.capture(100).contains("folder"));
        }
        let folder = find(
            &ui.screen,
            inspector_left,
            ui.screen.grid.cols,
            3,
            ui.screen.grid.rows - 2,
            "folder",
        )
        .unwrap();
        let editor = current(&f);
        ui.context(folder.0, folder.1, "Open directory");
        assert_eq!(current(&f), editor);
        ui.artifact(&format!("context-directory-{width}"));
        ui.key(b"\r");
        ui.wait(|s| s.capture(100).contains("nested.txt"));
        assert_eq!(current(&f), editor);
        assert!(scene.source_dir.join("folder/nested.txt").is_file());
        assert_eq!(
            fs::read_to_string(&scene.file).unwrap(),
            "EXACT_EDITOR_FILE\n"
        );
        ui.finish();
    }
}
