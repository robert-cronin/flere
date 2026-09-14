use super::*;
use std::path::Path;

pub(super) fn fixture_home<'a>(command: &'a mut Command, home: &Path) -> &'a mut Command {
    // Replacing HOME alone leaves user config and installation paths inherited.
    command
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_STATE_HOME")
}

#[test]
fn remote_fixture_commands_keep_parent_config_and_data_unchanged() {
    const PROBE_ROOT: &str = "FLERE_REMOTE_CONFIG_PROBE_ROOT";
    const PROBE_BINARY: &str = "FLERE_REMOTE_CONFIG_PROBE_BINARY";
    if let Some(root) = std::env::var_os(PROBE_ROOT) {
        let root = PathBuf::from(root);
        let mut command = Command::new(std::env::var_os(PROBE_BINARY).unwrap());
        let output = fixture_home(&mut command, &root.join("fixture-home"))
            .args(["connections", "save", "fixture", "fixture-host"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = fixture_home(
            &mut Command::new(env!("CARGO_BIN_EXE_flere")),
            &root.join("fixture-home"),
        )
        .args(["dev-source", env!("CARGO_MANIFEST_DIR")])
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let root = PathBuf::from(std::env::var_os("HOME").unwrap())
        .join(".cache/flere/tests")
        .join(format!("remote-config-{}", os::nonce().unwrap()));
    let parent_home = root.join("parent-home");
    let parent_config = root.join("parent-config");
    let parent_data = root.join("parent-data");
    let fixture = root.join("fixture-home");
    let config_sentinel = br#"{"schema_version":1,"last":null,"connections":{}}"#;
    let data_sentinel = br#"{"schema_version":1,"checkout":"/parent-sentinel"}"#;
    let parent_files = [
        (
            parent_home.join(".config/flere-connect/connections.json"),
            config_sentinel.as_slice(),
        ),
        (
            parent_config.join("flere-connect/connections.json"),
            config_sentinel.as_slice(),
        ),
        (
            parent_home.join(".local/share/flere/install/local-source.json"),
            data_sentinel.as_slice(),
        ),
        (
            parent_data.join("flere/install/local-source.json"),
            data_sentinel.as_slice(),
        ),
    ];
    for (path, sentinel) in &parent_files {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(path, sentinel).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::create_dir(&fixture).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();

    // A subprocess gives the fixture a real inherited XDG override without
    // changing process-global environment while the other tests are running.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "remote::environment::remote_fixture_commands_keep_parent_config_and_data_unchanged",
            "--nocapture",
        ])
        .env("HOME", &parent_home)
        .env("XDG_CONFIG_HOME", &parent_config)
        .env("XDG_DATA_HOME", &parent_data)
        .env("XDG_CACHE_HOME", root.join("parent-cache"))
        .env("XDG_STATE_HOME", root.join("parent-state"))
        .env(PROBE_ROOT, &root)
        .env(PROBE_BINARY, companion_binary())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for (path, sentinel) in parent_files {
        assert_eq!(
            fs::read(&path).unwrap(),
            sentinel,
            "changed {}",
            path.display()
        );
    }
    let saved: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.join(".config/flere-connect/connections.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["connections"]["fixture"]["host"], "fixture-host");
    assert!(saved["last"].is_null(), "saving must not initiate SSH");
    let source: serde_json::Value = serde_json::from_slice(
        &fs::read(fixture.join(".local/share/flere/install/local-source.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(source["checkout"], env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("parent-cache").exists());
    assert!(!root.join("parent-state").exists());
    assert!(!fixture.join(".local/state").exists());
    fs::remove_dir_all(root).unwrap();
}
