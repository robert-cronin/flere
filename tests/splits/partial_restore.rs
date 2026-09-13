use super::*;

fn stored(f: &Fixture) -> Value {
    serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap()
}
fn restart(f: &mut Fixture) {
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
    assert!(panes(f).workspace().unwrap().tabs.is_empty());
}

#[test]
fn a_failed_restore_retains_exact_group_membership_and_retry_never_reopens_survivors() {
    let mut f = Fixture::new();
    f.new_workspace("Partial split restoration");
    let wid = panes(&f).active;
    for _ in 0..2 {
        f.req(&["tab", &wid.to_string()]);
    }
    pane(&f, "split-right", Some("move"));
    f.req(&["save-tabs"]);
    f.stop();
    let mut saved = stored(&f);
    let absent = f.root.join("temporarily-unavailable-directory");
    saved["workspaces"][0]["tabs"][2]["cwd"] = json!(absent);
    let expected = saved["workspaces"][0]["split"]["groups"].clone();
    fs::write(
        f.state.join("workspaces.v2.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    restart(&mut f);
    let epoch = panes(&f).epoch;
    for _ in 0..3 {
        f.req(&["restore-next", &epoch]);
    }
    let partial = panes(&f);
    assert_eq!(partial.workspace().unwrap().tabs.len(), 2);
    assert!(
        partial.split.is_none(),
        "an unavailable whole group has no fake terminal buffer"
    );
    let survivors = identities(&partial);
    let checkpoint = stored(&f);
    assert_eq!(
        checkpoint["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(checkpoint["workspaces"][0]["split"]["groups"], expected);
    // Repeated polling must not retry a failed path or duplicate surviving shells.
    for _ in 0..3 {
        f.req(&["restore-next", &epoch]);
    }
    assert_eq!(identities(&panes(&f)), survivors);
    fs::create_dir(&absent).unwrap();
    f.req(&["restore-retry", &epoch, &wid.to_string()]);
    f.req(&["restore-next", &epoch]);
    let restored = panes(&f);
    let split = restored.split.as_ref().unwrap();
    assert_eq!(
        (split.groups[0].tabs.len(), split.groups[1].tabs.len()),
        (2, 1)
    );
    assert_eq!(split.focused, 1);
    assert_eq!(
        os::process_cwd(restored.session().unwrap().pid).unwrap(),
        absent
    );
    let all = identities(&restored);
    assert!(survivors.iter().all(|identity| all.contains(identity)));
    assert_eq!(stored(&f)["workspaces"][0]["split"]["groups"], expected);
}

#[test]
fn failed_extra_tab_keeps_both_live_panes_and_retry_respects_later_user_selection() {
    let mut f = Fixture::new();
    f.new_workspace("Partially available groups");
    let wid = panes(&f).active;
    for _ in 0..2 {
        f.req(&["tab", &wid.to_string()]);
    }
    pane(&f, "split-right", Some("move"));
    f.req(&["save-tabs"]);
    f.stop();
    let mut saved = stored(&f);
    let absent = f.root.join("missing-extra-tab-directory");
    saved["workspaces"][0]["tabs"][0]["cwd"] = json!(absent);
    let expected_tabs = saved["workspaces"][0]["split"]["groups"][0]["tabs"].clone();
    fs::write(
        f.state.join("workspaces.v2.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    restart(&mut f);
    let epoch = panes(&f).epoch;
    for _ in 0..3 {
        f.req(&["restore-next", &epoch]);
    }
    let partial = panes(&f);
    let split = partial
        .split
        .as_ref()
        .expect("both groups have a live survivor");
    assert_eq!(
        (split.groups[0].tabs.len(), split.groups[1].tabs.len()),
        (1, 1)
    );
    assert_eq!(
        stored(&f)["workspaces"][0]["split"]["groups"][0]["tabs"],
        expected_tabs
    );
    let survivors = identities(&partial);
    let selected = pane(&f, "focus", Some("0"));
    let selected_id = selected.tab;
    let selected_run = selected.session().unwrap().run.clone();
    let saved_selection = stored(&f)["workspaces"][0]["split"]["groups"][0]["selected"].clone();
    assert_ne!(
        saved_selection,
        saved["workspaces"][0]["split"]["groups"][0]["selected"]
    );
    fs::create_dir(&absent).unwrap();
    f.req(&["restore-retry", &epoch, &wid.to_string()]);
    f.req(&["restore-next", &epoch]);
    let restored = panes(&f);
    assert_eq!(restored.tab, selected_id);
    assert_eq!(restored.session().unwrap().run, selected_run);
    assert_eq!(restored.split.as_ref().unwrap().focused, 0);
    assert_eq!(restored.split.as_ref().unwrap().groups[0].tabs.len(), 2);
    assert!(
        survivors
            .iter()
            .all(|id| identities(&restored).contains(id))
    );
    assert_eq!(
        stored(&f)["workspaces"][0]["split"]["groups"][0]["tabs"],
        expected_tabs
    );
}
