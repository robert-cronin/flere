use super::*;
use serde_json::json;

fn hex(value: &str) -> String {
    value.bytes().map(|b| format!("{b:02x}")).collect()
}

fn response(executable: &str, endpoint: &str, receipt: &str) -> Vec<u8> {
    let mut result = "FLERE-BOOTSTRAP-1\n".to_owned();
    for (key, value) in [
        ("os", "Linux"),
        ("arch", "x86_64"),
        ("home", "/home/test"),
        ("cache", "/home/test/.cache/flere/bootstrap"),
        ("state", "/home/test/.local/state/flere"),
        ("state_presence", "directory"),
        ("endpoint", endpoint),
        ("receipt", receipt),
        ("executable", executable),
        ("legacy_executable", "/home/test/.local/bin/railhand"),
    ] {
        result.push_str(&format!("{key}\t{}\n", hex(value)));
    }
    result.push_str("END\n");
    result.into_bytes()
}

fn builds() -> (BuildMetadata, BuildMetadata) {
    let mut companion: Value = serde_json::from_str(crate::build_info::json()).unwrap();
    companion["component"] = json!("flere-connect");
    let companion: BuildMetadata = serde_json::from_value(companion).unwrap();
    let protocol = companion.compatibility.remote_protocol.current.clone();
    let mut core = serde_json::to_value(&companion).unwrap();
    core["component"] = json!(COMPONENT);
    core["target"] = json!("x86_64-unknown-linux-gnu");
    core["compatibility"] = json!({
        "control_identity":"test-control-v1",
        "snapshot":{"current":5,"read_min":1,"read_max":5},
        "refresh_handoff":{"current":6,"read_min":1,"read_max":6},
        "saved_state":{"current":7,"read_min":2,"read_max":7},
        "remote_protocol":{"current":protocol,"accepts":[protocol]}
    });
    let core: BuildMetadata = serde_json::from_value(core).unwrap();
    core.validate().unwrap();
    companion.validate().unwrap();
    (companion, core)
}

#[test]
fn probe_requires_exact_bounded_records_and_unambiguous_paths() {
    let valid = response("/home/test/.local/bin/flere", "present", "present");
    let observed = parse_probe(&valid).unwrap();
    assert_eq!(observed.target, "x86_64-unknown-linux-gnu");
    assert_eq!(observed.state, "/home/test/.local/state/flere");
    assert!(observed.endpoint);
    assert_eq!(
        observed.legacy_executable.as_deref(),
        Some("/home/test/.local/bin/railhand")
    );
    let text = String::from_utf8(valid).unwrap();
    for invalid in [
        format!("banner\n{text}"),
        format!("{text}trailer\n"),
        text.replace("arch\t", "os\t"),
        text.replace(&hex("/home/test"), &hex("/home/test/../foreign")),
        text.replace(&hex("directory"), &hex("unsafe")),
        text.replace(&hex("x86_64"), &hex("aarch64")),
        text.replace(&hex("present"), &hex("unknown")),
        text.replace(&hex("/home/test"), "zz"),
        text.replace(&hex("/home/test"), &hex("/home/test\ncommand")),
    ] {
        assert!(
            parse_probe(invalid.as_bytes()).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert!(parse_probe(&vec![b'x'; MAX_JSON + 1]).is_err());
}

#[test]
fn automatic_install_requires_missing_command_receipt_and_endpoint() {
    let (companion, _) = builds();
    let missing = parse_probe(&response("", "absent", "absent")).unwrap();
    assert_eq!(
        decide(&missing, None, &companion, Selection::Automatic),
        Decision::Install
    );
    assert_eq!(
        decide(&missing, None, &companion, Selection::ExplicitExecutable),
        Decision::Refuse
    );
    for (executable, endpoint, receipt) in [
        ("", "present", "absent"),
        ("", "absent", "present"),
        ("/home/test/.local/bin/flere", "absent", "absent"),
    ] {
        let probe = parse_probe(&response(executable, endpoint, receipt)).unwrap();
        assert_eq!(
            decide(&probe, None, &companion, Selection::Automatic),
            Decision::Refuse
        );
    }
}

#[test]
fn live_compatibility_is_decided_from_runtime_and_bridge_not_version_text() {
    let (companion, core) = builds();
    let probe = parse_probe(&response(
        "/home/test/.local/bin/flere",
        "present",
        "present",
    ))
    .unwrap();
    let mut inspection = Inspection {
        bridge: core.clone(),
        runtime: Some(Runtime {
            pid: 7,
            epoch: "a".repeat(32),
            build: core,
        }),
    };
    inspection.bridge.package_version = "different-version".into();
    assert_eq!(
        decide(&probe, Some(&inspection), &companion, Selection::Automatic),
        Decision::Attach
    );
    inspection
        .runtime
        .as_mut()
        .unwrap()
        .build
        .compatibility
        .snapshot
        .as_mut()
        .unwrap()
        .current = 6;
    assert_eq!(
        decide(&probe, Some(&inspection), &companion, Selection::Automatic),
        Decision::Update
    );
    inspection.runtime = None;
    assert_eq!(
        decide(&probe, Some(&inspection), &companion, Selection::Automatic),
        Decision::Start
    );
    inspection.bridge.compatibility.remote_protocol.current = "future-incompatible".into();
    assert_eq!(
        decide(&probe, Some(&inspection), &companion, Selection::Automatic),
        Decision::Update
    );
}

#[test]
fn embedded_shell_scripts_accept_windows_checkout_line_endings() {
    for source in [
        include_str!("probe.sh"),
        include_str!("receive.sh"),
        include_str!("cleanup.sh"),
    ] {
        let unix = source.replace("\r\n", "\n");
        let windows = unix.replace('\n', "\r\n");
        let arguments = ["/home/a b/%!&^/it's literal", ""];
        let command = transport::script(&windows, &arguments).unwrap();
        assert!(!command.contains('\r'));
        assert_eq!(command, transport::script(&unix, &arguments).unwrap());
    }
    // Normalization is only for the embedded script, never caller arguments.
    assert!(transport::script("set -eu\r\n", &["bad\r\nargument"]).is_err());
}

#[test]
fn shell_arguments_are_literal_and_remote_paths_are_platform_independent() {
    let command = transport::command(
        "/home/a b/flere",
        &["--state", "/home/a'$(touch forbidden)/state"],
    )
    .unwrap();
    assert_eq!(
        command,
        "exec '/home/a b/flere' '--state' '/home/a'\\''$(touch forbidden)/state'"
    );
    assert!(transport::command("flere", &["bad\nargument"]).is_err());
    absolute("/home/a b/state").unwrap();
    assert!(absolute("C:\\Users\\state").is_err());
    assert!(absolute("/home/a/../b").is_err());
    assert!(package::https("https://github.com/robert-cronin/flere/releases/latest/download/flere-target.manifest.json").is_ok());
    for url in [
        "http://example/manifest.json",
        "https:///manifest.json",
        "https://user@example/manifest.json",
        "https://example/manifest.json?token=secret",
    ] {
        assert!(package::https(url).is_err());
    }
}

#[test]
fn cancellation_prevents_even_discovery() {
    let request = Request::new(
        Connection {
            host: "example".into(),
            remote: "flere".into(),
            remote_explicit: false,
            state: None,
            ssh: "must-not-be-executed".into(),
        },
        Selection::Automatic,
        Source::DefaultChannel,
    );
    request.cancellation.cancel();
    assert_eq!(
        prepare(&request).err().unwrap().kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn source_arguments_respect_profile_prefixes_and_never_enable_direct_bootstrap() {
    let args = |values: &[&str]| values.iter().map(|s| (*s).into()).collect::<Vec<String>>();
    for (input, retained) in [
        (
            args(&[
                "ssh",
                "host",
                "--package",
                "/local/package",
                "--state",
                "/remote/state",
            ]),
            args(&["host", "--state", "/remote/state"]),
        ),
        (
            args(&[
                "ssh",
                "--connection",
                "saved",
                "--package",
                "/local/package",
            ]),
            args(&["--connection", "saved"]),
        ),
        (
            args(&["ssh", "reconnect", "saved", "--package", "/local/package"]),
            args(&["reconnect", "saved"]),
        ),
        (
            args(&["ssh", "reconnect", "--package", "/local/package"]),
            args(&["reconnect"]),
        ),
    ] {
        let mut input = input;
        assert_eq!(
            arguments(&mut input).unwrap(),
            Some(Source::LocalPackage("/local/package".into()))
        );
        assert_eq!(input, retained);
    }
    let mut direct = args(&["host", "--package", "/local/package"]);
    let unchanged = direct.clone();
    assert_eq!(arguments(&mut direct).unwrap(), None);
    assert_eq!(direct, unchanged);
    for input in [
        args(&["ssh", "host", "--package"]),
        args(&["ssh", "host", "--package", "p", "--from-url", "url"]),
    ] {
        assert!(arguments(&mut input.clone()).is_err());
    }
    assert!(help(&args(&["ssh"])));
    assert!(help(&args(&["ssh", "--help"])));
    assert!(!help(&args(&["ssh", "host"])));
    let mut url = args(&[
        "ssh",
        "--connection",
        "saved",
        "--from-url",
        "https://example/manifest.json",
    ]);
    assert_eq!(
        arguments(&mut url).unwrap(),
        Some(Source::HttpsManifest(
            "https://example/manifest.json".into()
        ))
    );
    assert_eq!(url, args(&["--connection", "saved"]));
}

#[test]
fn resolved_reconnect_arguments_keep_exact_discovered_executable_and_state() {
    let requested = Connection::parse(&["host".into()]).unwrap();
    let request = Request::new(requested, Selection::Automatic, Source::DefaultChannel);
    let observed = parse_probe(&response(
        "/home/test/.local/bin/flere",
        "present",
        "present",
    ))
    .unwrap();
    let resolved = connection(&request, &observed, observed.executable.as_deref().unwrap());
    let remembered = Connection::parse(&resolved.args()).unwrap();
    assert!(remembered.remote_explicit);
    assert_eq!(remembered.remote, "/home/test/.local/bin/flere");
    assert_eq!(
        remembered.state.as_deref(),
        Some("/home/test/.local/state/flere")
    );
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::{
        fs,
        io::Write,
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!("bootstrap-{}", crate::update::nonce().unwrap()));
            crate::update::private_dir(&root).unwrap();
            Self(root)
        }
        fn shell(&self, script: &str) -> Command {
            let mut command = Command::new("/bin/sh");
            command
                .arg("-c")
                .arg(script)
                .arg("sh")
                .current_dir(&self.0)
                .env("HOME", &self.0)
                .env("PATH", "/usr/bin:/bin")
                .env("XDG_STATE_HOME", self.0.join("state"))
                .env("XDG_DATA_HOME", self.0.join("data"))
                .env("XDG_CACHE_HOME", self.0.join("cache"));
            command
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn actual_shell_probe_is_read_only_and_never_selects_legacy_state() {
        let f = Fixture::new();
        crate::update::private_dir(&f.0.join("state/railhand")).unwrap();
        fs::write(f.0.join("state/railhand/sentinel"), b"retained").unwrap();
        let output = f
            .shell(include_str!("probe.sh"))
            .args([
                "missing-flere",
                "missing-railhand",
                "flere",
                "flere",
                "",
                "",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let probe = parse_probe(&output.stdout).unwrap();
        assert!(!probe.state_exists);
        assert!(probe.state.ends_with("/state/flere"));
        assert!(!f.0.join("state/flere").exists());
        assert!(!f.0.join("cache").exists());
        assert_eq!(
            fs::read(f.0.join("state/railhand/sentinel")).unwrap(),
            b"retained"
        );
    }

    #[test]
    fn fixed_receiver_verifies_bytes_and_refuses_existing_writable_cache() {
        let f = Fixture::new();
        let cache = f.0.join("cache ' quoted");
        let payload = b"abc";
        let manifest = b"{}";
        let digest = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let mut command = f.shell(include_str!("receive.sh"));
        command
            .args([cache.to_str().unwrap(), "2", "3", digest])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(manifest).unwrap();
        stdin.write_all(payload).unwrap();
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let response = String::from_utf8(output.stdout).unwrap();
        let path = PathBuf::from(decode_hex(response.lines().nth(1).unwrap()).unwrap());
        assert_eq!(fs::read(path.join(COMPONENT)).unwrap(), payload);
        assert_eq!(
            fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path.join(COMPONENT))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o775)).unwrap();
        let output = f
            .shell(include_str!("receive.sh"))
            .args([cache.to_str().unwrap(), "2", "3", digest])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
            0o775
        );
        assert_eq!(fs::read(path.join(COMPONENT)).unwrap(), payload);
    }
}
