use super::*;

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
    base: flere::ui::Layout,
}
impl Ui {
    fn attach(f: &Fixture, width: u16, height: u16) -> Self {
        let prefs = flere::workspace::Preferences {
            left: 26,
            right: 32,
            reduced_motion: true,
            ..Default::default()
        };
        fs::write(f.state.join("ui.json"), serde_json::to_vec(&prefs).unwrap()).unwrap();
        let mut cmd = outer_ui_command();
        cmd.arg("--state")
            .arg(&f.state)
            .arg("attach")
            .env("HOME", &f.root)
            .env_remove("XDG_CACHE_HOME");
        let (master, child) = os::spawn_command_pty(&f.root, &mut cmd, width, height).unwrap();
        let mut ui = Self {
            master,
            child,
            screen: Terminal::new(width as usize, height as usize),
            base: flere::ui::Layout::with_preferences(width as usize, height as usize, &prefs),
        };
        ui.wait(|s| s.capture(100).contains("PANE_READY_"));
        ui
    }
    fn wait(&mut self, condition: impl Fn(&Terminal) -> bool) {
        wait_current_ui(&mut self.master, &mut self.screen, condition);
    }
    fn key(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 120);
    }
    fn both(&mut self, a: u64, b: u64) {
        self.wait(|s| {
            s.capture(100).contains(&format!("PANE_READY_{a}"))
                && s.capture(100).contains(&format!("PANE_READY_{b}"))
        });
    }
    fn focused(&mut self, f: &Fixture, group: usize) {
        let snapshot = panes(f);
        let split = snapshot.split.as_ref().unwrap();
        let rect = flere::panes::geometry(
            self.base.cols,
            self.base.rows,
            split.axis,
            split.ratio,
            split.zoomed,
            split.focused,
        )[group]
            .unwrap();
        let x = self.base.terminal_x + rect.x;
        let y = self.base.terminal_y + rect.y - 1;
        self.wait(|screen| {
            screen.grid.cells[y * screen.grid.cols + x..y * screen.grid.cols + x + rect.cols]
                .iter()
                .any(|cell| {
                    cell.text == "━" && cell.style.fg == flere::terminal::Color::Rgb(67, 227, 247)
                })
        });
    }
    fn artifact(&self, name: &str) {
        let Some(root) = std::env::var_os("FLERE_TEST_SPLIT_ARTIFACTS") else {
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
    fn finish_from_nav(mut self) {
        self.key(b"\r"); // Return to the terminal without forwarding Enter to it.
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

#[test]
fn actual_ui_right_and_below_group_tabs_and_directional_focus_are_shared_between_attachments() {
    for (split_key, toward_first, toward_second, label) in
        [(b"|", b"h", b"l", "right"), (b"_", b"k", b"j", "below")]
    {
        let f = Fixture::new();
        let tabs = probes(&f, 3);
        let mut first = Ui::attach(&f, 180, 42);
        first.key(&[b"\0".as_slice(), split_key].concat());
        first.both(tabs[0].id, tabs[2].id);
        assert_eq!(panes(&f).split.as_ref().unwrap().focused, 1);
        first.key(toward_first);
        assert_eq!(panes(&f).tab, tabs[0].id);
        first.key(b"\t");
        first.both(tabs[1].id, tabs[2].id);
        let selected = panes(&f);
        assert_eq!(selected.tab, tabs[1].id);
        assert_eq!(
            selected.split.as_ref().unwrap().groups[1].selected,
            tabs[2].id
        );
        first.key(b"\x1b[Z");
        assert_eq!(panes(&f).tab, tabs[0].id);
        first.key(b"\t");
        let mut second = Ui::attach(&f, 180, 42);
        second.both(tabs[1].id, tabs[2].id);
        first.key(toward_second);
        assert_eq!(panes(&f).tab, tabs[2].id);
        second.focused(&f, 1);
        second.key(&[b"\0".as_slice(), toward_first].concat());
        assert_eq!(
            panes(&f).tab,
            tabs[1].id,
            "second attachment must observe the remembered first-group tab"
        );
        first.focused(&f, 0);
        first.key(b"\t");
        assert_eq!(
            panes(&f).tab,
            tabs[0].id,
            "first attachment must follow the shared focused group"
        );
        first.both(tabs[0].id, tabs[2].id);
        second.both(tabs[0].id, tabs[2].id);
        first.artifact(&format!("split-{label}"));
        assert!(
            tabs.iter().all(|t| input(&f, t).is_empty()),
            "navigation reached a child PTY"
        );
        first.key(b"\rFIRST_UI_DRAFT");
        wait(
            || input(&f, &tabs[0]) == b"FIRST_UI_DRAFT",
            "typing reaches the selected first-pane tab",
        );
        first.key(&[b"\0".as_slice(), toward_second, b"\rSECOND_UI_DRAFT"].concat());
        wait(
            || input(&f, &tabs[2]) == b"SECOND_UI_DRAFT",
            "typing after a pane change reaches only the second pane",
        );
        assert_eq!(input(&f, &tabs[0]), b"FIRST_UI_DRAFT");
        assert!(input(&f, &tabs[1]).is_empty());
        first.key(b"\0");
        unchanged(&f, &tabs);
        second.finish_from_nav();
        first.finish_from_nav();
        assert_eq!(input(&f, &tabs[0]), b"FIRST_UI_DRAFT");
        assert!(input(&f, &tabs[1]).is_empty());
        assert_eq!(input(&f, &tabs[2]), b"SECOND_UI_DRAFT");
    }
}

#[test]
fn actual_ui_divider_zoom_and_tiny_window_keep_both_panes_and_cancel_stale_drags() {
    for (split_key, label) in [(b"|", "right"), (b"_", "below")] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        let mut ui = Ui::attach(&f, 180, 42);
        ui.key(&[b"\0".as_slice(), split_key].concat());
        ui.both(tabs[0].id, tabs[1].id);
        let base = ui.base;
        let right = label == "right";
        let point = if right {
            (base.terminal_x + (base.cols - 1) / 2, base.terminal_y + 2)
        } else {
            (base.terminal_x + 2, base.terminal_y + (base.rows - 3) / 2)
        };
        let target = if right {
            (base.terminal_x + (base.cols - 1) / 3, point.1)
        } else {
            (point.0, base.terminal_y + (base.rows - 3) / 3)
        };
        ui.key(
            format!(
                "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
                point.0 + 1,
                point.1 + 1,
                target.0 + 1,
                target.1 + 1,
                target.0 + 1,
                target.1 + 1
            )
            .as_bytes(),
        );
        let after_drag = panes(&f);
        let ratio = after_drag.split.as_ref().unwrap().ratio;
        assert!((300..=350).contains(&ratio), "divider ratio was {ratio}");
        let other = &after_drag.split.as_ref().unwrap().other;
        let divider = if right {
            (base.terminal_x + other.cols, point.1)
        } else {
            (point.0, base.terminal_y + other.rows)
        };
        ui.key(format!("\x1b[<0;{};{}M", divider.0 + 1, divider.1 + 1).as_bytes());
        let refocused = pane(&f, "focus", Some("0"));
        ui.focused(&f, 0);
        ui.key(
            format!(
                "\x1b[<32;{};{}M\x1b[<0;{};{}m",
                point.0 + 1,
                point.1 + 1,
                point.0 + 1,
                point.1 + 1
            )
            .as_bytes(),
        );
        assert_eq!(revision(&panes(&f)), revision(&refocused));
        assert_eq!(panes(&f).split.as_ref().unwrap().ratio, ratio);
        ui.key(b"Z");
        assert!(panes(&f).split.as_ref().unwrap().zoomed);
        ui.wait(|s| {
            s.capture(100)
                .contains(&format!("PANE_READY_{}", tabs[0].id))
                && !s
                    .capture(100)
                    .contains(&format!("PANE_READY_{}", tabs[1].id))
        });
        wait(
            || size(&f, &tabs[0]) == Some((base.cols, base.rows)),
            "zoomed pane owns full centre PTY size",
        );
        ui.artifact(&format!("split-{label}-zoom"));
        ui.key(b"Z");
        ui.both(tabs[0].id, tabs[1].id);
        os::resize(ui.master.as_raw_fd(), 38, 14).unwrap();
        ui.screen.resize(38, 14);
        let tiny = flere::ui::Layout::new(38, 14);
        ui.wait(|s| {
            let current = panes(&f);
            (current.cols, current.rows) == (tiny.cols, tiny.rows)
                && s.capture(100)
                    .contains(&format!("PANE_READY_{}", tabs[0].id))
                && !s
                    .capture(100)
                    .contains(&format!("PANE_READY_{}", tabs[1].id))
        });
        assert_eq!(panes(&f).split.as_ref().unwrap().ratio, ratio);
        assert!(!panes(&f).split.as_ref().unwrap().zoomed);
        unchanged(&f, &tabs);
        ui.artifact(&format!("split-{label}-collapsed"));
        os::resize(ui.master.as_raw_fd(), 180, 42).unwrap();
        ui.screen.resize(180, 42);
        ui.both(tabs[0].id, tabs[1].id);
        assert_eq!(panes(&f).split.as_ref().unwrap().ratio, ratio);
        ui.key(b"X");
        assert!(panes(&f).split.is_none());
        unchanged(&f, &tabs);
        assert!(tabs.iter().all(|t| input(&f, t).is_empty()));
        ui.finish_from_nav();
    }
}
