use super::*;

fn prefs(f: &Fixture, op: &str, value: Option<&serde_json::Value>) -> serde_json::Value {
    let epoch = f.snapshot().epoch;
    let text = value.map(serde_json::Value::to_string);
    let mut fields = vec!["ui-preferences", op, &epoch];
    if let Some(text) = &text {
        fields.push(text);
    }
    serde_json::from_slice(&f.req(&fields)).unwrap()
}
// Rendering and queue acceptance precede disk completion. UI tests that assert
// persisted preferences must wait for the receipt, including visible save errors.
pub(super) fn durable(f: &Fixture) -> serde_json::Value {
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        let status = prefs(f, "status", None);
        assert!(status["error"].is_null(), "{status}");
        if status["pending"] == false {
            return serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
        }
        assert!(Instant::now() < end, "preference writer did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn preferences_latest_survives_refresh_and_shutdown_without_workspace_writes() {
    let mut f = Fixture::new();
    let tab = f.new_workspace("preference fixture");
    let before = fs::read(f.state.join("workspaces.v2.json")).unwrap();
    fs::write(f.state.join("ui.json"), br#"{"left":33}"#).unwrap();
    assert_eq!(prefs(&f, "get", None)["preferences"]["left"], 33);
    for i in 0..40 {
        prefs(
            &f,
            "put",
            Some(&serde_json::json!({"left":26+i,"project":format!("synthetic-{i}")})),
        );
    }
    let expected = prefs(&f, "get", None)["preferences"].clone();
    assert_eq!(expected["left"], 65);
    assert_eq!(expected["project"], "synthetic-39");
    assert_eq!(
        fs::read(f.state.join("workspaces.v2.json")).unwrap(),
        before
    );
    // Refresh must flush the queue before exec. No synthetic model is launched.
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if wire::request(&f.state, &["snapshot"]).is_ok_and(|bytes| {
            Snapshot::decode(&bytes).is_ok_and(|s| s.notice.contains("Refreshed Flere"))
        }) {
            break;
        }
        assert!(Instant::now() < until, "refresh did not complete");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(prefs(&f, "get", None)["preferences"], expected);
    let current = f.snapshot().session().unwrap().clone();
    assert_eq!((current.pid, &current.run), (tab.pid, &tab.run));
    assert_eq!(durable(&f), expected);
    let last = serde_json::json!({"left":47,"project":"last-setting"});
    prefs(&f, "put", Some(&last));
    f.stop();
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state.join("ui.json")).unwrap()).unwrap();
    assert_eq!(saved["left"], 47);
    assert_eq!(saved["project"], "last-setting");
}
#[test]
fn preferences_failure_is_visible_and_retries_exact_settings() {
    let f = Fixture::new();
    let epoch = f.snapshot().epoch;
    fs::create_dir(f.state.join("ui.json")).unwrap();
    let desired = serde_json::json!({"left":51,"zoom":true});
    prefs(&f, "put", Some(&desired));
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        let status = prefs(&f, "status", None);
        if status["pending"] == false {
            assert!(
                status["error"]
                    .as_str()
                    .unwrap()
                    .contains("Could not save UI settings")
            );
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    // A new frontend gets the retained value and the failure, not defaults.
    let retained = prefs(&f, "get", None);
    assert_eq!(retained["preferences"]["left"], 51);
    assert!(retained["error"].is_string());
    assert!(wire::request(&f.state, &["ui-preferences", "put", "stale", "{}"]).is_err());
    assert!(wire::request(&f.state, &["ui-preferences", "put", &epoch, "{bad}"]).is_err());
    let big = serde_json::json!({"project":"x".repeat(65536)}).to_string();
    assert!(wire::request(&f.state, &["ui-preferences", "put", &epoch, &big]).is_err());
    assert_eq!(
        prefs(&f, "get", None)["preferences"],
        retained["preferences"]
    );
    fs::remove_dir(f.state.join("ui.json")).unwrap();
    prefs(&f, "put", Some(&desired));
    let saved = durable(&f);
    assert_eq!(saved["left"], 51);
    assert_eq!(saved["zoom"], true);
    assert!(prefs(&f, "status", None)["error"].is_null());
    // With no pending save, legacy frontend changes remain visible on attach.
    fs::write(f.state.join("ui.json"), br#"{"left":35}"#).unwrap();
    assert_eq!(prefs(&f, "get", None)["preferences"]["left"], 35);
}

#[test]
fn preferences_attachment_never_waits_for_a_nonregular_file() {
    let f = Fixture::new();
    assert!(
        Command::new("mkfifo")
            .arg(f.state.join("ui.json"))
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(prefs(&f, "get", None)["preferences"]["left"], 26);
    fs::remove_file(f.state.join("ui.json")).unwrap();
    let external = f.root.join("unrelated-settings");
    fs::write(&external, br#"{"left":59}"#).unwrap();
    std::os::unix::fs::symlink(&external, f.state.join("ui.json")).unwrap();
    assert_eq!(prefs(&f, "get", None)["preferences"]["left"], 26);
    prefs(&f, "put", Some(&serde_json::json!({"left":42})));
    assert_eq!(durable(&f)["left"], 42);
    assert_eq!(fs::read(external).unwrap(), br#"{"left":59}"#);
}
