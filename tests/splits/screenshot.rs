//! Included under remote so the actual bridge/receiver fixture is shared.
use super::*;
use crate::splits;

#[test]
fn full_screenshot_keeps_an_inactive_panes_visible_scrollback_and_both_live_panes() {
    use flere::screenshot::{CELL_H, CELL_W, Frame};
    let f = Fixture::new();
    let tabs = splits::probes(&f, 2);
    let prefs = flere::workspace::Preferences {
        left: 26,
        right: 32,
        reduced_motion: true,
        ..Default::default()
    };
    fs::write(f.state.join("ui.json"), serde_json::to_vec(&prefs).unwrap()).unwrap();
    splits::pane(&f, "split-right", Some("move"));
    let mut b = Bridge::new(&f, 180, 42);
    b.send(protocol::CAPABILITIES, 0, protocol::SCREENSHOT_CAP);
    b.until(|p, s| {
        p.tag == protocol::OUTPUT
            && s.capture(100)
                .contains(&format!("PANE_READY_{}", tabs[0].id))
            && s.capture(100)
                .contains(&format!("PANE_READY_{}", tabs[1].id))
    });
    let base = flere::ui::Layout::with_preferences(180, 42, &prefs);
    let a = (
        base.terminal_x,
        base.terminal_y,
        (base.cols - 1) / 2,
        base.rows,
    );
    let c = (a.0 + a.2 + 1, a.1, base.cols - a.2 - 1, a.3);
    for (tab, label, colour) in [
        (&tabs[0], "FIRST", "15;35;120"),
        (&tabs[1], "SECOND", "110;25;35"),
    ] {
        let output = format!("\x1b[48;2;{colour}m\x1b[2J\x1b[H")
            + &(0..100)
                .map(|n| format!("{label}_HISTORY_{n:03}\r\n"))
                .collect::<String>();
        splits::emit(&f, tab, 1, &output);
    }
    b.until(|p, s| {
        p.tag == protocol::OUTPUT
            && s.capture(100).contains("FIRST_HISTORY_099")
            && s.capture(100).contains("SECOND_HISTORY_099")
    });
    let row = |screen: &Terminal, rect: (usize, usize, usize, usize)| {
        screen.grid.cells
            [rect.1 * screen.grid.cols + rect.0..rect.1 * screen.grid.cols + rect.0 + rect.2]
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>()
    };
    let live = row(&b.screen, a);
    b.send(
        protocol::KEYS,
        0,
        format!("\x1b[<64;{};{}M", a.0 + 3, a.1 + 3).as_bytes(),
    );
    b.until(|p, s| {
        p.tag == protocol::OUTPUT && row(s, a) != live && splits::panes(&f).tab == tabs[0].id
    });
    let scrolled = row(&b.screen, a);
    assert!(scrolled.contains("FIRST_HISTORY_"));
    b.send(
        protocol::KEYS,
        0,
        format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            c.0 + 3,
            c.1 + c.3 - 1,
            c.0 + 3,
            c.1 + c.3 - 1
        )
        .as_bytes(),
    );
    b.until(|p, s| {
        p.tag == protocol::OUTPUT && splits::panes(&f).tab == tabs[1].id && row(s, a) == scrolled
    });
    let visible = b.screen.capture(100);
    let expected = Frame {
        width: 180,
        height: 42,
        cells: b.screen.grid.cells.clone(),
        layers: Vec::new(),
        cursor: None,
    }
    .png()
    .unwrap();
    let (_, _, expected) = flere::screenshot::decode(&expected).unwrap();
    b.send(protocol::KEYS, 0, b"\0s");
    let mut receiver = flere::screenshot_transfer::Receiver::default();
    let png = loop {
        // PNG rasterization runs on a worker and can exceed the ordinary UI
        // packet deadline in debug builds on a busy host. Use the screenshot
        // transfer's budget while still verifying every resulting pixel below.
        let packet = b.until_timeout(Duration::from_secs(30), |p, _| {
            matches!(
                p.tag,
                protocol::SCREENSHOT_BEGIN | protocol::SCREENSHOT_DATA | protocol::SCREENSHOT_END
            )
        });
        if let Some(png) = receiver.packet(&packet).unwrap() {
            break png;
        }
    };
    let (width, height, actual) = flere::screenshot::decode(&png).unwrap();
    assert_eq!((width, height), (180 * CELL_W, 42 * CELL_H));
    // Compare rendered pixels for both visible buffers, excluding the live
    // cursor's bottom row. A crop or a live-only repaint of FIRST cannot pass.
    for (x, y, cols, _) in [a, c, (0, 0, 180, 1)] {
        let rows = if y == 0 { 1 } else { 5 };
        for yy in y * CELL_H..(y + rows) * CELL_H {
            let start = (yy * width + x * CELL_W) * 4;
            let end = start + cols * CELL_W * 4;
            assert_eq!(
                &actual[start..end],
                &expected[start..end],
                "full-view screenshot diverged at pixel row {yy}, pane x{x}"
            );
        }
    }
    assert!(splits::input(&f, &tabs[0]).is_empty());
    assert!(splits::input(&f, &tabs[1]).is_empty());
    splits::unchanged(&f, &tabs);
    if let Some(root) = std::env::var_os("FLERE_TEST_SPLIT_ARTIFACTS") {
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("split-full-screenshot.png"), &png).unwrap();
        fs::write(root.join("split-full-screenshot.txt"), visible).unwrap();
    }
    b.disconnect(); // Deliberately never invokes a local clipboard writer.
}
