//! Read-only CLI and supervisor diagnostics, using disposable state and socket peers.
use super::*;
use serde_json::{Value, json};
use std::os::unix::net::UnixListener;

struct PrivateRoot(PathBuf);
impl PrivateRoot {
    fn new() -> Self {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(&os::nonce().unwrap()[..12]);
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
}
impl Drop for PrivateRoot {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retained failing build-status fixture {}", self.0.display());
        } else {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_flere"));
    command
        .current_dir(root)
        .env("HOME", root)
        .env("TMPDIR", root)
        .env_remove("FLERE")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_DATA_HOME")
        .stdin(Stdio::null());
    command
}

fn output(command: &mut Command) -> Value {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(result.stderr.is_empty(), "unexpected CLI diagnostics");
    serde_json::from_slice(&result.stdout).unwrap()
}

fn untracked(report: &Value) {
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["comparison_basis"], "embedded_metadata");
    assert_eq!(report["installation"], json!({"status": "untracked"}));
    if report["supervisor"]["status"] == "known" {
        assert_eq!(
            report["frontends"],
            json!({"schema_version":1,"status":"known","tracked":[],"untracked":0})
        );
    } else {
        assert_eq!(report["frontends"], json!({"status": "untracked"}));
    }
}

#[test]
fn build_info_without_home_is_embedded_json_and_creates_no_state() {
    let root = PrivateRoot::new();
    let info = output(command(&root.0).env_remove("HOME").arg("--build-info"));
    let embedded: Value = serde_json::from_str(flere::build_info::json()).unwrap();
    assert_eq!(info, embedded);
    assert_eq!(info["component"], "flere");
    assert_eq!(info["package_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(info["build_id"], flere::build_info::BUILD_ID);
    assert_eq!(info["target"], flere::build_info::TARGET);
    assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
}

#[test]
fn build_status_without_a_supervisor_is_unreachable_and_creates_no_directories() {
    let root = PrivateRoot::new();
    let home = root.0.join("never-created-home");
    for explicit in [false, true] {
        let state = if explicit {
            root.0.join("never-created-state/nested")
        } else {
            home.join(".local/state/flere")
        };
        let mut cmd = command(&root.0);
        cmd.env("HOME", &home);
        if explicit {
            cmd.env_remove("HOME").arg("--state").arg(&state);
        }
        let report = output(cmd.arg("build-status"));
        assert_eq!(report["state"], state.to_str().unwrap());
        assert_eq!(report["supervisor"]["status"], "unreachable");
        assert_eq!(report["comparison"], "unknown");
        untracked(&report);
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 0);
    }
}

#[test]
fn supervisor_build_status_preserves_saved_state_selection_and_native_drafts() {
    let f = Fixture::new();
    let shell = f.new_workspace("Unselected shell");
    f.send(&shell, b"UNSELECTED_SHELL_DRAFT");
    f.wait_text(&shell, "UNSELECTED_SHELL_DRAFT");
    f.new_workspace("Build identity native stand-in");
    let wid = f.snapshot().active;
    let script = f.root.join("image-native.py");
    fs::write(&script, include_str!("../fixtures/image_native.py")).unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/usr/bin/python3",script]}]))
            .unwrap(),
    )
    .unwrap();
    f.req(&["native", &wid.to_string(), "codex", ""]);
    let native = f.snapshot().session().unwrap().clone();
    f.wait_text(&native, "IMAGE_NATIVE_DRAFT");
    let draft = b"BUILD_STATUS_NATIVE_DRAFT";
    f.send(&native, draft);
    let input = f.root.join("image-input");
    let deadline = Instant::now() + Duration::from_secs(3);
    while fs::read(&input).unwrap() != draft {
        assert!(
            Instant::now() < deadline,
            "stand-in did not receive its draft"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    f.req(&["save-tabs"]); // Finish fixture setup before the read-only baseline.
    let saved_path = f.state.join("workspaces.v2.json");
    let saved = fs::read(&saved_path).unwrap();
    let modified = fs::metadata(&saved_path).unwrap().modified().unwrap();
    let snapshot = f.snapshot();
    let actions = fs::read(f.state.join("actions.log")).unwrap();
    let embedded: Value = serde_json::from_str(flere::build_info::json()).unwrap();
    let direct: Value = serde_json::from_slice(&f.req(&["build-info"])).unwrap();
    assert_eq!(direct["schema_version"], 1);
    assert_eq!(direct["build"], embedded);
    assert_eq!(direct["pid"], f.child.id());
    assert_eq!(direct["epoch"], snapshot.epoch);
    assert_eq!(direct["generation"], snapshot.generation);
    let report = output(
        command(&f.root)
            .arg("--state")
            .arg(&f.state)
            .arg("build-status"),
    );
    let info = output(command(&f.root).arg("--build-info"));
    assert_eq!(report["executable"]["build"], info);
    assert_eq!(info, embedded);
    assert_eq!(report["comparison"], "same_build");
    let mut expected_supervisor = direct;
    expected_supervisor["status"] = json!("known");
    assert_eq!(report["supervisor"], expected_supervisor);
    assert_eq!(report["state"], f.state.to_str().unwrap());
    assert_eq!(
        fs::canonicalize(report["executable"]["path"].as_str().unwrap()).unwrap(),
        fs::canonicalize(env!("CARGO_BIN_EXE_flere")).unwrap()
    );
    untracked(&report);
    assert_eq!(f.snapshot().encode(), snapshot.encode());
    assert_eq!(fs::read(&saved_path).unwrap(), saved);
    assert_eq!(
        fs::metadata(&saved_path).unwrap().modified().unwrap(),
        modified
    );
    assert_eq!(fs::read(f.state.join("actions.log")).unwrap(), actions);
    assert_eq!(fs::read(&input).unwrap(), draft);
    assert!(f.capture(&native).contains("IMAGE_NATIVE_DRAFT"));
    assert!(f.capture(&shell).contains("UNSELECTED_SHELL_DRAFT"));
}

#[test]
fn old_and_malformed_supervisors_remain_unknown_without_claiming_installation_state() {
    let root = PrivateRoot::new();
    let state = root.0.join("mock");
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    let mut unsupported: Value = serde_json::from_str(flere::build_info::json()).unwrap();
    unsupported["schema_version"] = json!(2);
    let responses = [
        b"!unknown command".to_vec(),
        b"{not valid JSON".to_vec(),
        serde_json::to_vec(&json!({
            "schema_version": 1, "pid": 123, "epoch": "a".repeat(32),
            "generation": 0, "build": unsupported,
        })).unwrap(),
        serde_json::to_vec(&json!({
            "schema_version": 2, "pid": 123, "epoch": "a".repeat(32),
            "generation": 0, "build": serde_json::from_str::<Value>(flere::build_info::json()).unwrap(),
        })).unwrap(),
        serde_json::to_vec(&json!({
            "schema_version": 1, "pid": 123, "epoch": "a".repeat(32), "generation": 0,
        })).unwrap(),
    ];
    for response in responses {
        let socket = state.join("control.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "CLI never queried mock supervisor"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("mock accept failed: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            assert_eq!(wire::read_frame(&mut stream).unwrap(), b"build-info");
            stream.write_all(&wire::frame(&response)).unwrap();
        });
        let report = output(
            command(&root.0)
                .arg("--state")
                .arg(&state)
                .arg("build-status"),
        );
        server.join().unwrap();
        assert_eq!(report["supervisor"]["status"], "unknown_build");
        assert_eq!(report["comparison"], "unknown");
        assert!(report["supervisor"]["build"].is_null());
        assert!(report["supervisor"]["pid"].is_null());
        untracked(&report);
        assert_eq!(fs::read_dir(&state).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&root.0).unwrap().count(), 1);
        fs::remove_file(socket).unwrap();
    }
}
