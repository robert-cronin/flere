//! Explicit stand-in task commands in private PTYs; no project build execution.
use super::*;
use flere::tasks::{Report, Summary};

fn start_at(
    f: &Fixture,
    snapshot: &Snapshot,
    label: &str,
    command: &str,
) -> std::io::Result<Summary> {
    let tab = snapshot.session();
    let bytes = wire::request(
        &f.state,
        &[
            "task-start",
            &snapshot.epoch,
            &snapshot.active.to_string(),
            &snapshot.tab.to_string(),
            tab.map(|t| t.run.as_str()).unwrap_or(""),
            &snapshot
                .split
                .as_ref()
                .map_or(0, |s| s.revision)
                .to_string(),
            &wire::hex(label.as_bytes()),
            &wire::hex(command.as_bytes()),
        ],
    )?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}
fn report(f: &Fixture, task: &Summary) -> Report {
    serde_json::from_slice(&f.req(&[
        "task-info",
        &f.snapshot().epoch,
        &task.workspace.to_string(),
        &task.session.to_string(),
        &task.run,
    ]))
    .unwrap()
}
fn wait(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < until, "task condition timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn refresh(f: &Fixture) {
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    wait(|| {
        wire::request(&f.state, &["refresh-status"])
            .is_ok_and(|b| String::from_utf8_lossy(&b).contains("all terminal sessions preserved"))
    });
}
fn standin(f: &Fixture) {
    fs::create_dir_all(f.root.join("src")).unwrap();
    fs::write(
        f.root.join("src/task-target.rs"),
        (1..=100)
            .map(|n| {
                if n == 73 {
                    "TASK_SOURCE_TARGET\n".into()
                } else {
                    format!("source line {n}\n")
                }
            })
            .collect::<String>(),
    )
    .unwrap();
    fs::write(
        f.root.join("task-standin.sh"),
        concat!(
            "printf 'once\\n' >> task-runs\n",
            "printf '\\033[31merror[E0308]: fixture error\\033[0m\\n'\n",
            "printf '  --> src/task-target.rs:73:9\\n'\n",
            "printf 'src/task-target.rs:12:3: warning: fixture warning\\n'\n",
            "printf '\\033]633;D;0\\007'\n",
            "printf 'TASK_READY\\n'\n",
            "while [ ! -f task-release ]; do sleep 0.03; done\n",
            "printf 'TASK_FINISHED\\n'\n",
            "exit 7\n"
        ),
    )
    .unwrap();
}

#[test]
fn tasks_own_new_ptys_and_retain_actual_exit_problems_and_refresh_identity() {
    let f = Fixture::new();
    let tabs = splits::probes(&f, 2);
    splits::pane(&f, "split-right", Some("move"));
    for tab in &tabs {
        f.send(tab, b"UNSUBMITTED_TASK_NEIGHBOR");
    }
    standin(&f);
    let before = splits::panes(&f);
    let task = start_at(&f, &before, "Fixture build", "sh task-standin.sh").unwrap();
    let task_tab = f.snapshot().session().unwrap().clone();
    assert_eq!(task_tab.kind, "shell");
    assert!(task_tab.title.starts_with("Task: Fixture build"));
    assert!(
        tabs.iter()
            .all(|t| t.pid != task_tab.pid && t.id != task_tab.id)
    );
    f.wait_text(&task_tab, "TASK_READY");
    wait(|| report(&f, &task).problems.len() == 2);
    let running = report(&f, &task);
    assert!(
        !running.task.ended,
        "OSC command markers are not task exit authority"
    );
    assert_eq!(running.task.exit_code, None);
    assert_eq!(running.problems[0].path, f.root.join("src/task-target.rs"));
    assert_eq!(
        (running.problems[0].line, running.problems[0].column),
        (73, 9)
    );
    let groups = splits::panes(&f).split.unwrap().groups;
    assert_eq!(groups[0].tabs, vec![tabs[0].id]);
    assert_eq!(groups[1].tabs, vec![tabs[1].id, task.session]);
    splits::emit(
        &f,
        &tabs[0],
        1,
        &format!("{}\r\nTASK_NEIGHBOR_DRAINED", "neighbor\r\n".repeat(6000)),
    );
    f.wait_text(&tabs[0], "TASK_NEIGHBOR_DRAINED");
    let supervisor = f.child.id();
    refresh(&f);
    assert_eq!(f.child.id(), supervisor);
    assert_eq!(report(&f, &task).task, running.task);
    assert_eq!(report(&f, &task).problems, running.problems);
    assert_eq!(f.snapshot().session().unwrap().pid, task_tab.pid);
    assert_eq!(splits::panes(&f).split.unwrap().groups, groups);
    fs::write(f.root.join("task-release"), "release").unwrap();
    wait(|| report(&f, &task).task.ended && !f.snapshot().session().unwrap().alive);
    let ended = report(&f, &task);
    assert_eq!(ended.task.exit_code, Some(7));
    assert!(f.capture(&task_tab).contains("TASK_FINISHED"));
    assert!(f.snapshot().session().unwrap().title.contains("exit 7"));
    refresh(&f);
    assert_eq!(report(&f, &task).task.exit_code, Some(7));
    assert!(f.capture(&task_tab).contains("TASK_FINISHED"));
    assert_eq!(
        fs::read_to_string(f.root.join("task-runs")).unwrap(),
        "once\n"
    );
    for tab in &tabs {
        assert_eq!(splits::input(&f, tab), b"UNSUBMITTED_TASK_NEIGHBOR");
    }
    assert!(start_at(&f, &before, "stale", "printf BAD").is_err());
    assert!(
        wire::request(
            &f.state,
            &[
                "task-info",
                &before.epoch,
                &task.workspace.to_string(),
                &task.session.to_string(),
                &tabs[0].run
            ]
        )
        .is_err()
    );
    assert!(
        wire::request(
            &f.state,
            &[
                "task-info",
                "stale-epoch",
                &task.workspace.to_string(),
                &task.session.to_string(),
                &task.run
            ]
        )
        .is_err()
    );
    let closed: serde_json::Value = serde_json::from_slice(&f.req(&[
        "close-check",
        &before.epoch,
        &task.workspace.to_string(),
        &task.session.to_string(),
        &task.run,
    ]))
    .unwrap();
    assert_eq!(closed["status"], "closed");
    wait(|| {
        !f.snapshot()
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .any(|t| t.id == task.session)
    });
    assert!(
        wire::request(
            &f.state,
            &[
                "task-info",
                &before.epoch,
                &task.workspace.to_string(),
                &task.session.to_string(),
                &task.run
            ]
        )
        .is_err()
    );
}

#[test]
fn tasks_full_restart_restores_shell_directory_without_replaying_command() {
    let mut f = Fixture::new();
    f.new_workspace("No task replay");
    standin(&f);
    let task = start_at(&f, &splits::panes(&f), "Run once", "sh task-standin.sh").unwrap();
    f.wait_text(f.snapshot().session().unwrap(), "TASK_READY");
    f.stop();
    let saved = fs::read_to_string(f.state.join("workspaces.v2.json")).unwrap();
    assert!(!saved.contains("sh task-standin.sh"));
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .env("SHELL", "/bin/sh")
        .env("HOME", &f.root)
        .env_remove("XDG_CACHE_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let after = f.snapshot();
    assert!(after.workspace().unwrap().tabs.is_empty());
    let list: Vec<Summary> =
        serde_json::from_slice(&f.req(&["task-list", &after.epoch, &after.active.to_string()]))
            .unwrap();
    assert!(list.is_empty());
    let mut command = outer_ui_command();
    command
        .args(["--state", f.state.to_str().unwrap(), "attach"])
        .env("HOME", &f.root)
        .env_remove("XDG_CACHE_HOME");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 35).unwrap();
    let mut screen = Terminal::new(160, 35);
    drain_pty(&mut master, &mut screen, "FLERE");
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().tabs.len() == 2
    });
    let restored = f.snapshot();
    assert!(
        restored
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .all(|t| t.kind == "shell" && t.run != task.run)
    );
    assert_eq!(
        fs::read_to_string(f.root.join("task-runs")).unwrap(),
        "once\n"
    );
    f.send(
        restored.session().unwrap(),
        b"printf 'RESTORED_%s\\n' SHELL; pwd\r",
    );
    f.wait_text(restored.session().unwrap(), "RESTORED_SHELL");
    assert!(
        f.capture(restored.session().unwrap())
            .contains(f.root.to_str().unwrap())
    );
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn actual_ui_tasks_launch_from_form_and_problems_open_the_exact_pane_without_draft_input() {
    let f = Fixture::new();
    let tabs = splits::probes(&f, 2);
    splits::pane(&f, "split-right", Some("move"));
    f.send(&tabs[1], b"KEEP_TASK_DRAFT");
    standin(&f);
    let mut command = outer_ui_command();
    command
        .args(["--state", f.state.to_str().unwrap(), "attach"])
        .env("HOME", &f.root)
        .env_remove("XDG_CACHE_HOME");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 180, 38).unwrap();
    let mut screen = Terminal::new(180, 38);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0!unsubmitted-command").unwrap();
    drain_pty(&mut master, &mut screen, "Run Task");
    master.write_all(b"\x1b[<0;1;1M\x1b[<0;1;1m\x1b").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 2);
    assert_eq!(splits::input(&f, &tabs[1]), b"KEEP_TASK_DRAFT");
    master.write_all(b"!sh task-standin.sh\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().tabs.len() == 3
    });
    let task_tab = f.snapshot().session().unwrap().clone();
    f.wait_text(&task_tab, "TASK_READY");
    master.write_all(b"\0;").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        let text = s.capture(100);
        text.contains("1 / 1 · Task") && text.contains("src/task-target.rs:73:9")
    });
    assert!(screen.capture(100).contains("Problems"));
    master.write_all(b"\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().session().is_some_and(|t| t.kind == "editor")
    });
    let editor = f.snapshot().session().unwrap().clone();
    assert_eq!(
        editor.path,
        f.root.join("src/task-target.rs").to_str().unwrap()
    );
    f.wait_text(&editor, "TASK_SOURCE_TARGET");
    let groups = splits::panes(&f).split.unwrap().groups;
    assert_eq!(groups[0].tabs, vec![tabs[0].id]);
    assert!(groups[1].tabs.contains(&editor.id));
    assert!(groups[1].tabs.contains(&task_tab.id));
    assert_eq!(splits::input(&f, &tabs[1]), b"KEEP_TASK_DRAFT");
    assert!(splits::input(&f, &tabs[0]).is_empty());
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn task_spawn_failures_do_not_allocate_tabs_and_partial_locations_wait_for_completion() {
    let f = Fixture::new();
    let cwd = f.root.join("task-directory");
    fs::create_dir(&cwd).unwrap();
    f.req(&[
        "new-stopped",
        &wire::hex(b"Task failure fixture"),
        &wire::hex(cwd.to_str().unwrap().as_bytes()),
    ]);
    let before = splits::panes(&f);
    for command in ["", "printf BAD\n", "printf BAD\0"] {
        assert!(start_at(&f, &before, "invalid", command).is_err());
    }
    fs::remove_dir(&cwd).unwrap();
    assert!(start_at(&f, &before, "missing directory", "printf BAD").is_err());
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    fs::create_dir(&cwd).unwrap();
    fs::write(
        cwd.join("partial.sh"),
        concat!(
            "printf 'src/task-target.rs:7'\n",
            "while [ ! -f suffix ]; do sleep 0.03; done\n",
            "printf '3:9: completed location'\n",
            "while [ ! -f finish ]; do sleep 0.03; done\n",
            "exit 9\n"
        ),
    )
    .unwrap();
    let task = start_at(&f, &before, "partial fixture", "sh partial.sh").unwrap();
    assert_eq!(
        task.session,
        before.active + 1,
        "failed starts cannot consume terminal IDs"
    );
    let tab = f.snapshot().session().unwrap().clone();
    f.wait_text(&tab, "src/task-target.rs:7");
    assert!(report(&f, &task).problems.is_empty());
    fs::write(cwd.join("suffix"), "go").unwrap();
    f.wait_text(&tab, ":73:9: completed location");
    assert!(
        report(&f, &task).problems.is_empty(),
        "unterminated live rows remain mutable"
    );
    fs::write(cwd.join("finish"), "go").unwrap();
    wait(|| report(&f, &task).task.ended && report(&f, &task).problems.len() == 1);
    let complete = report(&f, &task);
    assert_eq!(
        (complete.problems[0].line, complete.problems[0].column),
        (73, 9)
    );
    assert_eq!(complete.task.exit_code, Some(9));
    wait(|| !f.snapshot().session().unwrap().alive);
    f.req(&["close", &task.session.to_string(), &task.run]);
    wait(|| f.snapshot().workspace().unwrap().tabs.is_empty());
}
