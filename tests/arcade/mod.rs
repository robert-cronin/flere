//! Real local UI, harmless PTY probes, and exact draft ownership across Arcade.
use super::splits::{emit, input, panes, probes, unchanged};
use super::*;

struct ArcadeUi {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
}
impl ArcadeUi {
    fn attach(f: &Fixture, width: u16, height: u16) -> Self {
        let prefs = flere::workspace::Preferences {
            left: 26,
            right: 32,
            reduced_motion: true,
            pet: true,
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
        };
        ui.wait("PANE_READY_");
        ui
    }
    fn wait(&mut self, text: &str) {
        wait_current_ui(&mut self.master, &mut self.screen, |s| {
            s.capture(100).contains(text)
        });
    }
    fn key(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 100)
    }
    fn open(&mut self) -> Vec<u8> {
        let mut output = self.key(b"\0 /Arcade: Context Ruins");
        self.wait("Arcade: Context Ruins");
        output.extend(self.key(b"\r\r")); // The second Enter is still the launch burst, not Start.
        self.wait("Enter: explore");
        assert!(!self.screen.capture(100).contains("3 lives"));
        output
    }
    fn artifact(&self, name: &str) {
        let Some(root) = std::env::var_os("FLERE_TEST_ARCADE_ARTIFACTS") else {
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
    }
    fn finish(mut self) {
        finish_ui(&mut self.master, &mut self.screen, &mut self.child);
    }
}
impl Drop for ArcadeUi {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            os::hangup(self.master.as_raw_fd(), &mut self.child);
        }
    }
}

#[test]
fn actual_platformer_movement_jump_pause_and_exit_preserve_the_exact_native_draft() {
    for (width, height) in [(40, 20), (160, 42)] {
        let f = Fixture::new();
        let tabs = probes(&f, 1);
        f.send(&tabs[0], b"UNSUBMITTED_ARCADE_DRAFT");
        let mut ui = ArcadeUi::attach(&f, width, height);
        let before = panes(&f);
        ui.open();
        ui.artifact(&format!("arcade-live-{width}x{height}-picker"));
        ui.key(b"\x1b[200~4\rm\r\x1b[201~");
        assert!(ui.screen.capture(100).contains("Enter: explore"));
        ui.key(b"4");
        ui.key(b"\r");
        ui.wait("Space jump");
        let initial = ui.screen.grid.cells.clone();
        for _ in 0..6 {
            ui.key(b"l");
        }
        ui.wait("1/9");
        ui.key(b" ");
        assert_ne!(ui.screen.grid.cells, initial);
        ui.artifact(&format!("context-ruins-live-{width}x{height}-jump"));
        emit(&f, &tabs[0], 1, "\r\nARCADE_BACKGROUND_OUTPUT\r\n");
        f.wait_text(&tabs[0], "ARCADE_BACKGROUND_OUTPUT");
        assert!(!ui.screen.capture(100).contains("ARCADE_BACKGROUND_OUTPUT"));
        ui.key(b"\x1b[O");
        ui.wait("PAUSED");
        let paused = ui.screen.capture(100);
        pump_ui_bytes(&mut ui.master, &mut ui.screen, 1100);
        assert_eq!(ui.screen.capture(100), paused);
        ui.key(b"\x1b[I");
        assert!(ui.screen.capture(100).contains("PAUSED"));
        ui.key(b"p");
        ui.wait("Space jump");
        ui.key(b"\x1b[<0;8;8M"); // The overlay owns this press and its later release.
        ui.key(b"p");
        ui.wait("PAUSED");
        let output = ui.key(b"q\rSHOULD_NOT_REACH_NATIVE\x1b[200~PASTE_HEAD");
        assert!(!output.windows(5).any(|w| w == b"\x1b]52;"));
        ui.wait("ARCADE_BACKGROUND_OUTPUT");
        ui.key(b"PASTE_TAIL\r\x1b[201~");
        ui.key(b"\x1b[<32;12;9M\x1b[<0;12;9m");
        assert_eq!(input(&f, &tabs[0]), b"UNSUBMITTED_ARCADE_DRAFT");
        let after = panes(&f);
        assert_eq!(
            (after.epoch, after.active, after.tab),
            (before.epoch, before.active, before.tab)
        );
        unchanged(&f, &tabs);
        ui.key(b"\r"); // Deliberately leave the restored NAV mode in a fresh burst.
        ui.key(b"_AFTER_ARCADE");
        wait_current_ui(&mut ui.master, &mut ui.screen, |_| {
            input(&f, &tabs[0]).ends_with(b"_AFTER_ARCADE")
        });
        assert_eq!(
            input(&f, &tabs[0]),
            b"UNSUBMITTED_ARCADE_DRAFT_AFTER_ARCADE"
        );
        ui.finish();
    }
}

#[test]
fn actual_mascot_context_launch_and_tiny_resize_are_local_and_pause_safely() {
    let f = Fixture::new();
    let tabs = probes(&f, 1);
    let mut ui = ArcadeUi::attach(&f, 140, 34);
    ui.key(b"\x1b[<2;130;30M");
    ui.wait("Play Context Ruins");
    ui.key(b"\x1b[<2;130;30m");
    ui.key(b"\r");
    ui.wait("Enter: explore");
    ui.key(b"\r");
    ui.wait("3 lives");
    os::resize(ui.master.as_raw_fd(), 16, 8).unwrap();
    ui.screen.resize(16, 8);
    ui.wait("Paused.");
    ui.key(b"m\rc");
    os::resize(ui.master.as_raw_fd(), 80, 24).unwrap();
    ui.screen.resize(80, 24);
    ui.wait("PAUSED");
    assert!(ui.screen.capture(100).contains("3 lives"));
    ui.key(b"q\r");
    ui.wait("PANE_READY_");
    assert!(input(&f, &tabs[0]).is_empty());
    ui.key(b"fresh"); // A context-menu launch restores the original input mode.
    wait_current_ui(&mut ui.master, &mut ui.screen, |_| {
        input(&f, &tabs[0]) == b"fresh"
    });
    unchanged(&f, &tabs);
    ui.finish();
}

/// Locate the actual white disc pixels, excluding HUD prose and blue relics.
/// This measures rendered movement, not a test-only game coordinate API.
pub(super) fn avatar_x(screen: &Terminal) -> f32 {
    use flere::terminal::Color;
    let mut total = 0usize;
    let mut count = 0usize;
    for y in 3..screen.grid.rows.saturating_sub(3) {
        for x in 0..screen.grid.cols {
            let cell = &screen.grid.cells[y * screen.grid.cols + x];
            for color in [cell.style.fg, cell.style.bg] {
                if let Color::Rgb(r, g, b) = color
                    && r >= 225
                    && g >= 225
                    && (215..=249).contains(&b)
                    && r.abs_diff(g) <= 4
                {
                    total += x;
                    count += 1;
                }
            }
        }
    }
    assert!(
        count > 0,
        "Flere disc missing from gameplay canvas: {}",
        screen.capture(100)
    );
    total as f32 / count as f32
}
fn sequence_count(bytes: &[u8], sequence: &[u8]) -> usize {
    bytes
        .windows(sequence.len())
        .filter(|part| *part == sequence)
        .count()
}

#[test]
fn actual_keyboard_release_protocol_preserves_move_jump_and_fences_late_exit_events() {
    for (width, height) in [(40, 20), (160, 42)] {
        let f = Fixture::new();
        let tabs = probes(&f, 1);
        f.send(&tabs[0], b"KITTY_UNSUBMITTED_DRAFT");
        let mut ui = ArcadeUi::attach(&f, width, height);
        let before = panes(&f);
        let mut output = ui.open();
        assert_eq!(sequence_count(&output, b"\x1b[?u"), 1);
        let negotiation = ui.key(b"\x1b[?0u");
        assert!(
            negotiation
                .windows(b"\x1b[>11u\x1b[?u".len())
                .any(|part| part == b"\x1b[>11u\x1b[?u")
        );
        output.extend(negotiation);
        output.extend(ui.key(b"\x1b[?11u"));
        output.extend(ui.key(b"\x1b[13;1:1u"));
        ui.wait("Space jump");
        output.extend(ui.key(b"\x1b[1;1:1C"));
        output.extend(ui.key(b"\x1b[32;1:1u"));
        let launch = avatar_x(&ui.screen);
        output.extend(pump_ui_bytes(&mut ui.master, &mut ui.screen, 350));
        let continued = avatar_x(&ui.screen);
        assert!(
            continued > launch + 2.,
            "held Right stopped when Space suppressed repeats: {launch} -> {continued} ({width} columns)"
        );
        ui.artifact(&format!("context-ruins-held-jump-{width}x{height}"));
        output.extend(ui.key(b"\x1b[1;1:3C"));
        let released = avatar_x(&ui.screen);
        output.extend(pump_ui_bytes(&mut ui.master, &mut ui.screen, 240));
        assert!(
            (avatar_x(&ui.screen) - released).abs() < 1.25,
            "Right release did not stop movement"
        );
        let before_bare_press = avatar_x(&ui.screen);
        output.extend(ui.key(b"\x1b[C")); // Enhanced terminals may omit default press fields.
        assert!(avatar_x(&ui.screen) > before_bare_press + 0.2);
        output.extend(ui.key(b"\x1b[112;1:1u"));
        ui.wait("PAUSED");
        let paused = ui.screen.grid.cells.clone();
        output.extend(ui.key(b"\x1b[1;1:2C\x1b[32;1:2u"));
        assert_eq!(ui.screen.grid.cells, paused);
        output.extend(ui.key(b"\x1b[112;1:3u\x1b[112;1:1u"));
        ui.wait("Space jump");
        output.extend(ui.key(b"\x1b[1;1:2C\x1b[32;1:2u"));
        let resumed = avatar_x(&ui.screen);
        output.extend(pump_ui_bytes(&mut ui.master, &mut ui.screen, 240));
        assert!(
            (avatar_x(&ui.screen) - resumed).abs() < 1.25,
            "stale repeat revived a cleared direction"
        );
        output.extend(ui.key(b"\x1b[112;1:3u\x1b[112;1:1u"));
        ui.wait("PAUSED");
        let close = ui.key(b"\x1b[113;1:1u\x1b[13;1:1uEXIT_TAIL");
        assert_eq!(sequence_count(&close, b"\x1b[<u"), 1);
        assert!(
            close
                .windows(b"\x1b[<u\x1b[?u".len())
                .any(|part| part == b"\x1b[<u\x1b[?u")
        );
        output.extend(close);
        ui.wait("PANE_READY_");
        output.extend(ui.key(b"\x1b[1;1:3C\x1b[13;1:1u"));
        output.extend(ui.key(b"BEFORE_FENCE\r"));
        assert_eq!(input(&f, &tabs[0]), b"KITTY_UNSUBMITTED_DRAFT");
        output.extend(pump_ui_bytes(&mut ui.master, &mut ui.screen, 1100));
        ui.key(b"\r"); // Ordinary input resumes after timeout, even if the reply is delayed.
        output.extend(ui.key(b"\x1b[104;1:1u\x1b[13;1:1u"));
        assert_eq!(input(&f, &tabs[0]), b"KITTY_UNSUBMITTED_DRAFT");
        output.extend(ui.key(b"\x1b[?0u\rFENCE_TAIL"));
        assert_eq!(input(&f, &tabs[0]), b"KITTY_UNSUBMITTED_DRAFT");
        // Encoded releases remain inert even after the restore fence is complete.
        output.extend(ui.key(b"\x1b[32;1:3u\x1b[13;1:3u"));
        assert_eq!(input(&f, &tabs[0]), b"KITTY_UNSUBMITTED_DRAFT");
        assert_eq!(sequence_count(&output, b"\x1b[>11u"), 1);
        assert_eq!(sequence_count(&output, b"\x1b[<u"), 1);
        ui.key(b"_FRESH");
        wait_current_ui(&mut ui.master, &mut ui.screen, |_| {
            input(&f, &tabs[0]).ends_with(b"_FRESH")
        });
        assert_eq!(input(&f, &tabs[0]), b"KITTY_UNSUBMITTED_DRAFT_FRESH");
        let after = panes(&f);
        assert_eq!(
            (after.epoch, after.active, after.tab),
            (before.epoch, before.active, before.tab)
        );
        unchanged(&f, &tabs);
        ui.finish();
    }
}
