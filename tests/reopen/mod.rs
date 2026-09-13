use super::*;
use serde_json::{Value, json};

fn restart(f: &mut Fixture) {
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("SHELL", "/bin/sh")
        .env("HOME", &f.root)
        .env("PS1", "$ ")
        .env("VISUAL", "/usr/bin/vim -Nu NONE -n --noplugin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
}
fn standins(f: &Fixture) {
    let script = f.root.join("native-wait.sh");
    fs::write(
        &script,
        "#!/bin/sh\nprintf 'RESTORED_NATIVE\\n'\nread input\n",
    )
    .unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([
            {"name":"codex","command":["/bin/sh",script]},
            {"name":"claude","command":["/bin/sh",script]},
            {"name":"copilot","command":["/bin/sh",script]}
        ]))
        .unwrap(),
    )
    .unwrap();
}
fn attach(f: &Fixture, count: usize) -> (fs::File, os::Process, Terminal) {
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot()
            .workspaces
            .iter()
            .map(|w| w.tabs.len())
            .sum::<usize>()
            == count
    });
    // Wait for the restoration protocol itself, not only the last spawned process.
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(child.try_wait().unwrap().is_none());
    (master, child, screen)
}
fn spec(f: &Fixture, t: &TabView) -> Value {
    serde_json::from_slice(&fs::read(f.state.join("runs").join(format!("{}.json", t.run))).unwrap())
        .unwrap()
}
fn stored(f: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap()
}

#[test]
fn ui_restores_three_exact_chats_order_selection_and_never_duplicates_on_reattach() {
    let mut f = Fixture::new();
    standins(&f);
    f.req(&[
        "new-stopped",
        &wire::hex(b"Three chats"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    let profiles = ["codex", "claude", "copilot"];
    let mut uuids = Vec::new();
    let mut old = Vec::new();
    for (i, harness) in profiles.iter().enumerate() {
        let uuid = format!("00000000-1234-5678-9012-{i:012}");
        f.req(&["native", &wid.to_string(), harness, &uuid]);
        let t = f.snapshot().session().unwrap().clone();
        f.wait_text(&t, "RESTORED_NATIVE");
        uuids.push(uuid);
        old.push(t);
    }
    f.req(&["focus", &wid.to_string(), &old[1].id.to_string()]);
    f.req(&["save-tabs"]);
    let layout = stored(&f);
    for (i, t) in old.iter().enumerate() {
        assert_eq!(layout["workspaces"][0]["tabs"][i]["owner"][0], t.pid);
    }
    let old_epoch = f.snapshot().epoch;
    restart(&mut f);
    assert!(wire::request(&f.state, &["restore-next", &old_epoch]).is_err());
    let (mut master, mut child, mut screen) = attach(&f, 3);
    let snapshot = f.snapshot();
    let w = snapshot.workspace().unwrap();
    assert_eq!(w.id, wid);
    assert_eq!(snapshot.tab, w.tabs[1].id);
    for (i, t) in w.tabs.iter().enumerate() {
        assert_ne!(t.run, old[i].run);
        assert_ne!(t.pid, old[i].pid);
        let n = spec(&f, t);
        assert_eq!(n["conversation"], uuids[i]);
        assert_eq!(n["harness"], profiles[i]);
        assert_eq!(n["cwd"], f.root.to_str().unwrap());
        let args = n["argv"].as_array().unwrap();
        if i == 2 {
            assert_eq!(args[2], format!("--resume={}", uuids[i]));
            assert_eq!(args.len(), 3);
        } else {
            assert_eq!(args[2], if i == 0 { "resume" } else { "--resume" });
            assert_eq!(args[3], uuids[i]);
            assert_eq!(args.len(), 4);
        }
    }
    let identities: Vec<_> = w
        .tabs
        .iter()
        .map(|t| (t.id, t.pid, t.run.clone()))
        .collect();
    let (mut master2, mut child2, mut screen2) = attach(&f, 3);
    let again = f.snapshot();
    assert_eq!(
        again
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .map(|t| (t.id, t.pid, t.run.clone()))
            .collect::<Vec<_>>(),
        identities
    );
    assert!(!screen2.capture(100).contains("Conversation UUID"));
    finish_ui(&mut master2, &mut screen2, &mut child2);
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn shells_reopen_in_their_last_directories_and_editor_reopens_without_replaying_drafts() {
    let mut f = Fixture::new();
    let a = f.new_workspace("Shells and editor");
    let wid = f.snapshot().active;
    let one = f.root.join("first directory");
    let two = f.root.join("second directory");
    fs::create_dir(&one).unwrap();
    fs::create_dir(&two).unwrap();
    f.send(
        &a,
        format!("cd '{}'\nprintf 'DIRECTORY_ONE\\n'\n", one.display()).as_bytes(),
    );
    f.wait_text(&a, "DIRECTORY_ONE");
    f.req(&["tab", &wid.to_string()]);
    let b = f.snapshot().session().unwrap().clone();
    f.send(
        &b,
        format!("cd '{}'\nprintf 'DIRECTORY_TWO\\n'\n", two.display()).as_bytes(),
    );
    f.wait_text(&b, "DIRECTORY_TWO");
    f.send(&a, b"echo UNSUBMITTED_DO_NOT_REPLAY");
    let file = f.root.join("reopened.txt");
    fs::write(&file, "Saved editor content\n").unwrap();
    f.req(&[
        "open",
        &wid.to_string(),
        &wire::hex(file.to_str().unwrap().as_bytes()),
    ]);
    f.wait_text(f.snapshot().session().unwrap(), "Saved editor content");
    f.req(&["focus", &wid.to_string(), &b.id.to_string()]);
    restart(&mut f);
    let (mut master, mut child, mut screen) = attach(&f, 3);
    let snapshot = f.snapshot();
    let w = snapshot.workspace().unwrap();
    assert_eq!(
        w.tabs.iter().map(|t| t.kind.as_str()).collect::<Vec<_>>(),
        ["shell", "shell", "editor"]
    );
    assert_eq!(snapshot.tab, w.tabs[1].id);
    for (t, dir) in w.tabs.iter().zip([&one, &two]) {
        assert_eq!(os::process_cwd(t.pid).unwrap(), *dir);
        assert!(!f.capture(t).contains("UNSUBMITTED_DO_NOT_REPLAY"));
    }
    assert_eq!(w.tabs[2].path, file.to_str().unwrap());
    f.wait_text(&w.tabs[2], "Saved editor content");
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn closed_tabs_stay_closed_and_pending_tabs_survive_a_second_supervisor_restart() {
    let mut f = Fixture::new();
    let a = f.new_workspace("Keep only open tabs");
    let wid = f.snapshot().active;
    f.req(&["tab", &wid.to_string()]);
    let b = f.snapshot().session().unwrap().clone();
    f.req(&["close", &a.id.to_string(), &a.run]);
    let end = Instant::now() + Duration::from_secs(3);
    while f.snapshot().workspace().unwrap().tabs.len() != 1 {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    restart(&mut f);
    assert_eq!(
        stored(&f)["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let epoch = f.snapshot().epoch;
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let end = Instant::now() + Duration::from_secs(5);
    while !String::from_utf8(f.req(&["refresh-status"]))
        .unwrap()
        .contains("all terminal sessions preserved")
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().epoch, epoch);
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    assert_eq!(
        stored(&f)["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    f.req(&["rename", &wid.to_string(), &wire::hex(b"Still pending")]);
    restart(&mut f);
    let (mut master, mut child, mut screen) = attach(&f, 1);
    assert_ne!(f.snapshot().session().unwrap().run, b.run);
    assert_eq!(
        stored(&f)["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn failed_tab_restore_keeps_other_tabs_and_retries_only_on_explicit_request() {
    let mut f = Fixture::new();
    let a = f.new_workspace("Partial restore");
    let wid = f.snapshot().active;
    let directory = f.root.join("temporarily absent");
    fs::create_dir(&directory).unwrap();
    f.send(
        &a,
        format!(
            "cd '{}'\nprintf 'DIRECTORY_READY\\n'\n",
            directory.display()
        )
        .as_bytes(),
    );
    f.wait_text(&a, "DIRECTORY_READY");
    f.req(&["tab", &wid.to_string()]);
    restart(&mut f);
    let hidden = f.root.join("moved directory");
    fs::rename(&directory, &hidden).unwrap();
    let (mut master, mut child, mut screen) = attach(&f, 1);
    let epoch = f.snapshot().epoch;
    for _ in 0..3 {
        let r: Value = serde_json::from_slice(&f.req(&["restore-next", &epoch])).unwrap();
        assert_eq!(r["remaining"], 0);
    }
    assert_eq!(
        stored(&f)["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::rename(&hidden, &directory).unwrap();
    master.write_all(b"\0W").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().tabs.len() == 2
    });
    assert_eq!(
        os::process_cwd(f.snapshot().workspace().unwrap().tabs[0].pid).unwrap(),
        directory
    );
    master.write_all(b"\r").unwrap();
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn unknown_native_id_opens_the_harness_picker_and_stopped_s_never_asks_for_a_uuid() {
    let mut f = Fixture::new();
    standins(&f);
    f.req(&[
        "new-stopped",
        &wire::hex(b"Native picker"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 100, 24).unwrap();
    let mut screen = Terminal::new(100, 24);
    drain_pty(&mut master, &mut screen, "Workspace is stopped.");
    master.write_all(b"S").unwrap();
    drain_pty(&mut master, &mut screen, "Harness: codex");
    assert!(!screen.capture(100).contains("UUID"));
    master.write_all(b"\r").unwrap();
    drain_pty(&mut master, &mut screen, "RESTORED_NATIVE");
    assert_eq!(
        spec(&f, f.snapshot().session().unwrap())["argv"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    finish_ui(&mut master, &mut screen, &mut child);
    restart(&mut f);
    let (mut master, mut child, mut screen) = attach(&f, 1);
    let snapshot = f.snapshot();
    assert_eq!(snapshot.active, wid);
    let n = spec(&f, snapshot.session().unwrap());
    assert_eq!(n["argv"][2], "resume");
    assert_eq!(n["argv"].as_array().unwrap().len(), 3);
    assert_eq!(n["conversation"], "");
    assert!(!screen.capture(100).contains("UUID"));
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn saved_tab_refuses_to_duplicate_a_still_running_previous_process() {
    let mut f = Fixture::new();
    f.new_workspace("Owner guard");
    f.stop();
    let mut command = Command::new("/bin/sh");
    let (master, mut child) = os::spawn_command_pty(&f.root, &mut command, 80, 24).unwrap();
    let (_, start) = os::child_identity(child.id()).unwrap();
    let mut layout = stored(&f);
    layout["workspaces"][0]["tabs"][0]["owner"] = json!([child.id(), start]);
    fs::write(
        f.state.join("workspaces.v2.json"),
        serde_json::to_vec(&layout).unwrap(),
    )
    .unwrap();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("HOME", &f.root)
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    let epoch = f.snapshot().epoch;
    let result: Value = serde_json::from_slice(&f.req(&["restore-next", &epoch])).unwrap();
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("previous tab process is still running")
    );
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    assert_eq!(
        stored(&f)["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    child.kill().unwrap();
    drop(master);
    let end = Instant::now() + Duration::from_secs(3);
    while child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    let wid = f.snapshot().active;
    f.req(&["restore-retry", &epoch, &wid.to_string()]);
    let result: Value = serde_json::from_slice(&f.req(&["restore-next", &epoch])).unwrap();
    assert_eq!(result["error"], "");
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
}

#[test]
fn tab_tracks_conversation_switches_inside_the_owned_native_harness() {
    let f = Fixture::new();
    let executable = f.root.join("codex");
    copy_fixture_python(&executable);
    let ids = [
        "11111111-1234-5678-9012-123456789012",
        "22222222-1234-5678-9012-123456789012",
    ];
    let files = [
        f.root.join("rollout-first.jsonl"),
        f.root.join("rollout-second.jsonl"),
    ];
    for (id, file) in ids.iter().zip(&files) {
        fs::write(
            file,
            serde_json::to_string(
                &json!({"type":"session_meta","payload":{"id":id,"cwd":f.root,"source":"cli"}}),
            )
            .unwrap()
                + "\n",
        )
        .unwrap();
    }
    let mut argv = Vec::<String>::new();
    #[cfg(target_os = "macos")]
    argv.extend([
        "/usr/bin/env".into(),
        format!("PYTHONHOME={}", mac_python().1.display()),
    ]);
    argv.extend([executable.to_str().unwrap().into(), "-c".into(),
        "import sys; held=open(sys.argv[1]); print('FIRST_READY',flush=True); sys.stdin.readline(); held.close(); held=open(sys.argv[2]); print('SECOND_READY',flush=True); sys.stdin.readline(); held.close(); held=open(sys.argv[1]); print('BACK_READY',flush=True); sys.stdin.readline()".into(),
        files[0].to_str().unwrap().into(), files[1].to_str().unwrap().into()]);
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":argv}])).unwrap(),
    )
    .unwrap();
    f.req(&[
        "new-stopped",
        &wire::hex(b"Switch conversations"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    f.req(&["native", &wid.to_string(), "codex", ids[0]]);
    let t = f.snapshot().session().unwrap().clone();
    f.wait_text(&t, "FIRST_READY");
    for (id, marker) in [(ids[1], "SECOND_READY"), (ids[0], "BACK_READY")] {
        f.send(&t, b"\r");
        f.wait_text(&t, marker);
        let end = Instant::now() + Duration::from_secs(4);
        while stored(&f)["workspaces"][0]["tabs"][0]["conversation"] != id {
            assert!(
                Instant::now() < end,
                "owned conversation switch was not saved"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
    }
}
