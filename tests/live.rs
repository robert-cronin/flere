//! Real supervisor, Unix sockets and kernel PTYs. No tmux or paid harnesses.
#[path = "chat_messages/mod.rs"]
mod chat_messages;
#[path = "dock_flere/mod.rs"]
mod dock_flere;
#[path = "flere/mod.rs"]
mod flere_mascot;
#[path = "hover_tooltips/mod.rs"]
mod hover_tooltips;
#[path = "native_launcher/mod.rs"]
mod native_launcher;
#[path = "screensaver/mod.rs"]
mod screensaver;
#[path = "search/mod.rs"]
mod search;
#[path = "tasks/mod.rs"]
mod tasks;
#[path = "terminal_input/mod.rs"]
mod terminal_input;
#[path = "updating/mod.rs"]
mod updating;
#[path = "viewport/mod.rs"]
mod viewport;
use flere::{
    model::{Snapshot, TabView},
    os,
    terminal::Terminal,
    wire,
};
use std::{
    fs,
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::PermissionsExt, net::UnixStream},
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
// Test attachments model an ordinary outer terminal, rather than a workspace
// child. The PTY helper marks children, so clear only that nesting marker at exec.
fn outer_ui_command() -> Command {
    let mut command = Command::new("/usr/bin/env");
    command.args(["-u", "FLERE", env!("CARGO_BIN_EXE_flere")]);
    command
}
// Use the real CPython executable, not macOS's /usr/bin/python3 Xcode launcher.
// Relocated Darwin test copies need their library path repaired and ad-hoc signing;
// only disposable fixture executables are changed, never an installed interpreter.
#[cfg(target_os = "macos")]
fn mac_python() -> &'static (PathBuf, PathBuf) {
    static PYTHON: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();
    PYTHON.get_or_init(|| {
        let output = Command::new("/usr/bin/python3")
            .args(["-c", "import sys;print(sys.executable);print(sys.prefix)"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let mut lines = text.lines();
        {
            let _launcher = lines.next().unwrap();
            let prefix = PathBuf::from(lines.next().unwrap());
            (
                prefix.join("Resources/Python.app/Contents/MacOS/Python"),
                prefix,
            )
        }
    })
}
fn copy_fixture_python(destination: &std::path::Path) {
    #[cfg(target_os = "linux")]
    let source = std::path::Path::new("/usr/bin/python3");
    #[cfg(target_os = "macos")]
    let source = &mac_python().0;
    assert!(
        Command::new("cp")
            .arg(source)
            .arg(destination)
            .status()
            .unwrap()
            .success()
    );
    #[cfg(target_os = "macos")]
    {
        let library = mac_python().1.join("Python3");
        assert!(
            library.is_file(),
            "macOS fixture requires Xcode's Python3 framework"
        );
        assert!(
            Command::new("install_name_tool")
                .args(["-change", "@executable_path/../../../../Python3"])
                .arg(&library)
                .arg(destination)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(destination)
                .status()
                .unwrap()
                .success()
        );
    }
}
struct Fixture {
    root: PathBuf,
    state: PathBuf,
    child: Child,
}
impl Fixture {
    fn new() -> Self {
        Self::with_editor("/usr/bin/vim -Nu NONE -n --noplugin")
    }
    fn with_editor(editor: &str) -> Self {
        Self::with_shell_editor("/bin/sh", editor)
    }
    fn with_shell_editor(shell: &str, editor: &str) -> Self {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(&os::nonce().unwrap()[..12]);
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let state = root.join("s");
        let log = fs::File::create(root.join("log")).unwrap();
        let mut server = Command::new(env!("CARGO_BIN_EXE_flere"));
        #[cfg(target_os = "macos")]
        {
            // Keep every fixture on the same interpreter as the relocated native stand-in.
            // Otherwise a Homebrew Python on PATH can inherit an incompatible PYTHONHOME.
            server.env("PYTHONHOME", &mac_python().1).env(
                "PATH",
                format!(
                    "/usr/bin:/bin:/usr/sbin:/sbin:{}",
                    std::env::var("PATH").unwrap_or_default()
                ),
            );
        }
        let child = server
            .arg("--state")
            .arg(&state)
            .arg("serve")
            .env("SHELL", shell)
            .env("HOME", &root)
            .env("TMPDIR", &root)
            .env("NVIM_LOG_FILE", root.join("nvim.log"))
            .env_remove("XDG_CACHE_HOME")
            .env("ENV", "")
            .env("PS1", "$ ")
            .env("VISUAL", editor)
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut f = Self { root, state, child };
        f.ready();
        f
    }
    fn ready(&mut self) {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if wire::request(&self.state, &["ping"]).is_ok() {
                return;
            }
            if let Some(s) = self.child.try_wait().unwrap() {
                panic!(
                    "server exited {s}: {}",
                    fs::read_to_string(self.root.join("log")).unwrap()
                )
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("server readiness timeout")
    }
    fn req(&self, p: &[&str]) -> Vec<u8> {
        wire::request(&self.state, p).unwrap()
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot::decode(&self.req(&["snapshot"])).unwrap()
    }
    fn new_workspace(&self, name: &str) -> TabView {
        self.req(&[
            "new",
            &wire::hex(name.as_bytes()),
            &wire::hex(self.root.to_str().unwrap().as_bytes()),
        ]);
        self.snapshot().session().unwrap().clone()
    }
    fn send(&self, t: &TabView, b: &[u8]) {
        self.req(&["input", &t.id.to_string(), &t.run, &wire::hex(b)]);
    }
    fn capture(&self, t: &TabView) -> String {
        String::from_utf8(self.req(&["capture", &t.id.to_string(), &t.run, "80"])).unwrap()
    }
    fn wait_text(&self, t: &TabView, marker: &str) {
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if self.capture(t).contains(marker) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("missing {marker:?}: {}", self.capture(t))
    }
    fn stop(&mut self) {
        self.req(&["stop"]);
        let end = Instant::now() + Duration::from_secs(3);
        while Instant::now() < end {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("supervisor did not stop")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = wire::request(&self.state, &["stop"]);
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if std::thread::panicking() {
            eprintln!(
                "Retained failing fixture {}: {}",
                self.root.display(),
                fs::read_to_string(self.root.join("log")).unwrap_or_default()
            );
            return;
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn exact_target_drafts_tabs_watch_and_manual_restart() {
    let mut f = Fixture::new();
    let first = f.new_workspace("one");
    let first_w = f.snapshot().active;
    let second = f.new_workspace("two");
    let second_w = f.snapshot().active;
    f.send(&first, b"printf 'FIRST_%s\\n' OK\r");
    f.send(&second, b"printf 'SECOND_%s\\n' OK\r");
    f.wait_text(&first, "FIRST_OK");
    f.wait_text(&second, "SECOND_OK");
    assert!(!f.capture(&second).contains("FIRST_OK"));
    let bad = wire::request(
        &f.state,
        &[
            "input",
            &second.id.to_string(),
            &first.run,
            &wire::hex(b"WRONG\r"),
        ],
    );
    assert!(bad.is_err());
    assert!(!f.capture(&second).contains("WRONG"));
    f.send(&first, b"printf 'DRAFT_%s\\n' PRESERVED");
    f.req(&["focus", &first_w.to_string(), "0"]);
    f.req(&["focus", &second_w.to_string(), "0"]);
    f.req(&["resize", "113", "31"]);
    f.send(&first, b"\r");
    f.wait_text(&first, "DRAFT_PRESERVED");
    assert_eq!(f.snapshot().session().unwrap().pid, second.pid);
    f.req(&["tab", &first_w.to_string()]);
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
    assert_eq!(
        f.snapshot()
            .workspaces
            .iter()
            .find(|w| w.id == second_w)
            .unwrap()
            .tabs
            .len(),
        1
    );
    let mut watcher = wire::connect(&f.state).unwrap();
    watcher.write_all(&wire::frame(b"watch")).unwrap();
    let initial = Snapshot::decode(&wire::read_frame(&mut watcher).unwrap()).unwrap();
    assert_eq!(initial.cols, 113);
    drop(watcher);
    f.send(&first, b"sleep 0.1; printf 'DETACHED_%s\\n' ALIVE\r");
    f.wait_text(&first, "DETACHED_ALIVE");
    let audit = fs::read_to_string(f.state.join("actions.log")).unwrap();
    assert!(audit.contains("\tinput\t"));
    assert!(!audit.contains("DRAFT"));
    let old_epoch = f.snapshot().epoch;
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("SHELL", "/bin/sh")
        .env("HOME", &f.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let after = f.snapshot();
    assert_ne!(after.epoch, old_epoch);
    assert_eq!(after.workspaces.len(), 2);
    assert!(after.workspaces.iter().all(|w| w.tabs.is_empty()));
    assert!(
        wire::request(
            &f.state,
            &["input", &first.id.to_string(), &first.run, "61"]
        )
        .is_err()
    );
    f.req(&["focus", &second_w.to_string(), "0"]);
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
    f.req(&["tab", &first_w.to_string()]);
    assert_eq!(
        f.snapshot().workspaces.iter().flat_map(|w| &w.tabs).count(),
        1
    );
}
#[test]
fn malformed_slow_clients_do_not_block_output_and_lock_rejects_duplicate() {
    let f = Fixture::new();
    let t = f.new_workspace("isolation");
    let mut slow = UnixStream::connect(f.state.join("control.sock")).unwrap();
    slow.write_all(&[0, 0, 1]).unwrap();
    let mut oversized = UnixStream::connect(f.state.join("control.sock")).unwrap();
    oversized.write_all(&u32::MAX.to_be_bytes()).unwrap();
    f.send(&t, b"printf 'FAIR_%s\\n' OUTPUT\r");
    f.wait_text(&t, "FAIR_OUTPUT");
    let duplicate = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .output()
        .unwrap();
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("another supervisor"));
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(
        fs::metadata(f.state.join("control.sock"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
#[test]
fn watch_survives_a_slow_reader_and_resumes_without_restarting_the_shell() {
    let f = Fixture::new();
    let tab = f.new_workspace("slow UI");
    f.req(&["resize", "240", "100"]);
    let epoch = f.snapshot().epoch;
    let mut watcher = wire::connect(&f.state).unwrap();
    watcher.write_all(&wire::frame(b"watch")).unwrap();

    // A full-size snapshot exceeds the socket send buffer. Model a frontend
    // blocked on SSH output for longer than an ordinary request's two seconds.
    f.send(&tab, b"printf 'WATCH_%s\\n' ALIVE\r");
    f.wait_text(&tab, "WATCH_ALIVE");
    f.send(&tab, b"unsubmitted_draft");
    std::thread::sleep(Duration::from_secs(3));
    assert!(f.capture(&tab).contains("unsubmitted_draft"));

    let initial = Snapshot::decode(&wire::read_frame(&mut watcher).unwrap()).unwrap();
    assert_eq!(initial.epoch, epoch);
    assert_eq!(initial.session().unwrap().pid, tab.pid);
    assert_eq!(initial.session().unwrap().run, tab.run);

    // The same stream must deliver a fresh complete frame after its backlog.
    f.req(&["resize", "239", "100"]);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let frame = Snapshot::decode(&wire::read_frame(&mut watcher).unwrap()).unwrap();
        assert_eq!(frame.epoch, epoch);
        assert_eq!(frame.session().unwrap().pid, tab.pid);
        if frame.cols == 239 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "watch did not resume fresh frames"
        );
    }
    assert!(f.capture(&tab).contains("unsubmitted_draft"));
}
// A Darwin PTY may split a paint into many short reads. Assertions must observe
// a complete DEC 2026 frame, and bulk input must allow the UI to drain output.
fn read_ui_bytes(master: &mut fs::File, screen: &mut Terminal, raw: &mut Vec<u8>) {
    loop {
        let mut bytes = [0; 65536];
        match master.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => {
                screen.feed(&bytes[..n]);
                raw.extend_from_slice(&bytes[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.raw_os_error() == Some(libc::EIO) =>
            {
                break;
            }
            Err(e) => panic!("PTY read: {e}"),
        }
    }
}
fn complete_frame(raw: &[u8]) -> bool {
    let start = raw.windows(8).rposition(|w| w == b"\x1b[?2026h");
    let end = raw.windows(8).rposition(|w| w == b"\x1b[?2026l");
    start.is_none() || end > start
}
fn write_ui_bytes(master: &mut fs::File, screen: &mut Terminal, mut bytes: &[u8]) {
    let end = Instant::now() + Duration::from_secs(3);
    while !bytes.is_empty() && Instant::now() < end {
        match master.write(bytes) {
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("PTY write: {e}"),
        }
        read_ui_bytes(master, screen, &mut Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(bytes.is_empty(), "PTY input did not drain");
}
fn drain_pty(master: &mut fs::File, term: &mut Terminal, until: &str) {
    wait_current_ui(master, term, |s| s.capture(100).contains(until));
}
fn wait_ui_text(
    f: &Fixture,
    t: &TabView,
    master: &mut fs::File,
    screen: &mut Terminal,
    marker: &str,
) {
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        let mut b = [0; 65536];
        if let Ok(n) = master.read(&mut b) {
            screen.feed(&b[..n]);
        }
        if f.capture(t).contains(marker) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!(
        "missing {marker}: {}; UI: {}",
        f.capture(t),
        screen.capture(100)
    );
}
#[test]
fn actual_ui_navigation_detach_reattach_and_terminal_restore() {
    let f = Fixture::new();
    let first = f.new_workspace("first card");
    let first_w = f.snapshot().active;
    let second = f.new_workspace("second card");
    let (mut master, mut child) = os::spawn_pty(&f.root, "/bin/sh", 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    let command = format!(
        "exec /usr/bin/env -u FLERE '{}' --state '{}' attach\r",
        env!("CARGO_BIN_EXE_flere"),
        f.state.display()
    );
    master.write_all(command.as_bytes()).unwrap();
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"printf 'UI_%s\\n' SECOND\r").unwrap();
    wait_ui_text(&f, &second, &mut master, &mut screen, "UI_SECOND");
    master.write_all(b"\0hk").unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while f.snapshot().active != first_w && Instant::now() < end {
        let mut b = [0; 65536];
        if let Ok(n) = master.read(&mut b) {
            screen.feed(&b[..n]);
        }
        std::thread::sleep(Duration::from_millis(10))
    }
    assert_eq!(f.snapshot().active, first_w);
    master.write_all(b"\0l\rprintf 'UI_%s\\n' FIRST\r").unwrap();
    wait_ui_text(&f, &first, &mut master, &mut screen, "UI_FIRST");
    assert!(!f.capture(&second).contains("UI_FIRST"));
    master
        .write_all(b"\x1b[200~printf 'PASTE_%s\\n' OK\x1b[201~\r")
        .unwrap();
    wait_ui_text(&f, &first, &mut master, &mut screen, "PASTE_OK");
    assert_eq!(f.snapshot().workspaces.len(), 2);
    master.write_all(b"printf 'UNCHANGED_%s\\n' DRAFT").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    master.write_all(b"\0q").unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(child.try_wait().unwrap().unwrap().success());
    assert_eq!(f.snapshot().session().unwrap().pid, first.pid);
    let (mut second_ui, mut child2) = os::spawn_pty(&f.root, "/bin/sh", 80, 24).unwrap();
    let mut screen2 = Terminal::new(80, 24);
    second_ui.write_all(command.as_bytes()).unwrap();
    drain_pty(&mut second_ui, &mut screen2, "FLERE");
    second_ui.write_all(b"\r").unwrap();
    wait_ui_text(&f, &first, &mut second_ui, &mut screen2, "UNCHANGED_DRAFT");
    os::resize(second_ui.as_raw_fd(), 20, 12).unwrap();
    screen2.resize(20, 12);
    wait_current_ui(&mut second_ui, &mut screen2, |_| f.snapshot().cols == 18);
    let mut buf = [0; 65536];
    assert_eq!(f.snapshot().cols, 18);
    assert_eq!(f.snapshot().session().unwrap().pid, first.pid);
    second_ui.write_all(b"\0q").unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while child2.try_wait().unwrap().is_none() && Instant::now() < end {
        let _ = second_ui.read(&mut buf);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(child2.try_wait().unwrap().unwrap().success());
}

#[test]
fn real_vim_alternate_screen_edit_and_return_to_shell() {
    let vim = std::path::Path::new("/usr/bin/vim");
    if !vim.is_file() {
        eprintln!("optional native Vim proof skipped: /usr/bin/vim unavailable");
        return;
    }
    let f = Fixture::new();
    let t = f.new_workspace("native TUI");
    fs::write(f.root.join("scratch.txt"), "original line\n").unwrap();
    f.send(&t, b"/usr/bin/vim -Nu NONE -n --noplugin scratch.txt\r");
    f.wait_text(&t, "original line");
    f.send(&t, b"ggIflere \x1b:wq\r");
    let end = Instant::now() + Duration::from_secs(3);
    while fs::read_to_string(f.root.join("scratch.txt")).unwrap() != "flere original line\n"
        && Instant::now() < end
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(f.root.join("scratch.txt")).unwrap(),
        "flere original line\n"
    );
    f.wait_text(&t, "\\n$");
    f.send(&t, b"printf 'SAME_%s\n' SHELL\r");
    f.wait_text(&t, "SAME_SHELL");
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
}
#[test]
fn cli_literal_text_does_not_become_a_key_or_option() {
    let f = Fixture::new();
    let t = f.new_workspace("literal");
    for value in ["--state", "--key", "--terminate"] {
        let out = Command::new(env!("CARGO_BIN_EXE_flere"))
            .arg("--state")
            .arg(&f.state)
            .args([
                "send",
                "--session",
                &t.id.to_string(),
                "--run",
                &t.run,
                "--text",
                value,
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        f.wait_text(&t, value);
        f.send(&t, b"\x15");
    }
    assert!(f.snapshot().session().unwrap().alive);
}

#[test]
fn editor_tabs_reuse_canonical_path_and_exit_leaves_shell_draft() {
    assert!(
        std::path::Path::new("/usr/bin/vim").is_file(),
        "native editor test requires Vim"
    );
    let f = Fixture::new();
    let shell = f.new_workspace("editor card");
    let wid = f.snapshot().active;
    f.send(&shell, b"printf 'KEPT_%s\\n' DRAFT");
    let file = f.root.join("--strange ' file.txt");
    fs::write(&file, "starting line\n").unwrap();
    f.req(&[
        "open",
        &wid.to_string(),
        &wire::hex(file.to_str().unwrap().as_bytes()),
    ]);
    let editor = f.snapshot().session().unwrap().clone();
    assert_eq!(editor.kind, "editor");
    assert_ne!(editor.id, shell.id);
    f.wait_text(&editor, "starting line");
    f.send(&editor, b"ggIedited \x1b");
    let alias = f.root.join("alias.txt");
    std::os::unix::fs::symlink(&file, &alias).unwrap();
    f.req(&[
        "open",
        &wid.to_string(),
        &wire::hex(alias.to_str().unwrap().as_bytes()),
    ]);
    assert_eq!(f.snapshot().session().unwrap().pid, editor.pid);
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
    f.send(&editor, b":wq\r");
    let end = Instant::now() + Duration::from_secs(3);
    while f.snapshot().workspace().unwrap().tabs.len() != 1 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().session().unwrap().pid, shell.pid);
    assert_eq!(fs::read_to_string(&file).unwrap(), "edited starting line\n");
    assert!(
        wire::request(
            &f.state,
            &["input", &editor.id.to_string(), &editor.run, "61"]
        )
        .is_err()
    );
    f.send(&shell, b"\r");
    f.wait_text(&shell, "KEPT_DRAFT");
    f.send(&shell, b"exit\r");
    let end = Instant::now() + Duration::from_secs(3);
    while !f.snapshot().workspace().unwrap().tabs.is_empty() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    assert_eq!(f.snapshot().workspaces.len(), 1);
}
#[test]
fn workflow_persistence_archiving_guard_and_no_automatic_restart() {
    let mut f = Fixture::new();
    let t = f.new_workspace("Lead");
    let wid = f.snapshot().active;
    let mut meta = flere::workspace::CardMeta {
        project: "Flere".into(),
        legacy_lead: true,
        status: flere::workspace::Workflow::NeedsMe,
        ..Default::default()
    };
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    assert!(!f.snapshot().workspace().unwrap().meta.pinned);
    meta.archived = true;
    assert!(
        wire::request(
            &f.state,
            &[
                "metadata",
                &wid.to_string(),
                &wire::hex(&serde_json::to_vec(&meta).unwrap())
            ]
        )
        .is_err()
    );
    assert!(f.snapshot().session().unwrap().alive);
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let snap = f.snapshot();
    let w = snap.workspace().unwrap();
    assert!(w.meta.legacy_lead && !w.meta.pinned);
    assert_eq!(w.meta.project, "Flere");
    assert_eq!(w.meta.status, flere::workspace::Workflow::NeedsMe);
    assert!(w.tabs.is_empty());
    assert!(wire::request(&f.state, &["capture", &t.id.to_string(), &t.run, "10"]).is_err());
}
#[test]
fn installed_git_view_reads_unusual_paths_and_staged_diff_without_mutations() {
    let f = Fixture::new();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&f.root)
            .status()
            .unwrap()
            .success()
    );
    let filename = "--draft ' file.txt";
    fs::write(f.root.join(filename), "hello\n").unwrap();
    let view = flere::git::inspect(&f.root);
    assert!(view.error.is_empty(), "{}", view.error);
    assert_eq!(view.changes[0].path, filename);
    assert_eq!(view.changes[0].status, "??");
    assert!(
        Command::new("git")
            .current_dir(&f.root)
            .args(["add", "--", filename])
            .status()
            .unwrap()
            .success()
    );
    let before = fs::read(f.root.join(".git/index")).unwrap();
    let diff = flere::git::diff(&f.root, filename).unwrap();
    assert!(diff.contains("+hello"));
    assert_eq!(fs::read(f.root.join(".git/index")).unwrap(), before);
}

#[test]
fn native_host_exact_resume_arguments_waits_and_returns_to_default_shell() {
    let f = Fixture::new();
    let first = f.new_workspace("native controls");
    let wid = f.snapshot().active;
    let script = f.root.join("native stand-in.sh");
    fs::write(&script,"#!/bin/sh\nprintf '%s\\n' \"$@\" > native-argv\nprintf 'STANDIN_%s\\n' WAITING\nread response\nprintf 'STANDIN_%s\\n' FINISHED\n").unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&serde_json::json!([{"name":"codex","command":["/bin/sh",script]}]))
            .unwrap(),
    )
    .unwrap();
    let uuid = "12345678-1234-1234-1234-123456789012";
    f.req(&["native", &wid.to_string(), "codex", uuid]);
    let native = f.snapshot().session().unwrap().clone();
    f.wait_text(&native, "STANDIN_WAITING");
    assert_eq!(native.kind, "agent");
    assert_eq!(
        fs::read_to_string(f.root.join("native-argv")).unwrap(),
        format!("resume\n{uuid}\n")
    );
    assert!(wire::request(&f.state, &["native", &wid.to_string(), "codex", uuid]).is_err());
    assert!(!f.capture(&native).contains("STANDIN_FINISHED"));
    f.send(&native, b"finish\r");
    f.wait_text(&native, "Returning to /bin/sh");
    let end = Instant::now() + Duration::from_secs(2);
    while f.snapshot().session().unwrap().kind != "shell" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    f.send(&native, b"printf 'AFTER_%s\\n' NATIVE\r");
    f.wait_text(&native, "AFTER_NATIVE");
    assert_eq!(f.snapshot().session().unwrap().pid, native.pid);
    assert_eq!(f.snapshot().session().unwrap().kind, "shell");
    assert!(
        f.snapshot()
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.pid == first.pid)
    );
}

#[test]
fn actual_ui_search_forms_git_shared_width_and_exact_close_cancel() {
    let f = Fixture::new();
    let first = f.new_workspace("alpha");
    let first_w = f.snapshot().active;
    let second = f.new_workspace("bravo");
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"printf 'DRAFT_%s\\n' SAFE").unwrap();
    master.write_all(b"\0/alpha\r").unwrap();
    drain_pty(&mut master, &mut screen, "alpha");
    let end = Instant::now() + Duration::from_secs(2);
    while f.snapshot().active != first_w && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().active, first_w);
    master.write_all(b"\0]]gp2").unwrap();
    // All inspector tabs are now visible before selection. Wait for the actual
    // metadata actions, rather than treating the static Git label as readiness.
    wait_current_ui(&mut master, &mut screen, |_| {
        let s = f.snapshot();
        s.workspace().is_some_and(|w| {
            w.meta.pinned && w.meta.status == flere::workspace::Workflow::InProgress
        }) && s.cols == 68
    });
    let s = f.snapshot();
    assert!(s.workspace().unwrap().meta.pinned);
    assert_eq!(
        s.workspace().unwrap().meta.status,
        flere::workspace::Workflow::InProgress
    );
    assert_eq!(s.cols, 68); // 140 - shared left 30 - right 40 - borders 2
    master.write_all(b"/bravo\r\0xno").unwrap(); // 'n' cancels; remaining nav text must never type into native draft
    let end = Instant::now() + Duration::from_secs(2);
    while f.snapshot().session().unwrap().id != second.id && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().cols, 68);
    assert_eq!(f.snapshot().session().unwrap().pid, second.pid);
    master.write_all(b"\0\r").unwrap();
    wait_ui_text(&f, &second, &mut master, &mut screen, "DRAFT_SAFE");
    assert!(!f.capture(&first).contains("DRAFT_SAFE"));
    master.write_all(b"\0q").unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(child.try_wait().unwrap().unwrap().success());
    let prefs: flere::workspace::Preferences =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(prefs.left, 30);
    assert_eq!(prefs.inspector, flere::workspace::Inspector::Git);
}

#[test]
fn ui_refresh_keeps_exact_supervisor_shell_editor_and_unsubmitted_draft() {
    let f = Fixture::new();
    let shell = f.new_workspace("refresh fixture");
    let mut cmd = outer_ui_command();
    cmd.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut ui_child) = os::spawn_command_pty(&f.root, &mut cmd, 100, 30).unwrap();
    let ui_pid = ui_child.id();
    let mut screen = Terminal::new(100, 30);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"printf 'REFRESH_%s\\n' DRAFT").unwrap();
    master.write_all(b"\0 R").unwrap();
    // Drain through the re-exec and fresh watch subscription; a native process must not restart.
    let until = Instant::now() + Duration::from_millis(350);
    while Instant::now() < until {
        let mut b = [0; 65536];
        if let Ok(n) = master.read(&mut b) {
            screen.feed(&b[..n]);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ui_child.try_wait().unwrap().is_none());
    assert_eq!(ui_child.id(), ui_pid);
    assert_eq!(f.snapshot().session().unwrap().pid, shell.pid);
    master.write_all(b"\r").unwrap();
    wait_ui_text(&f, &shell, &mut master, &mut screen, "REFRESH_DRAFT");
    os::resize(master.as_raw_fd(), 40, 20).unwrap();
    std::thread::sleep(Duration::from_millis(250));
    screen.resize(40, 20);
    master.write_all(b"\0h").unwrap();
    drain_pty(&mut master, &mut screen, "Workspaces");
    master.write_all(b"ll").unwrap();
    drain_pty(&mut master, &mut screen, "Files");
    master.write_all(b"q").unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while ui_child.try_wait().unwrap().is_none() && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ui_child.try_wait().unwrap().unwrap().success());
}
#[test]
fn codex_identity_discovery_uses_only_owned_open_cli_metadata() {
    let f = Fixture::new();
    let executable = f.root.join("codex");
    copy_fixture_python(&executable);
    let id = "12345678-1234-1234-1234-123456789012";
    let file = f.root.join("rollout-owned.jsonl");
    fs::write(&file,serde_json::to_string(&serde_json::json!({"type":"session_meta","payload":{"id":id,"cwd":f.root,"source":"cli"}})).unwrap()+"\n").unwrap();
    let mut command = Command::new(&executable);
    command.env("FLERE_TEST_METADATA", "value with spaces=retained");
    command
        .args([
            "-c",
            "import sys; held=open(sys.argv[1]); print('READY', flush=True); sys.stdin.readline()",
        ])
        .arg(&file);
    #[cfg(target_os = "macos")]
    command.env("PYTHONHOME", &mac_python().1);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 80, 24).unwrap();
    let mut screen = Terminal::new(80, 24);
    drain_pty(&mut master, &mut screen, "READY");
    let (parent, start) = os::child_identity(child.id()).unwrap();
    assert_eq!(parent, std::process::id());
    assert!(!start.is_empty());
    assert!(os::Process::restore(child.id(), None, "wrong-start-token").is_err());
    assert!(
        os::process_arguments(child.id())
            .unwrap()
            .split(|b| *b == 0)
            .any(|arg| arg == file.as_os_str().as_encoded_bytes())
    );
    assert!(
        os::process_environment(child.id())
            .unwrap()
            .split(|b| *b == 0)
            .any(|entry| entry == b"FLERE_TEST_METADATA=value with spaces=retained")
    );
    assert!(os::process_children(child.id(), 0).unwrap().is_empty());
    assert!(os::process_files(child.id(), 0).is_err());
    assert_eq!(os::process_executable(child.id()).unwrap(), executable);
    assert_eq!(os::process_cwd(child.id()).unwrap(), f.root);
    let files = os::process_files(child.id(), 256).expect("inspect owned descriptors");
    let held = files
        .iter()
        .find(|entry| entry.path == file)
        .expect("owned rollout descriptor");
    let mut metadata = String::new();
    held.open()
        .expect("open verified descriptor")
        .read_to_string(&mut metadata)
        .unwrap();
    assert!(metadata.contains(id));
    assert_eq!(
        flere::native::discover_codex(child.id(), &f.root)
            .unwrap()
            .as_deref(),
        Some(id)
    );
    assert!(
        flere::native::discover_codex(child.id(), &f.root.join("elsewhere"))
            .unwrap()
            .is_none()
    );
    #[cfg(target_os = "macos")]
    {
        // A same-name replacement must never be mistaken for the owned open FD.
        fs::rename(&file, f.root.join("original.jsonl")).unwrap();
        fs::write(&file, "replacement").unwrap();
        assert!(held.open().is_err());
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(f.root.join("original.jsonl"), &file).unwrap();
        assert!(held.open().is_err());
    }
    master.write_all(b"done\r").unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() && Instant::now() < end {
        let mut bytes = [0; 65536];
        let _ = master.read(&mut bytes);
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = child.try_wait().unwrap();
    if status.is_none() {
        let _ = child.kill();
        drop(master);
    }
    assert!(status.is_some_and(|s| s.success()), "fixture did not exit");
}

#[test]
fn asynchronous_worktree_creation_preserves_source_checkout_and_focus() {
    let f = Fixture::new();
    let repo = f.root.join("source");
    fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Flere Fixture"],
        vec!["config", "user.email", "fixture@example.invalid"],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(repo.join("tracked.txt"), "committed\n").unwrap();
    for args in [
        vec!["add", "tracked.txt"],
        vec!["-c", "commit.gpgsign=false", "commit", "-qm", "fixture"],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    fs::write(repo.join("tracked.txt"), "user draft\n").unwrap();
    let original = flere::git::output(&repo, &["rev-parse", "HEAD"]).unwrap();
    let first = f.new_workspace("first");
    let first_w = f.snapshot().active;
    f.req(&[
        "worktree",
        &wire::hex(b"new checkout"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
        &wire::hex(b"flere-fixture"),
        &wire::hex(b"HEAD"),
        &wire::hex(b"Demo"),
    ]);
    let wid = f.snapshot().active;
    f.req(&["focus", &first_w.to_string(), &first.id.to_string()]);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let snap = f.snapshot();
        let w = snap.workspaces.iter().find(|w| w.id == wid).unwrap();
        if !w.tabs.is_empty() {
            assert!(w.meta.operation.is_empty());
            assert_eq!(w.meta.base_sha, String::from_utf8_lossy(&original).trim());
            assert!(PathBuf::from(&w.cwd).starts_with(f.state.join("worktrees")));
            assert_eq!(
                fs::read_to_string(PathBuf::from(&w.cwd).join("tracked.txt")).unwrap(),
                "committed\n"
            );
            break;
        }
        assert!(
            Instant::now() < end,
            "worktree not ready: {}",
            w.meta.operation
        );
        std::thread::sleep(Duration::from_millis(15));
    }
    assert_eq!(f.snapshot().active, first_w);
    assert_eq!(f.snapshot().session().unwrap().pid, first.pid);
    assert_eq!(
        fs::read_to_string(repo.join("tracked.txt")).unwrap(),
        "user draft\n"
    );
    assert_eq!(
        flere::git::output(&repo, &["rev-parse", "HEAD"]).unwrap(),
        original
    );
}

#[test]
fn supervisor_refresh_preserves_pid_pty_screens_drafts_watch_and_reaps_children() {
    let f = Fixture::new();
    let first = f.new_workspace("one");
    let wid = f.snapshot().active;
    f.send(&first, b"printf 'BEFORE_%s\\n' REFRESH\r");
    f.wait_text(&first, "BEFORE_REFRESH");
    f.send(&first, b"printf 'PENDING_%s\\n' DRAFT");
    let file = f.root.join("refresh-edit.txt");
    fs::write(&file, "original\n").unwrap();
    f.req(&[
        "open",
        &wid.to_string(),
        &wire::hex(file.to_str().unwrap().as_bytes()),
    ]);
    let editor = f.snapshot().session().unwrap().clone();
    f.wait_text(&editor, "original");
    f.send(&editor, b"ggIretained \x1b");
    f.wait_text(&editor, "retained original");
    let second = f.new_workspace("two");
    let epoch = f.snapshot().epoch;
    let supervisor_pid = f.child.id();
    let mut watch = wire::connect(&f.state).unwrap();
    watch.write_all(&wire::frame(b"watch")).unwrap();
    wire::read_frame(&mut watch).unwrap();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let status = String::from_utf8(f.req(&["refresh-status"])).unwrap();
        if status.contains("all terminal sessions preserved") {
            break;
        }
        assert!(Instant::now() < end, "refresh incomplete: {status}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.child.id(), supervisor_pid);
    assert_eq!(f.snapshot().epoch, epoch);
    assert_eq!(f.snapshot().session().unwrap().pid, second.pid);
    assert!(
        f.snapshot()
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .any(|t| t.id == editor.id && t.pid == editor.pid)
    );
    f.send(&editor, b":wq\r");
    let end = Instant::now() + Duration::from_secs(3);
    while fs::read_to_string(&file).unwrap() != "retained original\n" && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), "retained original\n");
    f.req(&["focus", &wid.to_string(), &first.id.to_string()]);
    f.send(&first, b"\r");
    f.wait_text(&first, "PENDING_DRAFT");
    assert!(f.capture(&first).contains("BEFORE_REFRESH"));
    let bytes = wire::read_frame(&mut watch).unwrap();
    assert_eq!(Snapshot::decode(&bytes).unwrap().epoch, epoch);
    f.req(&["close", &second.id.to_string(), &second.run]);
    let end = Instant::now() + Duration::from_secs(3);
    while f
        .snapshot()
        .workspaces
        .iter()
        .any(|w| w.tabs.iter().any(|t| t.id == second.id))
        && Instant::now() < end
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !f.snapshot()
            .workspaces
            .iter()
            .any(|w| w.tabs.iter().any(|t| t.id == second.id))
    );
    assert_eq!(
        fs::read_dir(&f.state)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("refresh-"))
            .count(),
        0
    );
    // :wq writes the file before the editor finishes exiting. Wait for reaping
    // so this assertion tests candidate rejection, not a concurrently exiting tab.
    let end = Instant::now() + Duration::from_secs(3);
    while f
        .snapshot()
        .workspaces
        .iter()
        .any(|w| w.tabs.iter().any(|t| t.id == editor.id))
    {
        assert!(Instant::now() < end, "editor did not finish exiting");
        std::thread::sleep(Duration::from_millis(10));
    }
    // A binary that cannot validate the handoff must leave this supervisor and draft alive.
    f.req(&["refresh", &wire::hex(b"/usr/bin/false")]);
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let status = String::from_utf8(f.req(&["refresh-status"])).unwrap();
        if status.contains("rejected") {
            break;
        }
        assert!(Instant::now() < end, "{status}");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().session().unwrap().pid, first.pid);
    f.send(&first, b"printf 'REJECTED_%s\\n' SAFE\r");
    f.wait_text(&first, "REJECTED_SAFE");
}

#[test]
fn local_coordination_exact_recipient_delivery_states_decisions_and_mcp_stdio() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let _ = f.new_workspace("sender");
    let sender_w = f.snapshot().active;
    let script = f.root.join("wait-native.sh");
    fs::write(&script, "#!/bin/sh\nprintf 'WAITING\\n'\nread input\n").unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/bin/sh",script]}])).unwrap(),
    )
    .unwrap();
    f.req(&["native", &sender_w.to_string(), "codex", ""]);
    let sender = f.snapshot().session().unwrap().clone();
    f.wait_text(&sender, "WAITING");
    f.new_workspace("recipient");
    let recipient_w = f.snapshot().active;
    f.req(&["native", &recipient_w.to_string(), "codex", ""]);
    let recipient = f.snapshot().session().unwrap().clone();
    f.wait_text(&recipient, "WAITING");
    let call = |t: &TabView, op: &str, args: Value| -> Value {
        serde_json::from_slice(&f.req(&[
            "agent-operation",
            &t.id.to_string(),
            &t.run,
            op,
            &wire::hex(&serde_json::to_vec(&args).unwrap()),
        ]))
        .unwrap()
    };
    let sent = call(
        &sender,
        "send_message",
        json!({"to":recipient_w,"body":"Please inspect the editor tab.","intent":"quiet"}),
    );
    let id = sent["message"]["id"].as_str().unwrap();
    assert!(sent["message"]["surfaced"].is_null());
    assert!(sent["message"]["acknowledged"].is_null());
    assert!(!f.capture(&recipient).contains("Please inspect")); // Saving alone must never pretend to type/wake.
    assert!(
        wire::request(
            &f.state,
            &[
                "agent-operation",
                &sender.id.to_string(),
                &sender.run,
                "inbox",
                &wire::hex(&serde_json::to_vec(&json!({"ack_ids":[id]})).unwrap())
            ]
        )
        .is_err()
    );
    let read = call(&recipient, "inbox", json!({}));
    assert!(!read["messages"][0]["surfaced"].is_null());
    assert!(read["messages"][0]["acknowledged"].is_null());
    let acked = call(&recipient, "inbox", json!({"ack_ids":[id]}));
    assert!(acked["messages"].as_array().unwrap().is_empty());
    // Retired records must never occupy the entire bounded context page.
    for _ in 0..35 {
        let sent = call(
            &sender,
            "send_message",
            json!({"to":recipient_w,"body":"retired"}),
        );
        call(
            &recipient,
            "inbox",
            json!({"ack_ids":[sent["message"]["id"]]}),
        );
    }
    let pending = call(
        &sender,
        "send_message",
        json!({"to":recipient_w,"body":"new pending work"}),
    );
    let human_context: Value = serde_json::from_slice(&f.req(&[
        "coordinate",
        &recipient_w.to_string(),
        "context",
        "7b7d",
    ]))
    .unwrap();
    assert_eq!(human_context["messages"][0]["id"], pending["message"]["id"]);
    assert!(human_context["messages"][0]["surfaced"].is_null());
    call(
        &sender,
        "checkpoint",
        json!({"body":"sender-private-progress"}),
    );
    call(
        &recipient,
        "checkpoint",
        json!({"body":"recipient-progress"}),
    );
    let current = call(&recipient, "context", json!({}));
    assert_eq!(current["checkpoints"].as_array().unwrap().len(), 1);
    assert_eq!(current["checkpoints"][0]["body"], "recipient-progress");
    let decision = call(
        &recipient,
        "request_decision",
        json!({"question":"Inspect this local result?","recommendation":"Read the exact diff first."}),
    );
    let did = decision["decision"]["id"].as_str().unwrap();
    assert!(
        wire::request(
            &f.state,
            &[
                "agent-operation",
                &recipient.id.to_string(),
                &recipient.run,
                "answer_decision",
                &wire::hex(&serde_json::to_vec(&json!({"id":did,"answer":"yes"})).unwrap())
            ]
        )
        .is_err()
    );
    f.req(&[
        "coordinate",
        &recipient_w.to_string(),
        "answer_decision",
        &wire::hex(
            &serde_json::to_vec(
                &json!({"id":did,"answer":"Local result reviewed; no publication permission."}),
            )
            .unwrap(),
        ),
    ]);
    call(
        &recipient,
        "submit_result",
        json!({"body":"Local work ready for review."}),
    );
    assert_eq!(
        f.snapshot().workspace().unwrap().meta.status,
        flere::workspace::Workflow::NeedsMe
    );
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("mcp")
        .env("FLERE_SESSION", recipient.id.to_string())
        .env("FLERE_RUN", &recipient.run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_context","arguments":{}}}),
    ];
    let mut stdin = mcp.stdin.take().unwrap();
    for r in requests {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let out = mcp.wait_with_output().unwrap();
    assert!(out.status.success());
    let responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "inbox")
    );
    let context: Value = serde_json::from_str(
        responses[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(context["workspace"]["id"], recipient_w);
    assert!(
        wire::request(
            &f.state,
            &[
                "agent-operation",
                &recipient.id.to_string(),
                &sender.run,
                "context",
                "7b7d"
            ]
        )
        .is_err()
    );
    let store: Value =
        serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap();
    assert!(!store["coordination"]["messages"][0]["acknowledged"].is_null());
    assert!(
        store["coordination"]["decisions"][0]["answer"]
            .as_str()
            .unwrap()
            .contains("no publication")
    );
}

#[test]
fn actual_ui_tab_picker_drag_messages_and_decision_review() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let first = f.new_workspace("UI integration");
    let wid = f.snapshot().active;
    f.req(&["tab", &wid.to_string()]);
    let second = f.snapshot().session().unwrap().clone();
    let human = |op: &str, args: Value| -> Value {
        serde_json::from_slice(&f.req(&[
            "coordinate",
            &wid.to_string(),
            op,
            &wire::hex(&serde_json::to_vec(&args).unwrap()),
        ]))
        .unwrap()
    };
    let msg = human(
        "send_message",
        json!({"to":wid,"body":format!("Read before acknowledging. {} WRAPPED_TAIL", "continuation ".repeat(16))}),
    );
    let mid = msg["message"]["id"].as_str().unwrap();
    let dec = human(
        "request_decision",
        json!({"question":"Review this local change?","recommendation":"Inspect the evidence first.","evidence":"No publication permission is requested."}),
    );
    let did = dec["decision"]["id"].as_str().unwrap();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    master
        .write_all(b"printf 'PICKER_%s\\n' DRAFT\0T\r")
        .unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while f.snapshot().tab != first.id && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().tab, first.id);
    master.write_all(b"\0b\0\r").unwrap();
    wait_ui_text(&f, &second, &mut master, &mut screen, "PICKER_DRAFT");
    master
        .write_all(b"\x1b[<0;26;12M\x1b[<32;36;12M\x1b[<0;36;12m")
        .unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while f.snapshot().cols != 63 && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().cols, 63);
    master.write_all(b"\0I\r").unwrap();
    drain_pty(&mut master, &mut screen, "WRAPPED_TAIL");
    let displayed = human("context", json!({}));
    let displayed = displayed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == mid)
        .unwrap();
    assert!(!displayed["surfaced"].is_null());
    assert!(displayed["acknowledged"].is_null());
    master.write_all(b"a").unwrap();
    drain_pty(&mut master, &mut screen, "Message acknowledged");
    let current = human("context", json!({}));
    assert!(
        !current["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == mid)
            .unwrap()["acknowledged"]
            .is_null()
    );
    master.write_all(b"\0D\r").unwrap();
    drain_pty(
        &mut master,
        &mut screen,
        "No publication permission is requested.",
    );
    master.write_all(b"aReviewed locally\r").unwrap();
    drain_pty(&mut master, &mut screen, "Decision answer recorded");
    let current = human("decisions", json!({}));
    assert_eq!(
        current["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == did)
            .unwrap()["answer"],
        "Reviewed locally"
    );
    master.write_all(b"\0q").unwrap();
    let end = Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() && Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(child.try_wait().unwrap().unwrap().success());
}
#[test]
fn literal_cli_text_rejects_embedded_enter_or_escape() {
    let f = Fixture::new();
    let t = f.new_workspace("literal controls");
    for text in ["printf BAD\n", "bad\x1btext"] {
        let out = Command::new(env!("CARGO_BIN_EXE_flere"))
            .arg("--state")
            .arg(&f.state)
            .args([
                "send",
                "--session",
                &t.id.to_string(),
                "--run",
                &t.run,
                "--text",
                text,
            ])
            .output()
            .unwrap();
        assert!(!out.status.success());
    }
    assert!(!f.capture(&t).contains("BAD"));
}

#[track_caller]
fn wait_current_ui(
    master: &mut fs::File,
    screen: &mut Terminal,
    condition: impl Fn(&Terminal) -> bool,
) {
    wait_current_ui_for(master, screen, Duration::from_secs(3), condition);
}
#[track_caller]
fn wait_current_ui_for(
    master: &mut fs::File,
    screen: &mut Terminal,
    timeout: Duration,
    condition: impl Fn(&Terminal) -> bool,
) {
    let until = Instant::now() + timeout;
    let mut raw = Vec::new();
    while Instant::now() < until {
        read_ui_bytes(master, screen, &mut raw);
        if complete_frame(&raw) && condition(screen) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("UI condition not reached: {}", screen.capture(100));
}
fn finish_ui(master: &mut fs::File, screen: &mut Terminal, child: &mut os::Process) {
    master.write_all(b"\0q").unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        let mut bytes = [0; 65536];
        if let Ok(n) = master.read(&mut bytes) {
            screen.feed(&bytes[..n]);
        }
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            // Exit can be observed before the final restore bytes have been read.
            // Drain the remaining PTY output before asserting cursor/alternate-screen state.
            loop {
                match master.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => screen.feed(&bytes[..n]),
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.raw_os_error() == Some(libc::EIO) =>
                    {
                        break;
                    }
                    Err(e) => panic!("UI tail read: {e}"),
                }
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("UI did not detach: {}", screen.capture(100));
}
#[test]
fn actual_ui_board_keeps_project_filter_and_scrolls_keyboard_and_mouse_together() {
    let f = Fixture::new();
    let mut keep = Vec::new();
    for i in 0..8 {
        let t = f.new_workspace(&format!("Board {i}"));
        let wid = f.snapshot().active;
        let meta = flere::workspace::CardMeta {
            project: if i == 1 { "Hidden" } else { "Visible" }.into(),
            status: flere::workspace::Workflow::NeedsMe,
            ..Default::default()
        };
        f.req(&[
            "metadata",
            &wid.to_string(),
            &wire::hex(&serde_json::to_vec(&meta).unwrap()),
        ]);
        if i != 1 {
            keep.push((wid, t));
        }
    }
    f.req(&["focus", &keep[0].0.to_string(), "0"]);
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 100, 18).unwrap();
    let mut screen = Terminal::new(100, 18);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0PVisible\rBj").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().active == keep[1].0
    });
    assert_ne!(f.snapshot().workspace().unwrap().meta.project, "Hidden");
    master.write_all(b"jjjjj").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().active == keep[6].0 && s.grid.line(12).contains("Board 7")
    });
    // Three separated cards fit; scrolling keeps the final card fully visible.
    // The first visible card is now the fifth filtered card.
    master.write_all(b"\x1b[<0;30;5M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().active == keep[4].0 && s.grid.line(12).contains("Board 5")
    });
    // Both the inter-card spacer and the bottom gap have no target.
    for mouse_y in [8, 16] {
        master
            .write_all(format!("\x1b[<0;30;{mouse_y}M").as_bytes())
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(f.snapshot().active, keep[4].0);
    }
    // The status row remains part of its card's hit region.
    master.write_all(b"\x1b[<0;30;7M").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().active == keep[2].0
    });
    master.write_all(b"B\0hj").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().active == keep[3].0
            && (3..s.grid.rows - 2)
                .any(|y| s.grid.line(y).contains("Board 4") && s.grid.line(y).contains('▌'))
            && !s.cursor
    });
    // Sidebar boxes consume two more rows than board cards. Locate the complete
    // preceding card from the rendered title; its border and spacer are inert.
    let title = (3..screen.grid.rows - 2)
        .find(|y| screen.grid.line(*y).contains("Board 3"))
        .unwrap();
    for y in [3, 4, title - 1, title + 3, title + 4] {
        master
            .write_all(format!("\x1b[<0;4;{}M", y + 1).as_bytes())
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(f.snapshot().active, keep[3].0);
    }
    master
        .write_all(format!("\x1b[<0;10;{}M", title + 3).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().active == keep[2].0
    });
    for (wid, t) in &keep {
        assert_eq!(
            f.snapshot()
                .workspaces
                .iter()
                .find(|w| w.id == *wid)
                .unwrap()
                .tabs[0]
                .pid,
            t.pid
        );
    }
    master.write_all(b"\r").unwrap();
    finish_ui(&mut master, &mut screen, &mut child);
}
#[test]
fn actual_ui_sidebar_wheel_moves_viewport_preserves_draft_and_tracks_clicks() {
    let f = Fixture::new();
    let mut cards = Vec::new();
    for i in 0..9 {
        let t = f.new_workspace(&format!("Wheel {i}"));
        cards.push((f.snapshot().active, t));
    }
    f.req(&["focus", &cards[0].0.to_string(), "0"]);
    f.send(&cards[0].1, b"echo WHEEL_DRAFT");
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 100, 18).unwrap();
    let mut screen = Terminal::new(100, 18);
    drain_pty(&mut master, &mut screen, "WHEEL_DRAFT");
    let sidebar = |s: &Terminal, needle: &str| {
        (3..16).any(|y| {
            s.grid
                .line(y)
                .chars()
                .take(28)
                .collect::<String>()
                .contains(needle)
        })
    };
    assert!(sidebar(&screen, "Wheel 0"));
    // Wheel over cards works even while the native terminal has keyboard focus.
    master.write_all(b"\x1b[<65;4;8M\x1b[<65;4;8M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !sidebar(s, "Wheel 0") && sidebar(s, "Wheel 2")
    });
    assert_eq!(f.snapshot().active, cards[0].0);
    assert!(f.capture(&cards[0].1).contains("echo WHEEL_DRAFT"));
    // A snapshot from terminal output must not snap a manually scrolled view back.
    pump_ui_bytes(&mut master, &mut screen, 150);
    assert!(!sidebar(&screen, "Wheel 0"));
    // Click a fully visible card using the same shifted row geometry.
    let y = (3..16)
        .find(|y| screen.grid.line(*y).contains("Wheel 2"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;4;{}M", y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().active == cards[2].0
    });
    // Selecting an already-visible card must not realign it at the bottom,
    // including after subsequent supervisor snapshots and repeated clicks.
    pump_ui_bytes(&mut master, &mut screen, 150);
    assert!(
        screen.grid.line(y).contains("Wheel 2"),
        "{}",
        screen.capture(30)
    );
    master
        .write_all(format!("\x1b[<0;4;{}M", y + 1).as_bytes())
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 150);
    assert_eq!(f.snapshot().active, cards[2].0);
    assert!(
        screen.grid.line(y).contains("Wheel 2"),
        "{}",
        screen.capture(30)
    );
    // Clamp at the bottom, and let a keyboard move reveal its selected card again.
    master
        .write_all(b"\x1b[<65;4;8M".repeat(30).as_slice())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| sidebar(s, "Wheel 8"));
    assert_eq!(f.snapshot().active, cards[2].0);
    master.write_all(b"j").unwrap(); // clicking the sidebar entered NAV/Cards.
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().active == cards[3].0 && sidebar(s, "Wheel 3")
    });
    // Resize clamps the manual offset. The narrow Cards overlay also owns its wheel.
    master
        .write_all(b"\x1b[<65;4;8M".repeat(30).as_slice())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| sidebar(s, "Wheel 8"));
    os::resize(master.as_raw_fd(), 60, 24).unwrap();
    screen.resize(60, 24);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(30).contains("Wheel 8")
    });
    master
        .write_all(b"\x1b[<64;4;8M".repeat(30).as_slice())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(30).contains("Wheel 0")
    });
    assert_eq!(f.snapshot().active, cards[3].0);
    // Folding after wheel movement keeps hit regions valid and clamps the view.
    master.write_all(b"\x1b[<0;4;4M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.capture(30).contains("Wheel 0")
    });
    master.write_all(b"\x1b[<0;4;4M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(30).contains("Wheel 0")
    });
    for (wid, t) in &cards {
        let now = f.snapshot();
        let tab = &now.workspaces.iter().find(|w| w.id == *wid).unwrap().tabs[0];
        assert_eq!((tab.pid, &tab.run), (t.pid, &t.run));
    }
    f.req(&["focus", &cards[0].0.to_string(), "0"]);
    assert!(f.capture(&cards[0].1).contains("echo WHEEL_DRAFT"));
    finish_ui(&mut master, &mut screen, &mut child);
}
// Choose a visibly painted half-cell, excluding transparent sprite margins.
fn mascot_cell(screen: &Terminal, x: usize, top: usize, width: usize) -> (usize, usize) {
    use flere::terminal::Color;
    (top..top + 6)
        .flat_map(|y| (x..x + width).map(move |xx| (xx, y)))
        .max_by_key(|(xx, y)| {
            let c = &screen.grid.cells[y * screen.grid.cols + xx];
            let value = |c| {
                if let Color::Rgb(r, g, b) = c {
                    u32::from(r) + u32::from(g) + u32::from(b)
                } else {
                    0
                }
            };
            if c.text == "▀" {
                value(c.style.fg).max(value(c.style.bg))
            } else {
                0
            }
        })
        .unwrap()
}
fn click_mascot(master: &mut fs::File, screen: &Terminal, x: usize, top: usize, width: usize) {
    let (x, y) = mascot_cell(screen, x, top, width);
    master
        .write_all(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes())
        .unwrap();
}
#[test]
fn actual_ui_pet_interactions_select_characters_without_native_input() {
    let f = Fixture::new();
    let t = f.new_workspace("Interactive pets");
    f.send(&t, b"echo PET_SAFE_DRAFT");
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":33,"pet_kind":"duck"}"#,
    )
    .unwrap();
    let before = f.snapshot();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 188, 39).unwrap();
    let mut screen = Terminal::new(188, 39);
    drain_pty(&mut master, &mut screen, "PET_SAFE_DRAFT");
    master.write_all(b"\0O").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET Ready") && s.capture(100).contains("PET / Duck")
    });
    master.write_all(b"p").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET Happy")
    });
    // These would detach, refresh, or start native work outside the focused controls.
    master
        .write_all(b"RS\x1b[200~echo PET_INPUT_MUST_NOT_REACH_SHELL\r\x1b[201~")
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(!f.capture(&t).contains("PET_INPUT_MUST_NOT_REACH_SHELL"));
    // Selection followed immediately by a play key must work in one input packet.
    master.write_all(b"2t").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET / Robot") && s.capture(100).contains("PET Chasing toy")
    });
    master.write_all(b"3k").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET / Cat") && s.capture(100).contains("PET Showing off")
    });
    master.write_all(b"n").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET Napping")
    });
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(saved["pet_kind"], "cat");
    assert_eq!(saved["pet"], true);
    // Escape returns to NAV; the next Escape returns to the untouched native draft.
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.capture(100).contains("Esc back") && s.grid.line(38).trim_start().starts_with("NAV")
    });
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.grid.line(38).trim_start().starts_with("NAV")
    });
    master.write_all(b"_BACK").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET_SAFE_DRAFT_BACK")
    });
    // First click explicitly expands the compact dock without submitting native input.
    master.write_all(b"\x1b[<0;160;32M\x1b[<0;160;32m").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("Esc back")
    });
    // The expanded mascot click pets it, and its button starts a toy chase.
    click_mascot(&mut master, &screen, 157, 30, 29);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET Happy")
    });
    master.write_all(b"\x1b[<0;165;37M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET Chasing toy")
    });
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.grid.line(38).trim_start().starts_with("NAV") && !s.grid.line(38).contains("Esc back")
    });
    master.write_all(b"_MOUSE_BACK").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("PET_SAFE_DRAFT_BACK_MOUSE_BACK")
    });
    master.write_all(b"\0O").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("Esc back")
    });
    os::resize(master.as_raw_fd(), 20, 8).unwrap();
    screen.resize(20, 8);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(20).contains("FLERE") && !s.capture(20).contains("PET /")
    });
    let after = f.snapshot();
    assert_eq!(after.active, before.active);
    assert_eq!(
        serde_json::to_value(&after.workspace().unwrap().meta).unwrap(),
        serde_json::to_value(&before.workspace().unwrap().meta).unwrap()
    );
    assert_eq!(
        after.workspace().unwrap().tabs.len(),
        before.workspace().unwrap().tabs.len()
    );
    assert_eq!(
        (after.session().unwrap().pid, &after.session().unwrap().run),
        (t.pid, &t.run)
    );
    assert!(!f.capture(&t).contains("PET_INPUT_MUST_NOT_REACH_SHELL"));
    finish_ui(&mut master, &mut screen, &mut child);
}
#[test]
fn actual_ui_pet_pointer_capture_preserves_chat_selection_and_native_draft() {
    let f = Fixture::new();
    let t = f.new_workspace("Physics pets");
    f.send(&t, b"echo PHYSICS_SAFE_DRAFT");
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":33,"pet":true,"pet_kind":"robot","reduced_motion":true}"#,
    )
    .unwrap();
    let before = f.snapshot();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 188, 39).unwrap();
    let mut screen = Terminal::new(188, 39);
    drain_pty(&mut master, &mut screen, "PHYSICS_SAFE_DRAFT");
    // Directory loading now completes independently of the native first frame.
    // Wait for its footer before comparing the dock and neighbouring file row.
    drain_pty(&mut master, &mut screen, "Enter open");
    // Chat press/selection must leave the entire dock rendered, including pixels.
    let pet_before = screen.grid.cells[30 * 188..36 * 188].to_vec();
    master.write_all(b"\x1b[<0;55;10M\x1b[<32;72;11M").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 200);
    assert!(screen.capture(100).contains("PET / Robot"));
    assert_eq!(&screen.grid.cells[30 * 188..36 * 188], &pet_before);
    master.write_all(b"\x1b[<0;72;11m").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    // Transparent top-left of the scene neither pets nor takes keyboard focus.
    master.write_all(b"\x1b[<0;158;31M\x1b[<0;158;31m").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(!screen.grid.line(38).contains("Esc back"));
    master.write_all(b"\0O").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Ready")
    });
    click_mascot(&mut master, &screen, 157, 30, 29);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Happy")
    });
    // A stationary long press lifts; drag/release outside the dock stays captured.
    master.write_all(b"n").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Napping")
    });
    let (x, y) = mascot_cell(&screen, 157, 30, 29);
    master
        .write_all(format!("\x1b[<0;{};{}M", x + 1, y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Dangling")
    });
    master
        .write_all(b"\x1b[<32;20;3M\x1b[200~NO_NATIVE_PASTE\x1b[201~")
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(screen.grid.line(38).contains("PET Dangling"));
    master.write_all(b"\x1b[<0;20;3m").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Ready")
    });
    // Spawn a stationary ball (reduced motion). Its cell comes from the scene's
    // normalized throw origin, opposite the character's current location.
    master.write_all(b"t").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Chasing toy")
    });
    master.write_all(b"\x1b[<0;179;33M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Ball held")
    });
    master.write_all(b"\x1b[<32;170;32M").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 180);
    master.write_all(b"\x1b[<0;170;32m").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Chasing toy")
    });
    // Resize during a second grab cancels the capture and leaves no native input.
    master.write_all(b"\x1b[<0;170;32M").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(38).contains("PET Ball held")
    });
    os::resize(master.as_raw_fd(), 60, 24).unwrap();
    screen.resize(60, 24);
    pump_ui_bytes(&mut master, &mut screen, 200);
    master.write_all(b"\x1b[<32;10;3M\x1b[<0;10;3m").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    let after = f.snapshot();
    assert_eq!(after.active, before.active);
    assert_eq!(
        serde_json::to_value(&after.workspace().unwrap().meta).unwrap(),
        serde_json::to_value(&before.workspace().unwrap().meta).unwrap()
    );
    assert_eq!(
        (after.session().unwrap().pid, &after.session().unwrap().run),
        (t.pid, &t.run)
    );
    assert!(f.capture(&t).contains("PHYSICS_SAFE_DRAFT"));
    assert!(!f.capture(&t).contains("NO_NATIVE_PASTE"));
    finish_ui(&mut master, &mut screen, &mut child);
}
#[test]
fn actual_ui_cat_window_reserves_inspector_and_preserves_native_draft() {
    let f = Fixture::new();
    // Preserve the existing pet's activity-pose coverage independently of defaults.
    fs::write(f.state.join("ui.json"), br#"{"pet_kind":"duck"}"#).unwrap();
    let t = f.new_workspace("Cat workspace");
    f.send(&t, b"echo CAT_NATIVE_DRAFT");
    let before = f.snapshot();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 120, 28).unwrap();
    let mut screen = Terminal::new(120, 28);
    drain_pty(&mut master, &mut screen, "CAT_NATIVE_DRAFT");
    assert!(!screen.capture(100).contains(" PET / "));
    master.write_all(b"\0o").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(20).contains(" PET / ")
    });
    assert!(!screen.capture(100).contains("(="));
    assert!(screen.grid.line(22).contains('▀'));
    master.write_all(b"l").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(25).contains("Looking around")
    });
    let first = screen.grid.cells.clone();
    wait_current_ui(&mut master, &mut screen, |s| s.grid.cells != first);
    master.write_all(b"M").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 150);
    let still = screen.grid.cells[21 * 120..25 * 120].to_vec();
    pump_ui_bytes(&mut master, &mut screen, 300);
    assert_eq!(&screen.grid.cells[21 * 120..25 * 120], &still);
    // Its reserved rows consume pointer events; no file is opened or native scroll started.
    master.write_all(b"O").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(27).contains("PET Ready")
    });
    click_mascot(&mut master, &screen, 82, 19, 36);
    master.write_all(b"\x1b[<65;112;23M").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(screen.capture(100).contains("PET Happy"));
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert_eq!(f.snapshot().active, before.active);
    assert_eq!(f.snapshot().session().unwrap().id, t.id);
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.status = flere::workspace::Workflow::NeedsMe;
    f.req(&[
        "metadata",
        &before.active.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(25).contains("Alert")
    });
    master.write_all(b" ").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("FLERE ACTIONS") && !s.capture(100).contains(" PET / ")
    });
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains(" PET / ")
    });
    os::resize(master.as_raw_fd(), 40, 24).unwrap();
    screen.resize(40, 24);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.line(16).contains(" PET / ")
    });
    os::resize(master.as_raw_fd(), 20, 8).unwrap();
    screen.resize(20, 8);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(20).contains("FLERE") && !s.capture(20).contains(" PET / ")
    });
    assert!(f.capture(&t).contains("echo CAT_NATIVE_DRAFT"));
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
    master.write_all(b"o").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(saved["pet"], false);
    finish_ui(&mut master, &mut screen, &mut child);
}
#[test]
fn actual_ui_short_archive_picker_restores_without_starting_any_terminal() {
    let f = Fixture::new();
    fs::write(f.root.join("retained.txt"), "keep this draft").unwrap();
    let mut ids = Vec::new();
    for i in 0..7 {
        let t = f.new_workspace(&format!("Archive {i}"));
        let wid = f.snapshot().active;
        ids.push(wid);
        f.send(&t, b"exit\r");
        let until = Instant::now() + Duration::from_secs(2);
        while !f.snapshot().workspace().unwrap().tabs.is_empty() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
        if i < 6 {
            let meta = flere::workspace::CardMeta {
                archived: true,
                ..Default::default()
            };
            f.req(&[
                "metadata",
                &wid.to_string(),
                &wire::hex(&serde_json::to_vec(&meta).unwrap()),
            ]);
        }
    }
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 60, 12).unwrap();
    let mut screen = Terminal::new(60, 12);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0Aa").unwrap();
    drain_pty(&mut master, &mut screen, "Archived workspaces");
    master
        .write_all(b"\x1b[B\x1b[B\x1b[B\x1b[B\x1b[B\x1b[B")
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(30).contains("› Archive 6")
    });
    master.write_all(b"\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        !f.snapshot()
            .workspaces
            .iter()
            .find(|w| w.id == ids[6])
            .unwrap()
            .meta
            .archived
    });
    assert_eq!(f.snapshot().active, ids[6]);
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
    assert_eq!(
        fs::read_to_string(f.root.join("retained.txt")).unwrap(),
        "keep this draft"
    );
    // At eight rows the picker still renders its selected result, not a blind Enter target.
    os::resize(master.as_raw_fd(), 40, 8).unwrap();
    screen.resize(40, 8);
    // Resizing this test's emulator does not acknowledge the frontend resize.
    // Wait for its viewport request before opening: a queued twelve-row frame
    // could otherwise satisfy the assertion while the actual tiny form is empty.
    let viewport = flere::ui::Layout::new(40, 8);
    wait_current_ui(&mut master, &mut screen, |_| {
        let snapshot = f.snapshot();
        (snapshot.cols, snapshot.rows) == (viewport.cols, viewport.rows)
    });
    master.write_all(b"\0a").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.grid.rows == 8 && s.capture(20).contains("› Archive 0")
    });
    master.write_all(b"\x1b[B").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(20).contains("› Archive 1") && !s.capture(20).contains("Archive 0")
    });
    master.write_all(b"\x1b").unwrap();
    // Wait for the UI to process lone Escape; a 50 ms sleep races its 40 ms
    // decoder deadline under full-suite load and can instead create Alt+Enter.
    wait_current_ui(&mut master, &mut screen, |s| {
        !(0..s.grid.rows).any(|y| s.grid.line(y).contains("Archived workspaces"))
    });
    master.write_all(b"\r").unwrap();
    finish_ui(&mut master, &mut screen, &mut child);
}
#[test]
fn actual_ui_start_agent_has_no_session_id_field_and_keeps_existing_shell() {
    let f = Fixture::new();
    f.new_workspace("Native identity");
    let wid = f.snapshot().active;
    let meta = flere::workspace::CardMeta {
        legacy_lead: true,
        pinned: true,
        ..Default::default()
    };
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    let script = f.root.join("native-standin.sh");
    fs::write(&script, "#!/bin/sh\nprintf 'CLAUDE_READY\\n'\nread input\n").unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&serde_json::json!([
            {"name":"claude","command":["/bin/sh",script]},
            {"name":"codex","command":["/usr/bin/false"]}
        ]))
        .unwrap(),
    )
    .unwrap();
    let uuid = "00000000-1234-5678-9012-123456789abc";
    f.req(&["native", &wid.to_string(), "claude", uuid]);
    let first = f.snapshot().session().unwrap().clone();
    f.wait_text(&first, "CLAUDE_READY");
    f.send(&first, b"\r");
    f.wait_text(&first, "Returning to");
    let until = Instant::now() + Duration::from_secs(2);
    while f.snapshot().session().unwrap().kind != "shell" && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    let w = f.snapshot().workspace().unwrap().clone();
    assert_eq!(w.meta.conversations.len(), 1);
    assert_eq!(w.meta.conversations[0].harness, "claude");
    assert_eq!(w.meta.conversations[0].uuid, uuid);
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 100, 24).unwrap();
    let mut screen = Terminal::new(100, 24);
    drain_pty(&mut master, &mut screen, "Native identi…");
    assert_eq!(f.snapshot().workspace().unwrap().name, "Native identity");
    assert_eq!(f.snapshot().session().unwrap().kind, "shell");
    assert_eq!(f.snapshot().session().unwrap().pid, first.pid);
    master.write_all(b"\0S").unwrap();
    drain_pty(&mut master, &mut screen, "› Claude Code");
    assert!(!screen.capture(100).contains(uuid));
    assert!(!screen.capture(100).contains("Conversation UUID"));
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
    master.write_all(b"j").unwrap();
    drain_pty(&mut master, &mut screen, "› GitHub Copilot");
    // This is a selection, not an editable harness name. Paste never starts a chat.
    master
        .write_all(b"arbitrary\x1b[200~codex\r\x1b[201~")
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(screen.capture(100).contains("› GitHub Copilot"));
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
    master.write_all(b"\x1b[A").unwrap();
    drain_pty(&mut master, &mut screen, "› Claude Code");
    let row = (0..screen.grid.rows)
        .find(|y| screen.grid.line(*y).contains("› Claude Code"))
        .unwrap()
        + 1;
    master
        .write_all(format!("\x1b[<0;24;{row}m").as_bytes())
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
    master
        .write_all(format!("\x1b[<0;24;{row}M\x1b[<0;24;{row}m").as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().session().unwrap().kind == "agent"
    });
    let resumed = f.snapshot().session().unwrap().clone();
    f.wait_text(&resumed, "CLAUDE_READY");
    let spec: serde_json::Value = serde_json::from_slice(
        &fs::read(f.state.join("runs").join(format!("{}.json", resumed.run))).unwrap(),
    )
    .unwrap();
    assert_eq!(spec["harness"], "claude");
    assert_eq!(spec["conversation"], "");
    assert_eq!(spec["argv"].as_array().unwrap().len(), 2);
    assert_eq!(f.snapshot().workspace().unwrap().tabs[1].pid, first.pid);
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn legacy_lead_metadata_is_inert_and_can_be_unpinned_without_replacing_work() {
    let f = Fixture::new();
    let shell = f.new_workspace("Lead");
    let wid = f.snapshot().active;
    let epoch = f.snapshot().epoch;
    f.send(&shell, b"printf 'ROLE_%s\\n' DRAFT");
    let mut meta = flere::workspace::CardMeta {
        legacy_lead: true,
        pinned: true,
        notes: "User's existing notes".into(),
        ..Default::default()
    };
    for pin in [true, false, true, false] {
        meta.pinned = pin;
        f.req(&[
            "metadata",
            &wid.to_string(),
            &wire::hex(&serde_json::to_vec(&meta).unwrap()),
        ]);
        let snapshot = f.snapshot();
        let w = snapshot.workspace().unwrap();
        assert_eq!(w.meta.pinned, pin);
        assert_eq!(w.name, "Lead");
        assert!(w.meta.legacy_lead);
        assert_eq!(w.meta.notes, "User's existing notes");
        assert_eq!(w.cwd, f.root.to_str().unwrap());
        assert_eq!(snapshot.epoch, epoch);
        assert_eq!(snapshot.session().unwrap().pid, shell.pid);
        assert_eq!(snapshot.session().unwrap().run, shell.run);
    }
    let before = fs::read(f.state.join("workspaces.v2.json")).unwrap();
    assert!(
        wire::request(&f.state, &["lead"])
            .unwrap_err()
            .to_string()
            .contains("pin it")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("lead")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("roles were removed"));
    assert_eq!(
        fs::read(f.state.join("workspaces.v2.json")).unwrap(),
        before
    );
    // Reopening keeps this ordinary workspace, and pinning stays a reversible UI action.
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "Lead");
    master.write_all(b"\0p").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().meta.pinned
    });
    master.write_all(b"p").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        !f.snapshot().workspace().unwrap().meta.pinned
    });
    master.write_all(b"\r").unwrap();
    finish_ui(&mut master, &mut screen, &mut child);
    assert_eq!(f.snapshot().workspaces.len(), 1);
    f.send(&shell, b"\r");
    f.wait_text(&shell, "ROLE_DRAFT");
}

#[test]
fn actual_first_open_creates_one_ordinary_workspace_and_reopen_reuses_it() {
    let f = Fixture::new();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "Press any key to start");
    assert!(f.snapshot().workspaces.is_empty());
    master.write_all(b"z").unwrap();
    drain_pty(&mut master, &mut screen, "Workspace 1");
    let before = f.snapshot();
    assert_eq!(before.workspaces.len(), 1);
    let workspace = before.workspace().unwrap();
    assert_eq!(workspace.cwd, f.root.to_str().unwrap());
    assert!(!workspace.meta.pinned && !workspace.meta.legacy_lead);
    assert_eq!(workspace.tabs.len(), 1);
    let shell = before.session().unwrap().clone();
    assert_eq!(shell.kind, "shell");
    finish_ui(&mut master, &mut screen, &mut child);
    // PTY setup installs a pre_exec callback; each attachment needs a fresh Command.
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "Workspace 1");
    let after = f.snapshot();
    assert_eq!(after.workspaces.len(), 1);
    let tabs: Vec<_> = after.workspaces.iter().flat_map(|w| &w.tabs).collect();
    assert_eq!(tabs.len(), 1);
    assert_eq!(
        (tabs[0].id, tabs[0].pid, &tabs[0].run),
        (shell.id, shell.pid, &shell.run)
    );
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn nested_ui_refuses_before_state_creation_but_diagnostics_still_work() {
    let f = Fixture::new();
    let t = f.new_workspace("Outer terminal");
    let state = f.root.join("must-not-be-created");
    for action in [None, Some("open"), Some("attach"), Some("intro")] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_flere"));
        command.arg("--state").arg(&state).env("FLERE", "1");
        if let Some(action) = action {
            command.arg(action);
        }
        let out = command.output().unwrap();
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("already inside Flere"));
        assert!(!state.exists());
    }
    let out = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("list")
        .env("FLERE", "1")
        .output()
        .unwrap();
    assert!(out.status.success());
    let listed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(listed["workspaces"][0]["tabs"][0]["run"], t.run);
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
}

#[test]
fn actual_child_shell_inherits_nested_ui_guard_and_keeps_its_terminal() {
    let f = Fixture::new();
    let t = f.new_workspace("Guarded shell");
    let before = f.snapshot();
    let command = format!(
        "{} --state {} attach\r",
        shell_words::quote(env!("CARGO_BIN_EXE_flere")),
        shell_words::quote(f.state.to_str().unwrap())
    );
    f.send(&t, command.as_bytes());
    f.wait_text(&t, "already inside Flere");
    f.send(&t, b"printf 'OUTER_%s\\n' ALIVE\r");
    f.wait_text(&t, "OUTER_ALIVE");
    assert_eq!(f.snapshot().epoch, before.epoch);
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().workspaces.len(), 1);
}

#[test]
fn native_color_probe_styles_ui_and_refresh_preserve_terminal_contract() {
    use flere::terminal::{Color, DEFAULT_BG, DEFAULT_FG};
    for (cols, rows) in [(60, 18), (180, 42)] {
        let f = Fixture::new();
        let t = f.new_workspace("terminal rendering");
        fs::write(
            f.root.join("color_probe.py"),
            include_str!("fixtures/color_probe.py"),
        )
        .unwrap();
        let mut cmd = outer_ui_command();
        cmd.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut cmd, cols, rows).unwrap();
        let mut screen = Terminal::new(cols as usize, rows as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        f.send(&t, b"python3 color_probe.py\r");
        f.wait_text(&t, "COLOR_QUERY_OK");
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("COLOR_QUERY_OK")
        });
        let before = f.snapshot();
        let style_of = |text: &str| {
            let y = (0..screen.grid.rows)
                .find(|y| screen.grid.line(*y).contains(text))
                .unwrap();
            let row = &screen.grid.cells[y * screen.grid.cols..(y + 1) * screen.grid.cols];
            // Match the full sample: a sidebar project initial can share its first letter.
            let x = row
                .windows(text.len())
                .position(|cells| cells.iter().map(|c| c.text.as_str()).collect::<String>() == text)
                .unwrap();
            row[x].style
        };
        assert_eq!(style_of("DEFAULT_TEXT").fg, DEFAULT_FG);
        assert_eq!(style_of("DEFAULT_TEXT").bg, DEFAULT_BG);
        assert!(style_of("DIM_TEXT").dim);
        assert!(style_of("ITALIC_TEXT").italic);
        let styled = style_of("STYLED_TEXT");
        assert!(styled.bold && styled.underline && styled.strikethrough);
        assert_eq!(style_of("SHADED_PROMPT").bg, Color::Rgb(41, 41, 41));
        assert_eq!(style_of("RGB_TEXT").fg, Color::Rgb(100, 150, 200));
        // Blanks from native erase-line retain the prompt background up to its edge.
        assert!(
            before.cells[4 * before.cols..5 * before.cols]
                .iter()
                .all(|c| c.style.bg == Color::Rgb(41, 41, 41))
        );
        // A native startup query must not leak into the visible UI as typed text.
        assert!(!screen.capture(100).contains("rgb:"));
        f.req(&[
            "refresh",
            &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
        ]);
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if String::from_utf8(f.req(&["refresh-status"]))
                .unwrap()
                .contains("all terminal sessions preserved")
            {
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(20));
        }
        let after = f.snapshot();
        assert_eq!(after.epoch, before.epoch);
        assert_eq!(after.session().unwrap().pid, t.pid);
        assert_eq!(after.session().unwrap().run, t.run);
        assert_eq!(after.cells, before.cells);
        finish_ui(&mut master, &mut screen, &mut child);
        f.send(&t, b"x");
        // The next command's CR requires the restored shell input mode. Do not
        // race it against the probe's raw-mode exit, even though the PID survives.
        f.wait_text(&t, "PROBE_RESTORED");
        f.send(&t, b"printf 'SAME_%s\\n' SHELL\r");
        f.wait_text(&t, "SAME_SHELL");
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    }
}

#[test]
fn actual_ui_text_selection_copies_on_release_without_native_input() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let f = Fixture::new();
    let t = f.new_workspace("Copy text");
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    f.send(&t, b"printf '\\033[0m\\033[2J\\033[Hcopy me\\nsecond line\\n\\033]52;c;QkFE\\007'; printf 'READY_%s\\n' COPY\r");
    let mut raw = Vec::new();
    let pump = |master: &mut fs::File, screen: &mut Terminal, raw: &mut Vec<u8>, ms: u64| {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            let mut b = [0; 65536];
            if let Ok(n) = master.read(&mut b) {
                screen.feed(&b[..n]);
                raw.extend_from_slice(&b[..n]);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    let ready_deadline = Instant::now() + Duration::from_secs(3);
    while !screen.capture(100).contains("READY_COPY") && Instant::now() < ready_deadline {
        pump(&mut master, &mut screen, &mut raw, 20);
    }
    assert!(
        screen.capture(100).contains("READY_COPY"),
        "selection scene not ready: UI={} native={}",
        screen.capture(100),
        f.capture(&t)
    );
    assert!(!raw.windows(5).any(|w| w == b"\x1b]52;")); // child's clipboard command never escapes
    let l = flere::ui::Layout::new(140, 38);
    let mouse = |button: usize, x: usize, y: usize, release: bool| {
        format!(
            "\x1b[<{button};{};{}{}",
            x + 1,
            y + 1,
            if release { 'm' } else { 'M' }
        )
    };
    let before = fs::read_to_string(f.state.join("actions.log")).unwrap();
    master
        .write_all(mouse(0, l.terminal_x, l.terminal_y, false).as_bytes())
        .unwrap();
    master
        .write_all(mouse(32, l.terminal_x + 6, l.terminal_y, false).as_bytes())
        .unwrap();
    pump(&mut master, &mut screen, &mut raw, 100);
    assert!(!raw.windows(5).any(|w| w == b"\x1b]52;")); // no copy before release
    assert_eq!(
        screen.grid.cells[l.terminal_y * 140 + l.terminal_x]
            .style
            .bg,
        flere::terminal::Color::Rgb(42, 92, 82)
    );
    master
        .write_all(mouse(0, l.terminal_x + 6, l.terminal_y, true).as_bytes())
        .unwrap();
    pump(&mut master, &mut screen, &mut raw, 100);
    let prefix = b"\x1b]52;c;";
    let i = raw.windows(prefix.len()).position(|w| w == prefix).unwrap() + prefix.len();
    let end = raw[i..].iter().position(|b| *b == 7).unwrap() + i;
    assert_eq!(STANDARD.decode(&raw[i..end]).unwrap(), b"copy me");
    assert_eq!(
        raw.windows(prefix.len()).filter(|w| *w == prefix).count(),
        1
    );
    assert_eq!(
        before,
        fs::read_to_string(f.state.join("actions.log")).unwrap()
    );
    // Escape only clears selection, and an ordinary click does not copy anything.
    raw.clear();
    master.write_all(b"\x1b").unwrap();
    pump(&mut master, &mut screen, &mut raw, 100);
    master
        .write_all(mouse(0, l.terminal_x, l.terminal_y, false).as_bytes())
        .unwrap();
    master
        .write_all(mouse(0, l.terminal_x, l.terminal_y, true).as_bytes())
        .unwrap();
    pump(&mut master, &mut screen, &mut raw, 100);
    assert!(!raw.windows(prefix.len()).any(|w| w == prefix));
    assert_eq!(
        before,
        fs::read_to_string(f.state.join("actions.log")).unwrap()
    );
    // A resize cancels a pending drag instead of copying with stale geometry.
    master
        .write_all(mouse(0, l.terminal_x, l.terminal_y, false).as_bytes())
        .unwrap();
    master
        .write_all(mouse(32, l.terminal_x + 6, l.terminal_y, false).as_bytes())
        .unwrap();
    pump(&mut master, &mut screen, &mut raw, 60);
    os::resize(master.as_raw_fd(), 100, 30).unwrap();
    screen.resize(100, 30);
    pump(&mut master, &mut screen, &mut raw, 300);
    master
        .write_all(mouse(0, l.terminal_x + 6, l.terminal_y, true).as_bytes())
        .unwrap();
    pump(&mut master, &mut screen, &mut raw, 100);
    assert!(!raw.windows(prefix.len()).any(|w| w == prefix));
    // Existing native input still works afterwards in the exact original shell.
    master.write_all(b"printf 'COPY_%s\\n' SAFE\r").unwrap();
    wait_ui_text(&f, &t, &mut master, &mut screen, "COPY_SAFE");
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
    finish_ui(&mut master, &mut screen, &mut child);
}

fn pump_ui_bytes(master: &mut fs::File, screen: &mut Terminal, ms: u64) -> Vec<u8> {
    let mut raw = Vec::new();
    let until = Instant::now() + Duration::from_millis(ms);
    let deadline = until + Duration::from_secs(3);
    while Instant::now() < deadline {
        read_ui_bytes(master, screen, &mut raw);
        if Instant::now() >= until && complete_frame(&raw) {
            return raw;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("PTY frame did not finish");
}

fn wheel_at(l: flere::ui::Layout, code: u8) -> String {
    format!("\x1b[<{code};{};{}M", l.terminal_x + 2, l.terminal_y + 2)
}
#[test]
fn actual_ui_nav_keyboard_scroll_is_local_and_returns_to_native_input() {
    for (width, height) in [(60, 24), (236, 54)] {
        let f = Fixture::new();
        let other = f.new_workspace("other keyboard card");
        let other_wid = f.snapshot().active;
        f.send(&other, b"printf 'OTHER_%s\n' KEYBOARD\r");
        f.wait_text(&other, "OTHER_KEYBOARD");
        let t = f.new_workspace("keyboard scroll");
        let wid = f.snapshot().active;
        fs::write(
            f.root.join("scroll_probe.py"),
            include_str!("fixtures/scroll_probe.py"),
        )
        .unwrap();
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        f.send(&t, b"python3 scroll_probe.py 300\r");
        wait_ui_text(&f, &t, &mut master, &mut screen, "DRAFT_UNSUBMITTED");
        let l = flere::ui::Layout::new(width as usize, height as usize);
        let top = |s: &Terminal| {
            let row: String = s.grid.cells[l.terminal_y * width as usize + l.terminal_x
                ..l.terminal_y * width as usize + l.terminal_x + l.cols]
                .iter()
                .map(|c| c.text.as_str())
                .collect();
            row.find("CHAT-")
                .and_then(|n| row[n + 5..].get(..3))
                .and_then(|n| n.parse::<i64>().ok())
        };
        wait_current_ui(&mut master, &mut screen, |s| {
            top(s).is_some() && s.capture(100).contains("DRAFT_UNSUBMITTED")
        });
        let live_top = top(&screen).expect("live transcript top row");
        let full = l.rows.max(1) as i64;
        let page = l.rows.saturating_sub(1).max(1) as i64;
        let audit = fs::read_to_string(f.state.join("actions.log")).unwrap();
        master.write_all(b"\0\x15").unwrap(); // NAV, Ctrl+U.
        let mut expected = live_top - full;
        wait_current_ui(&mut master, &mut screen, |s| {
            top(s) == Some(expected) && s.capture(100).contains("NAV SCROLL")
        });
        assert!(screen.capture(100).contains("u/d smooth"));
        for (key, delta) in [
            (b"\x19".as_slice(), -1),
            (b"\x05", 1),
            (b"\x15", -full),
            (b"\x04", full),
            (b"\x1b[5~", -page),
            (b"\x1b[6~", page),
            (b"\x1b[1;5A", -(l.rows as i64)),
            (b"\x1b[1;5B", l.rows as i64),
            (b"u", -3),
            (b"d", 3),
        ] {
            master.write_all(key).unwrap();
            expected += delta;
            wait_current_ui(&mut master, &mut screen, |s| top(s) == Some(expected));
        }
        // Inspect every synchronized frame, even if the PTY coalesces multiple paints.
        // Repeated page keys keep the exact total distance; reduced motion removes intermediates.
        for reduced in [false, true] {
            if reduced {
                master.write_all(b"M").unwrap();
            }
            let before = expected;
            let keys: &[u8] = if reduced { b"\x04\x04" } else { b"\x15\x15" };
            expected += if reduced {
                2 * l.rows as i64
            } else {
                -2 * l.rows as i64
            };
            master.write_all(keys).unwrap();
            let mut positions = vec![before];
            let mut frames = Vec::new();
            let marker = b"\x1b[?2026l";
            let deadline = Instant::now() + Duration::from_secs(3);
            while positions.last() != Some(&expected) {
                assert!(
                    Instant::now() < deadline,
                    "animation did not finish: {positions:?}"
                );
                let mut bytes = [0; 65536];
                if let Ok(n) = master.read(&mut bytes) {
                    frames.extend_from_slice(&bytes[..n]);
                }
                while let Some(end) = frames.windows(marker.len()).position(|w| w == marker) {
                    let bytes: Vec<_> = frames.drain(..end + marker.len()).collect();
                    screen.feed(&bytes);
                    if let Some(position) = top(&screen)
                        && positions.last() != Some(&position)
                    {
                        positions.push(position);
                    }
                }
                std::thread::sleep(Duration::from_millis(3));
            }
            if reduced {
                assert_eq!(positions, vec![before, expected]);
                master.write_all(b"M").unwrap();
            } else {
                assert!(
                    positions.len() >= 4,
                    "no intermediate scroll frames: {positions:?}"
                );
                assert!(positions.windows(2).all(|w| w[1] < w[0]));
            }
        }
        // The displayed old page stays fixed while the probe emits more output.
        fs::write(f.root.join("more"), b"go").unwrap();
        f.wait_text(&t, "CHAT-094");
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(top(&screen), Some(expected));
        master.write_all(b"\x1b[H").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("CHAT-000")
        });
        let oldest = screen.grid.cells.clone();
        master.write_all(b"\x19").unwrap(); // Clamp at oldest.
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(screen.grid.cells, oldest);
        master.write_all(b"\x1b[F").unwrap(); // End stays in NAV, at live output.
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("NAV Terminal") && s.capture(100).contains("DRAFT_UNSUBMITTED")
        });
        master.write_all(b"\x04\x05\x1b[6~").unwrap(); // Down at live is inert.
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert!(screen.capture(100).contains("NAV Terminal"));
        assert_eq!(
            fs::read_to_string(f.state.join("actions.log")).unwrap(),
            audit
        );
        assert!(fs::read(f.root.join("received")).unwrap().is_empty());
        // All three NAV exits discard scrollback and consume the exit key.
        // The same control keys are ordinary native input once NAV is off.
        let mut received = Vec::new();
        for exit in [b"\r".as_slice(), b"\x1b", b"\0"] {
            master.write_all(b"\x15").unwrap();
            wait_current_ui(&mut master, &mut screen, |s| {
                s.capture(100).contains("NAV SCROLL")
            });
            master.write_all(exit).unwrap();
            wait_current_ui(&mut master, &mut screen, |s| {
                !s.capture(100).contains("NAV SCROLL")
                    && !s.capture(100).contains("NAV Terminal")
                    && s.capture(100).contains("DRAFT_UNSUBMITTED")
            });
            assert_eq!(fs::read(f.root.join("received")).unwrap(), received);
            master
                .write_all(b"\x19\x05\x15\x04ud\x1b[1;5A\x1b[1;5B")
                .unwrap();
            received.extend_from_slice(b"\x19\x05\x15\x04ud\x1b[1;5A\x1b[1;5B");
            let end = Instant::now() + Duration::from_secs(3);
            while fs::read(f.root.join("received")).unwrap() != received {
                assert!(Instant::now() < end, "native control bytes missing");
                pump_ui_bytes(&mut master, &mut screen, 20);
            }
            master.write_all(b"\0").unwrap();
            wait_current_ui(&mut master, &mut screen, |s| {
                s.capture(100).contains("NAV Terminal")
            });
        }
        assert_eq!(f.snapshot().session().unwrap().id, t.id);
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
        // Existing k/j tab-focus navigation and h/l pane navigation remain usable.
        master.write_all(b"k").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("NAV Tabs")
        });
        master.write_all(b"\x15\x19\x1b[5~j").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("NAV Terminal")
        });
        assert!(!screen.capture(100).contains("NAV SCROLL"));
        master.write_all(b"hk").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("OTHER_KEYBOARD")
        });
        assert_eq!(f.snapshot().active, other_wid);
        assert_eq!(f.snapshot().tab, other.id);
        master.write_all(b"l").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("NAV Terminal")
        });
        f.req(&["focus", &wid.to_string(), &t.id.to_string()]);
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("DRAFT_UNSUBMITTED")
        });
        // Even an app that opts into native wheel scrolling receives no NAV keys.
        fs::write(f.root.join("alt"), b"go").unwrap();
        wait_ui_text(&f, &t, &mut master, &mut screen, "ALT_SCROLL_READY");
        master.write_all(b"\x15\x19\x1b[5~\x1b[H\x1b[F").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(fs::read(f.root.join("received")).unwrap(), received);
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
        // Finish from normal input mode; detach preserves the exact probe process.
        master.write_all(b"\r").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 50);
        finish_ui(&mut master, &mut screen, &mut child);
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    }
}

#[test]
fn actual_ui_scrollback_keeps_codex_style_history_selection_and_draft() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    for (width, height) in [(60, 18), (180, 42)] {
        let f = Fixture::new();
        let other = f.new_workspace("other chat");
        let other_wid = f.snapshot().active;
        f.send(&other, b"printf 'OTHER_%s\\n' CHAT\r");
        f.wait_text(&other, "OTHER_CHAT");
        let t = f.new_workspace("scroll chat");
        let wid = f.snapshot().active;
        fs::write(
            f.root.join("scroll_probe.py"),
            include_str!("fixtures/scroll_probe.py"),
        )
        .unwrap();
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        f.send(&t, b"python3 scroll_probe.py\r");
        wait_ui_text(&f, &t, &mut master, &mut screen, "DRAFT_UNSUBMITTED");
        let l = flere::ui::Layout::new(width as usize, height as usize);
        let before = f.snapshot();
        let audit = fs::read_to_string(f.state.join("actions.log")).unwrap();
        master.write_all(wheel_at(l, 64).as_bytes()).unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("SCROLLBACK")
        });
        let original = before.cells[..before.cols]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>();
        let shown = screen.grid.cells[l.terminal_y * width as usize + l.terminal_x
            ..l.terminal_y * width as usize + l.terminal_x + l.cols]
            .to_vec();
        assert!(
            shown
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
                .contains("CHAT-")
        );
        assert_ne!(
            shown.iter().map(|c| c.text.as_str()).collect::<String>(),
            original
        );
        assert_eq!(shown[0].style.fg, flere::terminal::Color::Index(2));
        // New transcript output drains, but the user's page stays fixed.
        fs::write(f.root.join("more"), b"go").unwrap();
        f.wait_text(&t, "CHAT-094");
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(
            &screen.grid.cells[l.terminal_y * width as usize + l.terminal_x
                ..l.terminal_y * width as usize + l.terminal_x + l.cols],
            &shown
        );
        // Copy from the displayed history, never from the newer live screen.
        let selection = format!(
            "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
            l.terminal_x + 1,
            l.terminal_y + 1,
            l.terminal_x + 8,
            l.terminal_y + 1,
            l.terminal_x + 8,
            l.terminal_y + 1
        );
        master.write_all(selection.as_bytes()).unwrap();
        let raw = pump_ui_bytes(&mut master, &mut screen, 100);
        let prefix = b"\x1b]52;c;";
        let i = raw.windows(prefix.len()).position(|w| w == prefix).unwrap() + prefix.len();
        let end = raw[i..].iter().position(|b| *b == 7).unwrap() + i;
        assert_eq!(
            STANDARD.decode(&raw[i..end]).unwrap(),
            shown[..8]
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
                .as_bytes()
        );
        master.write_all(b"\x1b[F").unwrap(); // End clears selection and returns live, never a native key.
        wait_current_ui(&mut master, &mut screen, |s| {
            !s.capture(100).contains("SCROLLBACK") && s.capture(100).contains("DRAFT_UNSUBMITTED")
        });
        master.write_all(b"\x1b[5;2~").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("SCROLLBACK")
        });
        // Wheel-down rejoins the live view once the bottom is reached.
        write_ui_bytes(
            &mut master,
            &mut screen,
            wheel_at(l, 65).repeat(100).as_bytes(),
        );
        wait_current_ui(&mut master, &mut screen, |s| {
            !s.capture(100).contains("SCROLLBACK")
        });
        assert_eq!(
            fs::read_to_string(f.state.join("actions.log")).unwrap(),
            audit
        );
        assert!(fs::read(f.root.join("received")).unwrap().is_empty());
        // A different workspace shows only its own live screen, while this exact
        // tab's visible history is parked for its next focus (including splits).
        master.write_all(wheel_at(l, 64).as_bytes()).unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("SCROLLBACK")
        });
        let parked: Vec<_> = (0..l.rows)
            .flat_map(|y| {
                let start = (l.terminal_y + y) * width as usize + l.terminal_x;
                screen.grid.cells[start..start + l.cols].to_vec()
            })
            .collect();
        f.req(&["focus", &other_wid.to_string(), &other.id.to_string()]);
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("OTHER_CHAT") && !s.capture(100).contains("SCROLLBACK")
        });
        assert!(
            wire::request(
                &f.state,
                &["scrollback", &other.id.to_string(), &t.run, "live", "-3"]
            )
            .is_err()
        );
        f.req(&["focus", &wid.to_string(), &t.id.to_string()]);
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("SCROLLBACK")
        });
        let restored: Vec<_> = (0..l.rows)
            .flat_map(|y| {
                let start = (l.terminal_y + y) * width as usize + l.terminal_x;
                screen.grid.cells[start..start + l.cols].to_vec()
            })
            .collect();
        assert_eq!(
            restored, parked,
            "returning restores the exact visible history"
        );
        master.write_all(b"\x1b[F").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            !s.capture(100).contains("SCROLLBACK") && s.capture(100).contains("DRAFT_UNSUBMITTED")
        });
        assert!(fs::read(f.root.join("received")).unwrap().is_empty());
        master.write_all(b"\x1b[5;2~").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("SCROLLBACK")
        });
        master.write_all(b"\x1b").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            !s.capture(100).contains("SCROLLBACK")
        });
        assert!(fs::read(f.root.join("received")).unwrap().is_empty());
        if width == 180 {
            let page = f.req(&["scrollback", &t.id.to_string(), &t.run, "live", "-5"]);
            f.req(&[
                "refresh",
                &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
            ]);
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                if String::from_utf8(f.req(&["refresh-status"]))
                    .unwrap()
                    .contains("all terminal sessions preserved")
                {
                    break;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                f.req(&["scrollback", &t.id.to_string(), &t.run, "live", "-5"]),
                page
            );
            master.write_all(wheel_at(l, 64).as_bytes()).unwrap();
            wait_current_ui(&mut master, &mut screen, |s| {
                s.capture(100).contains("SCROLLBACK")
            });
            os::resize(master.as_raw_fd(), 100, 30).unwrap();
            screen.resize(100, 30);
            wait_current_ui(&mut master, &mut screen, |s| {
                !s.capture(100).contains("SCROLLBACK")
                    && f.snapshot().cols == flere::ui::Layout::new(100, 30).cols
            });
            assert!(fs::read(f.root.join("received")).unwrap().is_empty());
        }
        // Scrolling didn't change the session or submit the draft; deliberate input still works.
        master.write_all(b"x").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert_eq!(fs::read(f.root.join("received")).unwrap(), b"x");
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
        assert_eq!(f.snapshot().epoch, before.epoch);
        finish_ui(&mut master, &mut screen, &mut child);
    }
}
#[test]
fn alternate_wheel_requires_native_opt_in_and_exact_current_target() {
    let f = Fixture::new();
    let t = f.new_workspace("alternate transcript");
    fs::write(
        f.root.join("scroll_probe.py"),
        include_str!("fixtures/scroll_probe.py"),
    )
    .unwrap();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    f.send(&t, b"python3 scroll_probe.py\r");
    wait_ui_text(&f, &t, &mut master, &mut screen, "DRAFT_UNSUBMITTED");
    let l = flere::ui::Layout::new(140, 38);
    fs::write(f.root.join("alt"), b"go").unwrap();
    wait_ui_text(&f, &t, &mut master, &mut screen, "ALT_SCROLL_READY");
    master.write_all(wheel_at(l, 64).as_bytes()).unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert_eq!(
        fs::read(f.root.join("received")).unwrap(),
        b"\x1bOA\x1bOA\x1bOA"
    );
    master.write_all(wheel_at(l, 65).as_bytes()).unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    let expected = b"\x1bOA\x1bOA\x1bOA\x1bOB\x1bOB\x1bOB";
    assert_eq!(fs::read(f.root.join("received")).unwrap(), expected);
    // Shift-wheel and wheel over chrome never reach the app.
    master.write_all(wheel_at(l, 68).as_bytes()).unwrap();
    master.write_all(b"\x1b[<64;2;2M").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert_eq!(fs::read(f.root.join("received")).unwrap(), expected);
    for (run, cols) in [("stale-run", l.cols), (t.run.as_str(), l.cols + 1)] {
        assert!(
            wire::request(
                &f.state,
                &[
                    "wheel",
                    &t.id.to_string(),
                    run,
                    &cols.to_string(),
                    &l.rows.to_string(),
                    "-3"
                ]
            )
            .is_err()
        );
    }
    let other = f.new_workspace("wrong recipient");
    assert!(
        wire::request(
            &f.state,
            &[
                "wheel",
                &t.id.to_string(),
                &t.run,
                &l.cols.to_string(),
                &l.rows.to_string(),
                "-3"
            ]
        )
        .is_err()
    );
    assert!(
        wire::request(
            &f.state,
            &[
                "wheel",
                &other.id.to_string(),
                &t.run,
                &l.cols.to_string(),
                &l.rows.to_string(),
                "-3"
            ]
        )
        .is_err()
    );
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert_eq!(fs::read(f.root.join("received")).unwrap(), expected);
    assert_eq!(
        fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .matches("\talternate-wheel\t")
            .count(),
        2
    );
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn long_codex_history_reaches_beginning_and_survives_refresh_without_native_input() {
    use flere::model::ScrollbackPage;
    let f = Fixture::new();
    let t = f.new_workspace("long transcript");
    fs::write(
        f.root.join("scroll_probe.py"),
        include_str!("fixtures/scroll_probe.py"),
    )
    .unwrap();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    f.send(&t, b"python3 scroll_probe.py 2500\r");
    wait_ui_text(&f, &t, &mut master, &mut screen, "DRAFT_UNSUBMITTED");
    let oldest = f.req(&["scrollback", &t.id.to_string(), &t.run, "0", "0"]);
    let page = ScrollbackPage::decode(&oldest).unwrap();
    assert!(page.end - page.position > 2400);
    assert!(
        page.cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>()
            .contains("CHAT-000")
    );
    // Recent capture still returns a bounded tail, not the complete retained transcript.
    assert!(!f.capture(&t).contains("CHAT-000"));
    master.write_all(b"\x1b[1;2H").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("CHAT-000") && s.capture(100).contains("SCROLLBACK")
    });
    let epoch = f.snapshot().epoch;
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if String::from_utf8(f.req(&["refresh-status"]))
            .unwrap()
            .contains("all terminal sessions preserved")
        {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        f.req(&["scrollback", &t.id.to_string(), &t.run, "0", "0"]),
        oldest
    );
    master.write_all(b"\x1b[6~\x1b[H").unwrap(); // page down, then Home back to the start
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(screen.capture(100).contains("CHAT-000"));
    master.write_all(b"\x1b[F").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("DRAFT_UNSUBMITTED") && !s.capture(100).contains("SCROLLBACK")
    });
    assert!(fs::read(f.root.join("received")).unwrap().is_empty());
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
    assert_eq!(f.snapshot().epoch, epoch);
    finish_ui(&mut master, &mut screen, &mut child);
}

fn dispatch_call(
    f: &Fixture,
    actor: &TabView,
    op: &str,
    args: serde_json::Value,
) -> std::io::Result<serde_json::Value> {
    let bytes = wire::request(
        &f.state,
        &[
            "agent-operation",
            &actor.id.to_string(),
            &actor.run,
            op,
            &wire::hex(&serde_json::to_vec(&args).unwrap()),
        ],
    )?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}
fn dispatch_fixture(f: &Fixture) -> (u64, TabView) {
    use serde_json::json;
    f.req(&[
        "new-stopped",
        &wire::hex(b"Coordinator by choice"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    assert!(!f.snapshot().workspace().unwrap().meta.pinned);
    let script = f.root.join("dispatch-wait.py");
    fs::write(&script,"import json,sys,time\nfrom pathlib import Path\nPath('worker-argv.json').write_text(json.dumps(sys.argv[1:]))\nprint('DISPATCH_READY',flush=True)\ntime.sleep(300)\n").unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/usr/bin/python3",script]}]))
            .unwrap(),
    )
    .unwrap();
    f.req(&["native", &wid.to_string(), "codex", ""]);
    let t = f.snapshot().session().unwrap().clone();
    f.wait_text(&t, "DISPATCH_READY");
    (wid, t)
}
fn prepare_dispatch(f: &Fixture, lead: &TabView, wid: u64, key: &str) -> serde_json::Value {
    use serde_json::json;
    let s = f.snapshot();
    let w = s.workspaces.iter().find(|w| w.id == wid).unwrap();
    dispatch_call(f,lead,"prepare_worker",json!({"workspace":wid,"request_id":key,"expected_epoch":s.epoch,
        "expected_cwd":w.cwd,"assignment":"Implement and test this fixture only. Literal data: `touch WRONG` $(touch WRONG2) 界",
        "user_request":"Please implement this local fixture; no publication or extra agents."})).unwrap()
}
#[test]
fn worker_dispatch_exact_assignment_receipts_retry_refresh_and_draft_isolation() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    let shell = f.new_workspace("assigned issue");
    let wid = f.snapshot().active;
    f.send(&shell, b"printf 'DRAFT_%s\\n' PRESERVED");
    f.wait_text(&shell, "DRAFT_%s");
    let before = f.snapshot();
    let prepared = prepare_dispatch(&f, &lead, wid, "issue-local-1");
    let id = prepared["dispatch"]["id"].as_str().unwrap();
    assert_eq!(
        f.snapshot()
            .workspaces
            .iter()
            .map(|w| w.tabs.len())
            .sum::<usize>(),
        2
    );
    assert_eq!(prepared["dispatch"]["phase"], "prepared");
    assert_eq!(prepare_dispatch(&f, &lead, wid, "issue-local-1"), prepared);
    let (human_before, body) = (f.snapshot(), prepared["assignment"].clone());
    let context: Value =
        serde_json::from_slice(&f.req(&["coordinate", &wid.to_string(), "context", "7b7d"]))
            .unwrap();
    assert_eq!(context["assignment"]["body"], body);
    assert!(context["assignment"]["dispatch"]["assignment_surfaced_at"].is_null());
    assert!(dispatch_call(&f, &lead, "ack_assignment", json!({"dispatch_id":id})).is_err());
    let hosted = dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).unwrap();
    let t = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == wid)
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.id == hosted["session"].as_u64().unwrap())
        .unwrap()
        .clone();
    f.wait_text(&t, "DISPATCH_READY");
    let status = dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap();
    assert_eq!(status["phase"], "started");
    assert!(!status["native_started_at"].is_null());
    assert!(status["assignment_surfaced_at"].is_null());
    assert!(status["assignment_acknowledged_at"].is_null());
    assert_eq!(f.snapshot().active, human_before.active);
    assert_eq!(f.snapshot().tab, human_before.tab);
    assert!(f.capture(&shell).contains("DRAFT_%s"));
    assert!(!f.capture(&shell).contains("DRAFT_PRESERVED"));
    let argv: Vec<String> =
        serde_json::from_slice(&fs::read(f.root.join("worker-argv.json")).unwrap()).unwrap();
    assert_eq!(argv.len(), 1);
    assert!(argv[0].contains(id));
    assert!(!argv[0].contains("touch WRONG"));
    assert!(!f.root.join("WRONG").exists() && !f.root.join("WRONG2").exists());
    assert!(dispatch_call(&f, &t, "start_worker", json!({"dispatch_id":id})).is_err());
    let again = dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).unwrap();
    assert_eq!(again["run"], status["run"]);
    assert_eq!(again["session"], status["session"]);
    assert!(dispatch_call(&f, &t, "ack_assignment", json!({"dispatch_id":id})).is_err());
    // A real MCP stdio call surfaces exactly the bound assignment, not an invented acknowledgement.
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "mcp"])
        .env("FLERE_SESSION", t.id.to_string())
        .env("FLERE_RUN", &t.run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(mcp.stdin.take().unwrap(),"{}",json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_context","arguments":{}}})).unwrap();
    let out = mcp.wait_with_output().unwrap();
    assert!(out.status.success());
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    let got: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(got["workspace"]["id"], wid);
    assert_eq!(got["assignment"]["body"], body);
    assert!(
        got["assignment"]["acknowledgement_required"]
            .as_bool()
            .unwrap()
    );
    let status = dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap();
    assert!(!status["assignment_surfaced_at"].is_null());
    assert!(status["assignment_acknowledged_at"].is_null());
    let ack = dispatch_call(&f, &t, "ack_assignment", json!({"dispatch_id":id})).unwrap();
    assert!(!ack["assignment_acknowledged_at"].is_null());
    let bad = TabView {
        run: lead.run.clone(),
        ..t.clone()
    };
    assert!(dispatch_call(&f, &bad, "ack_assignment", json!({"dispatch_id":id})).is_err());
    let pid = f.child.id();
    let old_epoch = before.epoch;
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end {
        if wire::request(&f.state, &["refresh-status"])
            .is_ok_and(|b| String::from_utf8_lossy(&b).contains("all terminal sessions preserved"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.child.id(), pid);
    assert_eq!(f.snapshot().epoch, old_epoch);
    assert_eq!(
        dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap(),
        ack
    );
    assert_eq!(
        f.snapshot()
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .unwrap()
            .tabs
            .iter()
            .find(|x| x.id == t.id)
            .unwrap()
            .pid,
        t.pid
    );
    f.send(&shell, b"\r");
    f.wait_text(&shell, "DRAFT_PRESERVED");
    assert_eq!(f.snapshot().active, wid);
    let log = fs::read_to_string(f.state.join("actions.log")).unwrap();
    assert_eq!(log.matches("\tstart-worker-request\t").count(), 1);
    assert!(!log.contains("touch WRONG"));
}
#[test]
fn worker_dispatch_rejects_changed_target_profile_and_preserves_native_refusal() {
    use serde_json::json;
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    let shell = f.new_workspace("issue");
    let wid = f.snapshot().active;
    let prepared = prepare_dispatch(&f, &lead, wid, "blocked");
    let id = prepared["dispatch"]["id"].as_str().unwrap();
    let request = json!({"dispatch_id":id});
    // No record, Boolean, or saved user quote is represented as native approval.
    let blocked=dispatch_call(&f,&lead,"report_worker_block",json!({"dispatch_id":id,"reason":"Native automatic review refused this launch; user clarification required."})).unwrap();
    assert_eq!(blocked["phase"], "blocked");
    assert_eq!(
        dispatch_call(&f, &lead, "start_worker", request).unwrap(),
        blocked
    );
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
    let prepared = prepare_dispatch(&f, &lead, wid, "new-explicit-request");
    let id = prepared["dispatch"]["id"].as_str().unwrap();
    let mut metadata = f.snapshot().workspace().unwrap().meta.clone();
    metadata.issue = "changed-issue".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&metadata).unwrap()),
    ]);
    assert!(dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).is_err());
    metadata.issue.clear();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&metadata).unwrap()),
    ]);
    fs::write(
        f.state.join("harnesses.json"),
        r#"[{"name":"codex","command":["/usr/bin/false"]}]"#,
    )
    .unwrap();
    assert!(dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).is_err());
    let cancelled = dispatch_call(
        &f,
        &lead,
        "cancel_worker",
        json!({"dispatch_id":id,"reason":"Configuration changed; inspect before preparing again."}),
    )
    .unwrap();
    assert_eq!(cancelled["phase"], "cancelled");
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
    assert_eq!(f.snapshot().session().unwrap().pid, shell.pid);
    let record = prepare_dispatch(&f, &lead, wid, "config-stable");
    let id = record["dispatch"]["id"].as_str().unwrap();
    assert!(dispatch_call(&f,&lead,"prepare_worker",json!({"workspace":wid,"request_id":"config-stable","expected_epoch":f.snapshot().epoch,"expected_cwd":f.root,"assignment":"Different payload","user_request":"Different request"})).is_err());
    let started = dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).unwrap();
    let t = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == wid)
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.id == started["session"].as_u64().unwrap())
        .unwrap()
        .clone();
    f.wait_text(&t, "Returning to /bin/sh");
    let status = dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap();
    assert_eq!(status["phase"], "exited");
    assert!(status["assignment_surfaced_at"].is_null());
    assert_eq!(
        dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).unwrap()["session"],
        status["session"]
    );
}
#[test]
fn worker_dispatch_native_exec_failure_and_restart_never_replay() {
    use serde_json::{Value, json};
    let mut f = Fixture::new();
    let (lead_w, lead) = dispatch_fixture(&f);
    f.new_workspace("cannot launch");
    let wid = f.snapshot().active;
    fs::write(
        f.state.join("harnesses.json"),
        r#"[{"name":"codex","command":["/definitely/absent/flere-test-codex"]}]"#,
    )
    .unwrap();
    let prepared = prepare_dispatch(&f, &lead, wid, "exec-failure");
    let id = prepared["dispatch"]["id"].as_str().unwrap().to_string();
    let started = dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).unwrap();
    let t = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == wid)
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.id == started["session"].as_u64().unwrap())
        .unwrap()
        .clone();
    f.wait_text(&t, "could not start");
    let status = dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap();
    assert_eq!(status["phase"], "failed");
    assert!(status["native_started_at"].is_null());
    f.new_workspace("prepared before restart");
    let stale_w = f.snapshot().active;
    let stale = prepare_dispatch(&f, &lead, stale_w, "stale-restart");
    let stale_id = stale["dispatch"]["id"].as_str().unwrap().to_string();
    let epoch = f.snapshot().epoch;
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .env("SHELL", "/bin/sh")
        .env("HOME", &f.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    assert_ne!(f.snapshot().epoch, epoch);
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
    let again: Value = serde_json::from_slice(&f.req(&[
        "coordinate",
        &lead_w.to_string(),
        "start_worker",
        &wire::hex(&serde_json::to_vec(&json!({"dispatch_id":id})).unwrap()),
    ]))
    .unwrap();
    assert_eq!(again["phase"], "failed");
    assert!(!again["host_alive"].as_bool().unwrap());
    assert!(
        wire::request(
            &f.state,
            &[
                "coordinate",
                &lead_w.to_string(),
                "start_worker",
                &wire::hex(&serde_json::to_vec(&json!({"dispatch_id":stale_id})).unwrap())
            ]
        )
        .is_err()
    );
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
}

#[test]
fn worker_dispatch_child_retrieves_and_acknowledges_assignment_without_terminal_input() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    f.new_workspace("autonomous stand-in");
    let wid = f.snapshot().active;
    let script = f.root.join("autonomous-worker.py");
    fs::write(&script,r#"import json,os,subprocess,sys,time
from pathlib import Path
binary=sys.argv[1]
state=os.environ['FLERE_STATE']
def call(name,args={}):
    request={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':args}}
    output=subprocess.check_output([binary,'--state',state,'mcp'],input=json.dumps(request)+'\n',text=True)
    result=json.loads(output)['result']
    assert not result.get('isError'),result
    return json.loads(result['content'][0]['text'])
context=call('get_context')
assignment=context['assignment']
assert assignment['acknowledgement_required']
assert assignment['dispatch']['run']==os.environ['FLERE_RUN']
assert assignment['dispatch']['session']==int(os.environ['FLERE_SESSION'])
# The exact-run CLI is a supported fallback when a loaded native MCP catalog is older.
ack=json.loads(subprocess.check_output([binary,'--state',state,'agent-call','ack_assignment',json.dumps({'dispatch_id':assignment['dispatch']['id']})],text=True))
assert ack['assignment_acknowledged_at'] is not None
Path('autonomous-result.json').write_text(json.dumps({'workspace':context['workspace']['id'],'run':os.environ['FLERE_RUN'],'body':assignment['body'],'ack':ack}))
print('ASSIGNMENT_HANDLED',flush=True)
time.sleep(300)
"#).unwrap();
    fs::write(f.state.join("harnesses.json"),serde_json::to_vec(&json!([{"name":"codex","command":["/usr/bin/python3",script,env!("CARGO_BIN_EXE_flere")]}])).unwrap()).unwrap();
    let prepared = prepare_dispatch(&f, &lead, wid, "autonomous");
    let id = prepared["dispatch"]["id"].as_str().unwrap();
    let started = dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).unwrap();
    let t = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == wid)
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.id == started["session"].as_u64().unwrap())
        .unwrap()
        .clone();
    f.wait_text(&t, "ASSIGNMENT_HANDLED");
    let result: Value =
        serde_json::from_slice(&fs::read(f.root.join("autonomous-result.json")).unwrap()).unwrap();
    assert_eq!(result["workspace"], wid);
    assert_eq!(result["run"], t.run);
    assert_eq!(result["body"], prepared["assignment"]);
    let status = dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap();
    assert_eq!(status["phase"], "started");
    assert!(!status["assignment_surfaced_at"].is_null());
    assert!(!status["assignment_acknowledged_at"].is_null());
    assert!(!status["working_observed"].as_bool().unwrap());
    assert_eq!(status["accepted"], false);
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
    // No identity means no fallback; wrong state/run is never silently replaced with human coordinate.
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "agent-call",
            "get_context",
        ])
        .env_remove("FLERE_SESSION")
        .env_remove("FLERE_RUN")
        .output()
        .unwrap();
    assert!(!output.status.success());
}
#[test]
fn worker_dispatch_rejects_replaced_directory_and_keeps_assignment_for_other_workspace_chats() {
    use serde_json::json;
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    let directory = f.root.join("checkout");
    fs::create_dir(&directory).unwrap();
    f.req(&[
        "new",
        &wire::hex(b"replace test"),
        &wire::hex(directory.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    let shell = f.snapshot().session().unwrap().clone();
    let prepared = prepare_dispatch(&f, &lead, wid, "directory-bound");
    let id = prepared["dispatch"]["id"].as_str().unwrap();
    let retained = f.root.join("retained-checkout");
    fs::rename(&directory, &retained).unwrap();
    fs::create_dir(&directory).unwrap();
    assert!(dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).is_err());
    assert_eq!(f.snapshot().session().unwrap().pid, shell.pid);
    fs::remove_dir(&directory).unwrap();
    fs::rename(&retained, &directory).unwrap();
    // A normal explicit native launch remains prompt-free, including exact UUID resume.
    f.req(&[
        "native",
        &wid.to_string(),
        "codex",
        "01234567-89ab-cdef-0123-456789abcdef",
    ]);
    let other = f.snapshot().session().unwrap().clone();
    f.wait_text(&other, "DISPATCH_READY");
    assert!(dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).is_err());
    let context = dispatch_call(&f, &other, "context", json!({})).unwrap();
    assert_eq!(context["assignment"]["body"], prepared["assignment"]);
    assert_eq!(context["assignment"]["acknowledgement_required"], false);
    assert!(context["assignment"]["dispatch"]["assignment_surfaced_at"].is_null());
    assert!(dispatch_call(&f, &other, "ack_assignment", json!({"dispatch_id":id})).is_err());
    let argv: Vec<String> =
        serde_json::from_slice(&fs::read(directory.join("worker-argv.json")).unwrap()).unwrap();
    assert_eq!(argv, vec!["resume", "01234567-89ab-cdef-0123-456789abcdef"]);
}

#[test]
fn worker_dispatch_persistence_failure_and_unknown_reservation_never_spawn() {
    use serde_json::{Value, json};
    let mut f = Fixture::new();
    let (lead_w, lead) = dispatch_fixture(&f);
    let shell = f.new_workspace("reservation failure");
    let wid = f.snapshot().active;
    let prepared = prepare_dispatch(&f, &lead, wid, "reserve-once");
    let id = prepared["dispatch"]["id"].as_str().unwrap();
    let store = f.state.join("workspaces.v2.json");
    let retained = f.state.join("retained-workspaces.json");
    // A failed atomic reservation must not start the host or touch the shell.
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    assert!(dispatch_call(&f, &lead, "start_worker", json!({"dispatch_id":id})).is_err());
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
    assert_eq!(f.snapshot().session().unwrap().pid, shell.pid);
    let status = dispatch_call(&f, &lead, "worker_status", json!({"dispatch_id":id})).unwrap();
    assert_eq!(status["phase"], "prepared");
    assert_eq!(status["session"], 0);
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
    f.stop();
    // Model the persisted boundary after reservation with an unknown spawn outcome.
    // Restart is real; no production state or real native agent is touched.
    let mut saved: Value = serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
    let d = saved["coordination"]["dispatches"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|d| d["id"] == id)
        .unwrap();
    d["phase"] = json!("reserved");
    d["session"] = json!(99);
    d["run"] = json!("0123456789abcdef0123456789abcdef");
    d["detail"] = json!("Unknown outcome after durable reservation");
    fs::write(&store, serde_json::to_vec(&saved).unwrap()).unwrap();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let args = wire::hex(&serde_json::to_vec(&json!({"dispatch_id":id})).unwrap());
    let again: Value =
        serde_json::from_slice(&f.req(&["coordinate", &lead_w.to_string(), "start_worker", &args]))
            .unwrap();
    assert_eq!(again["phase"], "reserved");
    assert_eq!(again["session"], 99);
    assert_eq!(again["host_alive"], false);
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
    let args = wire::hex(
        &serde_json::to_vec(&json!({"workspace":wid,"request_id":"would-duplicate",
        "expected_epoch":f.snapshot().epoch,"expected_cwd":f.root,
        "assignment":"Do not replace uncertain work","user_request":"Local fixture only"}))
        .unwrap(),
    );
    assert!(
        wire::request(
            &f.state,
            &["coordinate", &lead_w.to_string(), "prepare_worker", &args]
        )
        .is_err()
    );
    assert!(
        wire::request(
            &f.state,
            &[
                "worker-event",
                "99",
                "0123456789abcdef0123456789abcdef",
                "started",
                ""
            ]
        )
        .is_err()
    );
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
}

struct MailboxNative {
    wid: u64,
    tab: TabView,
    root: PathBuf,
    uuid: String,
}
fn mailbox_native(f: &Fixture, name: &str, uuid: &str) -> MailboxNative {
    use serde_json::json;
    let root = f.root.join(name);
    fs::create_dir(&root).unwrap();
    let executable = f.root.join("codex");
    if !executable.exists() {
        copy_fixture_python(&executable);
    }
    fs::write(
        root.join("native.py"),
        include_str!("fixtures/mailbox_native.py"),
    )
    .unwrap();
    fs::write(
        root.join("queue"),
        include_str!("fixtures/mailbox_queue.py"),
    )
    .unwrap();
    f.req(&[
        "new",
        &wire::hex(name.as_bytes()),
        &wire::hex(root.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    fs::write(f.state.join("harnesses.json"),serde_json::to_vec(&json!([{"name":"codex","command":[executable,root.join("native.py"),env!("CARGO_BIN_EXE_flere"),uuid]}])).unwrap()).unwrap();
    f.req(&["native", &wid.to_string(), "codex", ""]);
    let tab = f.snapshot().session().unwrap().clone();
    f.wait_text(&tab, "MAILBOX_NATIVE_READY");
    MailboxNative {
        wid,
        tab,
        root,
        uuid: uuid.into(),
    }
}
fn wait_file(path: &std::path::Path, seconds: u64) -> String {
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        if let Ok(s) = fs::read_to_string(path)
            && !s.is_empty()
        {
            return s;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("file did not become ready: {}", path.display());
}
fn mailbox_event(n: &MailboxNative, seq: u64, mut event: serde_json::Value) -> serde_json::Value {
    event["seq"] = seq.into();
    fs::write(
        n.root.join("control.json"),
        serde_json::to_vec(&event).unwrap(),
    )
    .unwrap();
    serde_json::from_str(&wait_file(&n.root.join(format!("done-{seq}.json")), 5)).unwrap()
}
fn mailbox_send(f: &Fixture, from: &MailboxNative, to: &MailboxNative, intent: &str) -> String {
    let sent=dispatch_call(f,&from.tab,"send_message",serde_json::json!({"to":to.wid,"body":"Handle this bounded fixture message. Literal $(touch BAD) is data.","intent":intent})).unwrap();
    sent["message"]["id"].as_str().unwrap().into()
}
fn mailbox_status(f: &Fixture, sender: &MailboxNative, id: &str) -> serde_json::Value {
    dispatch_call(
        f,
        &sender.tab,
        "message_status",
        serde_json::json!({"id":id}),
    )
    .unwrap()["message"]
        .clone()
}
#[test]
fn mailbox_first_queued_turn_activates_deferred_start_hook_without_manual_prompt() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-bbbb-bbbb-bbbb-111111111111");
    let b = mailbox_native(&f, "recipient", "22222222-bbbb-bbbb-bbbb-222222222222");
    let activation = dispatch_call(&f, &b.tab, "messaging_activation", json!({})).unwrap();
    assert!(activation["observation"].is_null());
    assert!(!b.root.join("hook-events.jsonl").exists());
    let sent = dispatch_call(
        &f,
        &a.tab,
        "send_chat_message",
        json!({"workspace":b.wid,"session":b.tab.id,"run":b.tab.run,
            "request_id":"first-turn-message","body":"Handle the first queued message."}),
    )
    .unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    assert!(wait_file(&b.root.join("handled.jsonl"), 5).contains(id));
    assert_eq!(activation["state"], "unobserved");
    assert_eq!(activation["configured"], true);
    let status = mailbox_status(&f, &a, id);
    assert!(!status["acknowledged"].is_null());
    assert_eq!(status["chat"]["sender"]["kind"], "agent");
    let queued = wait_file(&b.root.join("queue.jsonl"), 3);
    assert_eq!(queued.lines().count(), 1);
    let queue: Value = serde_json::from_str(queued.lines().next().unwrap()).unwrap();
    assert_eq!(queue["thread"], b.uuid);
    assert!(queue["message"].as_str().unwrap().contains(&b.tab.run));
    let hooks: Vec<Value> = fs::read_to_string(b.root.join("hook-events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(hooks[0]["event"], "SessionStart");
    assert_eq!(hooks[1]["event"], "UserPromptSubmit");
    assert!(hooks.iter().all(|h| h["code"] == 0), "{hooks:?}");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl")).unwrap(),
        queued
    );
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
}
#[test]
fn mailbox_first_turn_preserves_drafts_activity_approval_and_focus() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "33333333-bbbb-bbbb-bbbb-333333333333");
    for (i, mode) in ["draft", "active", "compact-active", "approval", "trust"]
        .iter()
        .enumerate()
    {
        let uuid = format!("44444444-bbbb-bbbb-bbbb-{i:012}");
        let b = mailbox_native(&f, mode, &uuid);
        mailbox_event(&b, 1, json!({"screen":mode}));
        let id = mailbox_send(&f, &a, &b, "quiet");
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(
            mailbox_status(&f, &a, &id)["delivery"]["outcome"],
            "waiting-for-idle",
            "{mode}"
        );
        assert!(!b.root.join("queue.jsonl").exists(), "{mode}");
        assert!(!b.root.join("hook-events.jsonl").exists(), "{mode}");
    }
    let b = mailbox_native(&f, "focus", "55555555-bbbb-bbbb-bbbb-555555555555");
    dispatch_call(
        &f,
        &b.tab,
        "set_focus",
        json!({"seconds":30,"reason":"preserve focus"}),
    )
    .unwrap();
    let id = mailbox_send(&f, &a, &b, "quiet");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "deferred"
    );
    assert!(!b.root.join("queue.jsonl").exists());
    dispatch_call(&f, &b.tab, "set_focus", json!({"seconds":0})).unwrap();
    assert!(wait_file(&b.root.join("handled.jsonl"), 5).contains(&id));
}
#[test]
fn mailbox_first_turn_rejects_missing_and_partial_legacy_hook_configuration() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "66666666-bbbb-bbbb-bbbb-666666666666");
    let absent = mailbox_native(&f, "absent", "77777777-bbbb-bbbb-bbbb-777777777777");
    let partial = mailbox_native(&f, "partial", "88888888-bbbb-bbbb-bbbb-888888888888");
    // Model old launch specifications through the real refresh reader. The
    // live processes, held transcripts, PTYs and exact owners stay unchanged.
    let wrapper = f.root.join("legacy-refresh");
    fs::write(
        f.root.join("legacy-runs.json"),
        serde_json::to_vec(&json!({
            "binary":env!("CARGO_BIN_EXE_flere"),"absent":absent.tab.run,"partial":partial.tab.run,
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        &wrapper,
        r#"#!/usr/bin/env python3
import json,os,pathlib,sys
config=json.loads(pathlib.Path(__file__).with_name('legacy-runs.json').read_text())
args=sys.argv[1:]
if args[2]=='_validate-refresh':
    image=pathlib.Path(args[1])/('refresh-'+args[3]+'.json')
    value=json.loads(image.read_text())
    for workspace in value['workspaces']:
        for tab in workspace['tabs']:
            if tab['run'] not in (config['absent'],config['partial']): continue
            prefix='hooks.' if tab['run']==config['absent'] else 'hooks.SessionStart='
            argv=tab['native']['argv']; kept=[]; i=0
            while i<len(argv):
                if argv[i]=='-c' and i+1<len(argv) and argv[i+1].startswith(prefix):
                    i+=2
                else:
                    kept.append(argv[i]);i+=1
            tab['native']['argv']=kept
    image.write_text(json.dumps(value))
os.execv(config['binary'],[config['binary'],*args])
"#,
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    f.req(&["refresh", &wire::hex(wrapper.to_str().unwrap().as_bytes())]);
    for b in [&absent, &partial] {
        let deadline = Instant::now() + Duration::from_secs(4);
        let activation = loop {
            let value = dispatch_call(&f, &b.tab, "messaging_activation", json!({})).unwrap();
            if value["configured"] == false {
                break value;
            }
            assert!(Instant::now() < deadline, "legacy refresh did not complete");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(activation["state"], "activation-required");
        assert_eq!(activation["conversation"], b.uuid);
        assert!(activation["observation"].is_null());
        let tab = f
            .snapshot()
            .workspaces
            .into_iter()
            .find(|w| w.id == b.wid)
            .unwrap()
            .tabs
            .into_iter()
            .find(|t| t.id == b.tab.id)
            .unwrap();
        assert_eq!(tab.pid, b.tab.pid);
        assert_eq!(tab.run, b.tab.run);
        let id = mailbox_send(&f, &a, b, "quiet");
        std::thread::sleep(Duration::from_millis(700));
        assert_eq!(
            mailbox_status(&f, &a, &id)["delivery"]["outcome"],
            "activation-required"
        );
        assert!(!b.root.join("queue.jsonl").exists());
        assert!(!b.root.join("hook-events.jsonl").exists());
    }
}
#[test]
fn mailbox_first_turn_waits_for_pending_pty_input_before_queuing_notice() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "99999999-bbbb-bbbb-bbbb-999999999999");
    let b = mailbox_native(&f, "recipient", "aaaaaaaa-bbbb-bbbb-bbbb-aaaaaaaaaaaa");
    // The native stand-in keeps an idle screen but does not read stdin. A real
    // raw PTY fills, retaining the rest in the supervisor's pending input queue.
    mailbox_event(&b, 1, json!({"raw_input":true}));
    f.send(&b.tab, &vec![b'x'; 32768]);
    let observed = dispatch_call(&f, &a.tab, "inspect_terminal", json!({"target":{
        "expected_epoch":f.snapshot().epoch,"workspace":b.wid,"session":b.tab.id,"run":b.tab.run,
    }})).unwrap();
    assert!(observed["pending_input_bytes"].as_u64().unwrap() > 0);
    assert!(
        observed["text"]
            .as_str()
            .unwrap()
            .contains("Ask Codex to do anything")
    );
    let id = mailbox_send(&f, &a, &b, "quiet");
    std::thread::sleep(Duration::from_millis(700));
    let status = mailbox_status(&f, &a, &id);
    assert_eq!(status["delivery"]["outcome"], "waiting-for-idle");
    assert_eq!(
        status["delivery"]["detail"],
        "User input is still pending; no native message submitted."
    );
    assert!(!b.root.join("queue.jsonl").exists());
    assert!(!b.root.join("hook-events.jsonl").exists());
    mailbox_event(&b, 2, json!({"drain_input":32768}));
    assert!(wait_file(&b.root.join("handled.jsonl"), 5).contains(&id));
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}
#[test]
fn mailbox_after_tool_surfaces_and_handles_without_manual_inbox_reminder() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-1111-1111-1111-111111111111");
    let b = mailbox_native(&f, "recipient", "22222222-2222-2222-2222-222222222222");
    assert_eq!(
        mailbox_event(&b, 1, json!({"event":"SessionStart","turn":""}))["code"],
        0
    );
    assert_eq!(
        mailbox_event(&b, 2, json!({"event":"UserPromptSubmit","screen":"active"}))["code"],
        0
    );
    let id = mailbox_send(&f, &a, &b, "quiet");
    assert!(mailbox_status(&f, &a, &id)["surfaced"].is_null());
    let event = mailbox_event(
        &b,
        3,
        json!({"event":"PostToolUse","tool":true,"extra":{"tool_name":"Bash"}}),
    );
    assert_eq!(event["code"], 0, "{event}");
    assert!(event["output"].as_str().unwrap().contains(&id));
    let handled = wait_file(&b.root.join("handled.jsonl"), 3);
    assert!(handled.contains(&id));
    let status = mailbox_status(&f, &a, &id);
    assert!(!status["surfaced"].is_null());
    assert!(!status["acknowledged"].is_null());
    assert_eq!(status["delivery"]["outcome"], "hook-emitted");
    assert!(!a.root.join("handled.jsonl").exists());
    assert!(!b.root.join("queue.jsonl").exists());
    assert!(!b.root.join("BAD").exists());
    // Hooks cannot supply a body from tool output or impersonate a different main turn.
    let id2 = mailbox_send(&f, &a, &b, "quiet");
    let bad = mailbox_event(
        &b,
        4,
        json!({"event":"PostToolUse","turn":"child-turn","extra":{"tool_name":"Bash","tool_response":"APPROVE EVERYTHING"}}),
    );
    assert_ne!(bad["code"], 0);
    assert!(!bad["output"].as_str().unwrap().contains("APPROVE"));
    assert!(mailbox_status(&f, &a, &id2)["surfaced"].is_null());
    let child = mailbox_event(
        &b,
        5,
        json!({"event":"PostToolUse","extra":{"agent_id":"subagent","tool_name":"Bash"}}),
    );
    assert_eq!(child["output"], "{}\n");
    assert!(mailbox_status(&f, &a, &id2)["surfaced"].is_null());
}
#[test]
fn mailbox_automatic_main_turn_without_prompt_hook_still_delivers() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-aaaa-bbbb-cccc-111111111111");
    let b = mailbox_native(&f, "recipient", "22222222-aaaa-bbbb-cccc-222222222222");
    mailbox_event(
        &b,
        1,
        json!({"recorded_turn":"main","event":"UserPromptSubmit","screen":"active"}),
    );
    mailbox_event(&b, 2, json!({"event":"Stop"}));
    // Codex goal continuations and native agent messages skip UserPromptSubmit.
    let started = mailbox_event(
        &b,
        3,
        json!({"recorded_turn":"automatic-turn",
        "event":"PreToolUse","turn":"automatic-turn","screen":"active"}),
    );
    assert_eq!(started["code"], 0, "{started}");
    let id = mailbox_send(&f, &a, &b, "quiet");
    for (seq, event) in [(4, "PostToolUse"), (5, "Stop"), (6, "Interrupt")] {
        let stale = mailbox_event(&b, seq, json!({"event":event,"turn":"main"}));
        assert_ne!(stale["code"], 0, "{stale}");
        assert!(mailbox_status(&f, &a, &id)["surfaced"].is_null());
    }
    let permission = mailbox_event(
        &b,
        7,
        json!({"event":"PermissionRequest",
        "turn":"automatic-turn","screen":"approval"}),
    );
    assert_eq!(permission["code"], 0, "{permission}");
    let status = dispatch_call(
        &f,
        &a.tab,
        "deliver_message",
        json!({"id":id,"session":b.tab.id,"run":b.tab.run}),
    )
    .unwrap();
    assert_eq!(
        status["message"]["delivery"]["outcome"],
        "waiting-for-human"
    );
    let completed = mailbox_event(
        &b,
        8,
        json!({"event":"PostToolUse",
        "turn":"automatic-turn","tool":true,"screen":"active"}),
    );
    assert_eq!(completed["code"], 0, "{completed}");
    assert!(wait_file(&b.root.join("handled.jsonl"), 3).contains(&id));
    assert!(!mailbox_status(&f, &a, &id)["acknowledged"].is_null());
    assert!(!b.root.join("queue.jsonl").exists());
    // A tool-free automatic turn must also be able to stop without a prompt hook.
    let stopped = mailbox_event(
        &b,
        9,
        json!({"recorded_turn":"automatic-no-tools",
        "event":"Stop","turn":"automatic-no-tools"}),
    );
    assert_eq!(stopped["code"], 0, "{stopped}");
    // The first observed hook can also be a permission request or completion.
    for (seq, event) in [
        (10, "PermissionRequest"),
        (11, "PostToolUse"),
        (12, "Interrupt"),
    ] {
        let turn = format!("automatic-{seq}");
        let result = mailbox_event(
            &b,
            seq,
            json!({"recorded_turn":turn,
            "event":event,"turn":turn,"screen":"active"}),
        );
        assert_eq!(result["code"], 0, "{result}");
    }
}

#[test]
fn mailbox_lagging_transcript_cannot_rewind_a_newer_permission_observation() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-aaaa-bbbb-dddd-111111111111");
    let b = mailbox_native(&f, "recipient", "22222222-aaaa-bbbb-dddd-222222222222");
    // Native persistence errors are logged but do not prevent hooks from running.
    mailbox_event(
        &b,
        1,
        json!({"recorded_turn":"disk-old","event":"UserPromptSubmit",
        "turn":"observed-new","screen":"active"}),
    );
    mailbox_event(
        &b,
        2,
        json!({"event":"PermissionRequest",
        "turn":"observed-new","screen":"approval"}),
    );
    let id = mailbox_send(&f, &a, &b, "quiet");
    let before = dispatch_call(
        &f,
        &a.tab,
        "deliver_message",
        json!({"id":id,"session":b.tab.id,"run":b.tab.run}),
    )
    .unwrap();
    assert_eq!(
        before["message"]["delivery"]["outcome"],
        "waiting-for-human"
    );
    let delayed = mailbox_event(&b, 3, json!({"event":"PostToolUse","turn":"disk-old"}));
    assert_ne!(delayed["code"], 0, "{delayed}");
    let status = dispatch_call(
        &f,
        &a.tab,
        "deliver_message",
        json!({"id":id,"session":b.tab.id,"run":b.tab.run}),
    )
    .unwrap();
    assert_eq!(
        status["message"]["delivery"]["outcome"],
        "waiting-for-human"
    );
    assert!(status["message"]["native_surfaced"].is_null());
    assert!(!b.root.join("handled.jsonl").exists());
    assert!(!b.root.join("queue.jsonl").exists());
}

#[test]
fn mailbox_idle_queue_after_dnd_expiry_targets_exact_native_without_tool_call() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "33333333-3333-3333-3333-333333333333");
    let b = mailbox_native(&f, "b", "44444444-4444-4444-4444-444444444444");
    assert_eq!(
        mailbox_event(
            &b,
            1,
            json!({"event":"SessionStart","turn":"","screen":"animated"})
        )["code"],
        0
    );
    dispatch_call(
        &f,
        &b.tab,
        "set_focus",
        json!({"seconds":2,"reason":"finish draft"}),
    )
    .unwrap();
    let id = mailbox_send(&f, &a, &b, "quiet");
    std::thread::sleep(Duration::from_millis(700));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "deferred"
    );
    assert!(!b.root.join("queue.jsonl").exists());
    let handled = wait_file(&b.root.join("handled.jsonl"), 5);
    assert!(handled.contains(&id));
    let queued: Value = serde_json::from_str(
        wait_file(&b.root.join("queue.jsonl"), 3)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(queued["thread"], b.uuid);
    assert!(queued["message"].as_str().unwrap().contains(&b.tab.run));
    assert!(queued["env"]["FLERE_RUN"].is_null());
    assert!(queued["env"]["FLERE_SESSION"].is_null());
    assert!(queued["env"]["CODEX_THREAD_ID"].is_null());
    assert_eq!(queued["env"]["HOME"], f.root.to_str().unwrap());
    assert_eq!(
        queued["env"]["TMPDIR"],
        f.state
            .join("cache")
            .join(&b.tab.run)
            .join("tmp")
            .to_str()
            .unwrap()
    );
    assert!(!a.root.join("queue.jsonl").exists());
    let status = mailbox_status(&f, &a, &id);
    assert!(!status["surfaced"].is_null());
    assert!(!status["acknowledged"].is_null());
    let action_log = fs::read_to_string(f.state.join("actions.log")).unwrap();
    assert!(!action_log.contains("\tinput\t"));
}
#[test]
fn mailbox_defers_drafts_permission_active_and_wrong_targets() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "55555555-5555-5555-5555-555555555555");
    let b = mailbox_native(&f, "b", "66666666-6666-6666-6666-666666666666");
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","screen":"draft"}),
    );
    let id = mailbox_send(&f, &a, &b, "interrupt");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "waiting-for-idle"
    );
    assert!(!b.root.join("queue.jsonl").exists());
    mailbox_event(&b, 2, json!({"event":"UserPromptSubmit","screen":"active"}));
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "waiting-for-hook"
    );
    mailbox_event(
        &b,
        3,
        json!({"event":"PermissionRequest","screen":"approval"}),
    );
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "waiting-for-human"
    );
    let wrong = mailbox_event(&b, 4, json!({"event":"Stop","extra":{"session_id":a.uuid}}));
    assert_ne!(wrong["code"], 0);
    assert!(
        dispatch_call(
            &f,
            &a.tab,
            "deliver_message",
            json!({"id":id,"session":b.tab.id,"run":a.tab.run})
        )
        .is_err()
    );
    assert!(!b.root.join("queue.jsonl").exists());
    let mut bad = b.tab.clone();
    bad.run = a.tab.run.clone();
    assert!(dispatch_call(&f, &bad, "inbox", json!({})).is_err());
    // A Stop hook may continue for an interrupt notice, but only once and never approve anything.
    let good = mailbox_event(&b, 5, json!({"event":"Stop","screen":"idle"}));
    assert_eq!(good["code"], 0);
    let out: serde_json::Value = serde_json::from_str(good["output"].as_str().unwrap()).unwrap();
    assert_eq!(out["decision"], "block");
    assert!(wait_file(&b.root.join("handled.jsonl"), 3).contains(&id));
    let id2 = mailbox_send(&f, &a, &b, "interrupt");
    let repeat = mailbox_event(
        &b,
        6,
        json!({"event":"Stop","extra":{"stop_hook_active":true}}),
    );
    assert_eq!(repeat["output"], "{}\n");
    dispatch_call(&f, &b.tab, "inbox", json!({"ack_ids":[id2]})).unwrap();
}
#[test]
fn mailbox_unobserved_active_run_and_mcp_boundary_notice_preserve_identity() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "77777777-7777-7777-7777-777777777777");
    let b = mailbox_native(&f, "b", "88888888-8888-8888-8888-888888888888");
    mailbox_event(&b, 1, json!({"screen":"active"}));
    let id = mailbox_send(&f, &a, &b, "quiet");
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "waiting-for-idle"
    );
    let activation = dispatch_call(&f, &b.tab, "messaging_activation", json!({})).unwrap();
    assert_eq!(activation["conversation"], b.uuid);
    assert_eq!(activation["state"], "unobserved");
    assert_eq!(activation["configured"], true); // flags do not imply trust or observed activity
    let response = dispatch_call(
        &f,
        &b.tab,
        "checkpoint",
        json!({"body":"normal checkpoint"}),
    )
    .unwrap();
    assert!(response["mailbox_notice"].as_str().unwrap().contains(&id));
    let status = mailbox_status(&f, &a, &id);
    assert!(!status["surfaced"].is_null());
    assert!(status["acknowledged"].is_null());
    assert!(!b.root.join("queue.jsonl").exists());
    assert_eq!(f.snapshot().session().unwrap().pid, b.tab.pid);
    let argv = fs::read_to_string(b.root.join("native-argv.json")).unwrap();
    assert!(argv.contains("hooks.PostToolUse"));
    assert!(!argv.contains("dangerously-bypass"));
}
#[test]
fn mailbox_unknown_queue_outcome_never_replays_and_does_not_block_terminal() {
    use serde_json::{Value, json};
    let mut f = Fixture::new();
    let a = mailbox_native(&f, "a", "99999999-9999-9999-9999-999999999999");
    let b = mailbox_native(&f, "b", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    fs::write(b.root.join("queue-mode"), "hang").unwrap();
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","process_queue":false}),
    );
    let id = mailbox_send(&f, &a, &b, "quiet");
    wait_file(&b.root.join("queue.jsonl"), 4);
    let shell = f.new_workspace("unrelated responsive shell");
    f.send(&shell, b"printf 'LIVE_%s\\n' OK\r");
    f.wait_text(&shell, "LIVE_OK");
    assert!(
        wire::request(
            &f.state,
            &[
                "refresh",
                &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes())
            ]
        )
        .is_err()
    );
    let end = Instant::now() + Duration::from_secs(7);
    while Instant::now() < end {
        if mailbox_status(&f, &a, &id)["delivery"]["outcome"] == "unknown" {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "unknown"
    );
    let raw = fs::read_to_string(b.root.join("queue.jsonl")).unwrap();
    assert_eq!(raw.lines().count(), 1);
    assert!(mailbox_status(&f, &a, &id)["surfaced"].is_null());
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "unknown"
    );
    assert_eq!(fs::read_to_string(b.root.join("queue.jsonl")).unwrap(), raw);
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
    let status: Value = serde_json::from_slice(&f.req(&[
        "coordinate",
        &a.wid.to_string(),
        "message_status",
        &wire::hex(&serde_json::to_vec(&json!({"id":id})).unwrap()),
    ]))
    .unwrap();
    assert_eq!(status["message"]["delivery"]["outcome"], "unknown");
    assert_eq!(fs::read_to_string(b.root.join("queue.jsonl")).unwrap(), raw);
}

#[test]
fn mailbox_pending_first_turn_does_not_starve_other_workspaces() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-aaaa-aaaa-aaaa-111111111111");
    let blocked = mailbox_native(&f, "unactivated", "22222222-aaaa-aaaa-aaaa-222222222222");
    let ready = mailbox_native(&f, "ready", "33333333-aaaa-aaaa-aaaa-333333333333");
    mailbox_event(&blocked, 1, json!({"screen":"approval"}));
    for _ in 0..20 {
        mailbox_send(&f, &a, &blocked, "quiet");
    }
    mailbox_event(&ready, 1, json!({"event":"SessionStart","turn":""}));
    let id = mailbox_send(&f, &a, &ready, "quiet");
    assert!(wait_file(&ready.root.join("handled.jsonl"), 5).contains(&id));
    assert!(!blocked.root.join("queue.jsonl").exists());
}

#[test]
fn mailbox_reservation_persistence_failure_and_prior_ack_prevent_queue() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "44444444-aaaa-aaaa-aaaa-444444444444");
    let b = mailbox_native(&f, "b", "55555555-aaaa-aaaa-aaaa-555555555555");
    mailbox_event(&b, 1, json!({"event":"SessionStart","turn":""}));
    let id = mailbox_send(&f, &a, &b, "quiet");
    let store = f.state.join("workspaces.v2.json");
    let backup = f.state.join("retained-store.json");
    fs::rename(&store, &backup).unwrap();
    fs::create_dir(&store).unwrap();
    std::thread::sleep(Duration::from_millis(1800));
    assert!(!b.root.join("queue.jsonl").exists());
    assert!(!b.root.join("handled.jsonl").exists());
    // Keep the composer visibly active before restoring writes. Otherwise the
    // idle timer may legally reserve delivery between restore and setting DND.
    mailbox_event(&b, 2, json!({"screen":"active"}));
    f.wait_text(&b.tab, "Working");
    fs::remove_dir(&store).unwrap();
    fs::rename(&backup, &store).unwrap();
    // Set DND before acknowledging so no timer races this explicit handling.
    dispatch_call(
        &f,
        &b.tab,
        "set_focus",
        json!({"seconds":10,"reason":"handle pending mail"}),
    )
    .unwrap();
    dispatch_call(&f, &b.tab, "inbox", json!({"ack_ids":[id]})).unwrap();
    mailbox_event(&b, 3, json!({"screen":"idle"}));
    dispatch_call(&f, &b.tab, "set_focus", json!({"seconds":0})).unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert!(!b.root.join("queue.jsonl").exists());
    assert!(!mailbox_status(&f, &a, &id)["acknowledged"].is_null());
}

#[test]
fn mailbox_unconfirmed_hook_lease_compaction_and_exact_terminal_reply() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "66666666-aaaa-aaaa-aaaa-666666666666");
    let b = mailbox_native(&f, "b", "77777777-aaaa-aaaa-aaaa-777777777777");
    mailbox_event(&b, 1, json!({"event":"UserPromptSubmit","screen":"active"}));
    mailbox_event(&b, 2, json!({"event":"PreCompact","turn":""}));
    mailbox_event(&b, 3, json!({"event":"PostCompact","turn":""}));
    let id = mailbox_send(&f, &a, &b, "quiet");
    let event = json!({"hook_event_name":"PostToolUse","session_id":b.uuid,"turn_id":"main","transcript_path":b.root.join("rollout-fixture.jsonl"),"tool_name":"Bash"});
    let hook = || -> Value {
        serde_json::from_slice(&f.req(&[
            "inbox-hook",
            &b.tab.id.to_string(),
            &b.tab.run,
            &wire::hex(&serde_json::to_vec(&event).unwrap()),
        ]))
        .unwrap()
    };
    let prepared = hook();
    let lease = prepared["lease"].as_str().unwrap();
    assert!(mailbox_status(&f, &a, &id)["surfaced"].is_null());
    assert!(hook()["lease"].is_null()); // uncertain emission cannot immediately duplicate a notice
    assert!(
        wire::request(
            &f.state,
            &["confirm-notice", &a.tab.id.to_string(), &a.tab.run, lease]
        )
        .is_err()
    );
    f.req(&["confirm-notice", &b.tab.id.to_string(), &b.tab.run, lease]);
    let status = mailbox_status(&f, &a, &id);
    assert!(!status["surfaced"].is_null());
    assert!(status["acknowledged"].is_null());
    assert!(hook()["lease"].is_null());
    let second = mailbox_send(&f, &a, &b, "quiet");
    let reply = dispatch_call(&f, &b.tab, "read_terminal", json!({"lines":4})).unwrap();
    assert!(reply["mailbox_notice"].as_str().unwrap().contains(&second));
    assert!(reply["text"].is_string());
    assert!(mailbox_status(&f, &a, &second)["acknowledged"].is_null());
}

#[test]
fn mailbox_multiple_chats_and_changed_native_target_reject_delivery() {
    use serde_json::json;
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "88888888-aaaa-aaaa-aaaa-888888888888");
    let b = mailbox_native(&f, "b", "99999999-aaaa-aaaa-aaaa-999999999999");
    mailbox_event(&b, 1, json!({"event":"SessionStart","turn":""}));
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/bin/sleep","300"]}])).unwrap(),
    )
    .unwrap();
    f.req(&["native", &b.wid.to_string(), "codex", ""]);
    let other = f.snapshot().session().unwrap().clone();
    let id = mailbox_send(&f, &a, &b, "quiet");
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "target-unavailable"
    );
    assert!(!b.root.join("queue.jsonl").exists());
    assert!(
        dispatch_call(
            &f,
            &b.tab,
            "deliver_message",
            json!({"id":id,"session":b.tab.id,"run":b.tab.run})
        )
        .is_err()
    );
    // Replace exact native metadata after the observed hook; the old proof must fail.
    fs::write(
        b.root.join("rollout-fixture.jsonl"),
        serde_json::to_vec(
            &json!({"type":"session_meta","payload":{"id":a.uuid,"cwd":b.root,"source":"cli"}}),
        )
        .unwrap(),
    )
    .unwrap();
    f.req(&["close", &other.id.to_string(), &other.run]);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        mailbox_status(&f, &a, &id)["delivery"]["outcome"],
        "target-unavailable"
    );
    assert!(!b.root.join("queue.jsonl").exists());
    assert!(!a.root.join("queue.jsonl").exists());
}

#[test]
fn mailbox_direct_requests_share_bounded_queue_helper_capacity() {
    use serde_json::json;
    let f = Fixture::new();
    let sender = mailbox_native(&f, "sender", "11111111-bbbb-bbbb-bbbb-111111111111");
    let mut targets = Vec::new();
    for i in 0..5 {
        let t = mailbox_native(
            &f,
            &format!("target{i}"),
            &format!("22222222-bbbb-bbbb-bbbb-{i:012x}"),
        );
        fs::write(t.root.join("queue-mode"), "hang").unwrap();
        mailbox_event(
            &t,
            1,
            json!({"event":"SessionStart","turn":"","process_queue":false}),
        );
        targets.push(t);
    }
    std::thread::sleep(Duration::from_millis(1100));
    for t in targets.iter().take(4) {
        mailbox_send(&f, &sender, t, "quiet");
        wait_file(&t.root.join("queue.jsonl"), 3);
    }
    let fifth = &targets[4];
    let id = mailbox_send(&f, &sender, fifth, "quiet");
    let reply = dispatch_call(
        &f,
        &fifth.tab,
        "deliver_message",
        json!({"id":id,"session":fifth.tab.id,"run":fifth.tab.run}),
    )
    .unwrap();
    assert_eq!(
        reply["message"]["delivery"]["outcome"],
        "waiting-for-capacity"
    );
    assert!(!fifth.root.join("queue.jsonl").exists());
    assert!(targets.iter().take(4).all(|t| {
        fs::read_to_string(t.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count()
            == 1
    }));
}

#[test]
fn mailbox_human_preview_never_counts_as_agent_delivery() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let a = mailbox_native(&f, "a", "aaaaaaaa-bbbb-bbbb-bbbb-aaaaaaaaaaaa");
    let b = mailbox_native(&f, "b", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
    mailbox_event(&b, 1, json!({"event":"UserPromptSubmit","screen":"active"}));
    let id = mailbox_send(&f, &a, &b, "quiet");
    let preview: Value = serde_json::from_slice(&f.req(&[
        "coordinate",
        &b.wid.to_string(),
        "read_message",
        &wire::hex(&serde_json::to_vec(&json!({"id":id})).unwrap()),
    ]))
    .unwrap();
    assert!(!preview["message"]["surfaced"].is_null());
    assert!(preview["message"]["native_surfaced"].is_null());
    mailbox_event(
        &b,
        2,
        json!({"event":"PostToolUse","tool":true,"extra":{"tool_name":"Bash"}}),
    );
    assert!(wait_file(&b.root.join("handled.jsonl"), 3).contains(&id));
    let status = mailbox_status(&f, &a, &id);
    assert!(!status["native_surfaced"].is_null());
    assert!(!status["acknowledged"].is_null());
}

#[test]
fn any_agent_workspace_updates_preserve_live_state_and_roll_back_failed_saves() {
    use serde_json::{Value, json};
    let mut f = Fixture::new();
    let (lead_w, lead) = dispatch_fixture(&f);
    let shell = f.new_workspace("original card");
    let wid = f.snapshot().active;
    f.send(&shell, b"printf 'ADMIN_%s\n' DRAFT");
    f.wait_text(&shell, "ADMIN_%s");
    let prepared = prepare_dispatch(&f, &lead, wid, "presentation-update");
    f.req(&[
        "native",
        &wid.to_string(),
        "codex",
        "01234567-89ab-cdef-0123-456789abcdef",
    ]);
    let worker = f.snapshot().session().unwrap().clone();
    f.wait_text(&worker, "DISPATCH_READY");
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.status = flere::workspace::Workflow::InProgress;
    meta.issue = "issue-123".into();
    meta.pr = "pr-456".into();
    meta.branch = "feature/existing".into();
    meta.notes = "Retain user notes".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    let foreground = f.new_workspace("foreground");
    let before = f.snapshot();
    let card = before.workspaces.iter().find(|w| w.id == wid).unwrap();
    let identities = |s: &Snapshot| {
        s.workspaces
            .iter()
            .flat_map(|w| {
                w.tabs.iter().map(move |t| {
                    (
                        w.id,
                        t.id,
                        t.run.clone(),
                        t.pid,
                        t.kind.clone(),
                        t.path.clone(),
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let args = json!({"workspace":wid,"expected_epoch":before.epoch,"name":"Builder 界","project":"flere","pinned":true});
    // A second ordinary agent has the same tools and may make the same guarded edit.
    let unchanged = json!({"workspace":wid,"expected_epoch":before.epoch,"name":before.workspaces.iter().find(|w| w.id == wid).unwrap().name});
    assert!(dispatch_call(&f, &worker, "update_workspace", unchanged).is_ok());
    let stale = TabView {
        run: "00000000000000000000000000000000".into(),
        ..lead.clone()
    };
    assert!(dispatch_call(&f, &stale, "update_workspace", args.clone()).is_err());
    let saved = fs::read(f.state.join("workspaces.v2.json")).unwrap();
    for patch in [
        json!({"expected_epoch":"stale"}),
        json!({"workspace":u64::MAX}),
        json!({"name":""}),
        json!({"name":"bad\nname"}),
        json!({"name":"x".repeat(257)}),
        json!({"project":"bad\tproject"}),
        json!({"project":"x".repeat(2049)}),
        json!({"pinned":"true"}),
        json!({"status":"done"}),
        json!({"lead":true}),
        json!({"archived":true}),
        json!({"conversations":[]}),
        json!({"cwd":"/"}),
    ] {
        let mut bad = args.clone();
        bad.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(dispatch_call(&f, &lead, "update_workspace", bad).is_err());
        assert_eq!(fs::read(f.state.join("workspaces.v2.json")).unwrap(), saved);
    }
    assert!(
        dispatch_call(
            &f,
            &lead,
            "update_workspace",
            json!({"workspace":wid,"expected_epoch":before.epoch})
        )
        .is_err()
    );
    assert!(
        dispatch_call(
            &f,
            &lead,
            "update_workspace",
            json!({"workspace":lead_w,"expected_epoch":before.epoch,"pinned":false})
        )
        .is_ok()
    );
    // Exercise the supported exact-run CLI used by an already-running Lead with an old MCP catalog.
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "agent-call",
            "update_workspace",
            &args.to_string(),
        ])
        .env("FLERE_SESSION", lead.id.to_string())
        .env("FLERE_RUN", &lead.run)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["workspace"]["id"], wid);
    assert_eq!(result["workspace"]["name"], "Builder 界");
    let mut expected = serde_json::to_value(&card.meta).unwrap();
    expected["project"] = json!("flere");
    expected["pinned"] = json!(true);
    expected["last_conversation"] = expected["conversations"][0].clone();
    assert_eq!(result["workspace"]["meta"], expected);
    assert_eq!(result["workspace"]["cwd"], card.cwd);
    assert_eq!(
        dispatch_call(&f, &lead, "update_workspace", args.clone()).unwrap(),
        result
    );
    let after = f.snapshot();
    assert_eq!(identities(&after), identities(&before));
    assert_eq!((after.active, after.tab), (before.active, before.tab));
    assert_eq!(after.session().unwrap().pid, foreground.pid);
    assert_eq!(
        dispatch_call(
            &f,
            &lead,
            "worker_status",
            json!({"dispatch_id":prepared["dispatch"]["id"]})
        )
        .unwrap(),
        prepared["dispatch"]
    );
    assert!(f.capture(&shell).contains("ADMIN_%s"));
    assert!(!f.capture(&shell).contains("ADMIN_DRAFT"));
    // A combined rename and metadata update must not partially succeed on failed persistence.
    let store = f.state.join("workspaces.v2.json");
    let retained = f.state.join("retained-workspaces.json");
    let durable = fs::read(&store).unwrap();
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    assert!(dispatch_call(&f, &lead, "update_workspace", json!({"workspace":wid,"expected_epoch":before.epoch,"name":"unsaved","project":"unsaved","pinned":false})).is_err());
    let failed = f.snapshot();
    let kept = failed.workspaces.iter().find(|w| w.id == wid).unwrap();
    assert_eq!(kept.name, "Builder 界");
    let mut frontend_expected = expected.clone();
    frontend_expected
        .as_object_mut()
        .unwrap()
        .remove("last_conversation");
    assert_eq!(serde_json::to_value(&kept.meta).unwrap(), frontend_expected);
    assert_eq!(identities(&failed), identities(&before));
    assert_eq!((failed.active, failed.tab), (before.active, before.tab));
    assert_eq!(fs::read(&retained).unwrap(), durable);
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
    // Explicit clearing/unpinning preserves the omitted name and all other metadata.
    let clear = dispatch_call(
        &f,
        &lead,
        "update_workspace",
        json!({"workspace":wid,"expected_epoch":before.epoch,"project":"","pinned":false}),
    )
    .unwrap();
    assert_eq!(clear["workspace"]["name"], "Builder 界");
    expected["project"] = json!("");
    expected["pinned"] = json!(false);
    assert_eq!(clear["workspace"]["meta"], expected);
    f.send(&shell, b"\r");
    f.wait_text(&shell, "ADMIN_DRAFT");
    // Restart reads the update but launches no saved conversation or shell.
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .env("HOME", &f.root)
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let restarted = f.snapshot();
    assert!(restarted.workspaces.iter().all(|w| w.tabs.is_empty()));
    let kept = restarted.workspaces.iter().find(|w| w.id == wid).unwrap();
    assert_eq!(kept.name, "Builder 界");
    let mut frontend_expected = expected.clone();
    frontend_expected
        .as_object_mut()
        .unwrap()
        .remove("last_conversation");
    assert_eq!(serde_json::to_value(&kept.meta).unwrap(), frontend_expected);
    assert_ne!(restarted.epoch, before.epoch);
    assert!(dispatch_call(&f, &lead, "update_workspace", args).is_err());
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "coordinate",
            &lead_w.to_string(),
            "update_workspace",
            &json!({"workspace":wid,"expected_epoch":before.epoch,"name":"stale"}).to_string(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "coordinate",
            &lead_w.to_string(),
            "update_workspace",
            &json!({"workspace":wid,"expected_epoch":restarted.epoch,"pinned":true}).to_string(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut archive = kept.meta.clone();
    archive.archived = true;
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&archive).unwrap()),
    ]);
    assert!(
        wire::request(
            &f.state,
            &[
                "coordinate",
                &lead_w.to_string(),
                "update_workspace",
                &wire::hex(
                    json!({"workspace":wid,"expected_epoch":restarted.epoch,"name":"archived"})
                        .to_string()
                        .as_bytes()
                )
            ]
        )
        .is_err()
    );
}

// Build the guarded request from the same listing available to an actual Lead.
fn card_update(
    f: &Fixture,
    lead: &TabView,
    wid: u64,
    fields: serde_json::Value,
) -> serde_json::Value {
    use serde_json::json;
    let listed = dispatch_call(f, lead, "list_workspaces", json!({})).unwrap();
    let card = listed["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == wid)
        .unwrap();
    let mut args = json!({"workspace":wid,"expected_epoch":listed["epoch"],
        "expected":{"name":card["name"],"meta":card["meta"]}});
    args.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    args
}

#[test]
fn agent_reconciliation_rejects_stale_edits_and_preserves_live_state() {
    use serde_json::{Value, json};
    let mut f = Fixture::new();
    let (lead_w, lead) = dispatch_fixture(&f);
    let shell = f.new_workspace("reconciliation target");
    let wid = f.snapshot().active;
    f.send(&shell, b"printf 'RECONCILE_%s\\n' DRAFT");
    f.wait_text(&shell, "RECONCILE_%s");
    let prepared = prepare_dispatch(&f, &lead, wid, "reconcile");
    f.req(&[
        "native",
        &wid.to_string(),
        "codex",
        "01234567-89ab-cdef-0123-456789abcdef",
    ]);
    let worker = f.snapshot().session().unwrap().clone();
    f.wait_text(&worker, "DISPATCH_READY");
    let foreground = f.new_workspace("keep focus");
    let before = f.snapshot();
    let original = &before.workspaces.iter().find(|w| w.id == wid).unwrap().meta;
    let identities = |s: &Snapshot| {
        s.workspaces
            .iter()
            .flat_map(|w| {
                w.tabs.iter().map(move |t| {
                    (
                        w.id,
                        t.id,
                        t.run.clone(),
                        t.pid,
                        t.kind.clone(),
                        t.path.clone(),
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    let fields = json!({"name":"Reconciled 界","project":"flere","pinned":true,
        "status":"waiting","notes":"Deferred by user.\nPreserve candidate; no approval owed.",
        "issue":"https://example.test/issues/12","pr":"https://example.test/pull/34"});
    let args = card_update(&f, &lead, wid, fields.clone());
    let store = f.state.join("workspaces.v2.json");
    let saved = fs::read(&store).unwrap();
    for actor in [
        shell.clone(),
        TabView {
            id: worker.id,
            ..lead.clone()
        },
        TabView {
            id: u64::MAX,
            ..lead.clone()
        },
        TabView {
            run: "00000000000000000000000000000000".into(),
            ..lead.clone()
        },
    ] {
        assert!(dispatch_call(&f, &actor, "update_workspace", args.clone()).is_err());
        assert_eq!(fs::read(&store).unwrap(), saved);
    }
    // Every invalid mixed update must fail without saving its other valid fields.
    for patch in [
        json!({"expected_epoch":"stale"}),
        json!({"workspace":u64::MAX}),
        json!({"expected":null}),
        json!({"expected":{}}),
        json!({"expected":{"name":"reconciliation target","meta":{}}}),
        json!({"expected":{"name":"reconciliation target","meta":original,"extra":true}}),
        json!({"expected":{"name":"reconciliation target","meta":{"unknown":true}}}),
        json!({"expected":{"name":"reconciliation target","meta":null}}),
        json!({"status":"done"}),
        json!({"status":"deferred"}),
        json!({"status":42}),
        json!({"status":null}),
        json!({"notes":null}),
        json!({"notes":false}),
        json!({"notes":"x".repeat(65537)}),
        json!({"issue":"bad\nlink"}),
        json!({"issue":"x".repeat(2049)}),
        json!({"pr":"bad\tlink"}),
        json!({"pr":"界".repeat(683)}),
        json!({"pr":[]}),
        json!({"issue":null}),
        json!({"name":null}),
        json!({"project":null}),
        json!({"pinned":null}),
        json!({"lead":true}),
        json!({"archived":true}),
        json!({"cwd":"/"}),
        json!({"branch":"other"}),
        json!({"base_sha":"other"}),
        json!({"conversations":[]}),
        json!({"operation":"other"}),
        json!({"accepted":true}),
        json!({"unknown":true}),
    ] {
        let mut bad = args.clone();
        bad.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(
            dispatch_call(&f, &lead, "update_workspace", bad).is_err(),
            "{patch}"
        );
        assert_eq!(fs::read(&store).unwrap(), saved);
    }
    let mut missing = args.clone();
    missing.as_object_mut().unwrap().remove("expected");
    assert!(
        dispatch_call(&f, &lead, "update_workspace", missing)
            .unwrap_err()
            .to_string()
            .contains("expected")
    );
    assert!(
        dispatch_call(
            &f,
            &lead,
            "update_workspace",
            card_update(&f, &lead, wid, json!({}))
        )
        .is_err()
    );
    assert!(
        dispatch_call(
            &f,
            &lead,
            "update_workspace",
            card_update(
                &f,
                &lead,
                lead_w,
                json!({"notes":"cannot partially save","pinned":"invalid"})
            )
        )
        .is_err()
    );
    // A worker's own status update invalidates a Lead read in the SAME epoch.
    dispatch_call(&f, &worker, "set_status", json!({"status":"in-progress"})).unwrap();
    let worker_saved = fs::read(&store).unwrap();
    assert!(
        dispatch_call(&f, &lead, "update_workspace", args)
            .unwrap_err()
            .to_string()
            .contains("stale workspace metadata")
    );
    assert_eq!(fs::read(&store).unwrap(), worker_saved);
    assert_eq!(f.snapshot().epoch, before.epoch);
    let stale = card_update(&f, &lead, wid, fields.clone());
    let mut human_meta = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == wid)
        .unwrap()
        .meta
        .clone();
    human_meta.notes = "New human note".into();
    human_meta.branch = "feature/retained".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&human_meta).unwrap()),
    ]);
    let human_saved = fs::read(&store).unwrap();
    assert!(
        dispatch_call(&f, &lead, "update_workspace", stale)
            .unwrap_err()
            .to_string()
            .contains("stale workspace metadata")
    );
    assert_eq!(fs::read(&store).unwrap(), human_saved);
    let stale = card_update(&f, &lead, wid, json!({"notes":"stale rename"}));
    f.req(&["rename", &wid.to_string(), &wire::hex(b"Human rename")]);
    assert!(dispatch_call(&f, &lead, "update_workspace", stale).is_err());
    // Fresh partial update preserves the worker's status and human's other fields.
    let partial = dispatch_call(
        &f,
        &lead,
        "update_workspace",
        card_update(&f, &lead, wid, json!({"issue":"new issue"})),
    )
    .unwrap();
    assert_eq!(partial["workspace"]["meta"]["status"], "in-progress");
    assert_eq!(partial["workspace"]["meta"]["notes"], "New human note");
    assert_eq!(partial["workspace"]["name"], "Human rename");
    let args = card_update(&f, &lead, wid, fields);
    // Supported exact-run CLI handles the new fields even with an old MCP catalog.
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "agent-call",
            "update_workspace",
            &args.to_string(),
        ])
        .env("FLERE_SESSION", lead.id.to_string())
        .env("FLERE_RUN", &lead.run)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut expected = serde_json::to_value(human_meta).unwrap();
    expected["project"] = json!("flere");
    expected["pinned"] = json!(true);
    expected["last_conversation"] = expected["conversations"][0].clone();
    expected["status"] = json!("waiting");
    expected["notes"] = json!("Deferred by user.\nPreserve candidate; no approval owed.");
    expected["issue"] = json!("https://example.test/issues/12");
    expected["pr"] = json!("https://example.test/pull/34");
    assert_eq!(result["workspace"]["meta"], expected);
    assert_eq!(result["workspace"]["name"], "Reconciled 界");
    assert!(dispatch_call(&f, &lead, "update_workspace", args).is_err());
    for status in ["todo", "in-progress", "needs-me", "waiting"] {
        let updated = dispatch_call(
            &f,
            &lead,
            "update_workspace",
            card_update(&f, &lead, wid, json!({"status":status})),
        )
        .unwrap();
        expected["status"] = json!(status);
        assert_eq!(updated["workspace"]["meta"], expected);
    }
    // A fully valid mixed edit reaches persistence, fails, and rolls back all fields.
    let retained = f.state.join("retained-workspaces.json");
    let durable = fs::read(&store).unwrap();
    let fail_args = card_update(
        &f,
        &lead,
        wid,
        json!({"name":"unsaved","project":"unsaved",
        "pinned":false,"status":"todo","notes":"unsaved","issue":"unsaved","pr":"unsaved"}),
    );
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    assert!(dispatch_call(&f, &lead, "update_workspace", fail_args).is_err());
    let failed = f.snapshot();
    let kept = failed.workspaces.iter().find(|w| w.id == wid).unwrap();
    assert_eq!(kept.name, "Reconciled 界");
    let mut frontend_expected = expected.clone();
    frontend_expected
        .as_object_mut()
        .unwrap()
        .remove("last_conversation");
    assert_eq!(serde_json::to_value(&kept.meta).unwrap(), frontend_expected);
    assert_eq!(fs::read(&retained).unwrap(), durable);
    assert_eq!(identities(&failed), identities(&before));
    assert_eq!((failed.active, failed.tab), (before.active, before.tab));
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
    // Explicit clearing works; omission preserves name, status and immutable metadata.
    let cleared = dispatch_call(
        &f,
        &lead,
        "update_workspace",
        card_update(
            &f,
            &lead,
            wid,
            json!({"notes":"","issue":"","pr":"","project":""}),
        ),
    )
    .unwrap();
    for field in ["notes", "issue", "pr", "project"] {
        expected[field] = json!("");
    }
    assert_eq!(cleared["workspace"]["meta"], expected);
    // Leave a nonempty retained next action for refresh/restart proof.
    let retained_update = dispatch_call(
        &f,
        &lead,
        "update_workspace",
        card_update(
            &f,
            &lead,
            wid,
            json!({"notes":"Deferred; user opted out.","pr":"candidate-34"}),
        ),
    )
    .unwrap();
    expected["notes"] = json!("Deferred; user opted out.");
    expected["pr"] = json!("candidate-34");
    assert_eq!(retained_update["workspace"]["meta"], expected);
    let next = card_update(&f, &lead, wid, json!({"notes":"Deferred; user opted out."}));
    let pid = f.child.id();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let end = Instant::now() + Duration::from_secs(5);
    while !String::from_utf8(f.req(&["refresh-status"]))
        .unwrap()
        .contains("all terminal sessions preserved")
    {
        assert!(Instant::now() < end, "refresh timeout");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.child.id(), pid);
    let refreshed = f.snapshot();
    assert_eq!(refreshed.epoch, before.epoch);
    assert_eq!(identities(&refreshed), identities(&before));
    assert_eq!(
        (refreshed.active, refreshed.tab),
        (before.active, before.tab)
    );
    assert_eq!(refreshed.session().unwrap().pid, foreground.pid);
    assert_eq!(
        dispatch_call(&f, &lead, "update_workspace", next).unwrap(),
        retained_update
    );
    assert_eq!(
        dispatch_call(
            &f,
            &lead,
            "worker_status",
            json!({"dispatch_id":prepared["dispatch"]["id"]})
        )
        .unwrap(),
        prepared["dispatch"]
    );
    assert!(f.capture(&shell).contains("RECONCILE_%s"));
    assert!(!f.capture(&shell).contains("RECONCILE_DRAFT"));
    f.send(&shell, b"\r");
    f.wait_text(&shell, "RECONCILE_DRAFT");
    let next = card_update(&f, &lead, wid, json!({"status":"todo"}));
    f.req(&["close", &lead.id.to_string(), &lead.run]);
    let end = Instant::now() + Duration::from_secs(3);
    while f
        .snapshot()
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .any(|t| t.id == lead.id)
    {
        assert!(Instant::now() < end, "Lead fixture did not close");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(dispatch_call(&f, &lead, "update_workspace", next.clone()).is_err());
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .env("HOME", &f.root)
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let restarted = f.snapshot();
    assert!(restarted.workspaces.iter().all(|w| w.tabs.is_empty()));
    assert_ne!(restarted.epoch, before.epoch);
    let kept = restarted.workspaces.iter().find(|w| w.id == wid).unwrap();
    assert_eq!(kept.name, "Reconciled 界");
    let mut frontend_expected = expected.clone();
    frontend_expected
        .as_object_mut()
        .unwrap()
        .remove("last_conversation");
    assert_eq!(serde_json::to_value(&kept.meta).unwrap(), frontend_expected);
    // The human wrapper enforces the same epoch and Done restriction.
    assert!(
        wire::request(
            &f.state,
            &[
                "coordinate",
                &lead_w.to_string(),
                "update_workspace",
                &wire::hex(next.to_string().as_bytes())
            ]
        )
        .is_err()
    );
    let done = json!({"workspace":wid,"expected_epoch":restarted.epoch,
        "expected":{"name":kept.name,"meta":kept.meta},"status":"done"});
    assert!(
        wire::request(
            &f.state,
            &[
                "coordinate",
                &lead_w.to_string(),
                "update_workspace",
                &wire::hex(done.to_string().as_bytes())
            ]
        )
        .unwrap_err()
        .to_string()
        .contains("Done is human-controlled")
    );
    let mut archive = kept.meta.clone();
    archive.archived = true;
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&archive).unwrap()),
    ]);
    let archived = json!({"workspace":wid,"expected_epoch":restarted.epoch,"expected":{"name":kept.name,"meta":archive},"notes":"cannot edit archive"});
    assert!(
        wire::request(
            &f.state,
            &[
                "coordinate",
                &lead_w.to_string(),
                "update_workspace",
                &wire::hex(archived.to_string().as_bytes())
            ]
        )
        .is_err()
    );
}

#[test]
fn agent_reconciliation_mcp_catalog_and_calls_enforce_guard() {
    use serde_json::{Value, json};
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    f.new_workspace("MCP target");
    let wid = f.snapshot().active;
    let args = card_update(
        &f,
        &lead,
        wid,
        json!({"status":"waiting","notes":"Next: human review"}),
    );
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "mcp"])
        .env("FLERE_SESSION", lead.id.to_string())
        .env("FLERE_RUN", &lead.run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = mcp.stdin.take().unwrap();
    for r in [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"update_workspace","arguments":args}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"update_workspace","arguments":args}}),
    ] {
        writeln!(stdin, "{r}").unwrap();
    }
    drop(stdin);
    let out = mcp.wait_with_output().unwrap();
    assert!(out.status.success());
    let responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 3);
    let tool = responses[0]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "update_workspace")
        .unwrap();
    let input = &tool["inputSchema"];
    assert_eq!(input["additionalProperties"], false);
    assert_eq!(
        input["properties"]["status"]["enum"],
        json!(["todo", "in-progress", "needs-me", "waiting"])
    );
    for field in [
        "expected", "name", "project", "pinned", "status", "notes", "issue", "pr",
    ] {
        assert!(!input["properties"][field].is_null());
    }
    assert_eq!(
        input["properties"]["expected"]["required"],
        json!(["name", "meta"])
    );
    let result: Value = serde_json::from_str(
        responses[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["workspace"]["id"], wid);
    assert_eq!(result["workspace"]["meta"]["status"], "waiting");
    assert_eq!(result["workspace"]["meta"]["notes"], "Next: human review");
    assert_eq!(responses[2]["result"]["isError"], true);
    assert!(
        responses[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("stale workspace metadata")
    );
}

#[test]
fn worker_preparation_starts_one_tab_and_native_exit_returns_same_default_shell() {
    use serde_json::json;
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    let existing = f.new_workspace("existing draft");
    f.send(&existing, b"printf 'REFERENCE_%s\\n' READY\r");
    f.wait_text(&existing, "REFERENCE_READY");
    f.send(&existing, b"printf 'EXISTING_%s\\n' DRAFT");
    f.wait_text(&existing, "EXISTING_%s");
    // macOS /bin/sh is a dispatcher; its final executable may be bash/dash/zsh.
    // Compare with the already-running configured shell, not the dispatcher path.
    let expected_shell = os::process_executable(existing.pid).unwrap();
    let before = f.snapshot();
    let args = json!({"name":"worker only","cwd":f.root,"project":"Fixture project"});
    let saved = fs::read(f.state.join("workspaces.v2.json")).unwrap();
    for patch in [
        json!({"project":42}),
        json!({"project":"bad\nproject"}),
        json!({"project":"x".repeat(2049)}),
        json!({"branch":"unexpected"}),
    ] {
        let mut bad = args.clone();
        bad.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(dispatch_call(&f, &lead, "prepare_workspace", bad).is_err());
        assert_eq!(fs::read(f.state.join("workspaces.v2.json")).unwrap(), saved);
        let after = f.snapshot();
        assert_eq!(
            (after.active, after.tab, after.workspaces.len()),
            (before.active, before.tab, before.workspaces.len())
        );
    }
    assert!(dispatch_call(&f, &existing, "prepare_workspace", args.clone()).is_err());
    let created = dispatch_call(&f, &lead, "prepare_workspace", args).unwrap();
    let wid = created["workspace"].as_u64().unwrap();
    let s = f.snapshot();
    let w = s.workspaces.iter().find(|w| w.id == wid).unwrap();
    assert_eq!(w.meta.project, "Fixture project");
    let durable: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap();
    assert_eq!(
        durable["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == wid)
            .unwrap()["meta"]["project"],
        "Fixture project"
    );
    assert!(w.tabs.is_empty());
    assert_eq!(s.active, before.active);
    assert_eq!(s.tab, before.tab);
    let script = f.root.join("worker-exit.py");
    fs::write(&script,"from pathlib import Path\nimport time\nprint('SINGLE_WORKER_READY',flush=True)\nwhile not Path('exit-worker').exists(): time.sleep(.025)\n").unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/usr/bin/python3",script]}]))
            .unwrap(),
    )
    .unwrap();
    let prepared = prepare_dispatch(&f, &lead, wid, "single-worker");
    let hosted = dispatch_call(
        &f,
        &lead,
        "start_worker",
        json!({"dispatch_id":prepared["dispatch"]["id"]}),
    )
    .unwrap();
    let s = f.snapshot();
    let w = s.workspaces.iter().find(|w| w.id == wid).unwrap();
    assert_eq!(w.tabs.len(), 1);
    let t = w.tabs[0].clone();
    assert_eq!(t.id, hosted["session"].as_u64().unwrap());
    f.wait_text(&t, "SINGLE_WORKER_READY");
    fs::write(f.root.join("exit-worker"), "exit").unwrap();
    f.wait_text(&t, "Returning to");
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        let s = f.snapshot();
        let w = s.workspaces.iter().find(|w| w.id == wid).unwrap();
        assert_eq!(w.tabs.len(), 1);
        // Classification can observe the wrapper after native exit but before
        // its exec of the configured shell. Wait for both observable boundaries.
        if w.tabs[0].kind == "shell"
            && os::process_executable(t.pid).is_ok_and(|exe| exe == expected_shell)
        {
            assert_eq!(w.tabs[0].pid, t.pid);
            assert_eq!(w.tabs[0].run, t.run);
            assert_eq!(w.tabs[0].id, t.id);
            assert_eq!(os::process_executable(t.pid).unwrap(), expected_shell);
            break;
        }
        assert!(
            Instant::now() < until,
            "native did not return to shell: kind={} exe={:?} argv={:?}",
            w.tabs[0].kind,
            os::process_executable(t.pid),
            os::process_arguments(t.pid).map(|b| String::from_utf8_lossy(&b).into_owned())
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(f.snapshot().active, before.active);
    assert_eq!(f.snapshot().tab, before.tab);
    assert!(f.capture(&existing).contains("EXISTING_%s"));
    assert!(!f.capture(&existing).contains("EXISTING_DRAFT"));
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "new",
            "--name",
            "CLI prepared",
            "--cwd",
            f.root.to_str().unwrap(),
            "--no-shell",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    let human = f.new_workspace("human shell");
    assert_eq!(human.kind, "shell");
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
}

#[test]
fn worker_worktree_preparation_remains_stopped_after_git_finishes() {
    use serde_json::json;
    let f = Fixture::new();
    let (_, lead) = dispatch_fixture(&f);
    let repo = f.root.join("repo");
    fs::create_dir(&repo).unwrap();
    for args in [
        vec!["init", "-q"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "-qm",
            "fixture",
        ],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&repo)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let before = f.snapshot();
    let result = dispatch_call(&f,&lead,"prepare_workspace",json!({"name":"worker checkout","repository":repo,"branch":"worker-only","base":"HEAD","project":"Fixture"})).unwrap();
    let wid = result["workspace"].as_u64().unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let s = f.snapshot();
        let w = s.workspaces.iter().find(|w| w.id == wid).unwrap();
        assert!(w.tabs.is_empty());
        assert_eq!(s.active, before.active);
        assert_eq!(s.tab, before.tab);
        if w.meta.operation.is_empty() {
            assert_eq!(
                w.meta.base_sha,
                String::from_utf8(flere::git::output(&repo, &["rev-parse", "HEAD"]).unwrap())
                    .unwrap()
                    .trim()
            );
            assert!(PathBuf::from(&w.cwd).join(".git").is_file());
            break;
        }
        assert!(Instant::now() < until, "{}", w.meta.operation);
        std::thread::sleep(Duration::from_millis(25));
    }
    let prepared = prepare_dispatch(&f, &lead, wid, "worktree-only");
    assert_eq!(prepared["dispatch"]["phase"], "prepared");
    assert!(
        f.snapshot()
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .unwrap()
            .tabs
            .is_empty()
    );
}

#[test]
fn actual_ui_working_cards_animate_background_tabs_without_cursor_paint_flicker() {
    use serde_json::json;
    for (width, height) in [(60, 18), (180, 42)] {
        let f = Fixture::new();
        let busy = mailbox_native(&f, "Busy chat", "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        let shell = f
            .snapshot()
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .find(|t| t.kind == "shell")
            .unwrap()
            .clone();
        let idle = f.new_workspace("Idle card");
        let idle_wid = f.snapshot().active;
        for wid in [busy.wid, idle_wid] {
            let mut meta = f
                .snapshot()
                .workspaces
                .iter()
                .find(|w| w.id == wid)
                .unwrap()
                .meta
                .clone();
            meta.status = flere::workspace::Workflow::NeedsMe;
            f.req(&[
                "metadata",
                &wid.to_string(),
                &wire::hex(&serde_json::to_vec(&meta).unwrap()),
            ]);
        }
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        // Work is on an unselected card, with a native tab hidden behind its shell.
        f.req(&["focus", &busy.wid.to_string(), &shell.id.to_string()]);
        f.req(&["focus", &idle_wid.to_string(), &idle.id.to_string()]);
        mailbox_event(
            &busy,
            1,
            json!({"event":"UserPromptSubmit","screen":"compact-active"}),
        );
        let l = flere::ui::Layout::new(width as usize, height as usize);
        let working_visible = |s: &Terminal| {
            (3..l.height - 2).any(|y| {
                (1..l.left - 1).any(|x| {
                    let cell = &s.grid.cells[y * l.width + x];
                    cell.style.fg == flere::terminal::Color::Rgb(104, 172, 255)
                        && !cell.text.trim().is_empty()
                })
            })
        };
        wait_current_ui(&mut master, &mut screen, working_visible);
        let baseline = screen.grid.cells.clone();
        let cursor = (screen.grid.x, screen.grid.y);
        let raw = pump_ui_bytes(&mut master, &mut screen, 290);
        assert_ne!(screen.grid.cells, baseline, "busy card must move");
        assert_eq!((screen.grid.x, screen.grid.y), cursor);
        assert_eq!(f.snapshot().active, idle_wid);
        assert_eq!(f.snapshot().tab, idle.id);
        // Changed frames hide the cursor before paint, and expose it only at
        // the same native prompt. Byte-wise parsing also tests terminals which
        // ignore synchronized-output mode.
        let mut observed = Terminal::new(width as usize, height as usize);
        observed.feed(format!("\x1b[{};{}H", cursor.1 + 1, cursor.0 + 1).as_bytes());
        assert!(raw.starts_with(b"\x1b[?2026h\x1b[?25l"));
        for byte in raw {
            observed.feed(&[byte]);
            if observed.cursor {
                assert_eq!((observed.grid.x, observed.grid.y), cursor);
            }
        }
        assert_eq!(
            &screen.grid.cells[l.width..2 * l.width],
            &baseline[l.width..2 * l.width]
        );
        // The idle card body, enclosing borders and gap remain identical.
        let idle_row = (0..screen.grid.rows)
            .find(|y| screen.grid.line(*y).contains("Idle card"))
            .unwrap();
        for y in idle_row - 1..=idle_row + 4 {
            assert_eq!(
                &screen.grid.cells[y * l.width..y * l.width + l.left],
                &baseline[y * l.width..y * l.width + l.left]
            );
        }
        // Selecting its shell still reports the background agent in the tab strip.
        f.req(&["focus", &busy.wid.to_string(), &shell.id.to_string()]);
        wait_current_ui(&mut master, &mut screen, |_| {
            f.snapshot().active == busy.wid
        });
        pump_ui_bytes(&mut master, &mut screen, 100);
        let tabs = screen.grid.cells[2 * l.width..3 * l.width].to_vec();
        pump_ui_bytes(&mut master, &mut screen, 290);
        assert_ne!(&screen.grid.cells[2 * l.width..3 * l.width], tabs);
        // Board and reduced motion use the same observations, with no agent input.
        master.write_all(b"\0B").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 150);
        let board = screen.grid.cells.clone();
        pump_ui_bytes(&mut master, &mut screen, 290);
        assert!((l.left..l.width - l.right).any(|x| {
            (4..l.height - 2).any(|y| screen.grid.cells[y * l.width + x] != board[y * l.width + x])
        }));
        assert_eq!(
            &screen.grid.cells[2 * l.width..3 * l.width],
            &board[2 * l.width..3 * l.width]
        );
        master.write_all(b"M").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 150);
        let reduced = screen.grid.cells.clone();
        assert!(pump_ui_bytes(&mut master, &mut screen, 300).is_empty());
        assert_eq!(screen.grid.cells, reduced);
        mailbox_event(
            &busy,
            2,
            json!({"event":"PermissionRequest","screen":"approval"}),
        );
        wait_current_ui(&mut master, &mut screen, |_| {
            !f.snapshot()
                .workspaces
                .iter()
                .flat_map(|w| &w.tabs)
                .any(|t| t.working)
        });
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert!(!screen.capture(100).contains("Working"));
        assert!(!working_visible(&screen));
        let log = fs::read_to_string(f.state.join("actions.log")).unwrap();
        assert!(!log.contains("\tinput\t"));
        // Leave board/NAV before using the shared terminal-mode detach helper.
        master.write_all(b"B").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| s.cursor);
        finish_ui(&mut master, &mut screen, &mut child);
        assert!(screen.cursor);
    }
}

fn history_git(repo: &std::path::Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "user.name=History Fixture",
            "-c",
            "user.email=history@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
fn history_repo(f: &Fixture) -> PathBuf {
    let repo = f.root.join("repo");
    fs::create_dir(&repo).unwrap();
    history_git(&repo, &["init", "-q", "-b", "main"]);
    repo
}
#[test]
fn git_history_pages_merges_renames_literal_paths_and_empty_repository_are_read_only() {
    use flere::git::history;
    let f = Fixture::new();
    let repo = history_repo(&f);
    assert!(history::history(&repo, None, 0).unwrap().commits.is_empty());
    fs::create_dir(repo.join("src")).unwrap();
    fs::write(
        repo.join("src/base.txt"),
        "unchanged one\nunchanged two\nbase value\n",
    )
    .unwrap();
    fs::write(repo.join(":(glob)*.txt"), "literal base\n").unwrap();
    fs::write(repo.join("other.txt"), "other base\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(
        &repo,
        &["commit", "-qm", "ROOT-HISTORY\n\nRoot full message."],
    );
    let root = String::from_utf8(history_git(&repo, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_string();
    history_git(&repo, &["tag", "v1"]);
    history_git(&repo, &["checkout", "-qb", "feature"]);
    let renamed = "src/renamed\t界.txt";
    history_git(&repo, &["mv", "--", "src/base.txt", renamed]);
    fs::write(
        repo.join(renamed),
        "unchanged one\nunchanged two\nfeature value\n",
    )
    .unwrap();
    fs::write(repo.join(":(glob)*.txt"), "literal changed\n").unwrap();
    fs::write(repo.join("other.txt"), "other changed\n").unwrap();
    fs::write(repo.join("binary.bin"), b"\0binary\xff").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "FEATURE-HISTORY"]);
    history_git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("main.txt"), "main value\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "MAIN-HISTORY"]);
    history_git(
        &repo,
        &[
            "merge",
            "--no-ff",
            "-qm",
            "MERGE-HISTORY\n\nHOVER_FULL_MESSAGE\n\nSigned-off-by: History Fixture",
            "feature",
        ],
    );
    history_git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    let head = history_git(&repo, &["rev-parse", "HEAD"]);
    let index = fs::read(repo.join(".git/index")).unwrap();
    let h = history::history(&repo, None, 0).unwrap();
    assert_eq!(h.commits.len(), 4);
    assert_eq!(h.commits[0].subject, "MERGE-HISTORY");
    assert_eq!(h.commits[0].parents.len(), 2);
    assert!(h.commits[0].refs.contains("origin/main"));
    assert!(
        h.commits
            .iter()
            .any(|c| !c.edges.is_empty() || c.graph.contains('|'))
    );
    let d = history::details(&repo, &h.commits[0].sha).unwrap();
    assert!(d.message.contains("HOVER_FULL_MESSAGE") && d.message.contains("first parent"));
    let rename = d.files.iter().find(|c| c.path == renamed).unwrap();
    assert_eq!(rename.old_path.as_deref(), Some("src/base.txt"));
    assert!(
        history::diff(&repo, &d, Some(rename))
            .unwrap()
            .contains("+feature value")
    );
    let literal = d.files.iter().find(|c| c.path == ":(glob)*.txt").unwrap();
    let patch = history::diff(&repo, &d, Some(literal)).unwrap();
    assert!(patch.contains("+literal changed"));
    assert!(!patch.contains("+other changed"));
    assert!(
        history::diff(&repo, &d, None)
            .unwrap()
            .contains("Binary files")
    );
    let root = history::details(&repo, &root).unwrap();
    assert!(root.parent.is_none());
    assert!(
        history::diff(&repo, &root, None)
            .unwrap()
            .contains("+base value")
    );
    assert!(history::details(&repo, "--all").is_err());
    let sub = flere::git::inspect(&repo.join("src"));
    assert_eq!(sub.root, repo);
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(history_git(&repo, &["rev-parse", "HEAD"]), head);
    for n in 0..51 {
        history_git(
            &repo,
            &["commit", "--allow-empty", "-qm", &format!("PAGE-{n:02}")],
        );
    }
    let first = history::history(&repo, None, 0).unwrap();
    assert_eq!(first.commits.len(), 50);
    assert!(first.more);
    history_git(&repo, &["commit", "--allow-empty", "-qm", "NEW-AFTER-PAGE"]);
    let second = history::history(&repo, Some(&first.tips), 1).unwrap();
    assert_eq!(second.commits.len(), 5);
    assert!(!second.more);
    assert!(
        second
            .commits
            .iter()
            .all(|c| !first.commits.iter().any(|p| p.sha == c.sha))
    );
    assert!(!second.commits.iter().any(|c| c.subject == "NEW-AFTER-PAGE"));
}

#[test]
fn actual_ui_git_history_expands_hover_message_editor_diff_and_preserves_draft() {
    for width in [60, 140] {
        let f = Fixture::new();
        let repo = history_repo(&f);
        fs::write(repo.join("hello.txt"), "before value\n").unwrap();
        history_git(&repo, &["add", "."]);
        history_git(&repo, &["commit", "-qm", "ROOT-UI"]);
        fs::write(repo.join("hello.txt"), "after value\n").unwrap();
        history_git(
            &repo,
            &[
                "commit",
                "-qam",
                "COMMIT-UI\n\nHOVER_MESSAGE_UI\n\nFull commit explanation.",
            ],
        );
        history_git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        fs::write(repo.join("draft.txt"), "working file\n").unwrap();
        let index = fs::read(repo.join(".git/index")).unwrap();
        let head = history_git(&repo, &["rev-parse", "HEAD"]);
        f.req(&[
            "new",
            &wire::hex(b"Git history"),
            &wire::hex(repo.to_str().unwrap().as_bytes()),
        ]);
        let shell = f.snapshot().session().unwrap().clone();
        let wid = f.snapshot().active;
        f.send(&shell, b"printf 'HISTORY_%s\\n' DRAFT");
        f.wait_text(&shell, "HISTORY_%s");
        let mut command = outer_ui_command();
        command
            .arg("--state")
            .arg(&f.state)
            .arg("attach")
            .env("HOME", &f.root)
            .env("XDG_CACHE_HOME", f.root.join(".cache"));
        let (mut master, mut child) =
            os::spawn_command_pty(&repo, &mut command, width, 30).unwrap();
        let mut screen = Terminal::new(width as usize, 30);
        drain_pty(&mut master, &mut screen, "FLERE");
        master.write_all(b"\0g").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("COMMIT-UI")
        });
        let row_for = |s: &Terminal, needle: &str| {
            (0..s.grid.rows)
                .find(|y| s.grid.line(*y).contains(needle))
                .unwrap()
        };
        let y = row_for(&screen, "COMMIT-UI");
        let x = if width == 60 { 3 } else { 116 };
        // Hover is a no-button motion event. It opens no tab and sends no child input.
        master
            .write_all(format!("\x1b[<35;{};{}M", x + 1, y + 1).as_bytes())
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("HOVER_MESSAGE_UI")
        });
        assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
        master.write_all(b"\x1b[<35;1;1M").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 80);
        let y = row_for(&screen, "COMMIT-UI");
        master
            .write_all(
                format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes(),
            )
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("hello.txt")
        });
        master.write_all(b"?").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("Commit message") && s.capture(100).contains("HOVER_MESSAGE_UI")
        });
        master.write_all(b"q").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 80);
        let y = row_for(&screen, "hello.txt");
        master
            .write_all(
                format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes(),
            )
            .unwrap();
        let until = Instant::now() + Duration::from_secs(4);
        let editor = loop {
            pump_ui_bytes(&mut master, &mut screen, 40);
            let snapshot = f.snapshot();
            if let Some(t) = snapshot
                .workspace()
                .unwrap()
                .tabs
                .iter()
                .find(|t| t.kind == "editor")
            {
                break t.clone();
            }
            assert!(
                Instant::now() < until,
                "diff editor did not open: {}",
                screen.capture(100)
            );
        };
        assert!(
            editor
                .path
                .starts_with(f.state.join("git-diffs-v1").to_str().unwrap())
        );
        let after = fs::read_to_string(&editor.path).unwrap();
        assert_eq!(after, "after value\n");
        let pair = std::path::Path::new(&editor.path).parent().unwrap();
        let before = fs::read_dir(pair)
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-")
            })
            .unwrap();
        assert_eq!(fs::read_to_string(before).unwrap(), "before value\n");
        assert_eq!(
            fs::metadata(&editor.path).unwrap().permissions().mode() & 0o777,
            0o400
        );
        f.wait_text(&editor, if width < 100 { "befor" } else { "before value" });
        f.wait_text(&editor, "after value");
        // Re-select the same historical file; it reuses the native comparison tab.
        master.write_all(b"\0g\r").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 250);
        assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
        assert_eq!(
            f.snapshot()
                .workspace()
                .unwrap()
                .tabs
                .iter()
                .find(|t| t.kind == "editor")
                .unwrap()
                .id,
            editor.id
        );
        assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
        assert_eq!(history_git(&repo, &["rev-parse", "HEAD"]), head);
        assert!(f.capture(&shell).contains("HISTORY_%s"));
        assert!(!f.capture(&shell).contains("HISTORY_DRAFT"));
        f.req(&["focus", &wid.to_string(), &shell.id.to_string()]);
        f.send(&shell, b"\r");
        f.wait_text(&shell, "HISTORY_DRAFT");
        finish_ui(&mut master, &mut screen, &mut child);
    }
}

#[test]
fn actual_ui_git_history_pages_and_scrolled_mouse_targets_match() {
    let f = Fixture::new();
    let repo = history_repo(&f);
    for n in 0..55 {
        fs::write(repo.join("file.txt"), format!("revision {n}\n")).unwrap();
        history_git(&repo, &["add", "."]);
        history_git(&repo, &["commit", "-qm", &format!("REV-{n:02}")]);
    }
    f.req(&[
        "new",
        &wire::hex(b"Paged history"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
    ]);
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root)
        .env("XDG_CACHE_HOME", f.root.join(".cache"));
    let (mut master, mut child) = os::spawn_command_pty(&repo, &mut command, 140, 24).unwrap();
    let mut screen = Terminal::new(140, 24);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0g").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("REV-54")
    });
    master.write_all(b"G\x1b[F").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("Next 50 commits")
    });
    // After scrolling to the bottom, click a visible commit by its rendered row.
    let y = (0..screen.grid.rows)
        .find(|y| screen.grid.line(*y).contains("REV-06"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;117;{}M\x1b[<0;117;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("file.txt")
    });
    master.write_all(b"?").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("Commit message") && s.capture(100).contains("REV-06")
    });
    master.write_all(b"q\x1b[F\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("Commits 51–55") && s.capture(100).contains("REV-04")
    });
    assert!(screen.capture(100).contains("REV-04"));
    let y = (0..screen.grid.rows)
        .find(|y| screen.grid.line(*y).contains("Previous 50"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;117;{}M\x1b[<0;117;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("REV-54")
    });
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_git_history_pending_diff_cannot_open_in_another_workspace() {
    let f = Fixture::new();
    let repo = history_repo(&f);
    fs::write(repo.join("file.txt"), "original\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "DELAYED-COMMIT"]);
    f.req(&[
        "new",
        &wire::hex(b"Other card"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
    ]);
    let other = f.snapshot().session().unwrap().clone();
    let other_w = f.snapshot().active;
    let third = f.new_workspace("Third card");
    let third_w = f.snapshot().active;
    f.req(&[
        "new",
        &wire::hex(b"Git card"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
    ]);
    let original_w = f.snapshot().active;
    let wrap = f.root.join("tools");
    fs::create_dir(&wrap).unwrap();
    let script = wrap.join("git");
    fs::write(&script, "#!/usr/bin/python3\nimport os,sys,time\nfrom pathlib import Path\nif 'cat-file' in sys.argv:\n p=Path(os.environ['HISTORY_DIFF_GATE']);p.write_text('waiting')\n until=time.monotonic()+2\n while not p.with_suffix('.release').exists() and time.monotonic()<until: time.sleep(.01)\nos.execv('/usr/bin/git',['git']+sys.argv[1:])\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let gate = f.root.join("diff-gate");
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root)
        .env("XDG_CACHE_HOME", f.root.join(".cache"))
        .env(
            "PATH",
            format!("{}:{}", wrap.display(), std::env::var("PATH").unwrap()),
        )
        .env("HISTORY_DIFF_GATE", &gate);
    let (mut master, mut child) = os::spawn_command_pty(&repo, &mut command, 140, 30).unwrap();
    let mut screen = Terminal::new(140, 30);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0g").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("DELAYED-COMMIT")
    });
    let y = (0..screen.grid.rows)
        .find(|y| screen.grid.line(*y).contains("DELAYED-COMMIT"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;117;{}M\x1b[<0;117;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("file.txt")
    });
    let y = (0..screen.grid.rows)
        .find(|y| screen.grid.line(*y).contains("file.txt"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;117;{}M\x1b[<0;117;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while !gate.exists() {
        pump_ui_bytes(&mut master, &mut screen, 20);
        assert!(Instant::now() < until, "diff request did not reach gate");
    }
    f.req(&["focus", &other_w.to_string(), &other.id.to_string()]);
    wait_current_ui(&mut master, &mut screen, |s| {
        (0..s.grid.rows)
            .any(|y| s.grid.line(y).contains("Other card") && s.grid.line(y).contains('▌'))
            && s.grid.line(3).contains("main")
    });
    // Queue the new card's first page behind the blocked diff, then leave it
    // before the worker is available. Returning must retry that discarded page.
    f.req(&["focus", &third_w.to_string(), &third.id.to_string()]);
    wait_current_ui(&mut master, &mut screen, |s| {
        (0..s.grid.rows)
            .any(|y| s.grid.line(y).contains("Third card") && s.grid.line(y).contains('▌'))
    });
    fs::write(gate.with_extension("release"), "continue").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 400);
    f.req(&["focus", &other_w.to_string(), &other.id.to_string()]);
    wait_current_ui(&mut master, &mut screen, |s| {
        (0..s.grid.rows)
            .any(|y| s.grid.line(y).contains("Other card") && s.grid.line(y).contains('▌'))
            && s.capture(100).contains("DELAYED-COMMIT")
    });
    let snap = f.snapshot();
    assert_eq!(snap.active, other_w);
    assert_eq!(snap.session().unwrap().run, other.run);
    assert!(
        snap.workspaces
            .iter()
            .all(|w| w.tabs.iter().all(|t| t.kind != "editor"))
    );
    assert!(
        wire::request(
            &f.state,
            &[
                "open-epoch",
                "stale",
                &original_w.to_string(),
                &wire::hex(repo.join("file.txt").to_str().unwrap().as_bytes())
            ]
        )
        .is_err()
    );
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn git_comparison_snapshots_match_staged_unstaged_unborn_rename_and_symlink_changes() {
    use flere::git::{self, versions};
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let repo = history_repo(&f);
    let name = ":(literal)[sample].rs";
    fs::write(repo.join(name), "staged version\n").unwrap();
    history_git(&repo, &["add", "."]);
    let view = git::inspect(&repo);
    let v = versions::working(&repo, &view.changes[0]).unwrap();
    assert!(v.before.is_empty());
    assert_eq!(v.after, b"staged version\n");
    assert_eq!(v.before_label, "empty");
    fs::write(repo.join(name), "working version\n").unwrap();
    let view = git::inspect(&repo);
    let index = fs::read(repo.join(".git/index")).unwrap();
    let v = versions::working(&repo, &view.changes[0]).unwrap();
    assert_eq!(v.before, b"staged version\n");
    assert_eq!(v.after, b"working version\n");
    assert_eq!(v.before_label, "index");
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    history_git(&repo, &["commit", "-qm", "original"]);
    let sha = String::from_utf8(history_git(&repo, &["rev-parse", "HEAD"])).unwrap();
    let details = git::history::details(&repo, sha.trim()).unwrap();
    let v = versions::commit(&repo, &details, &details.files[0]).unwrap();
    assert!(v.before.is_empty());
    assert_eq!(v.after, b"staged version\n");
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "working becomes committed"]);
    history_git(&repo, &["mv", "--", name, "renamed.rs"]);
    let view = git::inspect(&repo);
    assert_eq!(view.changes[0].old_path.as_deref(), Some(name));
    let v = versions::working(&repo, &view.changes[0]).unwrap();
    assert_eq!(v.before, b"working version\n");
    assert_eq!(v.after, b"working version\n");
    history_git(&repo, &["commit", "-qm", "rename"]);
    let sha = String::from_utf8(history_git(&repo, &["rev-parse", "HEAD"])).unwrap();
    let details = git::history::details(&repo, sha.trim()).unwrap();
    let v = versions::commit(&repo, &details, &details.files[0]).unwrap();
    assert_eq!(v.before, v.after);
    fs::remove_file(repo.join("renamed.rs")).unwrap();
    let view = git::inspect(&repo);
    let v = versions::working(&repo, &view.changes[0]).unwrap();
    assert_eq!(v.before, b"working version\n");
    assert!(v.after.is_empty());
    history_git(&repo, &["add", "--", "renamed.rs"]);
    let view = git::inspect(&repo);
    let v = versions::working(&repo, &view.changes[0]).unwrap();
    assert_eq!(v.before, b"working version\n");
    assert!(v.after.is_empty());
    assert_eq!(v.after_label, "index");
    history_git(&repo, &["commit", "-qm", "delete"]);
    let sha = String::from_utf8(history_git(&repo, &["rev-parse", "HEAD"])).unwrap();
    let details = git::history::details(&repo, sha.trim()).unwrap();
    let v = versions::commit(&repo, &details, &details.files[0]).unwrap();
    assert_eq!(v.before, b"working version\n");
    assert!(v.after.is_empty());
    let secret = f.root.join("outside");
    fs::write(&secret, "outside content must not be copied\n").unwrap();
    symlink(&secret, repo.join("renamed.rs")).unwrap();
    let view = git::inspect(&repo);
    let v = versions::working(&repo, &view.changes[0]).unwrap();
    assert_eq!(v.after, secret.to_str().unwrap().as_bytes());
    assert!(
        !String::from_utf8(v.after)
            .unwrap()
            .contains("outside content")
    );
    fs::write(repo.join("untracked.txt"), "brand new\n").unwrap();
    let view = git::inspect(&repo);
    let change = view
        .changes
        .iter()
        .find(|c| c.path == "untracked.txt")
        .unwrap();
    let v = versions::working(&repo, change).unwrap();
    assert!(v.before.is_empty());
    assert_eq!(v.after, b"brand new\n");
    let mut binary = v;
    binary.after = vec![0, 1, 2];
    assert!(flere::editor::comparison(&binary).is_err());
    binary.after = vec![b'x'; 512 * 1024 + 1];
    assert!(flere::editor::comparison(&binary).is_err());
}

#[test]
fn actual_ui_git_working_comparison_uses_native_diff_and_preserves_sources() {
    for editor in [
        "vim -Nu NONE -n --noplugin",
        "nvim -u NONE -i NONE --noplugin",
    ] {
        if !flere::editor::executable(editor.split_whitespace().next().unwrap()) {
            continue;
        }
        let f = Fixture::with_editor(editor);
        let repo = history_repo(&f);
        fs::write(repo.join("sample.rs"), "fn before() {}\n").unwrap();
        history_git(&repo, &["add", "."]);
        history_git(&repo, &["commit", "-qm", "base"]);
        fs::write(repo.join("sample.rs"), "fn after() {}\n").unwrap();
        let index = fs::read(repo.join(".git/index")).unwrap();
        f.req(&[
            "new",
            &wire::hex(b"Native diff"),
            &wire::hex(repo.to_str().unwrap().as_bytes()),
        ]);
        let mut command = outer_ui_command();
        command
            .arg("--state")
            .arg(&f.state)
            .arg("attach")
            .env("HOME", &f.root)
            .env("XDG_CACHE_HOME", f.root.join(".cache"));
        let (mut master, mut child) = os::spawn_command_pty(&repo, &mut command, 236, 54).unwrap();
        let mut screen = Terminal::new(236, 54);
        drain_pty(&mut master, &mut screen, "FLERE");
        master.write_all(b"\0g").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(3).contains("main") && s.grid.line(5).contains("Working tree · 1")
        });
        master.write_all(b"\x1b[Hjjd").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            let text = s.capture(100);
            text.contains("fn before()") && text.contains("fn after()")
        });
        let snapshot = f.snapshot();
        let diff = snapshot
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .find(|t| t.kind == "editor")
            .unwrap();
        assert!(diff.title.contains("diff"));
        assert_eq!(fs::read_to_string(&diff.path).unwrap(), "fn after() {}\n");
        assert_eq!(
            fs::metadata(&diff.path).unwrap().permissions().mode() & 0o777,
            0o400
        );
        let args = os::process_arguments(diff.pid).unwrap();
        assert!(args.split(|b| *b == 0).any(|a| a == b"-d"));
        assert!(args.split(|b| *b == 0).any(|a| a == b"-M"));
        assert_eq!(
            fs::read_to_string(repo.join("sample.rs")).unwrap(),
            "fn after() {}\n"
        );
        assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
        let pair = std::path::Path::new(&diff.path).parent().unwrap();
        let before = fs::read_dir(pair)
            .unwrap()
            .map(|p| p.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("before-")
            })
            .unwrap();
        assert!(
            wire::request(
                &f.state,
                &[
                    "open-diff-epoch",
                    "stale",
                    &snapshot.active.to_string(),
                    &wire::hex(before.to_str().unwrap().as_bytes()),
                    &wire::hex(diff.path.as_bytes())
                ]
            )
            .is_err()
        );
        master.write_all(b"\0g\x1b[Hjju").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("-fn before()")
        });
        finish_ui(&mut master, &mut screen, &mut child);
    }
}

#[test]
fn first_use_intro_waits_consumes_input_and_restores_terminal_on_cancel() {
    let f = Fixture::new();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 236, 54).unwrap();
    let mut screen = Terminal::new(236, 54);
    drain_pty(&mut master, &mut screen, "Press any key to start");
    let initial = screen.grid.cells.clone();
    master
        .write_all(b"\x1b[200~PASTED_TEXT_MUST_NOT_RUN\r")
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 110);
    master.write_all(b"\x1b[201~\x1b[<0;10;4M\x1b[I").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 140);
    assert!(f.snapshot().workspaces.is_empty());
    assert_ne!(initial, screen.grid.cells);
    os::resize(master.as_raw_fd(), 60, 24).unwrap();
    screen.resize(60, 24);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(40).contains("Press any key to start")
    });
    master.write_all(b"\x03").unwrap();
    let output = pump_ui_bytes(&mut master, &mut screen, 250);
    assert!(child.wait().unwrap().success());
    assert!(
        output
            .windows(b"\x1b[?1049l".len())
            .any(|b| b == b"\x1b[?1049l")
    );
    assert!(screen.cursor);
    assert!(f.snapshot().workspaces.is_empty());
    assert!(!f.state.join("ui.json").exists());

    // Retry is still first use; the entire start-key read is consumed.
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 100, 30).unwrap();
    let mut screen = Terminal::new(100, 30);
    drain_pty(&mut master, &mut screen, "Press any key to start");
    master.write_all(b"START_KEY_MUST_NOT_RUN").unwrap();
    drain_pty(&mut master, &mut screen, "Workspace 1");
    let snap = f.snapshot();
    assert_eq!(snap.workspaces.len(), 1);
    assert_eq!(snap.workspaces.iter().flat_map(|w| &w.tabs).count(), 1);
    let shell = snap.session().unwrap();
    assert_eq!(shell.kind, "shell");
    assert!(!f.capture(shell).contains("START_KEY_MUST_NOT_RUN"));
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn standalone_intro_creates_no_state_and_honors_reduced_motion() {
    let f = Fixture::new();
    let preview_state = f.root.join("not-created");
    for reduced in [false, true] {
        if reduced {
            fs::create_dir(&preview_state).unwrap();
            fs::write(preview_state.join("ui.json"), b"{\"reduced_motion\":true}").unwrap();
        }
        let mut command = outer_ui_command();
        command.arg("--state").arg(&preview_state).arg("intro");
        let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 60, 24).unwrap();
        let mut screen = Terminal::new(60, 24);
        drain_pty(&mut master, &mut screen, "Press any key to start");
        pump_ui_bytes(&mut master, &mut screen, 60);
        let before = screen.grid.cells.clone();
        let raw = pump_ui_bytes(&mut master, &mut screen, 350);
        if reduced {
            assert!(
                before == screen.grid.cells,
                "reduced-motion screen must remain static"
            );
            assert!(raw.is_empty());
        } else {
            assert_ne!(before, screen.grid.cells);
            assert!(!preview_state.exists());
        }
        master.write_all(b"\x1b[A").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 150);
        assert!(child.wait().unwrap().success());
        assert!(!preview_state.join("control.sock").exists());
        if reduced {
            assert_eq!(fs::read_dir(&preview_state).unwrap().count(), 1);
        }
    }
}

mod remote;

#[test]
fn actual_ui_inspector_tabs_paging_and_context_preserve_native_draft() {
    for (width, height) in [(236, 46), (60, 24)] {
        let f = Fixture::new();
        let files = f.root.join("project");
        fs::create_dir(&files).unwrap();
        for i in 0..40 {
            fs::write(files.join(format!("f{i:02}.rs")), "// fixture\n").unwrap();
        }
        f.req(&[
            "new",
            &wire::hex(b"Inspector fixture with a wrapped title"),
            &wire::hex(files.to_str().unwrap().as_bytes()),
        ]);
        let before = f.snapshot();
        let tab = before.session().unwrap().clone();
        let wid = before.active;
        let meta = flere::workspace::CardMeta {
            notes: (0..30)
                .map(|i| format!("Task note {i:02}: preserve this multiline context.\n"))
                .collect(),
            project: "flere".into(),
            ..Default::default()
        };
        f.req(&[
            "metadata",
            &wid.to_string(),
            &wire::hex(&serde_json::to_vec(&meta).unwrap()),
        ]);
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&files, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        master.write_all(b"printf 'NAV_%s\\n' RETAINED\0l").unwrap();
        // Directory reads are asynchronous. End targets the loaded listing, not
        // the initial parent-only placeholder. Focus Files first so its overlay
        // is also visible when the terminal is too narrow for a right sidebar.
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("40 items")
        });
        master.write_all(b"\x1b[F").unwrap();
        let context = height as usize - 4;
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(context).contains("f39.rs")
        });
        master.write_all(b"\x1b[H\x1b[6~").unwrap();
        let page = height as usize - 10;
        let expected = format!("f{:02}.rs", page - 1);
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(context).contains(&expected)
        });
        let layout = flere::ui::Layout::new(width as usize, height as usize);
        let pane = if layout.right == 0 {
            0
        } else {
            layout.width - layout.right
        };
        // The top visible file after paging is f00, rather than the parent entry.
        master
            .write_all(format!("\x1b[<0;{};6M", pane + 6).as_bytes())
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(context).contains("f00.rs")
        });
        // The selected-file panel must not act as additional file rows.
        master
            .write_all(format!("\x1b[<0;{};{}M", pane + 6, height - 4).as_bytes())
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 80);
        assert!(screen.grid.line(context).contains("f00.rs"));
        // Click Details directly, scroll to connection info, then back to the top.
        master
            .write_all(format!("\x1b[<0;{};3M\x1b[F", pane + 16).as_bytes())
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100)
                .contains("Terminal image support unavailable")
        });
        master.write_all(b"\x1b[H").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(5).contains("WORKSPACE")
        });
        master.write_all(b"\x1b[6~").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("Task note")
        });
        // A direct Files click returns to the exact previous selection.
        master
            .write_all(format!("\x1b[<0;{};3M", pane + 3).as_bytes())
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(context).contains("f00.rs")
        });
        let current = f.snapshot();
        assert_eq!(
            (&current.epoch, current.active, current.tab),
            (&before.epoch, wid, tab.id)
        );
        assert_eq!(current.session().unwrap().pid, tab.pid);
        assert!(!f.capture(&tab).contains("NAV_RETAINED"));
        master.write_all(b"\0h\r\r").unwrap();
        wait_ui_text(&f, &tab, &mut master, &mut screen, "NAV_RETAINED");
        finish_ui(&mut master, &mut screen, &mut child);
    }
}

mod build_status;
mod cache_retention;
mod closing;
mod context_menus;
mod interactions;
mod sidebars;
mod splits;

#[test]
fn git_branch_comparison_hides_equal_reverted_and_behind_trees() {
    use flere::git::branch;
    let f = Fixture::new();
    let repo = history_repo(&f);
    assert!(branch::inspect(&repo).unwrap().is_none());
    fs::write(repo.join("file.txt"), "base\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "base"]);
    assert!(
        branch::inspect(&repo)
            .unwrap()
            .unwrap()
            .details
            .files
            .is_empty()
    );
    history_git(&repo, &["checkout", "-qb", "feature"]);
    history_git(&repo, &["config", "branch.feature.gh-merge-base", "main"]);
    history_git(
        &repo,
        &[
            "commit",
            "--allow-empty",
            "-qm",
            "different history, same tree",
        ],
    );
    assert!(
        branch::inspect(&repo)
            .unwrap()
            .unwrap()
            .details
            .files
            .is_empty()
    );
    fs::write(repo.join("file.txt"), "feature\n").unwrap();
    history_git(&repo, &["commit", "-qam", "feature change"]);
    let changed = branch::inspect(&repo).unwrap().unwrap();
    assert_eq!(changed.details.files.len(), 1);
    history_git(&repo, &["revert", "--no-edit", "HEAD"]);
    assert!(
        branch::inspect(&repo)
            .unwrap()
            .unwrap()
            .details
            .files
            .is_empty()
    );
    // An upstream-only change must not appear as something committed by feature.
    history_git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("upstream.txt"), "upstream\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "upstream"]);
    history_git(&repo, &["checkout", "-q", "feature"]);
    assert!(
        branch::inspect(&repo)
            .unwrap()
            .unwrap()
            .details
            .files
            .is_empty()
    );
    history_git(&repo, &["checkout", "-q", "--detach", "main~1"]);
    assert!(
        branch::inspect(&repo)
            .unwrap()
            .unwrap()
            .details
            .files
            .is_empty()
    );
}

#[test]
fn git_branch_comparison_resolves_targets_and_captures_literal_file_versions_read_only() {
    use flere::git::{branch, history, versions};
    let f = Fixture::new();
    let repo = history_repo(&f);
    fs::create_dir(repo.join("src")).unwrap();
    fs::write(repo.join("src/old.txt"), "one\ntwo\nthree\nfour\nfive\n").unwrap();
    fs::write(repo.join(":(glob)*.txt"), "literal base\n").unwrap();
    fs::write(repo.join("delete.txt"), "deleted\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "base"]);
    history_git(
        &repo,
        &["update-ref", "refs/remotes/upstream/trunk", "HEAD"],
    );
    history_git(
        &repo,
        &[
            "symbolic-ref",
            "refs/remotes/upstream/HEAD",
            "refs/remotes/upstream/trunk",
        ],
    );
    history_git(&repo, &["checkout", "-qb", "feature"]);
    history_git(&repo, &["config", "branch.feature.remote", "upstream"]);
    let renamed = "src/renamed\t界.txt";
    history_git(&repo, &["mv", "--", "src/old.txt", renamed]);
    history_git(&repo, &["rm", "-q", "--", "delete.txt"]);
    fs::write(repo.join(":(glob)*.txt"), "literal committed\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "branch changes"]);
    history_git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("upstream-only.txt"), "upstream\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "advance target"]);
    history_git(
        &repo,
        &["update-ref", "refs/remotes/upstream/trunk", "HEAD"],
    );
    history_git(&repo, &["checkout", "-q", "feature"]);
    fs::write(repo.join(":(glob)*.txt"), "dirty working value\n").unwrap();
    let index = fs::read(repo.join(".git/index")).unwrap();
    let head = history_git(&repo, &["rev-parse", "HEAD"]);
    let comparison = branch::inspect(&repo).unwrap().unwrap();
    assert_eq!(comparison.target, "upstream/trunk");
    assert_eq!(comparison.details.files.len(), 3);
    assert!(
        !comparison
            .details
            .files
            .iter()
            .any(|c| c.path == "upstream-only.txt")
    );
    let rename = comparison
        .details
        .files
        .iter()
        .find(|c| c.path == renamed)
        .unwrap();
    assert_eq!(rename.old_path.as_deref(), Some("src/old.txt"));
    let literal = comparison
        .details
        .files
        .iter()
        .find(|c| c.path == ":(glob)*.txt")
        .unwrap();
    let patch = history::diff(&repo, &comparison.details, Some(literal)).unwrap();
    assert!(patch.contains("+literal committed"));
    assert!(!patch.contains("dirty working value"));
    let pair = versions::commit(&repo, &comparison.details, literal).unwrap();
    assert_eq!(pair.before, b"literal base\n");
    assert_eq!(pair.after, b"literal committed\n");
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(history_git(&repo, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        fs::read(repo.join(":(glob)*.txt")).unwrap(),
        b"dirty working value\n"
    );
    // A locally configured PR target wins over the remote default.
    history_git(&repo, &["branch", "release", "HEAD"]);
    history_git(
        &repo,
        &["config", "branch.feature.gh-merge-base", "release"],
    );
    let explicit = branch::inspect(&repo).unwrap().unwrap();
    assert_eq!(explicit.target, "release");
    assert!(explicit.details.files.is_empty());
    // The earlier selection remains tied to its captured commits as refs advance.
    history_git(
        &repo,
        &["update-ref", "refs/remotes/upstream/trunk", "HEAD"],
    );
    assert_eq!(
        versions::commit(&repo, &comparison.details, literal)
            .unwrap()
            .after,
        b"literal committed\n"
    );
}

#[test]
fn git_branch_missing_and_unrelated_targets_report_an_error_without_hiding_working_files() {
    let f = Fixture::new();
    let repo = history_repo(&f);
    fs::write(repo.join("file.txt"), "base\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "base"]);
    history_git(&repo, &["checkout", "-qb", "feature"]);
    history_git(
        &repo,
        &["config", "branch.feature.gh-merge-base", "missing"],
    );
    fs::write(repo.join("file.txt"), "working\n").unwrap();
    let missing = flere::git::inspect(&repo);
    assert!(missing.comparison.is_none());
    assert!(missing.comparison_error.contains("unavailable locally"));
    assert!(missing.error.is_empty());
    assert_eq!(missing.changes.len(), 1);
    history_git(&repo, &["checkout", "-q", "--orphan", "unrelated"]);
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "other root"]);
    history_git(&repo, &["config", "branch.unrelated.gh-merge-base", "main"]);
    let unrelated = flere::git::inspect(&repo);
    assert!(unrelated.comparison_error.contains("no common ancestor"));
}

#[test]
fn git_group_previews_keep_staged_unstaged_and_untracked_files_distinct() {
    use flere::git::{self, Change};
    let f = Fixture::new();
    let repo = history_repo(&f);
    fs::write(repo.join("file.txt"), "base\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "base"]);
    fs::write(repo.join("file.txt"), "staged\n").unwrap();
    history_git(&repo, &["add", "."]);
    fs::write(repo.join("file.txt"), "working\n").unwrap();
    fs::create_dir(repo.join("new")).unwrap();
    fs::write(repo.join("new/untracked.txt"), "new file\n").unwrap();
    let index = fs::read(repo.join(".git/index")).unwrap();
    let view = git::inspect(&repo);
    assert!(
        view.changes
            .iter()
            .any(|c| c.status == "MM" && c.path == "file.txt")
    );
    let new = view
        .changes
        .iter()
        .find(|c| c.path == "new/untracked.txt")
        .unwrap();
    assert!(git::diff_change(&repo, new).unwrap().contains("new file"));
    let mut change = Change {
        status: "M ".into(),
        path: "file.txt".into(),
        old_path: None,
    };
    let staged = git::diff_change(&repo, &change).unwrap();
    assert!(staged.contains("-base") && staged.contains("+staged"));
    assert!(!staged.contains("+working"));
    let pair = git::versions::working(&repo, &change).unwrap();
    assert_eq!(pair.before, b"base\n");
    assert_eq!(pair.after, b"staged\n");
    change.status = " M".into();
    let working = git::diff_change(&repo, &change).unwrap();
    assert!(working.contains("-staged") && working.contains("+working"));
    let pair = git::versions::working(&repo, &change).unwrap();
    assert_eq!(pair.before, b"staged\n");
    assert_eq!(pair.after, b"working\n");
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
}

#[test]
fn actual_ui_git_tree_folds_and_opens_exact_staged_and_committed_versions() {
    let f = Fixture::new();
    let repo = history_repo(&f);
    fs::create_dir(repo.join("src")).unwrap();
    fs::write(repo.join("src/sample.rs"), "base\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "base"]);
    history_git(&repo, &["checkout", "-qb", "feature"]);
    fs::write(repo.join("src/branch.txt"), "committed value\n").unwrap();
    history_git(&repo, &["add", "."]);
    history_git(&repo, &["commit", "-qm", "branch change"]);
    fs::write(repo.join("src/sample.rs"), "staged\n").unwrap();
    history_git(&repo, &["add", "."]);
    fs::write(repo.join("src/sample.rs"), "working\n").unwrap();
    let index = fs::read(repo.join(".git/index")).unwrap();
    f.req(&[
        "new",
        &wire::hex(b"Git tree"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
    ]);
    let shell = f.snapshot().session().unwrap().clone();
    f.send(&shell, b"printf 'TREE_%s\\n' DRAFT");
    f.wait_text(&shell, "TREE_%s");
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root)
        .env("XDG_CACHE_HOME", f.root.join(".cache"));
    let (mut master, mut child) = os::spawn_command_pty(&repo, &mut command, 236, 54).unwrap();
    let mut screen = Terminal::new(236, 54);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0g").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("Committed on branch")
            && s.capture(100).matches("sample.rs").count() == 2
    });
    // Home -> Working tree; j -> Staged; j -> src/. Folding must retain NAV mode.
    master.write_all(b"\x1b[Hjj\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).matches("sample.rs").count() == 1 && s.grid.line(53).contains("NAV")
    });
    master.write_all(b"\rjd").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().session().is_some_and(|t| t.kind == "editor")
    });
    let staged = f.snapshot().session().unwrap().clone();
    assert_eq!(fs::read(&staged.path).unwrap(), b"staged\n");
    master.write_all(b"\0g").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("branch.txt")
    });
    let y = (0..screen.grid.rows)
        .find(|y| screen.grid.line(*y).contains("branch.txt"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;200;{}M\x1b[<0;200;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot()
            .session()
            .is_some_and(|t| t.kind == "editor" && t.id != staged.id)
    });
    let committed = f.snapshot().session().unwrap().clone();
    assert_eq!(fs::read(&committed.path).unwrap(), b"committed value\n");
    assert_eq!(fs::read(repo.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(repo.join("src/sample.rs")).unwrap(), b"working\n");
    assert!(!f.capture(&shell).contains("TREE_DRAFT"));
    history_git(&repo, &["checkout", "-q", "main"]);
    master.write_all(b"\0gF").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        let text = s.capture(100);
        text.contains("compared with main") && !text.contains("Committed on branch")
    });
    assert_eq!(fs::read(&committed.path).unwrap(), b"committed value\n");
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn local_graphics_negotiation_preview_resize_and_detach_preserve_native_draft() {
    let f = Fixture::new();
    let repo = f.root.join("images");
    fs::create_dir(&repo).unwrap();
    history_git(&repo, &["init", "-q"]);
    fs::write(
        repo.join("a.png"),
        include_bytes!("fixtures/local-image.png"),
    )
    .unwrap();
    fs::write(
        repo.join("logo.png"),
        include_bytes!("fixtures/local-image.png"),
    )
    .unwrap();
    f.req(&[
        "new",
        &wire::hex(b"Local graphics"),
        &wire::hex(repo.to_str().unwrap().as_bytes()),
    ]);
    let t = f.snapshot().session().unwrap().clone();
    f.send(&t, b"printf 'LOCAL_%s\\n' DRAFT");
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":32,"right":36,"pet":true,"reduced_motion":true}"#,
    )
    .unwrap();
    let before = f.snapshot();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&repo, &mut command, 160, 40).unwrap();
    let mut screen = Terminal::new(160, 40);
    drain_pty(&mut master, &mut screen, "DRAFT");
    let unsupported = pump_ui_bytes(&mut master, &mut screen, 80);
    assert!(!String::from_utf8_lossy(&unsupported).contains("a=t,t=d"));
    let reply = format!("\x1b_Gi={};OK\x1b\\\x1b[6;20;8t", 0x5248_0000u32);
    for b in reply.bytes() {
        master.write_all(&[b]).unwrap();
    }
    let mut output = Vec::new();
    let end = Instant::now() + Duration::from_secs(3);
    let pet_id = (0x5248_0001u32).to_string();
    let icon_id = (0x5248_0065u32).to_string();
    loop {
        output.extend(pump_ui_bytes(&mut master, &mut screen, 40));
        let text = String::from_utf8_lossy(&output);
        if text.contains(&format!("a=p,i={pet_id}")) && text.contains(&format!("a=p,i={icon_id}")) {
            break;
        }
        if Instant::now() >= end {
            fs::write(f.root.join("local-pixels-output.bin"), &output).unwrap();
            panic!("local pixels did not become ready: {}", screen.capture(100));
        }
    }
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("f=24,s=256,v=80")); // Four pixel rows in the compact dock, not the text fallback.
    assert!(text.contains("o=z,q=2"));
    assert!(!f.capture(&t).contains("OK"));
    // Expanding and collapsing changes both the pixel dimensions and placement.
    master.write_all(b"\0O").unwrap();
    let expanded = pump_ui_bytes(&mut master, &mut screen, 200);
    let expanded = String::from_utf8_lossy(&expanded);
    assert!(expanded.contains("f=24,s=256,v=120"));
    assert!(expanded.contains(&format!("a=p,i={pet_id}")));
    assert!(expanded.contains("\x1b[2J"));
    master.write_all(b"\x1b").unwrap();
    let compact = pump_ui_bytes(&mut master, &mut screen, 200);
    let compact = String::from_utf8_lossy(&compact);
    assert!(compact.contains("f=24,s=256,v=80"));
    assert!(compact.contains(&format!("a=p,i={pet_id}")));
    assert!(compact.contains("\x1b[2J"));
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    master.write_all(b"\0l\x1b[Hjjp").unwrap();
    drain_pty(&mut master, &mut screen, "IMAGE PREVIEW");
    let preview_id = (0x5248_0002u32).to_string();
    let mut preview = Vec::new();
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        preview.extend(pump_ui_bytes(&mut master, &mut screen, 40));
        if String::from_utf8_lossy(&preview).contains(&format!("a=p,i={preview_id}")) {
            break;
        }
        assert!(
            Instant::now() < end,
            "local preview never placed: {}",
            screen.capture(100)
        );
    }
    master
        .write_all(b"\x1b[200~MUST_NOT_TYPE\r\x1b[201~")
        .unwrap();
    os::resize(master.as_raw_fd(), 140, 36).unwrap();
    screen.resize(140, 36);
    let resized = pump_ui_bytes(&mut master, &mut screen, 350);
    assert!(String::from_utf8_lossy(&resized).contains(&format!("a=p,i={preview_id}")));
    master.write_all(b"q").unwrap();
    let closed = pump_ui_bytes(&mut master, &mut screen, 150);
    assert!(String::from_utf8_lossy(&closed).contains(&format!("d=I,i={preview_id}")));
    assert!(!f.capture(&t).contains("MUST_NOT_TYPE"));
    assert!(!screen.capture(100).contains("IMAGE PREVIEW"));
    master.write_all(b"\x1b").unwrap();
    // Escape's timeout begins when the UI reads it. Wait for the mode change,
    // otherwise the next Enter can combine with a still-buffered Escape.
    wait_current_ui(&mut master, &mut screen, |s| {
        let text = s.capture(100);
        text.contains("Ctrl+Space navigate")
            && !text.contains("NAV Files")
            && !text.contains("IMAGE PREVIEW")
    });
    master.write_all(b"\r").unwrap();
    // Keep consuming graphics/restore output while waiting for native input.
    // A real terminal drains its PTY; leaving it unread can block the UI writer.
    wait_ui_text(&f, &t, &mut master, &mut screen, "LOCAL_DRAFT");
    master.write_all(b"\0q").unwrap();
    let cleanup = pump_ui_bytes(&mut master, &mut screen, 150);
    assert!(String::from_utf8_lossy(&cleanup).contains(&format!("d=I,i={pet_id}")));
    assert!(child.wait().unwrap().success());
    let after = f.snapshot();
    assert_eq!(after.epoch, before.epoch);
    let current = after.session().unwrap();
    assert_eq!(
        (current.pid, &current.run, current.alive),
        (t.pid, &t.run, true)
    );
}
#[test]
fn actual_ui_compact_cards_attention_and_action_search_preserve_native_draft() {
    let f = Fixture::new();
    let mut cards = Vec::new();
    for (name, project, needs) in [
        (
            "Alpha long workspace name that must stay two rows",
            "flere",
            true,
        ),
        ("Beta review", "flere", true),
        ("Elsewhere", "other", true),
    ] {
        let t = f.new_workspace(name);
        let snap = f.snapshot();
        let mut meta = snap.workspace().unwrap().meta.clone();
        meta.project = project.into();
        meta.branch = "feat/compact".into();
        meta.status = if needs {
            flere::workspace::Workflow::NeedsMe
        } else {
            flere::workspace::Workflow::Todo
        };
        f.req(&[
            "metadata",
            &snap.active.to_string(),
            &wire::hex(&serde_json::to_vec(&meta).unwrap()),
        ]);
        cards.push((snap.active, t));
    }
    f.req(&["focus", &cards[0].0.to_string(), "0"]);
    f.send(&cards[0].1, b"echo POLISH_UNSUBMITTED");
    fs::write(f.state.join("ui.json"), br#"{"left":38,"right":40,"compact_cards":true,"project":"flere","folds":["Needs me"],"reduced_motion":true}"#).unwrap();
    let before = f.snapshot();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 35).unwrap();
    let mut screen = Terminal::new(160, 35);
    drain_pty(&mut master, &mut screen, "POLISH_UNSUBMITTED");
    assert!(screen.grid.line(0).contains("2 need me · 0 working"));
    master.write_all(b"\0J").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().active == cards[1].0 && s.capture(100).contains("Beta review")
    });
    master.write_all(b"J").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().active == cards[0].0
    });
    let alpha_y = (3..30)
        .find(|y| screen.grid.line(*y).contains("Alpha long"))
        .unwrap();
    let beta_y = (3..30)
        .find(|y| screen.grid.line(*y).contains("Beta review"))
        .unwrap();
    assert_eq!(beta_y - alpha_y, 4); // Two content rows and adjacent enclosing borders.
    assert!(screen.grid.line(alpha_y + 1).contains("feat/compact"));
    // Both lines of the second card resolve to that exact workspace.
    master
        .write_all(format!("\x1b[<0;8;{}M", beta_y + 2).as_bytes())
        .unwrap();
    // The command response can precede its rendered hint. Observe both parts of
    // the click result instead of racing the next frontend frame.
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().active == cards[1].0 && s.grid.line(34).contains("j/k select workspace")
    });
    master.write_all(b"J /git").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("Refresh files and Git") && s.capture(100).contains("Type query")
    });
    assert!(!screen.capture(100).contains("New terminal"));
    // A pasted command is just query text, including its newline; it cannot detach/submit.
    master.write_all(b"\x1b[200~q\r\x1b[201~").unwrap();
    drain_pty(&mut master, &mut screen, "No matching actions");
    assert!(child.try_wait().unwrap().is_none());
    master.write_all(b"\x15changes\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.capture(100).contains("FLERE ACTIONS") && s.grid.line(34).contains("NAV")
    });
    let pref: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(pref["inspector"], "Git");
    // Clearing the query and closing the menu are separate Escape steps.
    master.write_all(b" /not-a-command").unwrap();
    drain_pty(&mut master, &mut screen, "No matching actions");
    master.write_all(b"\x1b").unwrap();
    drain_pty(&mut master, &mut screen, "Press / to search");
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.capture(100).contains("FLERE ACTIONS")
    });
    master.write_all(b" C").unwrap(); // Direct shortcuts still work in Actions.
    wait_current_ui(&mut master, &mut screen, |s| {
        let pref: serde_json::Value =
            serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
        !s.capture(100).contains("FLERE ACTIONS") && pref["compact_cards"] == false
    });
    let pref: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(pref["compact_cards"], false);
    master.write_all(b"C").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    let after = f.snapshot();
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.active, cards[0].0);
    assert_eq!(
        (after.session().unwrap().pid, &after.session().unwrap().run),
        (cards[0].1.pid, &cards[0].1.run)
    );
    assert!(f.capture(&cards[0].1).contains("POLISH_UNSUBMITTED"));
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.grid.line(34).trim_start().starts_with("NAV")
    });
    master.write_all(b"_STILL_NATIVE").unwrap();
    drain_pty(&mut master, &mut screen, "POLISH_UNSUBMITTED_STILL_NATIVE");
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn card_exact_focus_and_notes_cas_preserve_other_metadata_and_native_identity() {
    let f = Fixture::new();
    let first = f.new_workspace("First");
    let wid = f.snapshot().active;
    f.req(&["tab", &wid.to_string()]);
    let second = f.snapshot().session().unwrap().clone();
    let other = f.new_workspace("Other");
    let other_wid = f.snapshot().active;
    let before = f.snapshot();
    for (epoch, id, tab, run) in [
        ("old-epoch", wid, first.id, first.run.as_str()),
        (before.epoch.as_str(), wid, first.id, "wrong-run"),
        (
            before.epoch.as_str(),
            other_wid,
            first.id,
            first.run.as_str(),
        ),
        (before.epoch.as_str(), wid, 0, "not-empty"),
    ] {
        assert!(
            wire::request(
                &f.state,
                &["focus-exact", epoch, &id.to_string(), &tab.to_string(), run]
            )
            .is_err()
        );
        assert_eq!(f.snapshot().tab, other.id);
    }
    f.req(&[
        "focus-exact",
        &before.epoch,
        &wid.to_string(),
        &second.id.to_string(),
        &second.run,
    ]);
    assert_eq!(f.snapshot().tab, second.id);
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.issue = "https://github.com/demo/fixture/issues/39".into();
    meta.status = flere::workspace::Workflow::NeedsMe;
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    f.req(&[
        "notes",
        &before.epoch,
        &wid.to_string(),
        "",
        &wire::hex("Unicode 界 notes\nsecond line".as_bytes()),
    ]);
    let after = f.snapshot();
    let saved = &after.workspace().unwrap().meta;
    assert_eq!(saved.issue, meta.issue);
    assert_eq!(saved.status, meta.status);
    assert_eq!(saved.notes, "Unicode 界 notes\nsecond line");
    for epoch in [&before.epoch, "old"] {
        assert!(
            wire::request(
                &f.state,
                &[
                    "notes",
                    epoch,
                    &wid.to_string(),
                    "",
                    &wire::hex(b"overwrite")
                ]
            )
            .is_err()
        );
    }
    assert_eq!(f.snapshot().workspace().unwrap().meta.notes, saved.notes);
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.session().unwrap().pid, second.pid);
    assert_eq!(after.session().unwrap().run, second.run);
}

#[test]
fn actual_ui_terminal_tree_and_notes_overlay_preserve_drafts_and_exact_focus() {
    let f = Fixture::new();
    let first = f.new_workspace("Tree fixture");
    let wid = f.snapshot().active;
    f.req(&["tab", &wid.to_string()]);
    let second = f.snapshot().session().unwrap().clone();
    f.send(&second, b"echo TREE_UNSUBMITTED");
    f.wait_text(&second, "TREE_UNSUBMITTED");
    f.req(&["focus", &wid.to_string(), &first.id.to_string()]);
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":44,"compact_cards":true,"reduced_motion":true}"#,
    )
    .unwrap();
    let before = f.snapshot();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 35).unwrap();
    let mut screen = Terminal::new(160, 35);
    drain_pty(&mut master, &mut screen, "Tree fixture");
    master.write_all(b"\0h\tj").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("running") && s.grid.line(34).contains("NAV")
    });
    assert_eq!(f.snapshot().tab, first.id); // Merely navigating children does not activate them.
    master.write_all(b"\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().tab == second.id && s.capture(100).contains("TREE_UNSUBMITTED")
    });
    master.write_all(b"\0h?").unwrap();
    drain_pty(&mut master, &mut screen, "Details · Tree fixture");
    master
        .write_all(b"\x1b[200~NO_NATIVE_SUBMISSION\r\x1b[201~")
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(!f.capture(&second).contains("NO_NATIVE"));
    master
        .write_all("e\x1b[200~Notes 界\nsecond line\x1b[201~".as_bytes())
        .unwrap();
    drain_pty(&mut master, &mut screen, "second line");
    // Concurrent workflow writes survive a notes-only save.
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.project = "updated project".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    master.write_all(b"\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().meta.notes == "Notes 界\nsecond line"
    });
    assert_eq!(
        f.snapshot().workspace().unwrap().meta.project,
        "updated project"
    );
    master.write_all(b"e\x15my conflicting edit").unwrap();
    drain_pty(&mut master, &mut screen, "my conflicting edit");
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.notes = "external newer notes".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    master.write_all(b"\r").unwrap();
    drain_pty(&mut master, &mut screen, "Notes changed elsewhere");
    assert_eq!(
        f.snapshot().workspace().unwrap().meta.notes,
        "external newer notes"
    );
    master.write_all(b"\x19").unwrap();
    let raw = pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(raw.windows(7).any(|b| b == b"\x1b]52;c;"));
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    master.write_all(b"\th").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80); // Explicit child mode returns to its card.
    let (_, title_y) = hover_tooltips::locate(&screen, "Tree fixture");
    master
        .write_all(format!("\x1b[<0;3;{}M\x1b[<0;3;{}m", title_y + 1, title_y + 1).as_bytes())
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80); // Disclosure still collapses the tree.
    let pref: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(pref["expanded_cards"], serde_json::json!([]));
    master.write_all(b"\r_STILL_NATIVE").unwrap();
    drain_pty(&mut master, &mut screen, "TREE_UNSUBMITTED_STILL_NATIVE");
    let after = f.snapshot();
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.session().unwrap().pid, second.pid);
    assert_eq!(after.session().unwrap().run, second.run);
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_hover_is_passive_fetches_only_on_focus_and_invalidates_changed_links() {
    let f = Fixture::new();
    let t = f.new_workspace("Hover fixture");
    let wid = f.snapshot().active;
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.pr = "https://github.com/demo/fixture/pull/193".into();
    meta.notes = "A deliberate hover preview".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    let bin = f.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let log = f.root.join("gh-calls");
    fs::write(bin.join("gh"),format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nprintf '%s' '{{\"title\":\"Fixture PR title\",\"state\":\"OPEN\",\"reviewDecision\":\"APPROVED\",\"statusCheckRollup\":[{{\"conclusion\":\"SUCCESS\"}}]}}'\n",log.display())).unwrap();
    fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":44,"compact_cards":true,"reduced_motion":true}"#,
    )
    .unwrap();
    f.send(&t, b"echo HOVER_DRAFT");
    f.wait_text(&t, "HOVER_DRAFT");
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()));
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 35).unwrap();
    let mut screen = Terminal::new(160, 35);
    drain_pty(&mut master, &mut screen, "Hover fixture");
    let y = (3..25)
        .find(|y| screen.grid.line(*y).contains("Hover fixture"))
        .unwrap();
    master
        .write_all(format!("\x1b[<35;10;{}M", y + 1).as_bytes())
        .unwrap();
    drain_pty(&mut master, &mut screen, "Preview · click to focus");
    assert!(!log.exists());
    assert_eq!(f.snapshot().tab, t.id);
    master.write_all(b"_TYPED").unwrap();
    drain_pty(&mut master, &mut screen, "HOVER_DRAFT_TYPED");
    assert!(!log.exists());
    master
        .write_all(format!("\x1b[<35;10;{}M", y + 1).as_bytes())
        .unwrap();
    drain_pty(&mut master, &mut screen, "Preview · click to focus");
    let (hover_x, hover_y) = hover_tooltips::locate(&screen, "Details · Hover fixture");
    master
        .write_all(format!("\x1b[<35;{};{}M", hover_x + 1, hover_y + 1).as_bytes())
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 300);
    assert!(screen.capture(100).contains("Preview · click to focus")); // Pointer travels into popup.
    master
        .write_all(
            format!(
                "\x1b[<0;{};{}M\x1b[<0;{};{}m",
                hover_x + 1,
                hover_y + 1,
                hover_x + 1,
                hover_y + 1
            )
            .as_bytes(),
        )
        .unwrap();
    drain_pty(&mut master, &mut screen, "Fixture PR title");
    let calls = fs::read_to_string(&log).unwrap();
    assert!(calls.starts_with("pr\nview\nhttps://github.com/demo/fixture/pull/193\n--json\n"));
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    master.write_all(b"\0h?").unwrap();
    drain_pty(&mut master, &mut screen, "Fixture PR title");
    assert_eq!(fs::read_to_string(&log).unwrap(), calls);
    // An outside click dismisses without switching the native pane or sending input.
    master.write_all(b"\x1b[<0;55;12M\x1b[<0;55;12m").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(!screen.capture(100).contains("Details · Hover fixture"));
    assert!(screen.grid.line(34).contains("NAV"));
    master.write_all(b"?").unwrap();
    drain_pty(&mut master, &mut screen, "Fixture PR title");
    meta.pr = "https://github.com/demo/fixture/pull/194".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    drain_pty(&mut master, &mut screen, "Workspace details changed");
    assert!(!screen.capture(100).contains("Details · Hover fixture"));
    master.write_all(b"\r_MORE").unwrap();
    drain_pty(&mut master, &mut screen, "HOVER_DRAFT_TYPED_MORE");
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_tree_overflow_mouse_actions_and_narrow_details_are_reachable() {
    let f = Fixture::new();
    let first = f.new_workspace("界 Tree overflow long name");
    let wid = f.snapshot().active;
    for _ in 0..18 {
        f.req(&["tab", &wid.to_string()]);
    }
    let last = f.snapshot().session().unwrap().clone();
    f.send(&last, b"printf 'OVERFLOW_%s\\n' DRAFT");
    f.wait_text(&last, "DRAFT");
    f.req(&["focus", &wid.to_string(), &first.id.to_string()]);
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":44,"compact_cards":true,"reduced_motion":true}"#,
    )
    .unwrap();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 20).unwrap();
    let mut screen = Terminal::new(160, 20);
    drain_pty(&mut master, &mut screen, "Tree overflow");
    let y = (3..15)
        .find(|y| screen.grid.line(*y).contains("Tree overflow"))
        .unwrap();
    master
        .write_all(format!("\x1b[<0;3;{}M\x1b[<0;3;{}m", y + 1, y + 1).as_bytes())
        .unwrap();
    drain_pty(&mut master, &mut screen, "running");
    master.write_all(b"\t").unwrap();
    master.write_all(&[b'j'; 18]).unwrap();
    pump_ui_bytes(&mut master, &mut screen, 150);
    let selected = (3..18)
        .find(|y| {
            screen.grid.cells[*y * screen.grid.cols + 2].text == ">"
                && screen.grid.line(*y).contains("└─")
        })
        .expect("last terminal stays on screen");
    assert_eq!(f.snapshot().tab, first.id);
    master
        .write_all(format!("\x1b[<0;12;{}M\x1b[<0;12;{}m", selected + 1, selected + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        f.snapshot().tab == last.id && s.capture(100).contains("OVERFLOW_%s")
    });
    master.write_all(b"\0h?").unwrap();
    drain_pty(&mut master, &mut screen, "Copy path");
    // Mouse actions require matching press/release; a drag or an unmatched release cannot copy.
    master.write_all(b"\x1b[<0;121;17m").unwrap();
    let raw = pump_ui_bytes(&mut master, &mut screen, 80);
    assert!(!raw.windows(7).any(|b| b == b"\x1b]52;c;"));
    master
        .write_all(b"\x1b[<0;121;17M\x1b[<32;122;17M\x1b[<0;122;17m")
        .unwrap();
    let raw = pump_ui_bytes(&mut master, &mut screen, 80);
    assert!(!raw.windows(7).any(|b| b == b"\x1b]52;c;"));
    master.write_all(b"\x1b[<0;121;17M\x1b[<0;121;17m").unwrap();
    let raw = pump_ui_bytes(&mut master, &mut screen, 80);
    assert!(raw.windows(7).any(|b| b == b"\x1b]52;c;"));
    // Resize while pinned, then reach every action in a tiny terminal.
    os::resize(master.as_raw_fd(), 20, 8).unwrap();
    screen.resize(20, 8);
    pump_ui_bytes(&mut master, &mut screen, 200);
    assert!(screen.capture(100).contains("Copy path"));
    master.write_all(b"\t").unwrap();
    drain_pty(&mut master, &mut screen, "Edit notes");
    master.write_all(b"j").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    assert!(
        screen.grid.line(3).chars().any(|c| c.is_alphanumeric()),
        "tiny details keep at least one readable body row"
    );
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    os::resize(master.as_raw_fd(), 160, 20).unwrap();
    screen.resize(160, 20);
    pump_ui_bytes(&mut master, &mut screen, 150);
    master.write_all(b"\r_STILL_NATIVE").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    // Linux /bin/sh can use kernel echo and cannot redraw its pending line after
    // a narrow resize. Verify the shell's actual retained bytes by submitting
    // this harmless fixture only after confirming NAV did not execute it.
    assert!(!f.capture(&last).contains("OVERFLOW_DRAFT_STILL_NATIVE"));
    master.write_all(b"\r").unwrap();
    drain_pty(&mut master, &mut screen, "OVERFLOW_DRAFT_STILL_NATIVE");
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_stopped_details_keep_notes_without_dead_links_and_close_on_archive() {
    let f = Fixture::new();
    let t = f.new_workspace("Stopped fixture");
    let wid = f.snapshot().active;
    f.req(&["close", &t.id.to_string(), &t.run]);
    let until = Instant::now() + Duration::from_secs(3);
    while !f.snapshot().workspace().unwrap().tabs.is_empty() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.notes = "Long 界 notes\n".repeat(300);
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 60, 24).unwrap();
    let mut screen = Terminal::new(60, 24);
    drain_pty(&mut master, &mut screen, "Stopped fixtu…");
    master.write_all(b"\0h?").unwrap();
    drain_pty(&mut master, &mut screen, "No terminals · stopped");
    assert!(screen.capture(100).contains("Details · Stopped fixture"));
    assert!(
        !screen.capture(100).contains("Open issue") && !screen.capture(100).contains("Open PR")
    );
    master.write_all(b"\x1b[6~").unwrap();
    drain_pty(&mut master, &mut screen, "Long 界 notes");
    master.write_all(b"e\x15KEEP_UNSAVED").unwrap();
    drain_pty(&mut master, &mut screen, "KEEP_UNSAVED");
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    meta.archived = true;
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    drain_pty(&mut master, &mut screen, "Workspace details changed");
    assert!(!screen.capture(100).contains("Details · Stopped"));
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 80);
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_missing_directory_is_visible_and_retry_preserves_sessions() {
    let f = Fixture::new();
    let t = f.new_workspace("Existing shell");
    f.send(&t, b"echo FOLDER_DRAFT_UNSUBMITTED");
    let folder = f.root.join("temporarily-unavailable");
    fs::create_dir(&folder).unwrap();
    f.req(&[
        "new-stopped",
        &wire::hex(b"Unavailable folder"),
        &wire::hex(folder.to_str().unwrap().as_bytes()),
    ]);
    fs::remove_dir(&folder).unwrap();
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":47,"pet":false,"reduced_motion":true}"#,
    )
    .unwrap();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 36).unwrap();
    let mut screen = Terminal::new(160, 36);
    drain_pty(&mut master, &mut screen, "Directory no longer exists");
    assert!(screen.capture(100).contains("Workspace is stopped."));
    assert!(child.try_wait().unwrap().is_none());
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("recovered-file.txt"), b"fixture").unwrap();
    master.write_all(b"\0lr").unwrap();
    drain_pty(&mut master, &mut screen, "recovered-file.txt");
    assert!(!screen.capture(100).contains("Folder unavailable"));
    let after = f.snapshot();
    assert!(after.workspace().unwrap().tabs.is_empty());
    let same = after
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .find(|tab| tab.id == t.id)
        .unwrap();
    assert_eq!(
        (same.pid, same.run.as_str(), same.alive),
        (t.pid, t.run.as_str(), true)
    );
    assert!(f.capture(&t).contains("FOLDER_DRAFT_UNSUBMITTED"));
    finish_ui(&mut master, &mut screen, &mut child);
}

mod projects;
mod reopen;

#[test]
fn ordinary_agents_share_coordination_tools_and_can_delegate_without_pinning() {
    use serde_json::json;
    let f = Fixture::new();
    let (a, first) = dispatch_fixture(&f);
    let (b, second) = dispatch_fixture(&f);
    let ca = dispatch_call(&f, &first, "context", json!({})).unwrap();
    let cb = dispatch_call(&f, &second, "context", json!({})).unwrap();
    assert_eq!(ca["workspaces"], cb["workspaces"]);
    assert_eq!(ca["workspaces"].as_array().unwrap().len(), 2);
    assert_eq!(ca["coordination_workflow"], cb["coordination_workflow"]);
    assert!(
        ca["coordination_workflow"]
            .as_str()
            .unwrap()
            .contains("prepare_worker")
    );
    assert!(ca.get("lead_workflow").is_none());
    assert!(
        f.snapshot()
            .workspaces
            .iter()
            .all(|w| !w.meta.pinned && !w.meta.legacy_lead)
    );
    let prepared = dispatch_call(
        &f,
        &second,
        "prepare_workspace",
        json!({"name":"Delegated work","cwd":f.root}),
    )
    .unwrap();
    let target = prepared["workspace"].as_u64().unwrap();
    let assignment = prepare_dispatch(&f, &second, target, "any-agent");
    let id = assignment["dispatch"]["id"].as_str().unwrap();
    assert!(dispatch_call(&f, &first, "start_worker", json!({"dispatch_id":id})).is_err());
    dispatch_call(&f, &second, "start_worker", json!({"dispatch_id":id})).unwrap();
    let spawned = f
        .snapshot()
        .workspaces
        .iter()
        .find(|w| w.id == target)
        .unwrap()
        .tabs[0]
        .clone();
    f.wait_text(&spawned, "DISPATCH_READY");
    // Being started through delegation does not turn this agent into a restricted role.
    let sub = dispatch_call(
        &f,
        &spawned,
        "prepare_workspace",
        json!({"name":"Further work","cwd":f.root}),
    )
    .unwrap();
    let sub_id = sub["workspace"].as_u64().unwrap();
    let assignment = prepare_dispatch(&f, &spawned, sub_id, "peer-delegates");
    assert_eq!(assignment["dispatch"]["phase"], "prepared");
    let message = dispatch_call(
        &f,
        &second,
        "send_message",
        json!({"to":a,"body":"Coordinate this","intent":"quiet"}),
    )
    .unwrap();
    let mid = message["message"]["id"].as_str().unwrap();
    for actor in [&first, &second] {
        assert!(dispatch_call(&f, actor, "message_status", json!({"id":mid})).is_ok());
        assert!(
            dispatch_call(
                &f,
                actor,
                "deliver_message",
                json!({"id":mid,"session":first.id,"run":first.run})
            )
            .is_ok()
        );
    }
    assert!(dispatch_call(&f, &spawned, "message_status", json!({"id":mid})).is_err());
    assert!(dispatch_call(&f, &second, "inbox", json!({"ack_ids":[mid]})).is_err());
    assert!(dispatch_call(&f, &first, "inbox", json!({"ack_ids":[mid]})).is_ok());
    assert!(
        dispatch_call(
            &f,
            &second,
            "send_message",
            json!({"to":"lead","body":"No inferred recipient"})
        )
        .is_err()
    );
    assert!(
        dispatch_call(
            &f,
            &second,
            "show_workspace",
            json!({"workspace":a,"user_requested":false,"reason":"Not requested"})
        )
        .is_err()
    );
    dispatch_call(
        &f,
        &second,
        "show_workspace",
        json!({"workspace":a,"user_requested":true,"reason":"User asked to see this workspace"}),
    )
    .unwrap();
    assert_eq!(f.snapshot().active, a);
    for actor in [&first, &second, &spawned] {
        assert!(dispatch_call(&f, actor, "set_status", json!({"status":"done"})).is_err());
    }
    assert!(f.snapshot().workspaces.iter().any(|w| w.id == b));
}
#[path = "arcade/mod.rs"]
mod arcade;
