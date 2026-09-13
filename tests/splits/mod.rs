//! Shared split views over real disposable PTYs. No native models or system clipboard.
use super::*;
use serde_json::{Value, json};

mod ownership;
mod partial_restore;
mod persistence;
mod server;
mod ui;

pub(super) fn panes(f: &Fixture) -> Snapshot {
    let bytes = f.req(&["snapshot-panes"]);
    assert_eq!(bytes[0], 5);
    Snapshot::decode(&bytes).unwrap()
}

fn revision(snapshot: &Snapshot) -> u64 {
    snapshot.split.as_ref().map_or(0, |split| split.revision)
}

fn pane_request(
    f: &Fixture,
    snapshot: &Snapshot,
    op: &str,
    arg: Option<&str>,
) -> std::io::Result<Vec<u8>> {
    let tab = snapshot.session().unwrap();
    let wid = snapshot.active.to_string();
    let id = tab.id.to_string();
    let revision = revision(snapshot).to_string();
    let mut fields = vec!["pane", &snapshot.epoch, &wid, &id, &tab.run, &revision, op];
    if let Some(arg) = arg {
        fields.push(arg);
    }
    wire::request(&f.state, &fields)
}

pub(super) fn pane(f: &Fixture, op: &str, arg: Option<&str>) -> Snapshot {
    pane_request(f, &panes(f), op, arg).unwrap();
    panes(f)
}

fn resize(f: &Fixture, cols: usize, rows: usize) {
    let snapshot = panes(f);
    f.req(&[
        "resize-panes",
        &snapshot.epoch,
        &snapshot.active.to_string(),
        &cols.to_string(),
        &rows.to_string(),
    ]);
}

pub(super) fn probes(f: &Fixture, count: usize) -> Vec<TabView> {
    f.req(&[
        "new-stopped",
        &wire::hex(b"Split fixture"),
        &wire::hex(f.root.to_str().unwrap().as_bytes()),
    ]);
    let wid = f.snapshot().active;
    let script = f.root.join("pane-probe.py");
    fs::write(&script, include_str!("probe.py")).unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/usr/bin/python3",script]}]))
            .unwrap(),
    )
    .unwrap();
    (0..count)
        .map(|_| {
            f.req(&["native", &wid.to_string(), "codex", ""]);
            let tab = f.snapshot().session().unwrap().clone();
            f.wait_text(&tab, &format!("PANE_READY_{}", tab.id));
            tab
        })
        .collect()
}

pub(super) fn input(f: &Fixture, tab: &TabView) -> Vec<u8> {
    fs::read(f.root.join(format!("pane-{}.input", tab.id))).unwrap()
}

pub(super) fn emit(f: &Fixture, tab: &TabView, seq: u64, output: &str) {
    fs::write(
        f.root.join(format!("pane-{}.control", tab.id)),
        serde_json::to_vec(&json!({"seq":seq,"output":output})).unwrap(),
    )
    .unwrap();
}

fn wait(condition: impl Fn() -> bool, reason: &str) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !condition() {
        assert!(Instant::now() < deadline, "{reason}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn size(f: &Fixture, tab: &TabView) -> Option<(usize, usize)> {
    let bytes = fs::read(f.root.join(format!("pane-{}.size", tab.id))).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(super) fn unchanged(f: &Fixture, tabs: &[TabView]) {
    let snapshot = panes(f);
    for before in tabs {
        let now = snapshot
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|t| t.id == before.id)
            .unwrap();
        assert_eq!(
            (now.pid, &now.run, now.alive),
            (before.pid, &before.run, true)
        );
    }
}

fn text(cells: &[flere::terminal::Cell]) -> String {
    cells.iter().map(|c| c.text.as_str()).collect()
}

fn identities(snapshot: &Snapshot) -> Vec<(u64, u32, String)> {
    snapshot
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .map(|t| (t.id, t.pid, t.run.clone()))
        .collect()
}
