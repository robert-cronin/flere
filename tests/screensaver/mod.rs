//! Full disposable UI: the screensaver is presentation, never a session owner.
use super::splits::{emit, input, pane, panes, probes, unchanged};
use super::*;

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
    layout: flere::ui::Layout,
}
impl Ui {
    fn attach(f: &Fixture, width: u16, height: u16) -> Self {
        Self::attach_motion(f, width, height, false)
    }
    fn attach_motion(f: &Fixture, width: u16, height: u16, quiet: bool) -> Self {
        let prefs = flere::workspace::Preferences {
            left: 26,
            right: 32,
            reduced_motion: quiet,
            screensaver_mascot: flere::pet::ScreensaverMascot::Duck,
            ..Default::default()
        };
        fs::write(f.state.join("ui.json"), serde_json::to_vec(&prefs).unwrap()).unwrap();
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
            layout: flere::ui::Layout::with_preferences(width as usize, height as usize, &prefs),
        };
        ui.wait(|s| s.capture(100).contains("PANE_READY_"));
        ui
    }
    fn wait(&mut self, condition: impl Fn(&Terminal) -> bool) {
        wait_current_ui(&mut self.master, &mut self.screen, condition);
    }
    fn key(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 90)
    }
    fn start(&mut self) {
        self.key(b"\0 /Start screensaver");
        self.wait(|s| s.capture(100).contains("Start screensaver"));
        self.key(b"\r");
        self.wait(|s| s.capture(100).contains("SCREENSAVER"));
        assert!(!self.screen.capture(100).contains("PANE_READY_"));
    }
    fn awake(&mut self) {
        self.wait(|s| !s.capture(100).contains("SCREENSAVER") && s.capture(100).contains("FLERE"));
        assert!(
            self.child.try_wait().unwrap().is_none(),
            "waking cannot detach the UI"
        );
    }
    fn artifact(&mut self, name: &str, delay_ms: u64) {
        let Some(root) = std::env::var_os("FLERE_TEST_SCREENSAVER_ARTIFACTS") else {
            return;
        };
        pump_ui_bytes(&mut self.master, &mut self.screen, delay_ms);
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

fn exact_view(snapshot: &Snapshot) -> (String, u64, u64, u64, [flere::panes::PaneGroup; 2]) {
    let split = snapshot.split.as_ref().unwrap();
    (
        snapshot.epoch.clone(),
        snapshot.active,
        snapshot.tab,
        split.revision,
        split.groups.clone(),
    )
}

fn drafts(f: &Fixture, tabs: &[TabView], expected: &[&[u8]]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if tabs
            .iter()
            .zip(expected)
            .all(|(t, bytes)| input(f, t) == *bytes)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "wrong exact draft bytes: {:?}",
            tabs.iter().map(|t| input(f, t)).collect::<Vec<_>>()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn actual_ui_screensaver_keeps_split_processes_draining_and_renders_the_intro() {
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    pane(&f, "split-right", Some("move"));
    let mut ui = Ui::attach(&f, 180, 42);
    f.send(&tabs[0], b"FIRST_DRAFT");
    f.send(&tabs[1], b"SECOND_DRAFT");
    drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
    let before = panes(&f);
    ui.start();
    for (i, tab) in tabs.iter().enumerate() {
        let output = format!(
            "{}\r\nUNDER_SAVER_PANE_{i}",
            "draining live output without changing the visible screensaver\r\n".repeat(700)
        );
        emit(&f, tab, 1, &output);
        f.wait_text(tab, &format!("UNDER_SAVER_PANE_{i}"));
    }
    ui.key(b"\x1b[I\x1b[O"); // Terminal focus reports are not wake gestures.
    assert!(ui.screen.capture(100).contains("SCREENSAVER"));
    assert!(!ui.screen.capture(100).contains("UNDER_SAVER_PANE_"));
    ui.artifact("intro-duck", 2600);
    ui.artifact("intro-duck-later", 800);
    assert_eq!(exact_view(&panes(&f)), exact_view(&before));
    unchanged(&f, &tabs);
    drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
    ui.key(b"x");
    ui.awake();
    ui.wait(|s| {
        s.capture(100).contains("UNDER_SAVER_PANE_0")
            && s.capture(100).contains("UNDER_SAVER_PANE_1")
    });
    drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
    ui.key(b"_AFTER_WAKE");
    drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT_AFTER_WAKE"]);
    assert_eq!(exact_view(&panes(&f)), exact_view(&before));
    ui.finish();
    unchanged(&f, &tabs);
}

#[test]
fn actual_ui_screensaver_keyboard_wakes_and_pointer_gestures_remain_inside() {
    for (gesture, width, height) in [
        ("keys", 180, 42),
        ("paste", 60, 24),
        ("mouse", 180, 42),
        ("movement", 180, 42),
    ] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        pane(&f, "split-right", Some("move"));
        let mut ui = Ui::attach(&f, width, height);
        f.send(&tabs[0], b"FIRST_DRAFT");
        f.send(&tabs[1], b"SECOND_DRAFT");
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
        let before = panes(&f);
        ui.start();
        match gesture {
            "keys" => {
                ui.key(b"x\0q\r");
            }
            "paste" => {
                ui.artifact("intro-duck-narrow", 2600);
                ui.key(b"\x1b[200~WAKE_PASTE_BEGIN");
                ui.awake();
                ui.key(b"\0q\rPASTE_MIDDLE");
                ui.key(b"PASTE_END\x1b[201~");
            }
            "mouse" => {
                let x = ui.layout.terminal_x + 3;
                let y = ui.layout.terminal_y + 2;
                ui.key(format!("\x1b[<0;{x};{y}M").as_bytes());
                assert!(ui.screen.capture(100).contains("SCREENSAVER"));
                let raw = ui.key(
                    format!("\x1b[<32;{};{}M\x1b[<0;{};{}m", x + 8, y + 2, x + 8, y + 2).as_bytes(),
                );
                assert!(
                    !raw.windows(5).any(|bytes| bytes == b"\x1b]52;"),
                    "wake drag cannot copy a hidden terminal selection"
                );
            }
            "movement" => {
                ui.key(
                    b"\x1b[<35;8;8M\x1b[<2;8;8M\x1b[<2;8;8m\x1b[<65;8;8M\x1b[<1;8;8M\x1b[<1;8;8m",
                );
            }
            _ => unreachable!(),
        }
        if matches!(gesture, "mouse" | "movement") {
            assert!(
                ui.screen.capture(100).contains("SCREENSAVER"),
                "pointer input dismissed screensaver"
            );
            assert_eq!(exact_view(&panes(&f)), exact_view(&before));
            drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
            ui.key(b"x");
        }
        ui.awake();
        assert_eq!(
            exact_view(&panes(&f)),
            exact_view(&before),
            "wake gesture must not focus the other pane"
        );
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
        ui.key(b"_NEXT_INPUT");
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT_NEXT_INPUT"]);
        unchanged(&f, &tabs);
        ui.finish();
        unchanged(&f, &tabs);
    }
}

fn duck_box(screen: &Terminal) -> (usize, usize, usize, usize) {
    let yellow = |color| matches!(color, flere::terminal::Color::Rgb(r, g, b) if r > 180 && g > 100 && b < 130);
    let cells: Vec<_> = screen
        .grid
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| yellow(c.style.fg) || yellow(c.style.bg))
        .map(|(i, _)| (i % screen.grid.cols, i / screen.grid.cols))
        .collect();
    assert!(
        !cells.is_empty(),
        "missing rendered duck: {}",
        screen.capture(100)
    );
    (
        cells.iter().map(|c| c.0).min().unwrap(),
        cells.iter().map(|c| c.1).min().unwrap(),
        cells.iter().map(|c| c.0).max().unwrap(),
        cells.iter().map(|c| c.1).max().unwrap(),
    )
}

#[test]
fn actual_ui_screensaver_duck_can_be_grabbed_thrown_and_placed_with_reduced_motion() {
    for quiet in [false, true] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        pane(&f, "split-right", Some("move"));
        let mut ui = Ui::attach_motion(&f, 180, 42, quiet);
        f.send(&tabs[0], b"FIRST_DRAFT");
        f.send(&tabs[1], b"SECOND_DRAFT");
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
        let before = panes(&f);
        ui.start();
        ui.artifact(
            if quiet {
                "duck-before-grab-quiet"
            } else {
                "duck-before-grab"
            },
            2600,
        );
        let first = duck_box(&ui.screen);
        let pointer = ((first.0 + first.2) / 2 + 1, (first.1 + first.3) / 2 + 1);
        ui.key(format!("\x1b[<0;{};{}M", pointer.0, pointer.1).as_bytes());
        let target = (pointer.0 + 40, pointer.1.saturating_sub(12));
        ui.key(format!("\x1b[<32;{};{}M", target.0, target.1).as_bytes());
        let held = duck_box(&ui.screen);
        assert!(
            held.0 > first.0 + 30 && held.1 + 8 < first.1,
            "duck did not follow pointer"
        );
        assert!(ui.screen.capture(100).contains("SCREENSAVER"));
        ui.key(format!("\x1b[<0;{};{}m", target.0, target.1).as_bytes());
        let released = duck_box(&ui.screen);
        pump_ui_bytes(&mut ui.master, &mut ui.screen, 400);
        let later = duck_box(&ui.screen);
        if quiet {
            assert_eq!(released, later, "reduced motion must suppress the fling");
        } else {
            assert!(
                released.0.abs_diff(later.0) >= 3 || released.1.abs_diff(later.1) >= 3,
                "release lost throw momentum"
            );
        }
        assert!(later.0 >= 2 && later.2 < 178 && later.1 >= 3 && later.3 < 39);
        ui.artifact(
            if quiet {
                "duck-placed-quiet"
            } else {
                "duck-thrown"
            },
            0,
        );
        assert_eq!(exact_view(&panes(&f)), exact_view(&before));
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
        // Dismissing while the pointer is held must own its eventual drag/release.
        let pointer = ((later.0 + later.2) / 2 + 1, (later.1 + later.3) / 2 + 1);
        ui.key(format!("\x1b[<0;{};{}M", pointer.0, pointer.1).as_bytes());
        ui.key(b"q");
        ui.awake();
        let out = ui.key(b"\x1b[<32;40;15M\x1b[<0;40;15m");
        assert!(!out.windows(5).any(|b| b == b"\x1b]52;"));
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT"]);
        ui.key(b"_AFTER_DUCK");
        drafts(&f, &tabs, &[b"FIRST_DRAFT", b"SECOND_DRAFT_AFTER_DUCK"]);
        unchanged(&f, &tabs);
        ui.finish();
    }
}
