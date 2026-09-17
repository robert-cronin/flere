use super::*;

fn stored(f: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap()
}

fn start_supervisor(f: &mut Fixture) {
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("HOME", &f.root)
        .env("SHELL", "/bin/sh")
        .env("ENV", "")
        .env("PS1", "$ ")
        .env("TMPDIR", &f.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
    assert!(
        f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()),
        "supervisor-only startup must not reopen any program"
    );
}

fn open_ui(f: &Fixture, split: bool) -> (fs::File, os::Process, Terminal) {
    let mut cmd = outer_ui_command();
    cmd.arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root);
    let (mut master, child) = os::spawn_command_pty(&f.root, &mut cmd, 180, 42).unwrap();
    let mut screen = Terminal::new(180, 42);
    wait_current_ui(&mut master, &mut screen, |_| {
        let snapshot = panes(f);
        snapshot.workspace().is_some_and(|w| w.tabs.len() == 3) && snapshot.split.is_some() == split
    });
    (master, child, screen)
}

fn group_directories(snapshot: &Snapshot) -> Vec<Vec<PathBuf>> {
    snapshot
        .split
        .as_ref()
        .unwrap()
        .groups
        .iter()
        .map(|group| {
            group
                .tabs
                .iter()
                .map(|id| {
                    let tab = snapshot
                        .workspace()
                        .unwrap()
                        .tabs
                        .iter()
                        .find(|t| t.id == *id)
                        .unwrap();
                    os::process_cwd(tab.pid).unwrap()
                })
                .collect()
        })
        .collect()
}

#[test]
fn reopening_restores_group_order_selections_and_shell_directories_without_replaying_drafts() {
    let mut f = Fixture::new();
    let first = f.new_workspace("Reopen split shells");
    let wid = f.snapshot().active;
    let mut old = vec![first];
    for _ in 0..2 {
        f.req(&["tab", &wid.to_string()]);
        old.push(f.snapshot().session().unwrap().clone());
    }
    for (index, tab) in old.iter().enumerate() {
        let directory = f.root.join(format!("directory-{index}"));
        fs::create_dir(&directory).unwrap();
        f.send(
            tab,
            format!(
                "cd '{}'\nprintf 'DIRECTORY_READY_{index}\\n'\n",
                directory.display()
            )
            .as_bytes(),
        );
        f.wait_text(tab, &format!("DIRECTORY_READY_{index}"));
        f.send(tab, format!("UNSUBMITTED_SPLIT_DRAFT_{index}").as_bytes());
        f.wait_text(tab, &format!("UNSUBMITTED_SPLIT_DRAFT_{index}"));
    }
    resize(&f, 101, 31);
    pane(&f, "split-right", Some("move"));
    pane(&f, "ratio", Some("300"));
    pane(&f, "focus", Some("0"));
    let s = panes(&f);
    f.req(&[
        "focus-exact",
        &s.epoch,
        &wid.to_string(),
        &old[1].id.to_string(),
        &old[1].run,
    ]);
    f.req(&["save-tabs"]);
    let before = panes(&f);
    let expected_groups = group_directories(&before);
    let saved = stored(&f);
    assert_eq!(saved["version"], 10);
    assert_eq!(saved["workspaces"][0]["split"]["ratio"], 300);
    f.stop();
    start_supervisor(&mut f);
    assert_ne!(f.snapshot().epoch, before.epoch);
    assert_eq!(
        stored(&f)["workspaces"][0]["split"],
        saved["workspaces"][0]["split"]
    );
    let (mut master, mut child, mut screen) = open_ui(&f, true);
    let after = panes(&f);
    assert_eq!(group_directories(&after), expected_groups);
    let split = after.split.as_ref().unwrap();
    assert_eq!((split.ratio, split.focused), (300, 0));
    assert_eq!(
        os::process_cwd(after.session().unwrap().pid).unwrap(),
        f.root.join("directory-1")
    );
    for tab in &after.workspace().unwrap().tabs {
        assert!(
            old.iter()
                .all(|before| before.run != tab.run && before.pid != tab.pid)
        );
        assert!(!f.capture(tab).contains("UNSUBMITTED_SPLIT_DRAFT"));
    }
    let live_ids = identities(&after);
    let (mut master2, mut child2, mut screen2) = open_ui(&f, true);
    assert_eq!(identities(&panes(&f)), live_ids);
    finish_ui(&mut master2, &mut screen2, &mut child2);
    finish_ui(&mut master, &mut screen, &mut child);

    // A v6 layout has the same saved tab records but no group field. It reopens
    // one ordered group, rather than inventing a split or duplicating programs.
    f.stop();
    let mut legacy = stored(&f);
    legacy["version"] = json!(6);
    let record = &mut legacy["workspaces"][0];
    record.as_object_mut().unwrap().remove("split");
    let expected_order = record["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tab| PathBuf::from(tab["cwd"].as_str().unwrap()))
        .collect::<Vec<_>>();
    fs::write(
        f.state.join("workspaces.v2.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    start_supervisor(&mut f);
    let (mut master, mut child, mut screen) = open_ui(&f, false);
    let restored = panes(&f);
    assert!(restored.split.is_none());
    assert_eq!(
        restored
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .map(|t| os::process_cwd(t.pid).unwrap())
            .collect::<Vec<_>>(),
        expected_order
    );
    assert_eq!(stored(&f)["version"], 10);
    finish_ui(&mut master, &mut screen, &mut child);
}
