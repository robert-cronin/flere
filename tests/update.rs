//! Installer boundaries exercised only inside disposable private home-cache files.
use flere::{
    install::{self, BuildMetadata, Manifest, PackageSource, Store},
    os,
};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, PermissionsExt, symlink},
    path::PathBuf,
};

struct Fixture {
    root: PathBuf,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("u-{}", &os::nonce().unwrap()[..12]));
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::new(root.join("data/install"), root.join("bin"), "flere").unwrap();
        Self { root, store }
    }
    fn build(&self, name: &str, suffix: &str) -> PathBuf {
        let binary = self.root.join(name);
        // A harmless executable speaks the real --build-info contract; distinct
        // payloads deliberately share a Cargo generation stamp to prove SHA binding.
        let json = flere::build_info::json();
        fs::write(&binary, format!("#!/bin/sh\n[ \"$#\" -eq 1 ] && [ \"$1\" = --build-info ] || exit 2\nprintf '%s\\n' '{}'\n# {suffix}\n", json.replace('\'', "'\\''"))).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        binary
    }
    fn package(&self, name: &str) -> PathBuf {
        let binary = self.build(&format!("{name}-binary"), name);
        let package = self.root.join(name);
        install::package(&binary, &package, None).unwrap();
        package
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Retained installer fixture {}", self.root.display());
        } else {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
fn first_install_retains_distinct_payloads_with_same_build_stamp_and_rolls_back() {
    let f = Fixture::new();
    assert!(f.store.status().unwrap().is_none());
    assert!(
        !f.store.root.exists(),
        "read-only status must create nothing"
    );
    let first = f.store.stage(&f.package("first")).unwrap();
    let second = f.store.stage(&f.package("second")).unwrap();
    assert_eq!(first.package.manifest.build, second.package.manifest.build);
    assert_ne!(first.package.id, second.package.id);
    let installed = f
        .store
        .install(&first, PackageSource::Local, false)
        .unwrap();
    assert!(installed.previous.is_none());
    assert_eq!(
        fs::metadata(&f.store.bin_dir).unwrap().permissions().mode() & 0o777,
        0o700,
        "a new bin directory must be private regardless of the caller's umask"
    );
    assert_eq!(
        install::sha256(&installed.destination).unwrap(),
        first.package.manifest.payload.sha256
    );
    let repeated = f
        .store
        .install(&first, PackageSource::Local, false)
        .unwrap();
    assert_eq!(repeated.attempt, installed.attempt);
    let updated = f
        .store
        .install(&second, PackageSource::Local, false)
        .unwrap();
    assert_eq!(updated.previous.unwrap().id, first.package.id);
    let rollback = f
        .store
        .rollback(Some(&first.package.manifest.build))
        .unwrap();
    assert_eq!(rollback.current.id, first.package.id);
    assert_eq!(rollback.previous.unwrap().id, second.package.id);
    assert_eq!(
        install::sha256(&rollback.destination).unwrap(),
        first.package.manifest.payload.sha256
    );
    assert!(!f.root.join(".local/state").exists());
}

#[test]
fn ssh_delegation_keeps_literal_arguments_and_precedes_local_state_or_nesting_checks() {
    let f = Fixture::new();
    let core = f.root.join("flere");
    fs::copy(env!("CARGO_BIN_EXE_flere"), &core).unwrap();
    let companion = f.root.join("flere-connect");
    fs::write(&companion, "#!/bin/sh\nprintf '%s\\n' \"$@\"\nexit 7\n").unwrap();
    fs::set_permissions(&companion, fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(&core)
        .args([
            "ssh",
            "fixture-host",
            "--state",
            "/private/a b;literal",
            "--ssh",
            "ssh with spaces",
        ])
        .env_remove("HOME")
        .env("FLERE", "existing-workbench")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(7));
    assert_eq!(
        result.stdout,
        b"ssh\nfixture-host\n--state\n/private/a b;literal\n--ssh\nssh with spaces\n"
    );
    assert!(result.stderr.is_empty());
    assert!(!f.root.join(".local").exists());
}

#[test]
fn bootstrap_install_refuses_a_concurrent_install_or_manual_destination() {
    let f = Fixture::new();
    let first = f.store.stage(&f.package("first")).unwrap();
    let second = f.store.stage(&f.package("second")).unwrap();
    let installed = f
        .store
        .install_if_missing(&first, PackageSource::Local)
        .unwrap();
    // The SSH probe may have observed absence before another client installed.
    // Recheck under the store lock, even if the new candidate is identical.
    for candidate in [&first, &second] {
        assert!(
            f.store
                .install_if_missing(candidate, PackageSource::Local)
                .unwrap_err()
                .to_string()
                .contains("already installed")
        );
        assert_eq!(
            f.store.status().unwrap().unwrap().attempt,
            installed.attempt
        );
        assert_eq!(
            install::sha256(&installed.destination).unwrap(),
            first.package.manifest.payload.sha256
        );
    }
    let other = Fixture::new();
    let staged = other.store.stage(&other.package("first")).unwrap();
    fs::create_dir(&other.store.bin_dir).unwrap();
    fs::set_permissions(&other.store.bin_dir, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(other.store.destination(), b"user-owned command").unwrap();
    assert!(
        other
            .store
            .install_if_missing(&staged, PackageSource::Local)
            .is_err()
    );
    assert_eq!(
        fs::read(other.store.destination()).unwrap(),
        b"user-owned command"
    );
    assert!(other.store.status().unwrap().is_none());
}

#[test]
fn corrupt_wrong_target_and_linked_packages_never_replace_installed_file() {
    let f = Fixture::new();
    let first = f.store.stage(&f.package("first")).unwrap();
    let receipt = f
        .store
        .install(&first, PackageSource::Local, false)
        .unwrap();
    let before = fs::read(&receipt.destination).unwrap();
    let corrupt = f.package("corrupt");
    fs::OpenOptions::new()
        .append(true)
        .open(corrupt.join("flere"))
        .unwrap()
        .write_all(b"corruption")
        .unwrap();
    assert!(
        f.store
            .stage(&corrupt)
            .unwrap_err()
            .to_string()
            .contains("SHA-256")
    );
    let wrong = f.package("wrong");
    let mut manifest = Manifest::load(&wrong).unwrap();
    manifest.build.target = "other-platform".into();
    fs::write(
        wrong.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(
        f.store
            .stage(&wrong)
            .unwrap_err()
            .to_string()
            .contains("target")
    );
    let linked = f.package("linked");
    fs::remove_file(linked.join("flere")).unwrap();
    symlink(&receipt.destination, linked.join("flere")).unwrap();
    assert!(f.store.stage(&linked).is_err());
    manifest.payload.file_name = "../escape".into();
    assert!(manifest.validate().is_err());
    assert_eq!(fs::read(&receipt.destination).unwrap(), before);
    assert_eq!(f.store.status().unwrap().unwrap().attempt, receipt.attempt);
}

#[test]
fn explicit_adoption_preserves_manual_package_and_refuses_other_owners() {
    let f = Fixture::new();
    let manual = f.build("manual", "manually installed predecessor");
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&f.store.bin_dir)
        .unwrap();
    fs::copy(&manual, f.store.destination()).unwrap();
    let old = install::sha256(&manual).unwrap();
    let staged = f.store.stage(&f.package("candidate")).unwrap();
    assert!(
        f.store
            .install(&staged, PackageSource::Local, false)
            .unwrap_err()
            .to_string()
            .contains("adoption")
    );
    assert_eq!(install::sha256(&f.store.destination()).unwrap(), old);
    let receipt = f
        .store
        .install(&staged, PackageSource::Local, true)
        .unwrap();
    assert_eq!(receipt.previous.unwrap().manifest.payload.sha256, old);

    let g = Fixture::new();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&g.store.bin_dir)
        .unwrap();
    symlink(&manual, g.store.destination()).unwrap();
    let staged = g.store.stage(&g.package("candidate")).unwrap();
    assert!(
        g.store
            .install(&staged, PackageSource::Local, true)
            .is_err()
    );
    assert_eq!(fs::read_link(g.store.destination()).unwrap(), manual);
}

#[test]
fn existing_shared_or_linked_bin_directories_are_rejected_without_chmod_or_install() {
    for linked in [false, true] {
        let f = Fixture::new();
        let directory = if linked {
            f.root.join("foreign-bin")
        } else {
            f.store.bin_dir.clone()
        };
        fs::create_dir(&directory).unwrap();
        let mode = if linked { 0o755 } else { 0o775 };
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode)).unwrap();
        if linked {
            symlink(&directory, &f.store.bin_dir).unwrap();
        }
        let staged = f.store.stage(&f.package("candidate")).unwrap();
        assert!(
            f.store
                .install(&staged, PackageSource::Local, false)
                .unwrap_err()
                .to_string()
                .contains("installation bin directory")
        );
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            mode
        );
        assert!(!f.store.destination().exists());
        assert!(f.store.status().unwrap().is_none());
        if linked {
            assert_eq!(fs::read_link(&f.store.bin_dir).unwrap(), directory);
        }
    }
}

#[test]
fn stale_receipts_busy_lock_and_incompatible_rollback_fail_without_replacement() {
    use std::os::fd::AsRawFd;
    let f = Fixture::new();
    let first = f.store.stage(&f.package("first")).unwrap();
    f.store
        .install(&first, PackageSource::Local, false)
        .unwrap();
    let second = f.store.stage(&f.package("second")).unwrap();
    let current = f
        .store
        .install(&second, PackageSource::Local, false)
        .unwrap();
    let lock = fs::File::open(f.store.root.join("install.lock")).unwrap();
    os::lock(lock.as_raw_fd()).unwrap();
    assert!(
        f.store
            .rollback(None)
            .unwrap_err()
            .to_string()
            .contains("lock")
    );
    drop(lock);
    let mut future: BuildMetadata = second.package.manifest.build.clone();
    future.compatibility.saved_state.as_mut().unwrap().current += 1;
    assert!(
        f.store
            .rollback(Some(&future))
            .unwrap_err()
            .to_string()
            .contains("cannot read")
    );
    assert_eq!(f.store.status().unwrap().unwrap().attempt, current.attempt);
    assert!(
        f.store
            .record_activation(
                "00000000000000000000000000000000",
                install::Activation::not_running()
            )
            .is_err()
    );
    fs::write(f.store.destination(), b"external replacement").unwrap();
    assert!(
        f.store
            .status()
            .unwrap_err()
            .to_string()
            .contains("differs")
    );
    assert!(
        f.store
            .install(&first, PackageSource::Local, false)
            .is_err()
    );
    assert_eq!(
        fs::read(f.store.destination()).unwrap(),
        b"external replacement"
    );
}

#[test]
fn cryptographic_digest_matches_standard_vectors_and_checks_every_source_input() {
    let f = Fixture::new();
    let file = f.root.join("vector");
    fs::write(&file, b"abc").unwrap();
    assert_eq!(
        install::sha256(&file).unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let source = f.root.join("source");
    fs::create_dir(&source).unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(&source)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(source.join("source.rs"), b"fn first() {}\n").unwrap();
    git(&["add", "source.rs"]);
    git(&[
        "-c",
        "user.name=Fixture",
        "-c",
        "user.email=fixture@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "fixture",
    ]);
    let scratch = f.root.join("scratch");
    let first = install::source_digest(&source, &scratch).unwrap();
    assert!(!first.dirty);
    fs::write(source.join("embedded-font.bin"), b"shared asset").unwrap();
    let changed = install::source_digest(&source, &scratch).unwrap();
    assert!(changed.dirty);
    assert_ne!(first.source_sha256, changed.source_sha256);
    fs::remove_file(source.join("embedded-font.bin")).unwrap();
    assert_eq!(
        install::source_digest(&source, &scratch)
            .unwrap()
            .source_sha256,
        first.source_sha256
    );
    fs::remove_file(source.join("source.rs")).unwrap();
    assert_ne!(
        install::source_digest(&source, &scratch)
            .unwrap()
            .source_sha256,
        first.source_sha256
    );
}

#[test]
fn remote_compatibility_requires_companion_to_read_each_core_during_transition() {
    let core: BuildMetadata = serde_json::from_str(flere::build_info::json()).unwrap();
    let mut old = core.clone();
    old.compatibility.remote_protocol.current = "flere-remote-old".into();
    let mut companion = core.clone();
    companion.component = "flere-connect".into();
    companion
        .compatibility
        .remote_protocol
        .accepts
        .push(old.compatibility.remote_protocol.current.clone());
    assert!(companion.reads_core(&core));
    assert!(companion.reads_core(&old));
    companion
        .compatibility
        .remote_protocol
        .accepts
        .retain(|p| p != &core.compatibility.remote_protocol.current);
    assert!(!companion.reads_core(&core));
}

#[test]
fn interrupted_install_reconciles_published_file_without_rolling_back_a_running_candidate() {
    let f = Fixture::new();
    let first = f.store.stage(&f.package("first")).unwrap();
    let old = f
        .store
        .install(&first, PackageSource::Local, false)
        .unwrap();
    let second = f.store.stage(&f.package("second")).unwrap();
    let mut next = old.clone();
    next.current = second.package.clone();
    next.previous = Some(first.package.clone());
    next.attempt = os::nonce().unwrap();
    // Persist the documented write-ahead journal, then simulate a process dying
    // after atomic executable rename but before publishing its receipt.
    let journal =
        serde_json::json!({"next":next,"previous_sha256":first.package.manifest.payload.sha256});
    fs::write(
        f.store.root.join("flere.pending.json"),
        serde_json::to_vec(&journal).unwrap(),
    )
    .unwrap();
    let staged_copy = f.store.bin_dir.join("candidate-copy");
    fs::copy(&second.package.executable, &staged_copy).unwrap();
    fs::rename(staged_copy, f.store.destination()).unwrap();
    assert!(
        f.store.status().is_err(),
        "passive inspection cannot repair an interrupted transaction"
    );
    let recovered = f
        .store
        .install(&second, PackageSource::Local, false)
        .unwrap();
    assert_eq!(recovered.attempt, next.attempt);
    assert_eq!(recovered.current.id, second.package.id);
    assert_eq!(recovered.previous.unwrap().id, first.package.id);
    assert!(!f.store.root.join("flere.pending.json").exists());
    assert_eq!(recovered.status, "installed_pending_activation");
}

#[test]
fn registered_development_source_is_explicit_private_and_revalidated() {
    let f = Fixture::new();
    assert!(f.store.local_source().unwrap().is_none());
    assert!(!f.store.root.exists());
    let checkout = f.root.join("registered-checkout");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(checkout.join("scripts"))
        .unwrap();
    let script = checkout.join("scripts/dev");
    fs::write(&script, b"#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    f.store.configure_local_source(&checkout).unwrap();
    assert_eq!(
        f.store.local_source().unwrap(),
        Some(fs::canonicalize(&checkout).unwrap())
    );
    fs::set_permissions(&script, fs::Permissions::from_mode(0o722)).unwrap();
    assert!(f.store.local_source().is_err());
    fs::remove_file(&script).unwrap();
    symlink(
        f.build("unrelated-script", "not the registered executable"),
        &script,
    )
    .unwrap();
    assert!(f.store.local_source().is_err());
}

#[test]
fn actual_supervisor_refresh_preserves_exact_shell_identity_and_draft_and_rejects_stale_epoch() {
    use flere::{model::Snapshot, wire};
    use std::{
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    struct Runtime {
        child: Child,
        state: PathBuf,
    }
    impl Drop for Runtime {
        fn drop(&mut self) {
            let _ = wire::request(&self.state, &["stop"]);
            if self.child.try_wait().ok().flatten().is_none() {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }
    }
    let f = Fixture::new();
    let state = f.root.join("runtime");
    let log = fs::File::create(f.root.join("runtime.log")).unwrap();
    let mut runtime = Runtime {
        child: Command::new(env!("CARGO_BIN_EXE_flere"))
            .arg("--state")
            .arg(&state)
            .arg("serve")
            .env("HOME", &f.root)
            .env("TMPDIR", &f.root)
            .env("SHELL", "/bin/sh")
            .env("ENV", "")
            .env("PS1", "$ ")
            .env_remove("FLERE")
            .env_remove("XDG_STATE_HOME")
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
        state: state.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while wire::request(&state, &["ping"]).is_err() {
        assert!(runtime.child.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    wire::request(
        &state,
        &[
            "new",
            &wire::hex(b"Update fixture shell"),
            &wire::hex(f.root.to_str().unwrap().as_bytes()),
        ],
    )
    .unwrap();
    let before = install::runtime_identity(&state).unwrap();
    let tab = before.sessions.first().unwrap();
    wire::request(
        &state,
        &[
            "input",
            &tab.tab.to_string(),
            &tab.run,
            &wire::hex(b"PRESERVED_UPDATE_DRAFT"),
        ],
    )
    .unwrap();
    let package = f.root.join("real-core");
    install::package(
        std::path::Path::new(env!("CARGO_BIN_EXE_flere")),
        &package,
        None,
    )
    .unwrap();
    let staged = f.store.stage(&package).unwrap();
    let receipt = f
        .store
        .install(&staged, PackageSource::Local, false)
        .unwrap();
    let mut stale = before.clone();
    stale.epoch = "f".repeat(32);
    let rejected = install::activate(&state, &receipt.current, &stale);
    assert_eq!(rejected.supervisor, "not_applied");
    assert_eq!(
        install::runtime_identity(&state).unwrap().sessions,
        before.sessions
    );
    let activation = install::activate(&state, &receipt.current, &before);
    assert_eq!(activation.supervisor, "applied", "{}", activation.detail);
    assert_eq!(
        activation.status, "partial",
        "the old frontend must not be claimed updated"
    );
    assert_eq!(activation.initiating_frontend, "pending");
    let after = activation.after.as_ref().unwrap();
    assert_eq!(after.pid, before.pid);
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(after.sessions, before.sessions);
    let snapshot = Snapshot::decode(&wire::request(&state, &["snapshot-panes"]).unwrap()).unwrap();
    assert!(
        snapshot
            .cells
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>()
            .contains("PRESERVED_UPDATE_DRAFT")
    );
    f.store
        .record_activation(&receipt.attempt, activation)
        .unwrap();
    let incompatible = f.store.stage(&f.package("reject-refresh")).unwrap();
    let installed = f
        .store
        .install(&incompatible, PackageSource::Local, false)
        .unwrap();
    let current = install::runtime_identity(&state).unwrap();
    let rejected = install::activate(&state, &installed.current, &current);
    assert_eq!(rejected.supervisor, "rejected", "{}", rejected.detail);
    assert!(rejected.can_restore_installation);
    let restored = f
        .store
        .restore_rejected(&installed.attempt, rejected)
        .unwrap();
    assert_eq!(restored.status, "restored_after_rejection");
    assert_eq!(restored.current.id, receipt.current.id);
    assert_eq!(
        install::sha256(&restored.destination).unwrap(),
        receipt.current.manifest.payload.sha256
    );
    let retained = install::runtime_identity(&state).unwrap();
    assert_eq!(retained.pid, before.pid);
    assert_eq!(retained.epoch, before.epoch);
    assert_eq!(retained.sessions, before.sessions);
}

#[test]
fn complete_large_activation_receipts_exceed_manifest_bound_without_losing_identities() {
    let f = Fixture::new();
    let candidate = f.store.stage(&f.package("large-receipt")).unwrap();
    let receipt = f
        .store
        .install(&candidate, PackageSource::Local, false)
        .unwrap();
    let sessions: Vec<_> = (0..2048).map(|i| serde_json::json!({"workspace":i / 8 + 1,"tab":i + 1,"run":format!("{i:032x}"),"pid":i + 1000})).collect();
    let runtime: install::RuntimeIdentity = serde_json::from_value(serde_json::json!({"state":f.root.join("s"),"epoch":"a".repeat(32),"pid":1234,"build":candidate.package.manifest.build,"sessions":sessions})).unwrap();
    let mut activation = install::Activation::not_running();
    activation.before = Some(runtime.clone());
    activation.after = Some(runtime);
    activation.status = "partial".into();
    f.store
        .record_activation(&receipt.attempt, activation)
        .unwrap();
    let bytes = fs::read(f.store.root.join("flere.json")).unwrap();
    assert!(bytes.len() > 65536);
    assert!(bytes.len() as u64 <= install::MAX_RECEIPT);
    let stored = f.store.status().unwrap().unwrap().activation.unwrap();
    assert_eq!(stored.before.unwrap().sessions.len(), 2048);
    assert_eq!(stored.after.unwrap().sessions.len(), 2048);
}
