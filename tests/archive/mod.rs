use super::*;
use serde_json::{Value, json};

fn stopped_card(f: &Fixture) -> u64 {
    let created: Value = serde_json::from_slice(&f.req(&[
        "new-stopped",
        &wire::hex(b"Retained work"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]))
    .unwrap();
    created["workspace"].as_u64().unwrap()
}

fn stored(f: &Fixture) -> Value {
    let value: Value =
        serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap();
    if value["version"] == 11 {
        flere::record_store::Store::read_layout(&f.state.join(value["records"].as_str().unwrap()))
            .unwrap()
    } else {
        value
    }
}

fn card(layout: &Value, wid: u64) -> Value {
    layout["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == wid)
        .unwrap()
        .clone()
}

fn restart(f: &mut Fixture) {
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
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
}

#[test]
fn agent_archive_and_restore_preserve_saved_layout_through_refresh_and_restart() {
    round_trip(Fixture::new());
}

#[test]
fn record_storage_agent_archive_and_restore_preserve_saved_layout() {
    round_trip(Fixture::with_record_history());
}

fn round_trip(mut f: Fixture) {
    let _ = dispatch_fixture(&f);
    let shell = f.new_workspace("Retained work");
    let wid = f.snapshot().active;
    fs::write(f.root.join("uncommitted.txt"), "keep these edits\n").unwrap();
    f.req(&[
        "native",
        &wid.to_string(),
        "codex",
        "12345678-1234-1234-1234-123456789abc",
    ]);
    let native = f.snapshot().session().unwrap().clone();
    f.wait_text(&native, "DISPATCH_READY");
    f.req(&["focus", &wid.to_string(), &shell.id.to_string()]);
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.pinned = true;
    meta.status = flere::workspace::Workflow::Waiting;
    meta.notes = "Parked by user; retain work and exact conversations.".into();
    meta.issue = "https://example.test/issues/12".into();
    meta.pr = "https://example.test/pull/34".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    let expected = card(&stored(&f), wid);
    assert_eq!(expected["tabs"].as_array().unwrap().len(), 2);
    assert_eq!(expected["selected"], shell.id);
    assert_eq!(
        expected["meta"]["conversations"][0]["uuid"],
        "12345678-1234-1234-1234-123456789abc"
    );
    // A stopped supervisor restores metadata and tab plans, not child programs.
    restart(&mut f);
    let (_, actor) = dispatch_fixture(&f);
    f.send(&actor, b"DRAFT_MUST_NOT_BE_REPLAYED");
    f.wait_text(&actor, "DRAFT_MUST_NOT_BE_REPLAYED");
    let before = f.snapshot();
    let args = card_update(&f, &actor, wid, json!({}));
    let result = dispatch_call(&f, &actor, "archive_workspace", args.clone()).unwrap();
    assert_eq!(
        result,
        json!({"epoch":before.epoch,"workspace":{"id":wid,"archived":true},"changed":true})
    );
    let mut archived = expected.clone();
    archived["meta"]["archived"] = json!(true);
    assert_eq!(card(&stored(&f), wid), archived);
    assert!(dispatch_call(&f, &actor, "archive_workspace", args).is_err());
    // A fresh no-op is allowed, but never accepts an old expectation.
    assert_eq!(
        dispatch_call(
            &f,
            &actor,
            "archive_workspace",
            card_update(&f, &actor, wid, json!({}))
        )
        .unwrap()["changed"],
        false
    );
    let after = f.snapshot();
    assert_eq!((after.active, after.tab), (before.active, before.tab));
    assert_eq!(after.session().unwrap().run, actor.run);
    assert_eq!(after.session().unwrap().pid, actor.pid);
    assert!(
        after
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .unwrap()
            .tabs
            .is_empty()
    );
    // Refresh retains the archived tab plan and the actor's exact live run.
    let restore = card_update(&f, &actor, wid, json!({}));
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
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(card(&stored(&f), wid), archived);
    assert_eq!(f.snapshot().session().unwrap().run, actor.run);
    assert_eq!(
        dispatch_call(&f, &actor, "restore_workspace", restore.clone()).unwrap()["workspace"]["archived"],
        false
    );
    assert!(dispatch_call(&f, &actor, "restore_workspace", restore).is_err());
    assert_eq!(card(&stored(&f), wid), expected);
    assert!(f.capture(&actor).contains("DRAFT_MUST_NOT_BE_REPLAYED"));
    assert_eq!(
        (f.snapshot().active, f.snapshot().tab),
        (before.active, before.tab)
    );
    assert!(
        f.snapshot()
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .unwrap()
            .tabs
            .is_empty()
    );
    assert_eq!(
        fs::read_to_string(f.root.join("uncommitted.txt")).unwrap(),
        "keep these edits\n"
    );
    let stale_epoch = card_update(&f, &actor, wid, json!({}));
    restart(&mut f);
    assert_eq!(card(&stored(&f), wid), expected);
    assert!(dispatch_call(&f, &actor, "archive_workspace", stale_epoch.clone()).is_err());
    let (_, actor) = dispatch_fixture(&f);
    assert!(
        dispatch_call(&f, &actor, "archive_workspace", stale_epoch)
            .unwrap_err()
            .to_string()
            .contains("stale workspace epoch")
    );
    // Leave the card archived across another cold restart as well.
    dispatch_call(
        &f,
        &actor,
        "archive_workspace",
        card_update(&f, &actor, wid, json!({})),
    )
    .unwrap();
    restart(&mut f);
    assert_eq!(card(&stored(&f), wid), archived);
}

#[test]
fn agent_archive_rejects_live_terminals_and_stale_or_malformed_requests() {
    let f = Fixture::new();
    let (actor_wid, actor) = dispatch_fixture(&f);
    let shell = f.new_workspace("Live draft");
    let wid = f.snapshot().active;
    f.send(&shell, b"printf 'ARCHIVE_%s\\n' DRAFT");
    f.wait_text(&shell, "ARCHIVE_%s");
    for id in [wid, actor_wid] {
        let error = dispatch_call(
            &f,
            &actor,
            "archive_workspace",
            card_update(&f, &actor, id, json!({})),
        )
        .unwrap_err();
        assert!(error.to_string().contains("close live terminals"));
    }
    assert!(f.snapshot().session().unwrap().alive);
    assert!(f.capture(&shell).contains("ARCHIVE_%s"));
    assert!(!f.capture(&shell).contains("ARCHIVE_DRAFT"));
    let target = stopped_card(&f);
    let args = card_update(&f, &actor, target, json!({}));
    let durable = stored(&f);
    for op in ["archive_workspace", "restore_workspace"] {
        for field in ["workspace", "expected_epoch", "expected"] {
            let mut missing = args.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(dispatch_call(&f, &actor, op, missing).is_err());
            let mut null = args.clone();
            null[field] = Value::Null;
            assert!(dispatch_call(&f, &actor, op, null).is_err());
        }
        for wrong in [
            json!({"workspace":true}),
            json!({"workspace":u64::MAX}),
            json!({"expected_epoch":"stale"}),
            json!({"expected":{}}),
            json!({"expected":{"name":"Retained work","meta":{}}}),
            json!({"expected":{"name":"Retained work","meta":{},"extra":true}}),
            json!({"archived":true}),
            json!({"status":"done"}),
            json!({"terminate":true}),
        ] {
            let mut bad = args.clone();
            bad.as_object_mut()
                .unwrap()
                .extend(wrong.as_object().unwrap().clone());
            assert!(dispatch_call(&f, &actor, op, bad).is_err());
        }
        for run in [
            shell.clone(),
            TabView {
                run: "0".repeat(32),
                ..actor.clone()
            },
        ] {
            assert!(dispatch_call(&f, &run, op, args.clone()).is_err());
        }
    }
    assert_eq!(stored(&f), durable);
    f.req(&["rename", &target.to_string(), &wire::hex(b"New human name")]);
    assert!(
        dispatch_call(&f, &actor, "archive_workspace", args)
            .unwrap_err()
            .to_string()
            .contains("stale workspace metadata")
    );
    let args = card_update(&f, &actor, target, json!({}));
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.notes = "Fresh human decision".into();
    f.req(&[
        "metadata",
        &target.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    assert!(
        dispatch_call(&f, &actor, "archive_workspace", args)
            .unwrap_err()
            .to_string()
            .contains("stale workspace metadata")
    );
    // A terminal opened after the observation must also block the old request.
    let args = card_update(&f, &actor, target, json!({}));
    f.req(&["tab", &target.to_string()]);
    assert!(
        dispatch_call(&f, &actor, "archive_workspace", args)
            .unwrap_err()
            .to_string()
            .contains("close live terminals")
    );
}

#[test]
fn agent_archive_waits_for_child_exit_even_after_pty_eof() {
    let f = Fixture::new();
    let (_, actor) = dispatch_fixture(&f);
    let shell = f.new_workspace("Closed PTY with live child");
    let wid = f.snapshot().active;
    let args = card_update(&f, &actor, wid, json!({}));
    fs::write(
        f.root.join("closed-pty.py"),
        "import os,time\nfrom pathlib import Path\nfor fd in (0,1,2): os.close(fd)\nwhile not Path('allow-child-exit').exists(): time.sleep(0.01)\n",
    ).unwrap();
    f.send(&shell, b"exec python3 closed-pty.py\r");
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = f.snapshot();
        let tab = snapshot.session().expect("child must still be running");
        if !tab.alive {
            break;
        }
        assert!(Instant::now() < end, "PTY never closed");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        dispatch_call(&f, &actor, "archive_workspace", args)
            .unwrap_err()
            .to_string()
            .contains("close live terminals")
    );
    fs::write(f.root.join("allow-child-exit"), "exit now").unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    while !f.snapshot().workspace().unwrap().tabs.is_empty() {
        assert!(Instant::now() < end, "child never exited");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        dispatch_call(
            &f,
            &actor,
            "archive_workspace",
            card_update(&f, &actor, wid, json!({}))
        )
        .unwrap()["changed"],
        true
    );
}

#[test]
fn agent_archive_and_restore_roll_back_failed_saves() {
    let f = Fixture::new();
    let (_, actor) = dispatch_fixture(&f);
    let wid = stopped_card(&f);
    for op in ["archive_workspace", "restore_workspace"] {
        let args = card_update(&f, &actor, wid, json!({}));
        let before = f.snapshot();
        let durable = stored(&f);
        let store = f.state.join("workspaces.v2.json");
        let retained = f.state.join("retained-workspaces.json");
        fs::rename(&store, &retained).unwrap();
        fs::create_dir(&store).unwrap();
        assert!(dispatch_call(&f, &actor, op, args.clone()).is_err());
        let after = f.snapshot();
        assert_eq!(
            after.workspace().unwrap().meta.archived,
            before.workspace().unwrap().meta.archived
        );
        assert_eq!((after.active, after.tab), (before.active, before.tab));
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&retained).unwrap()).unwrap(),
            durable
        );
        fs::remove_dir(&store).unwrap();
        fs::rename(&retained, &store).unwrap();
        assert_eq!(
            dispatch_call(&f, &actor, op, args).unwrap()["changed"],
            true
        );
    }
}

#[test]
fn agent_archive_mcp_discovery_and_scoped_cli_work_without_a_new_chat() {
    let f = Fixture::new();
    let (_, actor) = dispatch_fixture(&f);
    let wid = stopped_card(&f);
    let args = card_update(&f, &actor, wid, json!({}));
    let mut child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "mcp"])
        .env("FLERE_SESSION", actor.id.to_string())
        .env("FLERE_RUN", &actor.run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in [
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"archive_workspace","arguments":args}}),
    ] {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    for name in ["archive_workspace", "restore_workspace"] {
        let schema = &responses[0]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap()["inputSchema"];
        assert_eq!(
            schema["required"],
            json!(["workspace", "expected_epoch", "expected"])
        );
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["properties"]["expected"]["required"],
            json!(["name", "meta"])
        );
    }
    let result: Value = serde_json::from_str(
        responses[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(result["workspace"], json!({"id":wid,"archived":true}));
    let args = card_update(&f, &actor, wid, json!({}));
    let output = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "agent-call",
            "restore_workspace",
            &args.to_string(),
        ])
        .env("FLERE_SESSION", actor.id.to_string())
        .env("FLERE_RUN", &actor.run)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["workspace"], json!({"id":wid,"archived":false}));
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
}
