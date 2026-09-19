//! Actual terminal UI: mascot interactions never become native-session input.
use super::splits::{input, pane, panes, probes, unchanged};
use super::*;

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
    nav: bool,
}
impl Ui {
    fn new(f: &Fixture, quiet: bool) -> Self {
        let prefs = flere::workspace::Preferences {
            reduced_motion: quiet,
            pet: !quiet,
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
        let (master, child) = os::spawn_command_pty(&f.root, &mut command, 180, 42).unwrap();
        let mut ui = Self {
            master,
            child,
            screen: Terminal::new(180, 42),
            nav: false,
        };
        ui.wait(|s| s.capture(100).contains("PANE_READY_"));
        ui
    }
    fn wait(&mut self, condition: impl Fn(&Terminal) -> bool) {
        if condition(&self.screen) {
            return;
        }
        wait_current_ui(&mut self.master, &mut self.screen, condition);
    }
    fn key(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 90)
    }
    fn action(&mut self, name: &str) {
        self.key(format!("{} /{name}", if self.nav { "" } else { "\0" }).as_bytes());
        self.wait(|s| s.capture(100).contains(name));
        self.key(b"\r");
        self.nav = !(name == "Start screensaver" || name.starts_with("Mascot:"));
    }
    fn body(&self) -> (usize, usize, usize, usize) {
        let white =
            |c| matches!(c, flere::terminal::Color::Rgb(r,g,b) if r > 200 && g > 200 && b > 190);
        let cells = self
            .screen
            .grid
            .cells
            .iter()
            .enumerate()
            .filter(|(_, c)| c.text == "▀" && (white(c.style.fg) || white(c.style.bg)))
            .map(|(i, _)| (i % self.screen.grid.cols, i / self.screen.grid.cols))
            .collect::<Vec<_>>();
        assert!(
            !cells.is_empty(),
            "missing white disc: {}",
            self.screen.capture(100)
        );
        (
            cells.iter().map(|p| p.0).min().unwrap(),
            cells.iter().map(|p| p.1).min().unwrap(),
            cells.iter().map(|p| p.0).max().unwrap(),
            cells.iter().map(|p| p.1).max().unwrap(),
        )
    }
    fn artifact(&self, name: &str) {
        let Some(root) = std::env::var_os("FLERE_TEST_FLERE_ARTIFACTS") else {
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
}
impl Drop for Ui {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            os::hangup(self.master.as_raw_fd(), &mut self.child);
        }
    }
}
fn saved(f: &Fixture) -> flere::workspace::Preferences {
    serde_json::from_value(super::preferences::durable(f)).unwrap()
}
#[test]
fn flere_selector_petting_throw_and_wake_preserve_live_native_tabs_and_drafts() {
    for quiet in [false, true] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        pane(&f, "split-right", Some("move"));
        let mut ui = Ui::new(&f, quiet);
        assert_eq!(
            saved(&f).screensaver_mascot,
            flere::pet::ScreensaverMascot::Flere
        );
        assert_eq!(saved(&f).screensaver_minutes, 5);
        let pet_enabled = saved(&f).pet;
        ui.action("Mascot: Flere (screensaver + pet)");
        assert_eq!(
            saved(&f).screensaver_mascot,
            flere::pet::ScreensaverMascot::Flere
        );
        assert_eq!(saved(&f).pet_kind, flere::pet::Kind::Flere);
        assert_eq!(saved(&f).pet, pet_enabled);
        let before = panes(&f);
        let identity = |s: Snapshot| (s.epoch, s.active, s.tab, s.split.unwrap().revision);
        // Selecting the mascot immediately opens the original intro scene.
        ui.wait(|s| s.capture(100).contains("Pet Flere"));
        assert!(ui.screen.capture(100).contains("Press any key to return"));
        ui.key(b"\x1b[<35;90;18M\x1b[I\x1b[O");
        assert!(ui.screen.capture(100).contains("Pet Flere"));
        let body = ui.body();
        let point = ((body.0 + body.2) / 2 + 1, (body.1 + body.3) / 2 + 1);
        ui.key(
            format!(
                "\x1b[<0;{};{}M\x1b[<0;{};{}m",
                point.0, point.1, point.0, point.1
            )
            .as_bytes(),
        );
        ui.wait(|s| {
            let text = s.capture(100);
            text.contains("Quite satisfactory.") || text.contains("A little dignity, please.")
        });
        assert!(ui.screen.capture(100).contains("SCREENSAVER"));
        ui.artifact(if quiet {
            "flere-ui-pet-quiet"
        } else {
            "flere-ui-pet"
        });
        let body = ui.body();
        let point = ((body.0 + body.2) / 2 + 1, (body.1 + body.3) / 2 + 1);
        ui.key(format!("\x1b[<0;{};{}M", point.0, point.1).as_bytes());
        ui.key(format!("\x1b[<32;{};{}M", point.0 + 25, point.1.saturating_sub(5)).as_bytes());
        ui.key(format!("\x1b[<0;{};{}m", point.0 + 25, point.1.saturating_sub(5)).as_bytes());
        let released = ui.body();
        pump_ui_bytes(&mut ui.master, &mut ui.screen, 400);
        let later = ui.body();
        if quiet {
            assert_eq!(released, later, "reduced motion suppresses throw inertia");
        } else {
            assert_ne!(released, later, "a throw must retain momentum");
        }
        assert!(later.0 >= 2 && later.2 < 178 && later.1 >= 3 && later.3 < 39);
        for tab in &tabs {
            assert!(input(&f, tab).is_empty());
        }
        assert_eq!(identity(panes(&f)), identity(before));
        unchanged(&f, &tabs);
        // A paste wakes immediately and every fragment remains owned by the saver.
        ui.key(b"\x1b[200~WAKE_FRAGMENT");
        ui.wait(|s| !s.capture(100).contains("SCREENSAVER"));
        ui.key(b"_TAIL\x1b[201~");
        for tab in &tabs {
            assert!(input(&f, tab).is_empty());
        }
        ui.key(b"fresh draft");
        let until = Instant::now() + Duration::from_secs(3);
        while input(&f, &tabs[1]) != b"fresh draft" {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        ui.action("Mascot: Duck (screensaver + pet)");
        assert_eq!(
            saved(&f).screensaver_mascot,
            flere::pet::ScreensaverMascot::Duck
        );
        assert_eq!(saved(&f).pet_kind, flere::pet::Kind::Duck);
        assert_eq!(saved(&f).pet, pet_enabled);
        unchanged(&f, &tabs);
        ui.wait(|s| s.capture(100).contains("Drag the duck to throw"));
        ui.key(b"\x1b");
        ui.wait(|s| !s.capture(100).contains("SCREENSAVER"));
        assert_eq!(input(&f, &tabs[1]), b"fresh draft");
        finish_ui(&mut ui.master, &mut ui.screen, &mut ui.child);
    }
}

#[test]
fn missing_mascot_preferences_use_flere_but_deliberate_duck_choices_survive() {
    use flere::{pet::ScreensaverMascot, workspace::Preferences};
    for json in ["{}", r#"{"screensaver_minutes":15}"#] {
        let prefs: Preferences = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.screensaver_mascot, ScreensaverMascot::Flere);
    }
    let prefs: Preferences =
        serde_json::from_str(r#"{"screensaver_mascot":"duck","screensaver_minutes":0}"#).unwrap();
    let saved: Preferences = serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
    assert_eq!(saved.screensaver_mascot, ScreensaverMascot::Duck);
    assert_eq!(saved.screensaver_minutes, 0);
}
