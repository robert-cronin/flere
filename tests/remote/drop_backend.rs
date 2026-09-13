//! Generic attachments use the ordinary transfer worker and harmless native PTYs.
use super::*;
use crate::splits::{input as probe_input, pane, panes, probes};
use flere::remote_services as service;
use serde_json::Value;
use std::{
    io,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
};

fn origin(snapshot: &Snapshot) -> Vec<String> {
    let tab = snapshot.session().unwrap();
    vec![
        snapshot.epoch.clone(),
        snapshot.active.to_string(),
        tab.id.to_string(),
        tab.run.clone(),
        snapshot
            .split
            .as_ref()
            .map_or(0, |s| s.revision)
            .to_string(),
    ]
}
fn request(f: &Fixture, fields: &[String]) -> io::Result<Vec<u8>> {
    wire::request(
        &f.state,
        &fields.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}
fn choice(f: &Fixture, snapshot: &Snapshot) -> io::Result<String> {
    let mut fields = vec!["attachment-check".into()];
    fields.extend(origin(snapshot));
    String::from_utf8(request(f, &fields)?).map_err(io::Error::other)
}
fn poll(f: &Fixture, token: &str) -> io::Result<Vec<u8>> {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let value: Value = serde_json::from_slice(&wire::request(&f.state, &["file-poll", token])?)
            .map_err(io::Error::other)?;
        if value["pending"] == false {
            return wire::unhex(value["data"].as_str().unwrap());
        }
        assert!(Instant::now() < until, "attachment worker timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn begin_ticket(
    f: &Fixture,
    snapshot: &Snapshot,
    name: &str,
    size: u64,
    ticket: &str,
) -> io::Result<String> {
    let mut fields = vec!["file-begin".into(), "attach".into()];
    fields.extend(origin(snapshot));
    fields.extend([wire::hex(name.as_bytes()), size.to_string(), ticket.into()]);
    let reply: Value = serde_json::from_slice(&request(f, &fields)?).map_err(io::Error::other)?;
    let token = reply["token"].as_str().unwrap().to_owned();
    let metadata = poll(f, &token)?;
    assert_eq!(
        service::file_metadata(&metadata).unwrap(),
        (name.into(), size)
    );
    Ok(token)
}
fn begin(f: &Fixture, name: &str, size: u64) -> io::Result<String> {
    let snapshot = panes(f);
    let ticket = choice(f, &snapshot)?;
    begin_ticket(f, &snapshot, name, size, &ticket)
}
fn call(f: &Fixture, fields: &[&str]) -> io::Result<Vec<u8>> {
    wire::request(&f.state, fields)?;
    poll(f, fields[1])
}
fn write(f: &Fixture, token: &str, offset: usize, bytes: &[u8]) {
    call(
        f,
        &["file-write", token, &offset.to_string(), &wire::hex(bytes)],
    )
    .unwrap();
}
fn root(f: &Fixture) -> PathBuf {
    f.root.join(".cache/flere/chat-attachments")
}
fn directories(f: &Fixture) -> Vec<PathBuf> {
    fs::read_dir(root(f))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect()
}
fn wait_until(check: impl Fn() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < until, "attachment fixture timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn bracketed(bytes: &[u8]) -> Vec<u8> {
    [b"\x1b[200~".as_slice(), bytes, b"\x1b[201~".as_slice()].concat()
}

#[test]
fn attachment_stream_retains_original_names_and_pastes_each_path_exactly_once() {
    let f = Fixture::new();
    let tabs = probes(&f, 1);
    let snapshot = panes(&f);
    let ticket = choice(&f, &snapshot).unwrap();
    assert!(
        !root(&f).exists(),
        "checking a choice must not read or create files"
    );
    f.req(&["attachment-cancel", &ticket]);
    f.send(&tabs[0], b"existing draft ");
    let mut expected = b"existing draft ".to_vec();
    let bytes = (0..service::CHUNK + 19)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<_>>();
    let mut retained = Vec::new();
    for content in [&bytes, &png()] {
        let token = begin(&f, "notes ü.png", content.len() as u64).unwrap();
        let dir = directories(&f)
            .into_iter()
            .find(|p| {
                !retained
                    .iter()
                    .any(|p2: &PathBuf| p2.parent() == Some(p.as_path()))
            })
            .unwrap();
        let path = dir.join("notes ü.png");
        assert!(!path.exists());
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        for (i, bytes) in content.chunks(service::CHUNK).enumerate() {
            write(&f, &token, i * service::CHUNK, bytes);
        }
        let message = String::from_utf8(call(&f, &["file-finish", &token]).unwrap()).unwrap();
        assert!(message.contains(path.to_str().unwrap()));
        assert!(message.contains("without Enter"));
        assert!(!message.contains("Image paste"));
        assert_eq!(fs::read(&path).unwrap(), *content);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        expected.extend(bracketed(path.to_str().unwrap().as_bytes()));
        wait_until(|| probe_input(&f, &tabs[0]).len() >= expected.len());
        assert_eq!(probe_input(&f, &tabs[0]), expected);
        assert!(call(&f, &["file-finish", &token]).is_err());
        retained.push(path);
    }
    assert_ne!(retained[0], retained[1]);
    assert_eq!(fs::read(&retained[0]).unwrap(), bytes);
}

#[test]
fn attachment_choices_reject_replay_wrong_native_and_unsplit_away_and_back() {
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    let snapshot = panes(&f);
    assert!(snapshot.split.is_none());
    let ticket = choice(&f, &snapshot).unwrap();
    for tab in [&tabs[0], &tabs[1]] {
        f.req(&["focus", &snapshot.active.to_string(), &tab.id.to_string()]);
    }
    assert_eq!(panes(&f).tab, snapshot.tab);
    assert!(begin_ticket(&f, &snapshot, "never.txt", 1, &ticket).is_err());
    assert!(
        wire::request(
            &f.state,
            &["attachment-text", &ticket, &wire::hex(&bracketed(b"never"))]
        )
        .is_err()
    );
    assert!(!root(&f).exists());
    for (index, value) in [
        (0, "wrong-epoch"),
        (2, "999999"),
        (3, "wrong-run"),
        (4, "999999"),
    ] {
        let mut fields = origin(&snapshot);
        fields[index] = value.into();
        fields.insert(0, "attachment-check".into());
        assert!(request(&f, &fields).is_err());
    }
    f.new_workspace("Shell only");
    assert!(choice(&f, &panes(&f)).is_err());
    for tab in &tabs {
        assert!(probe_input(&f, tab).is_empty());
    }
}

#[test]
fn attachment_text_is_bounded_printable_one_use_and_never_enters_a_command() {
    let f = Fixture::new();
    let tabs = probes(&f, 1);
    let snapshot = panes(&f);
    let bytes = bracketed("'/local/report ü.md'".as_bytes());
    let ticket = choice(&f, &snapshot).unwrap();
    f.req(&["attachment-text", &ticket, &wire::hex(&bytes)]);
    wait_until(|| probe_input(&f, &tabs[0]).len() >= bytes.len());
    assert_eq!(probe_input(&f, &tabs[0]), bytes);
    assert!(wire::request(&f.state, &["attachment-text", &ticket, &wire::hex(&bytes)]).is_err());
    for bad in [
        b"plain".to_vec(),
        bracketed(b"bad\nline"),
        bracketed(b"\x1b[201~nested"),
        bracketed(&[0xff]),
        bracketed(&vec![b'x'; 8193]),
    ] {
        let ticket = choice(&f, &snapshot).unwrap();
        assert!(wire::request(&f.state, &["attachment-text", &ticket, &wire::hex(&bad)]).is_err());
        assert!(
            wire::request(&f.state, &["attachment-text", &ticket, &wire::hex(&bytes)]).is_err()
        );
    }
    assert_eq!(probe_input(&f, &tabs[0]), bytes);
    assert!(!root(&f).exists());
}

#[test]
fn attachment_transfer_cancels_on_focus_revision_and_native_paste_mode_changes() {
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    let token = begin(&f, "aborted.txt", 4).unwrap();
    write(&f, &token, 0, b"ab");
    let wid = f.snapshot().active.to_string();
    for tab in [&tabs[0], &tabs[1]] {
        f.req(&["focus", &wid, &tab.id.to_string()]);
    }
    assert!(call(&f, &["file-write", &token, "2", &wire::hex(b"cd")]).is_err());
    wait_until(|| directories(&f).is_empty());
    pane(&f, "split-right", Some("move"));
    let ticket = choice(&f, &panes(&f)).unwrap();
    let old = panes(&f);
    pane(&f, "focus", Some("0"));
    pane(&f, "focus", Some("1"));
    assert!(begin_ticket(&f, &old, "split.txt", 4, &ticket).is_err());
    let token = begin(&f, "mode.txt", 4).unwrap();
    write(&f, &token, 0, b"ab");
    fs::write(
        f.root.join(format!("pane-{}.control", tabs[1].id)),
        serde_json::to_vec(&serde_json::json!({"seq":1,"output":"\u{1b}[?2004lPASTE_DISABLED"}))
            .unwrap(),
    )
    .unwrap();
    f.wait_text(&tabs[1], "PASTE_DISABLED");
    assert!(call(&f, &["file-finish", &token]).is_err());
    wait_until(|| directories(&f).is_empty());
    for tab in &tabs {
        assert!(probe_input(&f, tab).is_empty());
    }
}

fn private_dir(path: &Path) {
    fs::DirBuilder::new().mode(0o700).create(path).unwrap();
}
fn private_file(path: &Path, size: u64) {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap()
        .set_len(size)
        .unwrap();
}
#[test]
fn attachment_cache_bounds_retain_history_and_refuse_overwrite_and_bad_names() {
    let f = Fixture::new();
    let tabs = probes(&f, 1);
    for name in ["../secret", "bad\nname", "NUL", "a/b"] {
        assert!(begin(&f, name, 1).is_err());
    }
    assert!(begin(&f, "huge.txt", service::FILE_LIMIT + 1).is_err());
    let token = begin(&f, "owned.txt", 3).unwrap();
    write(&f, &token, 0, b"new");
    let dir = directories(&f).pop().unwrap();
    let path = dir.join("owned.txt");
    private_file(&path, 0);
    fs::write(&path, b"owner created meanwhile").unwrap();
    assert!(call(&f, &["file-finish", &token]).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"owner created meanwhile");
    wait_until(|| fs::read_dir(&dir).unwrap().count() == 1);
    for i in 0..4 {
        let dir = root(&f).join(format!("retained-{i}"));
        private_dir(&dir);
        private_file(&dir.join("draft.bin"), service::FILE_LIMIT);
    }
    assert!(begin(&f, "capacity.txt", 1).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"owner created meanwhile");
    // Sparse fixture files model retained history without allocating 512 MiB.
    for i in 0..4 {
        fs::remove_dir_all(root(&f).join(format!("retained-{i}"))).unwrap();
    }
    for i in 0..255 {
        private_dir(&root(&f).join(format!("retained-{i}")));
    }
    assert!(begin(&f, "count.txt", 0).is_err());
    assert_eq!(directories(&f).len(), 256);
    assert_eq!(fs::read(path).unwrap(), b"owner created meanwhile");
    assert!(probe_input(&f, &tabs[0]).is_empty());
}
