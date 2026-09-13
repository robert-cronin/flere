//! Exercise the actual stdio bridge: buffered handshake bytes must not need a wake-up key.
use super::*;

fn append(bytes: &mut Vec<u8>, tag: u8, id: u64, data: impl Into<Vec<u8>>) {
    Packet::new(tag, id, data).write(bytes).unwrap();
}

fn negotiated(hello: &Packet, width: u16, height: u16, notice: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    protocol::hello_reply(hello, width, height)
        .unwrap()
        .write(&mut bytes)
        .unwrap();
    append(&mut bytes, protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    append(
        &mut bytes,
        protocol::CAPABILITIES,
        0,
        protocol::SCREENSHOT_CAP,
    );
    append(
        &mut bytes,
        protocol::CAPABILITIES,
        0,
        [
            protocol::AVATAR_SIZE_CAP,
            &flere::avatar::cell_bytes((7, 21)).unwrap(),
        ]
        .concat(),
    );
    append(&mut bytes, protocol::NOTICE, 0, notice.as_bytes());
    bytes
}

fn write_once(b: &mut Bridge, bytes: &[u8]) {
    // Keep this below the portable PIPE_BUF minimum, and use one syscall so the
    // buffered HELLO reader sees the following frames in the same available batch.
    assert!(bytes.len() <= 512);
    assert_eq!(b.input.as_mut().unwrap().write(bytes).unwrap(), bytes.len());
}

fn wait_negotiated(b: &mut Bridge, width: u16, height: u16, notice: &str) {
    let packet = b.until(|p, _| p.tag == protocol::AVATAR_LAYOUT_SIZED);
    let layout = flere::avatar::Layout::decode_sized(&packet.data).unwrap();
    assert_eq!((layout.width, layout.height), (width, height));
    b.wait_screen(notice);
}

fn unchanged(f: &Fixture, t: &TabView) {
    assert!(
        input(f).is_empty(),
        "handshake/tail frames reached the child"
    );
    let snapshot = f.snapshot();
    let now = snapshot.session().unwrap();
    assert_eq!(
        (now.id, now.pid, &now.run, now.alive),
        (t.id, t.pid, &t.run, true)
    );
    assert!(f.capture(t).contains("IMAGE_NATIVE_DRAFT"));
}

#[test]
fn initial_hello_and_capabilities_in_one_write_need_no_later_input() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    let mut b = Bridge::start(&f, 100, 30, std::env::var("PATH").unwrap_or_default());
    let hello = b.until(|p, _| p.tag == protocol::HELLO);
    write_once(
        &mut b,
        &negotiated(&hello, 100, 30, "INITIAL_CAPABILITIES_READY"),
    );
    // Do not send keys, resize, EOF, or a probe packet to wake stdin polling.
    wait_negotiated(&mut b, 100, 30, "INITIAL_CAPABILITIES_READY");
    unchanged(&f, &t);
    b.disconnect();
    unchanged(&f, &t);
}

#[test]
fn refresh_hello_and_capabilities_drain_after_discarding_the_old_tail() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    let mut b = Bridge::new(&f, 100, 30);
    b.wait_screen("IMAGE_NATIVE_DRAFT");
    b.send(protocol::KEYS, 0, b"\0 R");
    let hello = b.until(|p, _| p.tag == protocol::HELLO);
    b.screen = Terminal::new(100, 30); // Require a fresh post-exec frame.
    let mut bytes = Vec::new();
    append(
        &mut bytes,
        protocol::KEYS,
        0,
        b"STALE_DRAFT_MUST_NOT_REPLAY\r",
    );
    append(&mut bytes, protocol::DATA, 19, b"stale-upload-tail");
    append(&mut bytes, protocol::END, 19, Vec::new());
    append(
        &mut bytes,
        protocol::NOTICE,
        0,
        b"STALE_NOTICE_MUST_NOT_APPEAR",
    );
    bytes.extend(negotiated(&hello, 100, 30, "REFRESH_CAPABILITIES_READY"));
    write_once(&mut b, &bytes);
    wait_negotiated(&mut b, 100, 30, "REFRESH_CAPABILITIES_READY");
    assert!(!b.screen.capture(100).contains("STALE_NOTICE"));
    unchanged(&f, &t);
    b.disconnect();
    unchanged(&f, &t);
}

#[test]
fn fragmented_capability_after_hello_keeps_its_prefix_and_needs_no_probe() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    let mut b = Bridge::start(&f, 100, 30, std::env::var("PATH").unwrap_or_default());
    let hello = b.until(|p, _| p.tag == protocol::HELLO);
    let mut reply = Vec::new();
    protocol::hello_reply(&hello, 100, 30)
        .unwrap()
        .write(&mut reply)
        .unwrap();
    let mut tail = Vec::new();
    append(
        &mut tail,
        protocol::CAPABILITIES,
        0,
        [
            protocol::AVATAR_SIZE_CAP,
            &flere::avatar::cell_bytes((7, 21)).unwrap(),
        ]
        .concat(),
    );
    append(
        &mut tail,
        protocol::NOTICE,
        0,
        b"FRAGMENTED_CAPABILITIES_READY",
    );
    reply.extend_from_slice(&tail[..2]); // Split the frame's four-byte length prefix.
    write_once(&mut b, &reply);
    b.wait_screen("IMAGE_NATIVE_DRAFT");
    write_once(&mut b, &tail[2..3]);
    write_once(&mut b, &tail[3..11]);
    write_once(&mut b, &tail[11..]);
    wait_negotiated(&mut b, 100, 30, "FRAGMENTED_CAPABILITIES_READY");
    unchanged(&f, &t);
    b.disconnect();
    unchanged(&f, &t);
}
