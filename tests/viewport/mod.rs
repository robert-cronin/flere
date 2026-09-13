//! Scrolling reversal keeps the visible list stable; real mouse hits use that same range.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
}
impl Ui {
    fn attach(f: &Fixture) -> Self {
        fs::write(
            f.state.join("ui.json"),
            br#"{"left":30,"right":46,"pet":false,"reduced_motion":true}"#,
        )
        .unwrap();
        let mut command = outer_ui_command();
        command
            .arg("--state")
            .arg(&f.state)
            .arg("attach")
            .env("HOME", &f.root)
            .env_remove("XDG_CACHE_HOME");
        let (master, child) = os::spawn_command_pty(&f.root, &mut command, 160, 24).unwrap();
        let mut ui = Self {
            master,
            child,
            screen: Terminal::new(160, 24),
        };
        ui.wait("FLERE");
        ui
    }
    fn key(&mut self, bytes: &[u8]) -> Vec<u8> {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 120)
    }
    fn wait(&mut self, text: &str) {
        drain_pty(&mut self.master, &mut self.screen, text);
    }
    fn find(&self, text: &str) -> (usize, usize) {
        (5..21)
            .find_map(|y| {
                let line: String = self.screen.grid.cells[y * 160 + 114..y * 160 + 160]
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect();
                line.find(text).map(|at| {
                    (
                        114 + line[..at]
                            .chars()
                            .map(|ch| flere::terminal::char_width(ch) as usize)
                            .sum::<usize>(),
                        y,
                    )
                })
            })
            .unwrap_or_else(|| panic!("missing inspector {text:?}: {}", self.screen.capture(100)))
    }
    fn top(&self) -> String {
        self.screen.grid.cells[5 * 160 + 114..6 * 160]
            .iter()
            .map(|c| c.text.as_str())
            .collect()
    }
    fn click(&mut self, point: (usize, usize), button: usize) -> Vec<u8> {
        self.key(
            format!(
                "\x1b[<{};{};{}M\x1b[<{};{};{}m",
                button,
                point.0 + 1,
                point.1 + 1,
                button,
                point.0 + 1,
                point.1 + 1
            )
            .as_bytes(),
        )
    }
    fn copy_context(&mut self, point: (usize, usize), label: &str) -> Vec<u8> {
        self.click(point, 2);
        self.wait(label);
        let action = hover_tooltips::locate(&self.screen, label);
        let bytes = self.click(action, 0);
        let at = bytes
            .windows(7)
            .position(|w| w == b"\x1b]52;c;")
            .expect("clipboard stays in test PTY")
            + 7;
        let end = bytes[at..]
            .iter()
            .position(|b| matches!(*b, 7 | 27))
            .unwrap()
            + at;
        STANDARD.decode(&bytes[at..end]).unwrap()
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

#[test]
fn files_reversal_retains_top_and_click_context_use_the_visible_row() {
    let f = Fixture::new();
    let directory = f.root.join("files");
    fs::create_dir(&directory).unwrap();
    for i in 0..60 {
        fs::write(
            directory.join(format!("file-{i:02}.txt")),
            format!("EXACT_FILE_{i:02}\n"),
        )
        .unwrap();
    }
    f.req(&[
        "new",
        &wire::hex(b"Viewport Files"),
        &wire::hex(directory.to_str().unwrap().as_bytes()),
    ]);
    let before = f.snapshot();
    let mut ui = Ui::attach(&f);
    ui.key(b"\0l");
    ui.wait("60 items");
    ui.key(&[b'j'; 90]);
    ui.wait("file-59.txt");
    let old_y = ui.find("file-59.txt").1;
    let top = ui.top();
    ui.key(b"k");
    assert_eq!(
        ui.top(),
        top,
        "reversal moves selection before scrolling the list"
    );
    let point = ui.find("file-58.txt");
    assert_eq!(point.1, old_y - 1);
    assert_eq!(
        ui.screen.grid.cells[point.1 * 160 + point.0].style.bg,
        flere::terminal::Color::Rgb(14, 42, 55)
    );
    let clicked = ui.find("file-57.txt");
    ui.click(clicked, 0);
    assert_eq!(ui.top(), top);
    assert_eq!(
        ui.screen.grid.cells[clicked.1 * 160 + clicked.0].style.bg,
        flere::terminal::Color::Rgb(14, 42, 55)
    );
    assert_eq!(
        ui.copy_context(clicked, "Copy path"),
        directory.join("file-57.txt").to_str().unwrap().as_bytes()
    );
    assert_eq!(f.snapshot().tab, before.tab);
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
    ui.key(b"\r");
    wait_current_ui(&mut ui.master, &mut ui.screen, |_| {
        f.snapshot()
            .session()
            .is_some_and(|t| t.kind == "editor" && t.path.ends_with("file-57.txt"))
    });
    ui.wait("EXACT_FILE_57");
    ui.finish();
}

#[test]
fn git_reversal_retains_top_and_click_context_use_the_visible_commit() {
    let f = Fixture::new();
    let repo = f.root.join("repo");
    fs::create_dir(&repo).unwrap();
    history_git(&repo, &["init", "-q", "-b", "main"]);
    let mut commits = Vec::new();
    for i in 0..30 {
        fs::write(
            repo.join(format!("record-{i:02}.txt")),
            format!("REVISION_{i:02}\n"),
        )
        .unwrap();
        history_git(&repo, &["add", "."]);
        history_git(&repo, &["commit", "-qm", &format!("REV_{i:02}")]);
        commits.push(
            String::from_utf8(history_git(&repo, &["rev-parse", "HEAD"]))
                .unwrap()
                .trim()
                .to_owned(),
        );
    }
    f.req(&[
        "new",
        &wire::hex(b"Viewport Git"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
    ]);
    let before = f.snapshot();
    let index = fs::read(repo.join(".git/index")).unwrap();
    let mut ui = Ui::attach(&f);
    ui.key(b"\0g");
    ui.wait("REV_29");
    ui.key(&[b'j'; 90]);
    ui.wait("REV_00");
    let old_y = ui.find("REV_00").1;
    let top = ui.top();
    ui.key(b"\x1b[A");
    assert_eq!(ui.top(), top);
    let point = ui.find("REV_01");
    assert_eq!(point.1, old_y - 1);
    assert_eq!(
        ui.screen.grid.cells[point.1 * 160 + point.0].style.bg,
        flere::terminal::Color::Rgb(14, 42, 55)
    );
    let clicked = ui.find("REV_02");
    assert_eq!(
        ui.copy_context(clicked, "Copy commit SHA"),
        commits[2].as_bytes()
    );
    ui.click(ui.find("REV_02"), 0);
    ui.wait("record-02.txt");
    assert_eq!(f.snapshot().tab, before.tab);
    assert_eq!(
        f.snapshot().session().unwrap().run,
        before.session().unwrap().run
    );
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(
        history_git(&repo, &["rev-parse", "HEAD"]),
        format!("{}\n", commits[29]).as_bytes()
    );
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
    ui.finish();
}
