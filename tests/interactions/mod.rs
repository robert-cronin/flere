use super::*;
fn write_and_wait(master: &mut fs::File, screen: &mut Terminal, bytes: &[u8]) {
    write_ui_bytes(master, screen, bytes);
    pump_ui_bytes(master, screen, 100);
}
fn click(x: usize, y: usize) -> String {
    format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1)
}
fn find(screen: &Terminal, text: &str) -> (usize, usize) {
    for y in 0..screen.grid.rows {
        if let Some(at) = screen.grid.line(y).find(text) {
            return (
                screen.grid.line(y)[..at]
                    .chars()
                    .map(|c| flere::terminal::char_width(c) as usize)
                    .sum(),
                y,
            );
        }
    }
    panic!("missing {text}: {}", screen.capture(100));
}
fn finish(master: &mut fs::File, screen: &mut Terminal, child: &mut os::Process) {
    write_and_wait(master, screen, b"\x1b");
    let keys = if screen
        .grid
        .line(screen.grid.rows - 1)
        .trim_start()
        .starts_with("NAV")
    {
        b"q".as_slice()
    } else {
        b"\0q".as_slice()
    };
    master.write_all(keys).unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() {
        pump_ui_bytes(master, screen, 20);
        assert!(Instant::now() < until);
    }
}
#[test]
fn screenshot_action_is_searchable_and_cannot_write_a_remote_hosts_clipboard() {
    let f = Fixture::new();
    let t = f.new_workspace("screenshot");
    f.send(&t, b"UNSUBMITTED_SCREENSHOT_DRAFT");
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("SSH_CONNECTION", "fixture");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 120, 32).unwrap();
    let mut screen = Terminal::new(120, 32);
    drain_pty(&mut master, &mut screen, "FLERE");
    write_and_wait(&mut master, &mut screen, b"\0 /screenshot");
    assert!(screen.capture(100).contains("Copy Flere screenshot"));
    write_and_wait(&mut master, &mut screen, b"\r");
    assert!(
        screen
            .capture(100)
            .contains("Flere screenshots need a local clipboard")
    );
    assert!(f.capture(&t).contains("UNSUBMITTED_SCREENSHOT_DRAFT"));
    let log = fs::read_to_string(f.state.join("actions.log")).unwrap();
    assert_eq!(log.lines().filter(|l| l.contains("\tinput\t")).count(), 1);
    finish(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_close_dialog_buttons_default_cancel_drag_and_exact_target() {
    for (width, height) in [(180, 42), (60, 18), (20, 8)] {
        let f = Fixture::new();
        let first = f.new_workspace("first");
        let wid = f.snapshot().active;
        let second = f.new_workspace("second");
        let other = f.snapshot().active;
        f.req(&["focus", &wid.to_string(), &first.id.to_string()]);
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        write_and_wait(&mut master, &mut screen, b"\0x");
        let (x, y) = find(&screen, "Cancel");
        // Paste, stray clicks, and release without a press cannot close the target.
        write_and_wait(&mut master, &mut screen, b"\x1b[200~y\r\x1b[201~");
        write_and_wait(
            &mut master,
            &mut screen,
            format!("\x1b[<0;{};{}m", x + 1, y + 1).as_bytes(),
        );
        assert!(screen.capture(100).contains("Cancel"));
        write_and_wait(&mut master, &mut screen, b"\r");
        assert!(!screen.capture(100).contains("Cancel"));
        assert!(f.snapshot().session().unwrap().alive);
        write_and_wait(&mut master, &mut screen, b"x");
        let (x, y) = find(&screen, "Cancel");
        write_and_wait(&mut master, &mut screen, click(x, y).as_bytes());
        assert!(!screen.capture(100).contains("Cancel"));
        write_and_wait(&mut master, &mut screen, b"x");
        let (_, cancel_y) = find(&screen, "Cancel");
        let y = cancel_y;
        let line = screen.grid.line(y);
        let at = line.find("Close").unwrap();
        let x = line[..at]
            .chars()
            .map(|c| flere::terminal::char_width(c) as usize)
            .sum::<usize>();
        write_and_wait(
            &mut master,
            &mut screen,
            format!(
                "\x1b[<0;{};{}M\x1b[<32;1;1M\x1b[<0;{};{}m",
                x + 1,
                y + 1,
                x + 1,
                y + 1
            )
            .as_bytes(),
        );
        assert!(screen.capture(100).contains("Cancel"));
        // A dialog from the first workspace must never close a newly focused one.
        f.req(&["focus", &other.to_string(), &second.id.to_string()]);
        pump_ui_bytes(&mut master, &mut screen, 100);
        write_and_wait(&mut master, &mut screen, b"y");
        assert!(f.snapshot().session().unwrap().alive);
        f.req(&["focus", &wid.to_string(), &first.id.to_string()]);
        pump_ui_bytes(&mut master, &mut screen, 100);
        write_and_wait(&mut master, &mut screen, b"x");
        let (_, y) = find(&screen, "Cancel");
        write_and_wait(&mut master, &mut screen, click(x, y).as_bytes());
        wait_current_ui(&mut master, &mut screen, |_| {
            f.snapshot()
                .workspace()
                .unwrap()
                .tabs
                .iter()
                .all(|t| t.id != first.id || !t.alive)
        });
        let snapshot = f.snapshot();
        assert!(
            snapshot
                .workspaces
                .iter()
                .find(|w| w.id == other)
                .unwrap()
                .tabs
                .iter()
                .any(|t| t.id == second.id
                    && t.alive
                    && t.pid == second.pid
                    && t.run == second.run)
        );
        finish(&mut master, &mut screen, &mut child);
    }
}
#[test]
fn actual_ui_drag_selection_scrolls_both_ways_and_copies_frozen_history() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    for (width, height) in [(60, 18), (180, 42)] {
        let f = Fixture::new();
        let t = f.new_workspace("edge selection");
        fs::write(
            f.root.join("scroll_probe.py"),
            include_str!("../fixtures/scroll_probe.py"),
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
        drain_pty(&mut master, &mut screen, "DRAFT_UNSUBMITTED");
        let l = flere::ui::Layout::new(width as usize, height as usize);
        let audit = fs::read(f.state.join("actions.log")).unwrap();
        let start = (l.terminal_y + 2) * width as usize + l.terminal_x;
        let anchor = screen.grid.cells[start..start + 8]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>();
        master
            .write_all(
                format!(
                    "\x1b[<0;{};{}M\x1b[<32;{};1M",
                    l.terminal_x + 9,
                    l.terminal_y + 3,
                    l.terminal_x + 1
                )
                .as_bytes(),
            )
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 900);
        assert!(screen.capture(100).contains("Selecting text"));
        assert_ne!(
            screen.grid.cells[start..start + 8]
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>(),
            anchor
        );
        let top = screen.grid.line(l.terminal_y);
        fs::write(f.root.join("more"), b"go").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 400);
        assert_ne!(
            screen.grid.line(l.terminal_y),
            top,
            "stationary edge continues scrolling"
        );
        master
            .write_all(format!("\x1b[<0;{};1m", l.terminal_x + 1).as_bytes())
            .unwrap();
        let raw = pump_ui_bytes(&mut master, &mut screen, 150);
        let prefix = b"\x1b]52;c;";
        let start = raw.windows(prefix.len()).position(|w| w == prefix).unwrap() + prefix.len();
        let end = start + raw[start..].iter().position(|b| *b == 7).unwrap();
        let copied = String::from_utf8(STANDARD.decode(&raw[start..end]).unwrap()).unwrap();
        assert!(copied.contains(&anchor), "{anchor}: {copied}");
        assert!(copied.lines().count() > l.rows);
        assert!(!copied.contains("CHAT-094"));
        let top = screen.grid.line(l.terminal_y);
        pump_ui_bytes(&mut master, &mut screen, 300);
        assert_eq!(screen.grid.line(l.terminal_y), top);
        // Start in history and hold bottom edge; lower frozen pages remain selectable.
        write_and_wait(&mut master, &mut screen, b"\x1b");
        master
            .write_all(
                format!(
                    "\x1b[<0;{};{}M\x1b[<32;{};{}M",
                    l.terminal_x + 1,
                    l.terminal_y + 1,
                    l.terminal_x + 9,
                    l.height
                )
                .as_bytes(),
            )
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 700);
        assert_ne!(screen.grid.line(l.terminal_y), top);
        // Reversing the pointer traverses cached rows without native scroll keys.
        master
            .write_all(format!("\x1b[<32;{};1M", l.terminal_x + 1).as_bytes())
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 700);
        write_and_wait(&mut master, &mut screen, b"\x1b");
        let stopped = screen.grid.line(l.terminal_y);
        pump_ui_bytes(&mut master, &mut screen, 300);
        assert_eq!(screen.grid.line(l.terminal_y), stopped);
        assert!(fs::read(f.root.join("received")).unwrap().is_empty());
        assert_eq!(fs::read(f.state.join("actions.log")).unwrap(), audit);
        finish(&mut master, &mut screen, &mut child);
    }
}
#[test]
fn actual_ui_existing_paths_open_literal_files_in_current_workspace_and_reuse_editor() {
    for (width, height) in [(180, 42), (60, 24)] {
        let f = Fixture::new();
        let t = f.new_workspace("file links");
        let wid = f.snapshot().active;
        fs::write(
            f.root.join("image_native.py"),
            include_str!("../fixtures/image_native.py"),
        )
        .unwrap();
        fs::create_dir_all(f.root.join("src/ui")).unwrap();
        fs::write(f.root.join("src/ui/pet.rs"), b"LITERAL_RELATIVE_FILE\n").unwrap();
        fs::write(f.root.join("a file.txt"), b"QUOTED_FILE\n").unwrap();
        let absolute = f.root.join("checks.json");
        fs::write(&absolute, b"ABSOLUTE_CHECKS_FILE\n").unwrap();
        let shown = format!(
            "\x1b[2J\x1b[Hcat {} && wc -l src/ui/pet.rs\r\n'./a file.txt' README_MISSING.txt ./src/ui\r\nDO_NOT_SUBMIT",
            absolute.display()
        );
        fs::write(f.root.join("image-display"), &shown).unwrap();
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        f.send(&t, b"python3 image_native.py\r");
        wait_ui_text(&f, &t, &mut master, &mut screen, "DO_NOT_SUBMIT");
        drain_pty(&mut master, &mut screen, "DO_NOT_SUBMIT");
        let l = flere::ui::Layout::new(width as usize, height as usize);
        // Missing files, directories, and dragging over a real link are inert.
        {
            let word = "README_MISSING";
            let (x, y) = find(&screen, word);
            write_and_wait(&mut master, &mut screen, click(x, y).as_bytes());
            assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
        }
        let (x, y) = (l.terminal_x + 5, l.terminal_y);
        write_and_wait(
            &mut master,
            &mut screen,
            format!(
                "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
                x + 1,
                y + 1,
                x + 4,
                y + 1,
                x + 4,
                y + 1
            )
            .as_bytes(),
        );
        assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
        write_and_wait(&mut master, &mut screen, b"\x1b");
        let mut ids = Vec::new();
        for (word, text) in [
            (None, "ABSOLUTE_CHECKS_FILE"),
            (Some("pet.rs"), "LITERAL_RELATIVE_FILE"),
            (Some("file.txt"), "QUOTED_FILE"),
            (None, "ABSOLUTE_CHECKS_FILE"),
        ] {
            let (x, y) = word.map_or((l.terminal_x + 5, l.terminal_y), |word| find(&screen, word));
            master.write_all(click(x, y).as_bytes()).unwrap();
            wait_current_ui(&mut master, &mut screen, |s| s.capture(100).contains(text));
            let snapshot = f.snapshot();
            assert_eq!(snapshot.active, wid);
            let editor = snapshot.session().unwrap();
            assert_eq!(editor.kind, "editor");
            ids.push(editor.id);
            // Keyboard input goes straight to the opened editor, not a file inspector.
            write_and_wait(&mut master, &mut screen, b"gg");
            f.req(&["focus", &wid.to_string(), &t.id.to_string()]);
            wait_current_ui(&mut master, &mut screen, |s| {
                s.capture(100).contains("DO_NOT_SUBMIT")
            });
        }
        assert_eq!(ids[0], ids[3]);
        assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 4);
        assert!(fs::read(f.root.join("image-input")).unwrap().is_empty());
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
        finish(&mut master, &mut screen, &mut child);
    }
}
#[test]
fn actual_ui_directory_click_confirms_stopped_project_and_deduplicates() {
    let f = Fixture::new();
    let t = f.new_workspace("directory links");
    let origin = f.snapshot().active;
    let directory = f.root.join("new-project");
    fs::create_dir(&directory).unwrap();
    fs::write(
        f.root.join("image_native.py"),
        include_str!("../fixtures/image_native.py"),
    )
    .unwrap();
    fs::write(
        f.root.join("image-display"),
        b"\x1b[2J\x1b[Hcd ./new-project && ls\r\nDO_NOT_SUBMIT",
    )
    .unwrap();
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    f.send(&t, b"python3 image_native.py\r");
    drain_pty(&mut master, &mut screen, "DO_NOT_SUBMIT");
    let point = find(&screen, "./new-project");
    write_and_wait(&mut master, &mut screen, click(point.0, point.1).as_bytes());
    assert!(screen.capture(100).contains("Add project?"));
    // Opening, paste, default Enter, and explicit cancel do not add anything.
    write_and_wait(&mut master, &mut screen, b"\x1b[200~y\r\x1b[201~");
    assert_eq!(f.snapshot().workspaces.len(), 1);
    write_and_wait(&mut master, &mut screen, b"\r");
    assert_eq!(f.snapshot().workspaces.len(), 1);
    write_and_wait(&mut master, &mut screen, click(point.0, point.1).as_bytes());
    let cancel = find(&screen, "Cancel");
    write_and_wait(
        &mut master,
        &mut screen,
        click(cancel.0, cancel.1).as_bytes(),
    );
    assert_eq!(f.snapshot().workspaces.len(), 1);
    write_and_wait(&mut master, &mut screen, click(point.0, point.1).as_bytes());
    let (_, y) = find(&screen, "Cancel");
    let line = screen.grid.line(y);
    let at = line.find("Add project").unwrap();
    let x = line[..at]
        .chars()
        .map(|c| flere::terminal::char_width(c) as usize)
        .sum::<usize>();
    write_and_wait(&mut master, &mut screen, click(x, y).as_bytes());
    // Adding a directory creates the stopped card before its metadata request
    // completes. Observe completion instead of assuming a fixed 100 ms delay.
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot()
            .workspace()
            .is_some_and(|w| w.meta.project == "new-project")
    });
    let snapshot = f.snapshot();
    assert_eq!(snapshot.workspaces.len(), 2);
    let added = snapshot.workspace().unwrap();
    assert_eq!(added.cwd, directory.to_str().unwrap());
    assert_eq!(added.meta.project, "new-project");
    assert!(added.tabs.is_empty());
    // Returning to the originating chat and confirming again reuses the existing project.
    f.req(&["focus", &origin.to_string(), &t.id.to_string()]);
    drain_pty(&mut master, &mut screen, "DO_NOT_SUBMIT");
    write_and_wait(&mut master, &mut screen, click(point.0, point.1).as_bytes());
    write_and_wait(&mut master, &mut screen, b"\t\r");
    assert_eq!(f.snapshot().workspaces.len(), 2);
    assert_eq!(f.snapshot().active, added.id);
    assert!(fs::read(f.root.join("image-input")).unwrap().is_empty());
    let original = f
        .snapshot()
        .workspaces
        .into_iter()
        .find(|w| w.id == origin)
        .unwrap();
    assert_eq!(original.tabs[0].pid, t.pid);
    assert_eq!(original.tabs[0].run, t.run);
    finish(&mut master, &mut screen, &mut child);
}
