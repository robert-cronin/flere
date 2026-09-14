//! Workspace/output search and exact native editor jumps in disposable sessions.
use super::*;
use std::sync::atomic::AtomicBool;

#[test]
fn new_bash_and_zsh_shells_mark_prompts_without_changing_startup_or_drafts() {
    for shell in ["/bin/bash", "/bin/zsh"] {
        if !std::path::Path::new(shell).is_file() {
            continue;
        }
        let f = Fixture::with_shell_editor(shell, "/usr/bin/vim -Nu NONE -n --noplugin");
        let file = if shell.ends_with("zsh") {
            ".zshrc"
        } else {
            ".bashrc"
        };
        fs::write(
            f.root.join(file),
            "printf 'startup\\n' >> \"$HOME/startup-proof\"\nPS1='MARKER_PROMPT> '\n",
        )
        .unwrap();
        let tab = f.new_workspace("command markers");
        f.wait_text(&tab, "MARKER_PROMPT>");
        f.send(&tab, b"printf 'FIRST_OUTPUT\\n'\r");
        f.wait_text(&tab, "FIRST_OUTPUT");
        std::thread::sleep(Duration::from_millis(100));
        f.send(&tab, b"printf 'SECOND_OUTPUT\\n'\r");
        f.wait_text(&tab, "SECOND_OUTPUT");
        std::thread::sleep(Duration::from_millis(100));
        let page = flere::model::ScrollbackPage::decode(&f.req(&[
            "command-jump",
            &tab.id.to_string(),
            &tab.run,
            "live",
            "previous",
        ]))
        .unwrap();
        assert!(page.available);
        assert_eq!(
            fs::read_to_string(f.root.join("startup-proof")).unwrap(),
            "startup\n"
        );
        if shell.ends_with("zsh") {
            let output = f.req(&["command-output", &tab.id.to_string(), &tab.run, "live"]);
            assert_eq!(String::from_utf8(output).unwrap().trim(), "SECOND_OUTPUT");
        }
        f.send(&tab, b"UNSUBMITTED_MARKER_DRAFT");
        f.wait_text(&tab, "UNSUBMITTED_MARKER_DRAFT");
        f.req(&[
            "command-jump",
            &tab.id.to_string(),
            &tab.run,
            "live",
            "previous",
        ]);
        assert!(f.capture(&tab).contains("UNSUBMITTED_MARKER_DRAFT"));
    }
}

#[test]
fn workspace_search_respects_git_ignores_and_skips_binary_and_external_links() {
    let f = Fixture::new();
    let repo = f.root.join("repo");
    fs::create_dir(&repo).unwrap();
    history_git(&repo, &["init", "-q", "-b", "main"]);
    fs::write(repo.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(repo.join("ignored.txt"), "needle\n").unwrap();
    fs::write(repo.join("source.rs"), "first\nİ界 needle\n").unwrap();
    fs::write(repo.join("binary"), b"needle\0").unwrap();
    fs::write(f.root.join("secret"), "needle\n").unwrap();
    std::os::unix::fs::symlink(f.root.join("secret"), repo.join("outside")).unwrap();
    let cancel = AtomicBool::new(false);
    let hits =
        flere::search::workspace(&repo, flere::search::Kind::Contents, "NEEDLE", &cancel).unwrap();
    assert_eq!(hits.hits.len(), 1);
    assert_eq!((hits.hits[0].line, hits.hits[0].column), (2, 7));
    assert!(hits.hits[0].path.ends_with("source.rs"));
    let files =
        flere::search::workspace(&repo, flere::search::Kind::Files, "srs", &cancel).unwrap();
    assert_eq!(files.hits.len(), 1);
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        flere::search::workspace(&repo, flere::search::Kind::Contents, "needle", &cancel)
            .unwrap()
            .hits
            .is_empty()
    );
}

#[test]
fn output_search_pages_interpreted_history_and_rejects_another_run() {
    let f = Fixture::new();
    let tabs = splits::probes(&f, 1);
    let t = &tabs[0];
    let output = (0..1200)
        .map(|i| format!("RETAINED_SEARCH_{i:04}\r\n"))
        .collect::<String>();
    splits::emit(&f, t, 1, &output);
    f.wait_text(t, "RETAINED_SEARCH_1199");
    let mut from = 0;
    let mut found = Vec::new();
    loop {
        let page: flere::terminal::search::Page = serde_json::from_slice(&f.req(&[
            "search-output",
            &t.id.to_string(),
            &t.run,
            &from.to_string(),
            &wire::hex(b"RETAINED_SEARCH_0"),
        ]))
        .unwrap();
        found.extend(page.hits.into_iter().map(|h| h.text));
        if page.next >= page.end {
            break;
        }
        from = page.next;
    }
    assert_eq!(found.len(), 1000);
    assert!(found[0].contains("RETAINED_SEARCH_0000"));
    assert!(
        wire::request(
            &f.state,
            &[
                "search-output",
                &t.id.to_string(),
                "bad-run",
                "0",
                &wire::hex(b"SEARCH")
            ]
        )
        .is_err()
    );
    assert!(splits::input(&f, t).is_empty());
}

#[test]
fn actual_ui_search_keeps_drafts_and_opens_exact_file_line_in_its_pane() {
    let f = Fixture::new();
    let tabs = splits::probes(&f, 2);
    splits::pane(&f, "split-right", Some("move"));
    let path = f.root.join("jump-target.txt");
    let text = (1..=100)
        .map(|n| {
            if n == 73 {
                "UNIQUE_SEARCH_NEEDLE\n".into()
            } else {
                format!("line {n}\n")
            }
        })
        .collect::<String>();
    fs::write(&path, &text).unwrap();
    f.send(&tabs[1], b"RETAIN_MY_DRAFT");
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root)
        .env_remove("XDG_CACHE_HOME");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 180, 35).unwrap();
    let mut screen = Terminal::new(180, 35);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0Qjump-target").unwrap();
    drain_pty(&mut master, &mut screen, "1 matches");
    assert!(screen.capture(100).contains("Quick Open"));
    master.write_all(b"\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    master.write_all(b"HUNIQUE_SEARCH_NEEDLE").unwrap();
    drain_pty(&mut master, &mut screen, "jump-target.txt:73:1");
    master.write_all(b"\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().session().is_some_and(|t| t.kind == "editor")
    });
    let editor = f.snapshot().session().unwrap().clone();
    assert_eq!(editor.path, path.to_str().unwrap());
    f.wait_text(&editor, "UNIQUE_SEARCH_NEEDLE");
    assert_eq!(splits::input(&f, &tabs[1]), b"RETAIN_MY_DRAFT");
    let s = splits::panes(&f);
    let split = s.split.as_ref().unwrap();
    assert!(split.groups[1].tabs.contains(&editor.id));
    assert_eq!(split.groups[0].tabs, vec![tabs[0].id]);
    // Existing editor jump uses its private cooperating timer, with no injected
    // input and no buffer replacement or save. A stale pane must reject it.
    std::thread::sleep(Duration::from_millis(180));
    f.req(&[
        "open-at-pane-epoch",
        &s.epoch,
        &s.active.to_string(),
        &editor.id.to_string(),
        &editor.run,
        &split.revision.to_string(),
        &wire::hex(path.to_str().unwrap().as_bytes()),
        "25",
        "1",
    ]);
    std::thread::sleep(Duration::from_millis(180));
    f.send(
        &editor,
        b":call writefile([string(line('.'))], $HOME . '/cursor-proof')\r",
    );
    let until = Instant::now() + Duration::from_secs(3);
    let cursor = loop {
        match fs::read_to_string(f.root.join("cursor-proof")) {
            // Vim creates the file before writefile finishes its newline-terminated value.
            Ok(value) if value.ends_with('\n') => break value,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("cursor proof read failed: {error}"),
        }
        assert!(
            Instant::now() < until,
            "cursor proof write did not complete"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(cursor.trim(), "25");
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert!(
        wire::request(
            &f.state,
            &[
                "open-at-pane-epoch",
                &s.epoch,
                &s.active.to_string(),
                &tabs[1].id.to_string(),
                &tabs[1].run,
                &split.revision.to_string(),
                &wire::hex(path.to_str().unwrap().as_bytes()),
                "3",
                "1"
            ]
        )
        .is_err()
    );
    assert_eq!(f.snapshot().session().unwrap().id, editor.id);
    splits::unchanged(&f, &tabs);
    finish_ui(&mut master, &mut screen, &mut child);
}
