use super::*;

fn ready(f: &Fixture, t: &TabView) {
    let until = Instant::now() + Duration::from_secs(3);
    while fs::read_to_string(f.state.join("close").join(&t.run).join("ready"))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_none()
    {
        assert!(
            Instant::now() < until,
            "integration not ready: {}",
            f.capture(t)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(120));
}

fn close(f: &Fixture, t: &TabView) -> serde_json::Value {
    let snapshot = f.snapshot();
    let wid = snapshot
        .workspaces
        .iter()
        .find(|w| w.tabs.iter().any(|tab| tab.id == t.id))
        .unwrap()
        .id
        .to_string();
    let id = t.id.to_string();
    let mut result: serde_json::Value =
        serde_json::from_slice(&f.req(&["close-check", &snapshot.epoch, &wid, &id, &t.run]))
            .unwrap();
    let token = result["token"].as_str().unwrap_or("").to_owned();
    let until = Instant::now() + Duration::from_secs(4);
    while result["status"] == "pending" {
        assert!(Instant::now() < until, "close did not finish: {result}");
        std::thread::sleep(Duration::from_millis(20));
        result = serde_json::from_slice(&f.req(&[
            "close-poll",
            &snapshot.epoch,
            &wid,
            &id,
            &t.run,
            &token,
        ]))
        .unwrap();
    }
    if !token.is_empty() {
        f.req(&["close-cancel", &snapshot.epoch, &wid, &id, &t.run, &token]);
    }
    result
}

fn open_editor(f: &Fixture) -> TabView {
    f.new_workspace("close editor");
    let path = f.root.join("clean.txt");
    fs::write(&path, "ORIGINAL_FILE\n").unwrap();
    f.req(&[
        "open",
        &f.snapshot().active.to_string(),
        &wire::hex(path.to_str().unwrap().as_bytes()),
    ]);
    let editor = f.snapshot().session().unwrap().clone();
    ready(f, &editor);
    editor
}

fn editors() -> Vec<&'static str> {
    let mut editors = vec!["/usr/bin/vim -Nu NONE -n --noplugin"];
    if flere::editor::executable("nvim") {
        editors.push("nvim -u NONE -i NONE -n --noplugin");
    }
    editors
}

#[test]
fn clean_editors_quit_without_a_signal_and_dirty_hidden_buffers_stay_open() {
    for editor in editors() {
        let f = Fixture::with_editor(editor);
        let clean = open_editor(&f);
        let result = close(&f, &clean);
        assert_eq!(
            result["status"],
            "closed",
            "{editor}: {result}, {}",
            f.capture(&clean)
        );
        let audit = fs::read_to_string(f.state.join("actions.log")).unwrap();
        assert!(!audit.contains("close-request"));
        assert_eq!(
            fs::read_to_string(f.root.join("clean.txt")).unwrap(),
            "ORIGINAL_FILE\n"
        );

        f.req(&[
            "open",
            &f.snapshot().active.to_string(),
            &wire::hex(f.root.join("clean.txt").to_str().unwrap().as_bytes()),
        ]);
        let dirty = f.snapshot().session().unwrap().clone();
        ready(&f, &dirty);
        f.send(
            &dirty,
            b":set hidden\riUNSAVED_HIDDEN\x1b:enew\r:echo 'EMPTY_CURRENT_BUFFER'\r",
        );
        f.wait_text(&dirty, "EMPTY_CURRENT_BUFFER");
        let result = close(&f, &dirty);
        assert_eq!(result["status"], "confirm", "{editor}: {result}");
        assert!(
            result["reason"].as_str().unwrap().contains("unsaved"),
            "{result}"
        );
        assert!(
            f.snapshot()
                .workspace()
                .unwrap()
                .tabs
                .iter()
                .any(|t| t.id == dirty.id && t.run == dirty.run && t.pid == dirty.pid && t.alive)
        );
        assert_eq!(
            fs::read_to_string(f.root.join("clean.txt")).unwrap(),
            "ORIGINAL_FILE\n"
        );
    }
}

#[test]
fn editor_quit_hooks_cannot_turn_a_clean_close_into_unsaved_data_loss() {
    for editor in editors() {
        let f = Fixture::with_editor(editor);
        let t = open_editor(&f);
        f.send(
            &t,
            b":autocmd QuitPre * call setline(1, 'CHANGED_DURING_QUIT')\r:echo 'HOOK_READY'\r",
        );
        f.wait_text(&t, "HOOK_READY");
        let result = close(&f, &t);
        assert_eq!(result["status"], "confirm", "{editor}: {result}");
        assert!(
            f.snapshot()
                .workspace()
                .unwrap()
                .tabs
                .iter()
                .any(|tab| tab.id == t.id && tab.alive)
        );
        assert_eq!(
            fs::read_to_string(f.root.join("clean.txt")).unwrap(),
            "ORIGINAL_FILE\n"
        );
        f.send(
            &t,
            b"\x1b:call writefile([getline(1)], $HOME . '/after-quit')\r",
        );
        let until = Instant::now() + Duration::from_secs(2);
        while fs::read_to_string(f.root.join("after-quit"))
            .ok()
            .as_deref()
            != Some("CHANGED_DURING_QUIT\n")
        {
            assert!(Instant::now() < until, "{result}: {}", f.capture(&t));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(f.root.join("after-quit")).unwrap(),
            "CHANGED_DURING_QUIT\n",
            "{editor}: {result}"
        );
    }
}

#[test]
fn zsh_close_preserves_startup_files_drafts_and_background_jobs() {
    if !std::path::Path::new("/bin/zsh").is_file() {
        return;
    }
    let f = Fixture::with_shell_editor("/bin/zsh", "/usr/bin/vim -Nu NONE -n --noplugin");
    fs::write(f.root.join(".zshenv"), "print env >> $HOME/startups\n").unwrap();
    fs::write(
        f.root.join(".zshrc"),
        "print rc >> $HOME/startups\nPROMPT='CLOSE_SHELL> '\n",
    )
    .unwrap();
    let t = f.new_workspace("close shell");
    ready(&f, &t);
    f.wait_text(&t, "CLOSE_SHELL>");
    assert_eq!(
        fs::read_to_string(f.root.join("startups")).unwrap(),
        "env\nrc\n"
    );
    f.send(&t, b"UNSUBMITTED_DRAFT");
    f.wait_text(&t, "UNSUBMITTED_DRAFT");
    let result = close(&f, &t);
    assert_eq!(result["status"], "confirm", "{result}");
    assert!(
        result["reason"].as_str().unwrap().contains("unsubmitted"),
        "{result}"
    );
    assert!(f.capture(&t).contains("UNSUBMITTED_DRAFT"));
    f.send(&t, b"\x15sleep 30 &\rprint JOB_READY\r");
    f.wait_text(&t, "JOB_READY");
    let result = close(&f, &t);
    assert_eq!(result["status"], "confirm", "{result}");
    assert!(
        result["reason"].as_str().unwrap().contains("jobs"),
        "{result}"
    );
    f.send(&t, b"kill %1; wait; print JOB_FINISHED\r");
    f.wait_text(&t, "JOB_FINISHED");
    std::thread::sleep(Duration::from_millis(100));
    let result = close(&f, &t);
    assert_eq!(result["status"], "closed", "{result}: {}", f.capture(&t));
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("close-request")
    );
}

#[test]
fn nvim_protects_user_jobs_but_allows_registered_language_server_helpers() {
    if !flere::editor::executable("nvim") {
        return;
    }
    for (command, expect_closed) in [
        (
            ":lua vim.fn.jobstart({'/bin/sleep','30'})\r:echo 'JOB_STARTED'\r",
            false,
        ),
        (
            ":lua _G.close_test_job=(vim.uv or vim.loop).spawn('/bin/sleep',{args={'30'}},function() end)\r:echo 'JOB_STARTED'\r",
            false,
        ),
        (
            ":lua vim.lsp.start({name='close-helper',cmd={vim.fn.readfile(vim.env.HOME..'/python')[1],vim.env.HOME..'/lsp.py'},on_init=function() vim.fn.writefile({'ready'},vim.env.HOME..'/lsp-ready') end})\r:echo 'JOB_STARTED'\r",
            true,
        ),
    ] {
        let f = Fixture::with_editor("nvim -u NONE -i NONE -n --noplugin");
        #[cfg(target_os = "macos")]
        let python = mac_python().0.to_str().unwrap();
        #[cfg(target_os = "linux")]
        let python = "/usr/bin/python3";
        fs::write(f.root.join("python"), python).unwrap();
        fs::write(
            f.root.join("lsp.py"),
            r#"import sys,json
while True:
    header=sys.stdin.buffer.readline()
    if not header: break
    if not header.lower().startswith(b'content-length:'): continue
    size=int(header.split(b':')[1])
    while sys.stdin.buffer.readline().strip(): pass
    req=json.loads(sys.stdin.buffer.read(size))
    if req.get('method')=='exit': break
    if 'id' in req:
        result={'capabilities': {}} if req.get('method')=='initialize' else None
        body=json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}).encode()
        sys.stdout.buffer.write(b'Content-Length: '+str(len(body)).encode()+b'\r\n\r\n'+body)
        sys.stdout.buffer.flush()
"#,
        )
        .unwrap();
        let t = open_editor(&f);
        f.send(&t, command.as_bytes());
        f.wait_text(&t, "JOB_STARTED");
        if expect_closed {
            let until = Instant::now() + Duration::from_secs(3);
            while !f.root.join("lsp-ready").is_file() {
                assert!(
                    Instant::now() < until,
                    "language server did not initialize: {}",
                    f.capture(&t)
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        std::thread::sleep(Duration::from_millis(100));
        let result = close(&f, &t);
        assert_eq!(
            result["status"],
            if expect_closed { "closed" } else { "confirm" },
            "{result}: {}",
            f.capture(&t)
        );
        if !expect_closed {
            assert_eq!(
                result["reason"],
                "A command or background job is still running",
                "{result}: {}",
                f.capture(&t)
            );
            assert!(
                f.snapshot()
                    .workspace()
                    .unwrap()
                    .tabs
                    .iter()
                    .any(|tab| tab.id == t.id && tab.run == t.run && tab.alive)
            );
        }
    }
}

#[test]
fn close_checks_reject_wrong_identities_and_survive_supervisor_refresh() {
    let f = Fixture::with_editor("/usr/bin/vim -Nu NONE -n --noplugin");
    let t = open_editor(&f);
    let snapshot = f.snapshot();
    for (epoch, wid, run) in [
        ("wrong", snapshot.active.to_string(), t.run.as_str()),
        (snapshot.epoch.as_str(), "999999".into(), t.run.as_str()),
        (
            snapshot.epoch.as_str(),
            snapshot.active.to_string(),
            "wrong",
        ),
    ] {
        assert!(
            wire::request(
                &f.state,
                &["close-check", epoch, &wid, &t.id.to_string(), run]
            )
            .is_err()
        );
    }
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let until = Instant::now() + Duration::from_secs(3);
    while !String::from_utf8(f.req(&["refresh-status"]))
        .unwrap()
        .starts_with("Refreshed")
    {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(close(&f, &t)["status"], "closed");
}

#[test]
fn expired_cancelled_and_interrupted_shell_checks_never_replay_at_the_next_prompt() {
    if !std::path::Path::new("/bin/zsh").is_file() {
        return;
    }
    let f = Fixture::with_shell_editor("/bin/zsh", "/usr/bin/vim -Nu NONE -n --noplugin");
    let t = f.new_workspace("busy shell");
    ready(&f, &t);
    // A read builtin has no child process; PID inspection alone is insufficient.
    for action in ["timeout", "cancel", "input"] {
        // Publish only after print finishes; file creation alone can precede
        // its contents and race the assertion under filesystem load.
        f.send(
            &t,
            b"read answer; print -r -- $answer > $HOME/read-result.pending; mv $HOME/read-result.pending $HOME/read-result\r",
        );
        std::thread::sleep(Duration::from_millis(100));
        let snapshot = f.snapshot();
        let wid = snapshot.active.to_string();
        let id = t.id.to_string();
        let result: serde_json::Value =
            serde_json::from_slice(&f.req(&["close-check", &snapshot.epoch, &wid, &id, &t.run]))
                .unwrap();
        assert_eq!(result["status"], "pending", "{result}");
        let token = result["token"].as_str().unwrap();
        // Refresh must not strand a live close transaction.
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
        match action {
            "timeout" => std::thread::sleep(Duration::from_millis(1100)),
            "cancel" => {
                f.req(&["close-cancel", &snapshot.epoch, &wid, &id, &t.run, token]);
            }
            _ => {}
        }
        f.send(&t, b"PRESERVED_INPUT\r");
        let until = Instant::now() + Duration::from_secs(2);
        while !f.root.join("read-result").is_file() {
            assert!(Instant::now() < until, "{action}: {}", f.capture(&t));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(f.root.join("read-result")).unwrap(),
            "PRESERVED_INPUT\n"
        );
        fs::remove_file(f.root.join("read-result")).unwrap();
        std::thread::sleep(Duration::from_millis(120));
        assert!(f.snapshot().session().unwrap().alive);
        if action != "cancel" {
            let result: serde_json::Value = serde_json::from_slice(&f.req(&[
                "close-poll",
                &snapshot.epoch,
                &wid,
                &id,
                &t.run,
                token,
            ]))
            .unwrap();
            assert_eq!(result["status"], "confirm", "{result}");
            f.req(&["close-cancel", &snapshot.epoch, &wid, &id, &t.run, token]);
        }
    }
    assert_eq!(close(&f, &t)["status"], "closed");
}

#[test]
fn editor_terminals_and_special_buffers_require_confirmation() {
    if !flere::editor::executable("nvim") {
        return;
    }
    for command in [
        b":terminal sleep 30\r\x1c\x0e".as_slice(),
        b":enew\r:setlocal buftype=nofile nomodifiable\r",
    ] {
        let f = Fixture::with_editor("nvim -u NONE -i NONE -n --noplugin");
        let t = open_editor(&f);
        f.send(&t, command);
        std::thread::sleep(Duration::from_millis(200));
        let result = close(&f, &t);
        assert_eq!(result["status"], "confirm", "{result}: {}", f.capture(&t));
        assert!(f.snapshot().session().unwrap().alive);
    }
}

#[test]
fn ui_closes_clean_editor_quietly_and_explains_dirty_buffer_confirmation() {
    let f = Fixture::with_editor("/usr/bin/vim -Nu NONE -n --noplugin");
    let clean = open_editor(&f);
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 120, 32).unwrap();
    let mut screen = Terminal::new(120, 32);
    drain_pty(&mut master, &mut screen, "FLERE");
    write_ui_bytes(&mut master, &mut screen, b"\0x");
    wait_current_ui(&mut master, &mut screen, |screen| {
        assert!(!screen.capture(100).contains("Close terminal?"));
        f.snapshot()
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .all(|t| t.id != clean.id)
    });
    f.req(&[
        "open",
        &f.snapshot().active.to_string(),
        &wire::hex(f.root.join("clean.txt").to_str().unwrap().as_bytes()),
    ]);
    let dirty = f.snapshot().session().unwrap().clone();
    ready(&f, &dirty);
    f.send(&dirty, b"iUNSAVED\x1b");
    pump_ui_bytes(&mut master, &mut screen, 200);
    write_ui_bytes(&mut master, &mut screen, b"x");
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100).contains("1 unsaved buffer")
    });
    // Enter selects the default Cancel button; the editor's dirty contents survive.
    write_ui_bytes(&mut master, &mut screen, b"\r");
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(f.snapshot().session().unwrap().alive);
    assert!(!screen.capture(100).contains("Close terminal?"));
    write_ui_bytes(&mut master, &mut screen, b"q");
    let until = Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() {
        pump_ui_bytes(&mut master, &mut screen, 20);
        assert!(Instant::now() < until);
    }
}
