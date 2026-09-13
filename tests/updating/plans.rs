//! Prepared candidates are inert until explicit apply. Only disposable owned
//! supervisors, shell probes and package files are used; no native model launch.
use super::*;
use flere::install::{self, InstallReceipt, PackageSource, Store};
use serde_json::{Value, json};
use std::path::Path;

fn store(f: &Fixture) -> Store {
    Store::new(
        f.root.join(".local/share/flere/install"),
        f.root.join(".local/bin"),
        "flere",
    )
    .unwrap()
}

fn command(f: &Fixture, state: &Path, action: &str, argument: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(state)
        .args([action, argument])
        .env("HOME", &f.root)
        .env("TMPDIR", &f.root)
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("FLERE")
        .env_remove("FLERE_UPDATE_ACK")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn call(f: &Fixture, action: &str, argument: &str) -> Value {
    let output = command(f, &f.state, action, argument);
    assert!(
        output.status.success(),
        "{action}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn reject(f: &Fixture, action: &str, argument: &str, expected: &str) {
    let output = command(f, &f.state, action, argument);
    assert!(!output.status.success(), "{action} unexpectedly succeeded");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn package(f: &Fixture) -> PathBuf {
    let path = f.root.join("real-package");
    install::package(Path::new(env!("CARGO_BIN_EXE_flere")), &path, None).unwrap();
    path
}

fn seed(f: &Fixture) -> InstallReceipt {
    // A different harmless payload reports the same metadata. It is only queried
    // for --build-info, never run as a supervisor or used to start a terminal.
    let executable = f.root.join("seed-binary");
    let metadata = flere::build_info::json().replace('\'', "'\\''");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\n[ \"$#\" -eq 1 ] && [ \"$1\" = --build-info ] || exit 2\nprintf '%s\\n' '{metadata}'\n"
        ),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    let path = f.root.join("seed-package");
    install::package(&executable, &path, None).unwrap();
    let store = store(f);
    let staged = store.stage(&path).unwrap();
    store.install(&staged, PackageSource::Local, false).unwrap()
}

fn runtime(f: &Fixture) -> Value {
    serde_json::to_value(install::runtime_identity(&f.state).unwrap()).unwrap()
}

fn plan_path(f: &Fixture, plan: &Value) -> PathBuf {
    store(f)
        .root
        .join("plans")
        .join(format!("{}.json", plan["token"].as_str().unwrap()))
}

fn save_plan(f: &Fixture, plan: &Value) {
    fs::write(plan_path(f, plan), serde_json::to_vec(plan).unwrap()).unwrap();
}

fn assert_installed(f: &Fixture, before: &InstallReceipt) {
    assert_eq!(
        install::sha256(&before.destination).unwrap(),
        before.current.manifest.payload.sha256
    );
    assert_eq!(store(f).status().unwrap().unwrap().attempt, before.attempt);
}

#[test]
fn prepare_is_inert_and_apply_preserves_work_added_during_review_and_replays_once() {
    let f = Fixture::new();
    let mut tabs = splits::probes(&f, 2);
    splits::pane(&f, "split-right", Some("move"));
    f.send(&tabs[0], b"LEFT_REVIEW_DRAFT");
    f.send(&tabs[1], b"RIGHT_REVIEW_DRAFT");
    let before = runtime(&f);
    let plan = call(&f, "update-prepare", package(&f).to_str().unwrap());
    assert_eq!(plan["status"], "prepared");
    assert!(plan["receipt"].is_null());
    assert_eq!(plan["runtime"], before);
    assert!(!store(&f).destination().exists());
    assert!(store(&f).status().unwrap().is_none());
    let token = plan["token"].as_str().unwrap();
    let mut inspected = call(&f, "update-plan", token);
    assert_eq!(inspected["ready"], true);
    inspected.as_object_mut().unwrap().remove("ready");
    inspected
        .as_object_mut()
        .unwrap()
        .remove("readiness_detail");
    assert_eq!(inspected, plan);
    assert_eq!(runtime(&f), before);

    let wid = f.snapshot().active;
    f.req(&["native", &wid.to_string(), "codex", ""]);
    let added = f.snapshot().session().unwrap().clone();
    f.wait_text(&added, &format!("PANE_READY_{}", added.id));
    f.send(&added, b"OPENED_DURING_REVIEW");
    tabs.push(added);
    let reviewed = splits::panes(&f);
    let current = runtime(&f);
    assert_eq!(current["sessions"].as_array().unwrap().len(), 3);
    let applied = call(&f, "update-apply", token);
    assert_eq!(applied["activation"]["supervisor"], "applied");
    assert_eq!(applied["activation"]["before"], current);
    assert_eq!(runtime(&f), current);
    let after = splits::panes(&f);
    assert_eq!((after.active, after.tab), (reviewed.active, reviewed.tab));
    assert_eq!(after.split.unwrap().groups, reviewed.split.unwrap().groups);
    splits::unchanged(&f, &tabs);
    for (tab, expected) in tabs.iter().zip([
        b"LEFT_REVIEW_DRAFT".as_slice(),
        b"RIGHT_REVIEW_DRAFT".as_slice(),
        b"OPENED_DURING_REVIEW".as_slice(),
    ]) {
        assert_eq!(splits::input(&f, tab), expected);
    }
    let generation: Value = serde_json::from_slice(&f.req(&["build-info"])).unwrap();
    assert_eq!(call(&f, "update-apply", token), applied);
    let replayed: Value = serde_json::from_slice(&f.req(&["build-info"])).unwrap();
    assert_eq!(generation, replayed, "replay must not refresh again");
    assert_eq!(call(&f, "update-plan", token)["receipt"], applied);
}

#[test]
fn prepared_candidate_tampering_and_stale_runtime_identities_never_install() {
    let f = Fixture::new();
    let tab = f.new_workspace("Review guards");
    f.send(&tab, b"UNSUBMITTED_GUARD_DRAFT");
    f.wait_text(&tab, "UNSUBMITTED_GUARD_DRAFT");
    let installed = seed(&f);
    let before = runtime(&f);
    let plan = call(&f, "update-prepare", package(&f).to_str().unwrap());
    let token = plan["token"].as_str().unwrap();
    assert_installed(&f, &installed);
    for (field, value) in [
        ("epoch", json!("f".repeat(32))),
        ("pid", json!(before["pid"].as_u64().unwrap() + 1)),
    ] {
        let mut stale = plan.clone();
        stale["runtime"][field] = value;
        save_plan(&f, &stale);
        reject(&f, "update-apply", token, "different supervisor generation");
        assert_installed(&f, &installed);
        assert_eq!(runtime(&f), before);
    }
    let mut stale = plan.clone();
    stale["runtime"]["build"]["build_id"] = json!("a-different-compiled-generation");
    save_plan(&f, &stale);
    reject(&f, "update-apply", token, "different supervisor generation");
    save_plan(&f, &plan);
    let wrong = command(&f, &f.root.join("other-state"), "update-apply", token);
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("does not belong"));
    reject(
        &f,
        "update-plan",
        "../invalid",
        "invalid prepared update token",
    );

    let executable = Path::new(plan["candidate"]["executable"].as_str().unwrap());
    fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(executable)
        .unwrap()
        .write_all(b"corrupt reviewed bytes")
        .unwrap();
    reject(&f, "update-apply", token, "SHA-256");
    assert_installed(&f, &installed);
    assert_eq!(runtime(&f), before);
    assert!(f.capture(&tab).contains("UNSUBMITTED_GUARD_DRAFT"));
    assert_eq!(call(&f, "update-plan", token)["status"], "prepared");
}

#[test]
fn abandoned_expired_and_nonprepared_plans_cannot_trigger_automatic_apply() {
    let f = Fixture::new();
    f.new_workspace("Abandoned review");
    let installed = seed(&f);
    let before = runtime(&f);
    let plan = call(&f, "update-prepare", package(&f).to_str().unwrap());
    let token = plan["token"].as_str().unwrap();
    // Dropping the preparing CLI and reading its persisted review never applies
    // it. The core has no automatic resume/expiry worker for abandoned plans.
    for _ in 0..3 {
        assert_eq!(call(&f, "update-plan", token)["status"], "prepared");
        assert_installed(&f, &installed);
        assert_eq!(runtime(&f), before);
    }
    let mut expired = plan.clone();
    expired["expires"] = json!(0);
    save_plan(&f, &expired);
    reject(&f, "update-apply", token, "expired");
    for status in ["cancelled", "applying"] {
        let mut stopped = plan.clone();
        stopped["status"] = json!(status);
        save_plan(&f, &stopped);
        reject(&f, "update-apply", token, "uncertain prior attempt");
    }
    assert_installed(&f, &installed);
    assert_eq!(runtime(&f), before);
}

#[test]
fn compatible_rollback_preparation_is_inert_and_apply_restores_retained_payload() {
    let f = Fixture::new();
    let tabs = splits::probes(&f, 1);
    f.send(&tabs[0], b"ROLLBACK_REVIEW_DRAFT");
    let store = store(&f);
    let real = store.stage(&package(&f)).unwrap();
    store.install(&real, PackageSource::Local, false).unwrap();
    let installed = seed(&f);
    assert_eq!(installed.previous.as_ref().unwrap().id, real.package.id);
    let before = runtime(&f);
    let plan = call(&f, "update-prepare", "--rollback");
    assert_eq!(plan["candidate"]["id"], real.package.id);
    assert_installed(&f, &installed);
    assert_eq!(runtime(&f), before);
    let applied = call(&f, "update-apply", plan["token"].as_str().unwrap());
    assert_eq!(applied["current"]["id"], real.package.id);
    assert_eq!(applied["previous"]["id"], installed.current.id);
    assert_eq!(applied["activation"]["supervisor"], "applied");
    assert_eq!(runtime(&f), before);
    assert_eq!(
        install::sha256(&store.destination()).unwrap(),
        real.package.manifest.payload.sha256
    );
    splits::unchanged(&f, &tabs);
    assert_eq!(splits::input(&f, &tabs[0]), b"ROLLBACK_REVIEW_DRAFT");
}
