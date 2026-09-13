use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct Fixture {
    root: PathBuf,
    state: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".cache/flere/tests");
        fs::create_dir_all(&base).unwrap();
        let root = base.join(format!("gc-{}", &os::nonce().unwrap()[..12]));
        directory(&root, true).unwrap();
        let state = root.join("s");
        directory(&state, true).unwrap();
        let f = Self { root, state };
        f.saved(&[]);
        f
    }
    fn saved(&self, paths: &[&Path]) {
        write_json(
            &self.state.join("workspaces.v2.json"),
            &serde_json::json!({
                "version":7,"workspaces":[{"id":1,"tabs":paths.iter().map(|path|
                    serde_json::json!({"kind":"editor","path":path})).collect::<Vec<_>>()}]
            }),
        )
        .unwrap();
    }
    fn entry(&self, name: &str) -> (Vec<PathBuf>, Lease) {
        create(
            &self.state,
            name,
            &[
                ("before-file.rs", b"before\n"),
                ("after-file.rs", b"after\n"),
            ],
        )
        .unwrap()
    }
    fn sweep(&self, path: &Path, time: u64) -> io::Result<Report> {
        let ns = Namespace::open(&self.state, false)?;
        let mut report = Report::new(time);
        visit(&ns, path.parent().unwrap(), time, &mut report)?;
        Ok(report)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn complete_pairs_follow_last_saved_reference_and_full_unused_deadline() {
    let f = Fixture::new();
    let (paths, lease) = f.entry("pair");
    let start = now();
    f.saved(&[&paths[1]]); // Cold/pending/failed restoration only persists the after-file.
    drop(lease);
    assert_eq!(
        f.sweep(&paths[0], start + 10 * RETENTION_SECONDS)
            .unwrap()
            .referenced,
        1
    );
    assert!(paths.iter().all(|p| p.exists()));
    f.saved(&[]);
    let released = start + 11 * RETENTION_SECONDS;
    assert_eq!(f.sweep(&paths[0], released).unwrap().awaiting_retention, 1);
    assert_eq!(
        f.sweep(&paths[0], released + RETENTION_SECONDS - 1)
            .unwrap()
            .reclaimed_entries,
        0
    );
    f.saved(&[&paths[0]]); // Either side restarts protection for the complete pair.
    f.sweep(&paths[0], released + RETENTION_SECONDS).unwrap();
    f.saved(&[]);
    let closed = released + 2 * RETENTION_SECONDS;
    f.sweep(&paths[0], closed).unwrap();
    let report = f.sweep(&paths[0], closed + RETENTION_SECONDS).unwrap();
    assert_eq!((report.reclaimed_entries, report.reclaimed_bytes), (1, 13));
    assert!(paths.iter().all(|p| !p.exists()));
}

#[test]
fn ui_and_server_reservations_bridge_failed_checkpoint_and_stale_results() {
    let f = Fixture::new();
    let (paths, ui) = f.entry("pending");
    let server = reserve_open(&f.state, &paths[1]).unwrap().unwrap();
    // The UI can discard a stale result or receive a save error after the editor
    // started. Its lease cannot release the independent supervisor reservation.
    drop(ui);
    let future = now() + 10 * RETENTION_SECONDS;
    let report = f.sweep(&paths[1], future).unwrap();
    assert_eq!((report.reserved, report.reservation_markers), (1, 1));
    fs::write(f.state.join("workspaces.v2.json"), b"broken checkpoint").unwrap();
    assert!(f.sweep(&paths[1], future + RETENTION_SECONDS).is_err());
    assert!(paths.iter().all(|p| p.exists()));
    f.saved(&[&paths[1]]);
    server.release(); // Only after the successful durable checkpoint.
    assert_eq!(
        f.sweep(&paths[1], future + RETENTION_SECONDS)
            .unwrap()
            .referenced,
        1
    );
    let crash_pin = reserve_open(&f.state, &paths[1]).unwrap().unwrap();
    let mut pin: Pin = read_json(&crash_pin.path, MAX_METADATA).unwrap();
    pin.start = "previous-process-generation".into();
    write_json(&crash_pin.path, &pin).unwrap();
    drop(crash_pin); // Exec/crash/uncertain outcome deliberately retains its marker.
    f.saved(&[]);
    let report = f.sweep(&paths[1], future + 2 * RETENTION_SECONDS).unwrap();
    assert_eq!(
        (report.reserved, report.unconfirmed_reservation_markers),
        (1, 1)
    );
}

#[test]
fn entry_lock_orders_new_open_against_cleanup_and_latest_store_read() {
    let f = Fixture::new();
    let (paths, ui) = f.entry("race");
    drop(ui);
    let start = now();
    f.sweep(&paths[0], start).unwrap();
    let guard = lock(&paths[0].parent().unwrap().join(".lock"), false).unwrap();
    let state = f.state.clone();
    let path = paths[0].clone();
    let kind = std::thread::spawn(move || reserve_open(&state, &path).err().unwrap().kind())
        .join()
        .unwrap();
    assert_eq!(kind, io::ErrorKind::WouldBlock); // It cannot register/open behind an active deletion.
    drop(guard);
    let server = reserve_open(&f.state, &paths[0]).unwrap().unwrap();
    assert_eq!(
        f.sweep(&paths[0], start + RETENTION_SECONDS)
            .unwrap()
            .reserved,
        1
    );
    // A completed checkpoint between sweeps is read fresh, not from a queued snapshot.
    f.saved(&[&paths[0]]);
    server.release();
    assert_eq!(
        f.sweep(&paths[0], start + 2 * RETENTION_SECONDS)
            .unwrap()
            .referenced,
        1
    );
}

#[test]
fn isolated_states_and_foreign_references_never_authorize_cross_state_cleanup() {
    let a = Fixture::new();
    let b = Fixture::new();
    let (ap, al) = a.entry("same");
    let (bp, bl) = b.entry("same");
    assert_ne!(ap, bp);
    assert!(reserve_open(&b.state, &ap[0]).unwrap().is_none()); // Durable foreign pin, no release token.
    drop(al);
    drop(bl);
    let start = now();
    a.sweep(&ap[0], start).unwrap();
    b.sweep(&bp[0], start).unwrap();
    assert_eq!(
        b.sweep(&bp[0], start + RETENTION_SECONDS)
            .unwrap()
            .reclaimed_entries,
        1
    );
    assert_eq!(
        a.sweep(&ap[0], start + RETENTION_SECONDS).unwrap().reserved,
        1
    );
    assert!(ap.iter().all(|p| p.exists()));
}

#[test]
fn changed_files_symlinks_and_unknown_manifests_are_retained() {
    let f = Fixture::new();
    for damage in [
        "content",
        "symlink",
        "schema",
        "traversal",
        "missing",
        "extra",
        "hardlink",
    ] {
        let (paths, lease) = f.entry(damage);
        drop(lease);
        let start = now();
        f.sweep(&paths[0], start).unwrap();
        let entry = paths[0].parent().unwrap();
        let ns = Namespace::open(&f.state, false).unwrap();
        let mut manifest = ns.manifest(entry).unwrap();
        let external = f.root.join(format!("source-{damage}"));
        fs::write(&external, b"source\n").unwrap();
        match damage {
            "content" => {
                fs::set_permissions(&paths[1], fs::Permissions::from_mode(0o600)).unwrap();
                fs::write(&paths[1], b"CHANGD").unwrap();
                fs::set_permissions(&paths[1], fs::Permissions::from_mode(0o400)).unwrap();
            }
            "symlink" => {
                fs::remove_file(&paths[1]).unwrap();
                symlink(&external, &paths[1]).unwrap();
            }
            "schema" => {
                manifest.schema_version = 999;
                write_json(&entry.join("manifest.json"), &manifest).unwrap();
            }
            "traversal" => {
                manifest.files[1].name = "../../source".into();
                write_json(&entry.join("manifest.json"), &manifest).unwrap();
            }
            "missing" => {
                fs::remove_file(&paths[1]).unwrap();
            }
            "extra" => {
                fs::write(entry.join("user-file"), "keep").unwrap();
            }
            "hardlink" => {
                fs::hard_link(&paths[1], f.root.join("linked")).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            f.sweep(&paths[0], start + RETENTION_SECONDS).is_err(),
            "{damage}"
        );
        assert!(
            paths[0].exists(),
            "complete pair must be validated before either file is removed: {damage}"
        );
        assert_eq!(fs::read(&external).unwrap(), b"source\n");
    }
}

#[test]
fn copied_moved_or_replaced_state_and_unreadable_store_fail_closed() {
    let f = Fixture::new();
    let (paths, lease) = f.entry("ownership");
    drop(lease);
    let start = now();
    f.sweep(&paths[0], start).unwrap();
    let owner = f.state.join(DIRECTORY).join("owner.json");
    let mut record: Owner = read_json(&owner, MAX_METADATA).unwrap();
    record.state_identity.inode += 1;
    write_json(&owner, &record).unwrap();
    assert!(f.sweep(&paths[0], start + RETENTION_SECONDS).is_err());
    record.state_identity.inode -= 1;
    record.state = f.root.join("copied-state");
    write_json(&owner, &record).unwrap();
    assert!(f.sweep(&paths[0], start + RETENTION_SECONDS).is_err());
    record.state = f.state.clone();
    write_json(&owner, &record).unwrap();
    for bad in [
        b"{}".as_slice(),
        br#"{"version":6,"workspaces":[]}"#,
        br#"{"version":99,"workspaces":[]}"#,
        br#"{"version":7,"workspaces":[{"tabs":null}]}"#,
    ] {
        fs::write(f.state.join("workspaces.v2.json"), bad).unwrap();
        assert!(f.sweep(&paths[0], start + RETENTION_SECONDS).is_err());
    }
    fs::remove_file(f.state.join("workspaces.v2.json")).unwrap();
    assert!(f.sweep(&paths[0], start + RETENTION_SECONDS).is_err());
    assert!(paths.iter().all(|p| p.exists()));
}

#[test]
fn backward_clock_restarts_grace_and_abandoned_ui_creation_can_expire() {
    let f = Fixture::new();
    let (paths, lease) = f.entry("clock");
    let start = now();
    assert_eq!(f.sweep(&paths[0], start).unwrap().reserved, 1);
    drop(lease);
    f.sweep(&paths[0], start + 100).unwrap();
    f.sweep(&paths[0], start + 1000).unwrap();
    f.sweep(&paths[0], start + 50).unwrap();
    assert_eq!(
        f.sweep(&paths[0], start + 50 + RETENTION_SECONDS - 1)
            .unwrap()
            .reclaimed_entries,
        0
    );
    assert_eq!(
        f.sweep(&paths[0], start + 50 + RETENTION_SECONDS)
            .unwrap()
            .reclaimed_entries,
        1
    );
}

#[test]
fn concurrent_frontends_reuse_content_but_keep_independent_creation_reservations() {
    let f = Fixture::new();
    let workers: Vec<_> = (0..2)
        .map(|_| {
            let state = f.state.clone();
            std::thread::spawn(move || {
                create(
                    &state,
                    "two-frontends",
                    &[("patch.diff", b"same content\n")],
                )
                .unwrap()
            })
        })
        .collect();
    let mut results: Vec<_> = workers.into_iter().map(|w| w.join().unwrap()).collect();
    assert_eq!(results[0].0, results[1].0);
    let paths = results[0].0.clone();
    let start = now();
    assert_eq!(f.sweep(&paths[0], start).unwrap().reservation_markers, 2);
    drop(results.pop());
    assert_eq!(
        f.sweep(&paths[0], start + RETENTION_SECONDS)
            .unwrap()
            .reservation_markers,
        1
    );
    drop(results);
    assert_eq!(
        f.sweep(&paths[0], start + 2 * RETENTION_SECONDS)
            .unwrap()
            .awaiting_retention,
        1
    );
    assert_eq!(
        f.sweep(&paths[0], start + 3 * RETENTION_SECONDS)
            .unwrap()
            .reclaimed_entries,
        1
    );
}

#[test]
fn background_batches_are_bounded_and_progress_beyond_the_first_page() {
    let f = Fixture::new();
    let mut paths = Vec::new();
    for i in 0..BATCH + 2 {
        let (p, lease) = create(&f.state, &format!("batch-{i}"), &[("patch.diff", b"x")]).unwrap();
        drop(lease);
        let ns = Namespace::open(&f.state, false).unwrap();
        let entry = p[0].parent().unwrap();
        let mut m = ns.manifest(entry).unwrap();
        m.unused_since = Some(1);
        m.checked_at = 1;
        write_json(&entry.join("manifest.json"), &m).unwrap();
        paths.push(p[0].clone());
    }
    let mut cleaner = Cleaner::new(&f.state);
    let status = f.state.join(DIRECTORY).join("status.json");
    for batch in 0..5 {
        let _ = fs::remove_file(&status);
        if batch == 0 {
            cleaner.tick();
        } else {
            cleaner.request.send(()).unwrap();
        }
        let end = Instant::now() + Duration::from_secs(5);
        while !status.exists() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(10));
        }
        let report: serde_json::Value = read_json(&status, MAX_METADATA).unwrap();
        assert!(report["scanned"].as_u64().unwrap() <= BATCH as u64);
        assert!(report["reclaimed_entries"].as_u64().unwrap() <= BATCH as u64);
        if batch == 0 {
            assert!(paths.iter().any(|p| p.exists()));
        }
        if paths.iter().all(|p| !p.exists()) {
            return;
        }
    }
    panic!("bounded worker did not advance to remaining entries");
}
