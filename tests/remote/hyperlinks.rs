//! End-to-end PTYs -> supervisor -> negotiated UI -> remote OUTPUT -> terminal.
use super::*;

fn target(screen: &Terminal, uri: &str) -> bool {
    screen
        .grid
        .cells
        .iter()
        .any(|cell| cell.link.as_ref().is_some_and(|link| link.as_str() == uri))
}

fn emit(f: &Fixture, tab: &TabView, seq: u64, label: &str, uri: &str) {
    crate::splits::emit(
        f,
        tab,
        seq,
        &format!("\x1b[2J\x1b[H\x1b]8;;{uri}\x1b\\{label}\x1b]8;;\x1b\\"),
    );
}

#[test]
fn terminal_hyperlinks_cross_remote_split_rendering_and_supervisor_refresh() {
    let f = Fixture::new();
    let tabs = crate::splits::probes(&f, 2);
    let mut bridge = Bridge::new(&f, 180, 42);
    crate::splits::pane(&f, "split-right", Some("move"));
    let left = "https://example.com/left";
    let right = "https://example.com/right";
    emit(&f, &tabs[0], 1, "LEFT_LINK", left);
    emit(&f, &tabs[1], 1, "RIGHT_LINK", right);
    bridge.until(|_, screen| target(screen, left) && target(screen, right));
    assert!(bridge.screen.capture(100).contains("LEFT_LINK"));
    assert!(bridge.screen.capture(100).contains("RIGHT_LINK"));
    let old_bytes = f.req(&["snapshot-panes"]);
    assert_eq!(old_bytes[0], 5);
    let old = Snapshot::decode(&old_bytes).unwrap();
    assert!(old.cells.iter().all(|cell| cell.link.is_none()));
    assert!(
        old.split
            .as_ref()
            .unwrap()
            .other
            .cells
            .iter()
            .all(|cell| cell.link.is_none())
    );
    let current = Snapshot::decode(&f.req(&["snapshot-links"])).unwrap();
    assert!(current.cells.iter().any(|cell| cell.link.is_some()));
    assert!(
        current
            .split
            .as_ref()
            .unwrap()
            .other
            .cells
            .iter()
            .any(|cell| cell.link.is_some())
    );
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if wire::request(&f.state, &["refresh-status"]).is_ok_and(|bytes| {
            String::from_utf8_lossy(&bytes).contains("all terminal sessions preserved")
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "link-capable supervisor refresh did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let after = Snapshot::decode(&f.req(&["snapshot-links"])).unwrap();
    assert_eq!(after.epoch, current.epoch);
    assert_eq!(after.cells, current.cells);
    assert_eq!(
        after.split.as_ref().unwrap().other.cells,
        current.split.as_ref().unwrap().other.cells
    );
    emit(&f, &tabs[0], 2, "LEFT_AFTER_REFRESH", left);
    emit(&f, &tabs[1], 2, "RIGHT_AFTER_REFRESH", right);
    bridge.until(|_, screen| {
        target(screen, left)
            && target(screen, right)
            && screen.capture(100).contains("LEFT_AFTER_REFRESH")
            && screen.capture(100).contains("RIGHT_AFTER_REFRESH")
    });
    assert!(crate::splits::input(&f, &tabs[0]).is_empty());
    assert!(crate::splits::input(&f, &tabs[1]).is_empty());
    bridge.disconnect();
    let final_view = f.snapshot();
    let live = &final_view.workspace().unwrap().tabs;
    for tab in &tabs {
        let final_tab = live.iter().find(|entry| entry.id == tab.id).unwrap();
        assert_eq!((final_tab.pid, &final_tab.run), (tab.pid, &tab.run));
    }
}

#[test]
fn terminal_hyperlinks_survive_history_and_never_escape_into_ui_chrome() {
    let f = Fixture::new();
    let tab = crate::splits::probes(&f, 1).remove(0);
    let mut bridge = Bridge::new(&f, 120, 30);
    let uri = "https://example.com/history";
    let mut output = String::from("\x1b[2J\x1b[H");
    for n in 0..70 {
        output.push_str(&format!(
            "\x1b]8;;{uri}\x1b\\HISTORY_{n:03}\x1b]8;;\x1b\\\r\n"
        ));
    }
    output.push_str("HISTORY_FINISHED");
    crate::splits::emit(&f, &tab, 1, &output);
    bridge.wait_screen("HISTORY_FINISHED");
    bridge.send(protocol::KEYS, 0, b"\0\x1b[H");
    bridge.until(|_, screen| target(screen, uri) && screen.capture(100).contains("HISTORY_000"));
    let width = bridge.screen.grid.cols;
    let height = bridge.screen.grid.rows;
    assert!(
        bridge.screen.grid.cells[..width * 4]
            .iter()
            .all(|cell| cell.link.is_none())
    );
    assert!(
        bridge.screen.grid.cells[(height - 2) * width..]
            .iter()
            .all(|cell| cell.link.is_none())
    );
    assert!(
        crate::splits::input(&f, &tab).is_empty(),
        "navigation reached child input"
    );
    let legacy = flere::model::ScrollbackPage::decode(&f.req(&[
        "scrollback",
        &tab.id.to_string(),
        &tab.run,
        "0",
        "0",
    ]))
    .unwrap();
    assert!(legacy.cells.iter().all(|cell| cell.link.is_none()));
    let linked = flere::model::ScrollbackPage::decode(&f.req(&[
        "scrollback-links",
        &tab.id.to_string(),
        &tab.run,
        "0",
        "0",
    ]))
    .unwrap();
    assert!(
        linked
            .cells
            .iter()
            .any(|cell| cell.link.as_ref().is_some_and(|link| link.as_str() == uri))
    );
    bridge.disconnect();
}
