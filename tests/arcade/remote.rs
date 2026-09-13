//! Included under remote to exercise the production bridge without a clipboard.
use super::*;
use flere::remote_services as service;

#[test]
fn remote_arcade_exports_only_its_full_canvas_and_rejects_native_attachment_offers() {
    for (width, height) in [(40, 20), (120, 34)] {
        let f = Fixture::new();
        let (_, tab) = native(&f);
        let prefs = flere::workspace::Preferences {
            reduced_motion: true,
            ..Default::default()
        };
        fs::write(f.state.join("ui.json"), serde_json::to_vec(&prefs).unwrap()).unwrap();
        let mut bridge = Bridge::new(&f, width, height);
        bridge.wait_screen("IMAGE_NATIVE_DRAFT");
        bridge.send(protocol::CAPABILITIES, 0, protocol::SCREENSHOT_CAP);
        bridge.send(protocol::NOTICE, 0, service::DROP_PROBE);
        bridge.until(|p, _| p.tag == service::DROP_CONTEXT && p.data == [1]);
        bridge.send(protocol::KEYS, 0, b"\0&\r");
        bridge.wait_screen("Enter: explore");
        assert!(!bridge.screen.capture(100).contains("IMAGE_NATIVE_DRAFT"));
        bridge.send(protocol::IMAGE, 51, (png().len() as u64).to_be_bytes());
        bridge.result(51, false);
        bridge.send(service::DROP_OFFER, 52, []);
        let refused = bridge.until(|p, _| p.tag == service::DROP_RESULT && p.id == 52);
        assert!(String::from_utf8_lossy(&refused.data).contains("Focus the native chat"));
        let expected = flere::screenshot::Frame {
            width: width as usize,
            height: height as usize,
            cells: bridge.screen.grid.cells.clone(),
            layers: Vec::new(),
            cursor: None,
        }
        .png()
        .unwrap();
        bridge.send(protocol::KEYS, 0, b"s");
        let mut receiver = flere::screenshot_transfer::Receiver::default();
        let (id, actual) = loop {
            let packet = bridge.until(|p, _| {
                matches!(
                    p.tag,
                    protocol::SCREENSHOT_BEGIN
                        | protocol::SCREENSHOT_DATA
                        | protocol::SCREENSHOT_END
                )
            });
            if let Some(bytes) = receiver.packet(&packet).unwrap() {
                break (packet.id, bytes);
            }
        };
        assert_eq!(
            flere::screenshot::decode(&actual).unwrap(),
            flere::screenshot::decode(&expected).unwrap()
        );
        if let Some(root) = std::env::var_os("FLERE_TEST_ARCADE_ARTIFACTS") {
            let root = PathBuf::from(root);
            fs::create_dir_all(&root).unwrap();
            fs::write(
                root.join(format!("arcade-remote-{width}x{height}-screenshot.png")),
                actual,
            )
            .unwrap();
        }
        bridge.send(protocol::SCREENSHOT_RESULT, id, [1]);
        // The same wire read can contain exit plus another operation. Neither the
        // trailing Enter nor a native image offer may acquire the old chat input.
        let mut burst = Vec::new();
        Packet::new(protocol::KEYS, 0, b"q\r")
            .write(&mut burst)
            .unwrap();
        Packet::new(protocol::IMAGE, 53, (png().len() as u64).to_be_bytes())
            .write(&mut burst)
            .unwrap();
        bridge.input.as_mut().unwrap().write_all(&burst).unwrap();
        bridge.result(53, false);
        bridge.wait_screen("IMAGE_NATIVE_DRAFT");
        assert!(input(&f).is_empty());
        let current = f.snapshot();
        assert_eq!(
            (
                current.session().unwrap().id,
                current.session().unwrap().pid,
                &current.session().unwrap().run
            ),
            (tab.id, tab.pid, &tab.run)
        );
        bridge.disconnect();
    }
}

fn keyboard_pump(bridge: &mut Bridge, ms: u64) -> Vec<u8> {
    let end = Instant::now() + Duration::from_millis(ms);
    let mut output = Vec::new();
    loop {
        match bridge
            .packets
            .recv_timeout(end.saturating_duration_since(Instant::now()))
        {
            Ok(packet) if packet.tag == protocol::OUTPUT => {
                assert!(!packet.data.windows(5).any(|bytes| bytes == b"\x1b]52;"));
                assert!(!packet.data.windows(2).any(|bytes| bytes == b"\x1bP"));
                bridge.screen.feed(&packet.data);
                output.extend(packet.data);
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(error) => panic!("bridge ended during keyboard proof: {error}"),
        }
        if Instant::now() >= end {
            break;
        }
    }
    output
}
fn keyboard_sequence_count(bytes: &[u8], sequence: &[u8]) -> usize {
    bytes
        .windows(sequence.len())
        .filter(|bytes| *bytes == sequence)
        .count()
}

#[test]
fn remote_keyboard_hold_release_and_restore_fence_never_replay_into_native_draft() {
    let f = Fixture::new();
    let (_, tab) = native(&f);
    f.send(&tab, b"REMOTE_HELD_DRAFT");
    let prefs = flere::workspace::Preferences {
        reduced_motion: true,
        ..Default::default()
    };
    fs::write(f.state.join("ui.json"), serde_json::to_vec(&prefs).unwrap()).unwrap();
    let mut bridge = Bridge::new(&f, 120, 34);
    bridge.wait_screen("IMAGE_NATIVE_DRAFT");
    let mut output = Vec::new();
    bridge.send(protocol::KEYS, 0, b"\0&\r");
    bridge.until(|packet, screen| {
        if packet.tag == protocol::OUTPUT {
            output.extend_from_slice(&packet.data);
        }
        screen.capture(100).contains("Enter: explore")
            && keyboard_sequence_count(&output, b"\x1b[?u") == 1
    });
    bridge.send(protocol::KEYS, 0, b"\x1b[?0u");
    bridge.until(|packet, _| {
        if packet.tag == protocol::OUTPUT {
            output.extend_from_slice(&packet.data);
        }
        keyboard_sequence_count(&output, b"\x1b[>11u") == 1
            && keyboard_sequence_count(&output, b"\x1b[?u") == 2
    });
    bridge.send(protocol::KEYS, 0, b"\x1b[?11u");
    bridge.send(protocol::KEYS, 0, b"\x1b[13;1:1u");
    bridge.wait_screen("Space jump");
    bridge.send(protocol::KEYS, 0, b"\x1b[1;1:1C\x1b[32;1:1u");
    output.extend(keyboard_pump(&mut bridge, 100));
    let launch = crate::arcade::avatar_x(&bridge.screen);
    output.extend(keyboard_pump(&mut bridge, 350)); // No later Right repeats or probes.
    let continued = crate::arcade::avatar_x(&bridge.screen);
    assert!(
        continued > launch + 4.,
        "remote held movement stopped: {launch} -> {continued}"
    );
    bridge.send(protocol::KEYS, 0, b"\x1b[1;1:3C");
    output.extend(keyboard_pump(&mut bridge, 120));
    let released = crate::arcade::avatar_x(&bridge.screen);
    output.extend(keyboard_pump(&mut bridge, 240));
    assert!((crate::arcade::avatar_x(&bridge.screen) - released).abs() < 1.25);
    bridge.send(protocol::KEYS, 0, b"\x1b[112;1:1u");
    bridge.wait_screen("PAUSED");
    bridge.send(protocol::KEYS, 0, b"\x1b[113;1:1u\x1b[13;1:1uEXIT_TAIL");
    bridge.until(|packet, screen| {
        if packet.tag == protocol::OUTPUT {
            output.extend_from_slice(&packet.data);
        }
        screen.capture(100).contains("IMAGE_NATIVE_DRAFT")
            && keyboard_sequence_count(&output, b"\x1b[<u") == 1
    });
    assert!(
        output
            .windows(b"\x1b[<u\x1b[?u".len())
            .any(|bytes| bytes == b"\x1b[<u\x1b[?u")
    );
    bridge.send(protocol::KEYS, 0, b"\x1b[1;1:3C\x1b[13;1:1u");
    output.extend(keyboard_pump(&mut bridge, 60));
    bridge.send(protocol::KEYS, 0, b"REMOTE_BEFORE_FENCE\r");
    output.extend(keyboard_pump(&mut bridge, 60));
    assert_eq!(input(&f), b"REMOTE_HELD_DRAFT");
    output.extend(keyboard_pump(&mut bridge, 1100));
    bridge.send(protocol::KEYS, 0, b"\r");
    output.extend(keyboard_pump(&mut bridge, 60));
    bridge.send(protocol::KEYS, 0, b"\x1b[104;1:1u\x1b[13;1:1u");
    output.extend(keyboard_pump(&mut bridge, 60));
    assert_eq!(input(&f), b"REMOTE_HELD_DRAFT");
    bridge.send(protocol::KEYS, 0, b"\x1b[?0u\rFENCE_TAIL");
    output.extend(keyboard_pump(&mut bridge, 60));
    bridge.send(protocol::KEYS, 0, b"\x1b[32;1:3u\x1b[13;1:3u");
    output.extend(keyboard_pump(&mut bridge, 60));
    assert_eq!(input(&f), b"REMOTE_HELD_DRAFT");
    assert_eq!(keyboard_sequence_count(&output, b"\x1b[>11u"), 1);
    assert_eq!(keyboard_sequence_count(&output, b"\x1b[<u"), 1);
    bridge.send(protocol::KEYS, 0, b"_AFTER");
    assert_eq!(
        wait_input(&f, b"REMOTE_HELD_DRAFT_AFTER".len()),
        b"REMOTE_HELD_DRAFT_AFTER"
    );
    let current = f.snapshot();
    assert_eq!(
        (
            current.session().unwrap().id,
            current.session().unwrap().pid,
            &current.session().unwrap().run
        ),
        (tab.id, tab.pid, &tab.run)
    );
    bridge.disconnect();
}
