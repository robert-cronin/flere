use super::*;
use flere::editor::{self, cache};
use serde_json::{Value, json};

fn aged(path: &std::path::Path) {
    let manifest = path.parent().unwrap().join("manifest.json");
    let mut v: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    v["unused_since"] = json!(1);
    v["checked_at"] = json!(1);
    flere::workspace::atomic_write(&manifest, &serde_json::to_vec(&v).unwrap()).unwrap();
}
fn sweep(f: &Fixture) -> Value {
    let status = f.state.join(cache::DIRECTORY).join("status.json");
    let _ = fs::remove_file(&status);
    let mut cleaner = cache::Cleaner::new(&f.state);
    cleaner.tick();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = fs::read(&status) {
            return serde_json::from_slice(&bytes).unwrap();
        }
        assert!(Instant::now() < end, "cache worker did not report");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn restart(f: &mut Fixture, editor: &str) {
    f.stop();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "serve"])
        .env("HOME", &f.root)
        .env("SHELL", "/bin/sh")
        .env("VISUAL", editor)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    f.ready();
}
fn restore(f: &Fixture) -> Vec<Value> {
    let epoch = f.snapshot().epoch;
    let mut results = Vec::new();
    for _ in 0..5 {
        let value: Value = serde_json::from_slice(&f.req(&["restore-next", &epoch])).unwrap();
        let done = value["remaining"] == 0;
        results.push(value);
        if done {
            return results;
        }
    }
    panic!("bounded restore did not finish");
}

#[test]
fn managed_pair_survives_refresh_cold_pending_failed_and_successful_editor_restore() {
    let mut f = Fixture::new();
    let shell = f.new_workspace("Retained diff");
    let wid = f.snapshot().active;
    f.send(&shell, b"printf 'CACHE_%s\\n' DRAFT");
    f.wait_text(&shell, "CACHE_%s");
    let comparison = editor::managed_comparison(
        &f.state,
        &flere::git::versions::Versions {
            before: b"fn before() {}\n".to_vec(),
            after: b"fn after() {}\n".to_vec(),
            before_label: "HEAD".into(),
            after_label: "working".into(),
            name: "sample.rs".into(),
        },
    )
    .unwrap();
    let paths = [comparison.before.clone(), comparison.after.clone()];
    f.req(&[
        "open-diff-epoch",
        &f.snapshot().epoch,
        &wid.to_string(),
        &wire::hex(paths[0].to_str().unwrap().as_bytes()),
        &wire::hex(paths[1].to_str().unwrap().as_bytes()),
    ]);
    let editor = f.snapshot().session().unwrap().clone();
    f.wait_text(&editor, "fn before()");
    drop(comparison);
    aged(&paths[1]);
    assert_eq!(sweep(&f)["referenced"], 1);
    assert!(paths.iter().all(|p| p.is_file()));
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
    let current = f.snapshot();
    assert_eq!(current.epoch, epoch);
    for old in [&shell, &editor] {
        let same = current
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .find(|t| t.id == old.id)
            .unwrap();
        assert_eq!((&same.run, same.pid), (&old.run, old.pid));
    }
    assert!(f.capture(&shell).contains("CACHE_%s"));
    assert!(!f.capture(&shell).contains("CACHE_DRAFT"));
    restart(&mut f, "/nonexistent-flere-test-editor");
    assert!(f.snapshot().workspace().unwrap().tabs.is_empty());
    aged(&paths[1]);
    assert_eq!(sweep(&f)["referenced"], 1); // Pending tabs protect both sides before any UI starts.
    let results = restore(&f);
    assert!(
        results
            .iter()
            .any(|v| v.to_string().contains("Could not reopen"))
    );
    assert!(
        f.snapshot()
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .all(|t| t.kind != "editor")
    );
    aged(&paths[1]);
    assert_eq!(sweep(&f)["referenced"], 1); // Failed restoration remains saved.
    restart(&mut f, "/usr/bin/vim -Nu NONE -n --noplugin");
    restore(&f);
    let current = f.snapshot();
    let restored = current
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.kind == "editor")
        .unwrap();
    assert_eq!(restored.path, paths[1].to_str().unwrap());
    assert_ne!(restored.run, editor.run);
    f.wait_text(restored, "fn after()");
    assert!(paths.iter().all(|p| p.is_file()));
    // Cold restore promises the editor file, while the manifest still pins its pair.
    let args = os::process_arguments(restored.pid).unwrap();
    assert!(!args.split(|b| *b == 0).any(|arg| arg == b"-d"));
}

#[test]
fn failed_editor_checkpoint_keeps_independent_live_pin_then_saved_path_protection() {
    let f = Fixture::new();
    f.new_workspace("Save failure");
    let document = editor::managed_diff_file(&f.state, "retained patch\n", "checkpoint").unwrap();
    let path = document.path.clone();
    let store = f.state.join("workspaces.v2.json");
    let backup = f.state.join("saved-checkpoint-backup");
    fs::rename(&store, &backup).unwrap();
    fs::create_dir(&store).unwrap(); // Atomic checkpoint rename must fail after the child starts.
    let result = wire::request(
        &f.state,
        &[
            "open",
            &f.snapshot().active.to_string(),
            &wire::hex(path.to_str().unwrap().as_bytes()),
        ],
    );
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Tabs remain open, but their layout could not be saved")
    );
    drop(document);
    let snapshot = f.snapshot();
    let live = snapshot
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.kind == "editor")
        .unwrap();
    assert!(live.alive);
    assert_eq!(live.path, path.to_str().unwrap());
    f.wait_text(live, "retained patch");
    let markers = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("lease-"))
        .count();
    assert!(
        markers >= 1,
        "failed save must retain a supervisor pin after UI lease is dropped"
    );
    aged(&path);
    assert_eq!(sweep(&f)["uncertain"], 1); // Invalid checkpoint is never interpreted as zero references.
    assert!(path.is_file());
    // Remove the failure atomically, then let the supervisor write its current
    // layout. Restoring the old backup could overwrite a successful background
    // checkpoint between filesystem calls, after its live pin was released.
    fs::rename(&store, f.state.join("failed-checkpoint-directory")).unwrap();
    f.req(&["save-tabs"]);
    let current = f.snapshot();
    let same = current
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.id == live.id)
        .expect("the failed checkpoint must leave the editor open");
    assert!(same.alive);
    assert_eq!(
        (&same.run, same.pid, &same.path),
        (&live.run, live.pid, &live.path)
    );
    let saved: Value = serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
    assert!(
        saved["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|w| w["tabs"].as_array().unwrap())
            .any(|t| t["kind"] == "editor" && t["path"] == path.to_str().unwrap()),
        "the repaired checkpoint must durably protect the live editor path"
    );
    aged(&path);
    let report = sweep(&f);
    assert_eq!(report["referenced"], 1);
    assert_eq!(report["reservation_markers"], 0);
    f.req(&["close", &live.id.to_string(), &live.run]);
    let end = Instant::now() + Duration::from_secs(3);
    while f
        .snapshot()
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .any(|t| t.id == live.id)
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    f.req(&["save-tabs"]);
    let report = sweep(&f);
    assert_eq!(report["awaiting_retention"], 1); // Closing starts a fresh grace period.
    assert!(path.is_file());
    aged(&path); // Advance only the disposable entry's policy clock.
    assert_eq!(sweep(&f)["reclaimed_entries"], 1);
    assert!(!path.exists());
}

#[test]
fn closing_editor_with_a_live_pty_keeps_its_snapshot_until_eof() {
    retained_editor(false);
}

// The macOS fixture reaches EOF promptly after its session leader exits. Keep
// the detached-descendant variant for Linux alongside the portable close case;
// compiling it on macOS is not Linux runtime acceptance evidence.
#[cfg(target_os = "linux")]
#[test]
fn exited_editor_with_a_live_descendant_pty_keeps_its_snapshot_until_eof() {
    retained_editor(true);
}

fn retained_editor(parent_exits: bool) {
    let f = Fixture::with_editor("/usr/bin/python3");
    f.new_workspace("Editor output tail");
    let release = f.root.join("release-editor-tail");
    // A closing/exited editor may be omitted from cold restore before its PTY
    // reaches EOF. The snapshot must remain valid for that retained Session.
    let fork = if parent_exits {
        "r,w=os.pipe()\nif os.fork():\n os.close(w)\n os.read(r,1)\n os._exit(0)\nos.close(r)\nos.setsid()\nos.write(w,b'x')\nos.close(w)\n"
    } else {
        ""
    };
    let script = format!(
        "import os,signal,time\nsignal.signal(signal.SIGHUP,signal.SIG_IGN)\n{fork}os.write(1,b'EDITOR_TAIL_READY\\n')\nend=time.monotonic()+20\nwhile not os.path.exists({:?}) and time.monotonic()<end: time.sleep(.02)\n",
        release.to_str().unwrap()
    );
    let document = editor::managed_diff_file(&f.state, &script, "tail-fixture").unwrap();
    let path = document.path.clone();
    f.req(&[
        "open",
        &f.snapshot().active.to_string(),
        &wire::hex(path.to_str().unwrap().as_bytes()),
    ]);
    let tab = f.snapshot().session().unwrap().clone();
    f.wait_text(&tab, "EDITOR_TAIL_READY");
    drop(document);
    if !parent_exits {
        f.req(&["close", &tab.id.to_string(), &tab.run]);
    }
    let store = f.state.join("workspaces.v2.json");
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        f.req(&["save-tabs"]);
        let saved: Value = serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
        if !saved["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["kind"] == "editor")
        {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        f.snapshot()
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.id == tab.id)
    );
    aged(&path);
    let report = sweep(&f);
    assert_eq!(report["reserved"], 1);
    assert!(path.is_file());
    fs::write(&release, b"release").unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while f
        .snapshot()
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .any(|t| t.id == tab.id)
    {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    f.req(&["save-tabs"]);
    assert_eq!(sweep(&f)["awaiting_retention"], 1);
    aged(&path);
    assert_eq!(sweep(&f)["reclaimed_entries"], 1);
}
