//! The UI updates its own process, not merely the file or supervisor.
use super::*;

mod plans;

#[test]
fn ui_update_applies_the_verified_package_and_acknowledges_the_new_frontend() {
    let f = Fixture::new();
    let tabs = splits::probes(&f, 2);
    splits::pane(&f, "split-right", Some("move"));
    f.send(&tabs[0], b"KEEP_LEFT");
    f.send(&tabs[1], b"KEEP_RIGHT");
    let directory = f.root.join("package");
    flere::install::package(
        std::path::Path::new(env!("CARGO_BIN_EXE_flere")),
        &directory,
        None,
    )
    .unwrap();
    let before = splits::panes(&f);
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("FLERE_UPDATE_ACK");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 30).unwrap();
    let mut screen = Terminal::new(160, 30);
    drain_pty(&mut master, &mut screen, "FLERE");
    master.write_all(b"\0K").unwrap();
    drain_pty(&mut master, &mut screen, "Update Flere");
    master
        .write_all(directory.to_str().unwrap().as_bytes())
        .unwrap();
    master.write_all(b"\r").unwrap();
    // This performs multiple verified copies/hashes of the full debug payload
    // plus supervisor/frontend activation; it is not an ordinary UI redraw.
    wait_current_ui_for(&mut master, &mut screen, Duration::from_secs(30), |s| {
        s.capture(100).contains("this frontend applied")
    });
    let store = flere::install::Store::new(
        f.root.join(".local/share/flere/install"),
        f.root.join(".local/bin"),
        "flere",
    )
    .unwrap();
    let receipt = store.status().unwrap().unwrap();
    assert_eq!(
        receipt.activation.as_ref().unwrap().initiating_frontend,
        "applied"
    );
    assert_eq!(receipt.activation.as_ref().unwrap().supervisor, "applied");
    assert_eq!(
        os::process_executable(child.id()).unwrap(),
        receipt.current.executable
    );
    assert_eq!(before.epoch, f.snapshot().epoch);
    let after = splits::panes(&f);
    assert_eq!(before.active, after.active);
    assert_eq!(before.tab, after.tab);
    assert_eq!(before.split.unwrap().groups, after.split.unwrap().groups);
    splits::unchanged(&f, &tabs);
    assert_eq!(splits::input(&f, &tabs[0]), b"KEEP_LEFT");
    assert_eq!(splits::input(&f, &tabs[1]), b"KEEP_RIGHT");
    master.write_all(b"AFTER_UPDATE").unwrap();
    pump_ui_bytes(&mut master, &mut screen, 150);
    assert_eq!(splits::input(&f, &tabs[1]), b"KEEP_RIGHTAFTER_UPDATE");
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn frontend_inventory_retires_detached_watchers_and_preserves_legacy_unknowns() {
    let f = Fixture::new();
    f.new_workspace("inventory");
    let mut watch = wire::connect(&f.state).unwrap();
    watch.write_all(&wire::frame(b"watch")).unwrap();
    let _ = wire::read_frame(&mut watch).unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let value: serde_json::Value = serde_json::from_slice(&f.req(&["frontends"])).unwrap();
        if value["untracked"] == 1 {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(watch);
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let value: serde_json::Value = serde_json::from_slice(&f.req(&["frontends"])).unwrap();
        if value["untracked"] == 0 {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
}
