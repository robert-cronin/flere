//! Real bridge -> UI -> supervisor -> native PTY, with a harmless owned-history app.
use super::*;

fn owned_chat(f: &Fixture) -> TabView {
    let tab = f.new_workspace("owned transcript");
    fs::write(
        f.root.join("owned_scroll_probe.py"),
        include_str!("../fixtures/owned_scroll_probe.py"),
    )
    .unwrap();
    f.send(&tab, b"python3 owned_scroll_probe.py\r");
    f.wait_text(&tab, "OWNED_TOP=050");
    tab
}

#[test]
fn owned_transcript_wheel_crosses_remote_ui_with_pane_coordinates() {
    for (width, height) in [(80, 24), (180, 42)] {
        let f = Fixture::new();
        let tab = owned_chat(&f);
        let mut bridge = Bridge::new(&f, width, height);
        bridge.wait_screen("OWNED_TOP=050");
        let layout = flere::ui::Layout::new(width as usize, height as usize);
        bridge.send(protocol::KEYS, 0, wheel_at(layout, 64).as_bytes());
        bridge.wait_screen("OWNED_TOP=047");
        assert_eq!(fs::read(f.root.join("received")).unwrap(), b"\x1b[<64;2;2M");
        // Coalesced events retain wheel notches, not the local three-row multiplier.
        bridge.send(protocol::KEYS, 0, wheel_at(layout, 65).repeat(2).as_bytes());
        bridge.wait_screen("OWNED_TOP=053");
        assert_eq!(
            fs::read(f.root.join("received")).unwrap(),
            b"\x1b[<64;2;2M\x1b[<65;2;2M\x1b[<65;2;2M"
        );
        assert!(bridge.screen.capture(100).contains("DRAFT_UNSUBMITTED"));
        bridge.disconnect();
        let live = f.snapshot();
        assert_eq!(
            (live.session().unwrap().pid, &live.session().unwrap().run),
            (tab.pid, &tab.run)
        );
    }
}

#[test]
fn owned_transcript_shift_page_keys_scroll_without_touching_draft() {
    let f = Fixture::new();
    let tab = owned_chat(&f);
    let mut bridge = Bridge::new(&f, 120, 30);
    bridge.wait_screen("OWNED_TOP=050");
    let layout = flere::ui::Layout::new(120, 30);
    let page = layout.rows.saturating_sub(1).max(1);
    bridge.send(protocol::KEYS, 0, b"\x1b[5;2~");
    bridge.wait_screen(&format!("OWNED_TOP={:03}", 50 - page));
    bridge.send(protocol::KEYS, 0, b"\x1b[6;2~");
    bridge.wait_screen("OWNED_TOP=050");
    // Plain native PageUp/Down stay byte-for-byte input too.
    bridge.send(protocol::KEYS, 0, b"\x1b[5~");
    bridge.wait_screen(&format!("OWNED_TOP={:03}", 50 - page));
    bridge.send(protocol::KEYS, 0, b"\x1b[6~x");
    bridge.wait_screen("DRAFT_UNSUBMITTEDx");
    assert!(bridge.screen.capture(100).contains("OWNED_TOP=050"));
    assert_eq!(
        fs::read(f.root.join("received")).unwrap(),
        b"\x1b[5~\x1b[6~\x1b[5~\x1b[6~x"
    );
    bridge.disconnect();
    assert_eq!(f.snapshot().session().unwrap().run, tab.run);
}

#[test]
fn owned_scroll_refresh_mode_changes_and_stale_targets_preserve_draft() {
    let f = Fixture::new();
    let tab = owned_chat(&f);
    let wid = f.snapshot().active;
    let epoch = f.snapshot().epoch;
    let mut bridge = Bridge::new(&f, 140, 38);
    bridge.wait_screen("OWNED_TOP=050");
    let layout = flere::ui::Layout::new(140, 38);
    bridge.send(protocol::KEYS, 0, wheel_at(layout, 64).as_bytes());
    bridge.wait_screen("OWNED_TOP=047");
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
            "scroll-mode refresh did not finish"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    bridge.send(protocol::KEYS, 0, wheel_at(layout, 65).as_bytes());
    bridge.wait_screen("OWNED_TOP=050");
    assert_eq!(f.snapshot().epoch, epoch);
    assert_eq!(f.snapshot().session().unwrap().pid, tab.pid);
    // Local gestures and NAV must not become native input, even with capture on.
    bridge.send(protocol::KEYS, 0, wheel_at(layout, 68).as_bytes());
    bridge.send(
        protocol::KEYS,
        0,
        b"\x1b[<64;2;2M\0\x15\x19\x1b[5~\x1b[H\x1b[F\rx",
    );
    bridge.wait_screen("DRAFT_UNSUBMITTEDx");
    let expected = b"\x1b[<64;2;2M\x1b[<65;2;2Mx";
    assert_eq!(fs::read(f.root.join("received")).unwrap(), expected);
    for operation in ["wheel", "wheel-links", "scroll-key", "scroll-key-links"] {
        let mut args = vec![
            operation.to_owned(),
            tab.id.to_string(),
            tab.run.clone(),
            layout.cols.to_string(),
            layout.rows.to_string(),
            if operation.starts_with("wheel") {
                "-3"
            } else {
                "up"
            }
            .into(),
        ];
        if operation.starts_with("wheel") {
            args.extend(["1".into(), "1".into()]);
        }
        let request = |args: &[String]| {
            wire::request(
                &f.state,
                &args.iter().map(String::as_str).collect::<Vec<_>>(),
            )
        };
        let mut cases = vec![
            (1, "0".into()),
            (2, "stale-run".into()),
            (3, (layout.cols + 1).to_string()),
            (4, (layout.rows + 1).to_string()),
            (5, "not-a-scroll".into()),
        ];
        if operation.starts_with("wheel") {
            cases.extend([
                (6, layout.cols.to_string()),
                (7, layout.rows.to_string()),
                (6, "-1".into()),
            ]);
        }
        for (index, value) in cases {
            let mut bad = args.clone();
            bad[index] = value;
            assert!(
                request(&bad).is_err(),
                "accepted stale/malformed {operation}"
            );
        }
        let mut short = args.clone();
        short.pop();
        assert!(request(&short).is_err());
        let other = f.new_workspace("different scroll target");
        assert!(request(&args).is_err(), "accepted inactive target");
        assert_ne!(other.id, tab.id);
        f.req(&["focus", &wid.to_string(), &tab.id.to_string()]);
    }
    // A legacy frontend has no pointer; do not invent one or fall back to arrows.
    f.req(&[
        "wheel",
        &tab.id.to_string(),
        &tab.run,
        &layout.cols.to_string(),
        &layout.rows.to_string(),
        "-3",
    ]);
    fs::write(f.root.join("disable-mouse"), b"go").unwrap();
    bridge.wait_screen("MOUSE_OFF");
    bridge.send(protocol::KEYS, 0, wheel_at(layout, 64).as_bytes());
    bridge.send(protocol::KEYS, 0, b"y");
    bridge.wait_screen("DRAFT_UNSUBMITTEDxy");
    let mut expected = expected.to_vec();
    expected.push(b'y');
    assert_eq!(fs::read(f.root.join("received")).unwrap(), expected);
    assert!(bridge.screen.capture(100).contains("OWNED_TOP=050"));
    // Leaving the owned screen restores local scrollback, including page-down at live.
    fs::write(f.root.join("primary"), b"go").unwrap();
    bridge.wait_screen("INLINE-089");
    bridge.send(protocol::KEYS, 0, b"\x1b[5;2~");
    bridge.wait_screen("SCROLLBACK");
    bridge.send(protocol::KEYS, 0, b"\x1b[6;2~");
    bridge.until(|_, screen| {
        !screen.capture(100).contains("SCROLLBACK")
            && screen.capture(100).contains("DRAFT_UNSUBMITTEDxy")
    });
    bridge.send(protocol::KEYS, 0, b"\x1b[6;2~");
    bridge.send(protocol::KEYS, 0, wheel_at(layout, 64).as_bytes());
    bridge.wait_screen("SCROLLBACK");
    bridge.send(protocol::KEYS, 0, b"\x1b[F");
    bridge.until(|_, screen| {
        !screen.capture(100).contains("SCROLLBACK")
            && screen.capture(100).contains("DRAFT_UNSUBMITTEDxy")
    });
    assert_eq!(fs::read(f.root.join("received")).unwrap(), expected);
    bridge.disconnect();
    assert_eq!(f.snapshot().session().unwrap().run, tab.run);
}

#[test]
fn native_wheel_in_split_panes_reaches_only_the_pointed_at_run() {
    let f = Fixture::new();
    let tabs = crate::splits::probes(&f, 2);
    let mut bridge = Bridge::new(&f, 180, 42);
    let split = crate::splits::pane(&f, "split-right", Some("move"));
    for tab in &tabs {
        crate::splits::emit(
            &f,
            tab,
            1,
            &format!(
                "\x1b[?1049h\x1b[?1007l\x1b[?1000h\x1b[?1006hSPLIT_SCROLL_{}",
                tab.id
            ),
        );
    }
    bridge.until(|_, screen| {
        tabs.iter().all(|tab| {
            screen
                .capture(100)
                .contains(&format!("SPLIT_SCROLL_{}", tab.id))
        })
    });
    let base = flere::ui::Layout::new(180, 42);
    let left_cols = split.split.as_ref().unwrap().other.cols;
    for (group, tab) in tabs.iter().enumerate() {
        let x = base.terminal_x + if group == 0 { 1 } else { left_cols + 2 };
        let y = base.terminal_y + 1;
        bridge.send(protocol::KEYS, 0, format!("\x1b[<64;{};{}M", x + 1, y + 1));
        let deadline = Instant::now() + Duration::from_secs(3);
        while crate::splits::input(&f, tab).is_empty() {
            assert!(Instant::now() < deadline, "split wheel not delivered");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(crate::splits::input(&f, tab), b"\x1b[<64;2;2M");
        assert_eq!(f.snapshot().tab, tab.id);
    }
    bridge.disconnect();
    for tab in &tabs {
        assert_eq!(crate::splits::input(&f, tab), b"\x1b[<64;2;2M");
    }
}
