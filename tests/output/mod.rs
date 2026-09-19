//! Slow readers must not starve exact child input or corrupt eventual rendering.
use super::*;
use flere::remote_protocol::{self as protocol, Packet};

fn producer(f: &Fixture) -> TabView {
    let tab = f.new_workspace("output pressure");
    fs::write(
        f.root.join("output_probe.py"),
        include_str!("../fixtures/output_probe.py"),
    )
    .unwrap();
    f.send(&tab, b"exec python3 output_probe.py\r");
    let end = Instant::now() + Duration::from_secs(3);
    while !f.root.join("output-ready").exists() {
        assert!(Instant::now() < end, "owned output producer did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
    tab
}
fn received(f: &Fixture, expected: &[u8]) {
    // Deliberately do not drain the outer display here. The producer continues
    // changing all rows, exceeding the PTY/pipe capacity while input arrives.
    let end = Instant::now() + Duration::from_secs(3);
    while fs::read(f.root.join("output-input")).unwrap_or_default() != expected {
        assert!(
            Instant::now() < end,
            "input did not reach child with output blocked"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn current_grid(f: &Fixture, screen: &Terminal) -> bool {
    let layout = flere::ui::Layout::new(320, 106);
    let snapshot = f.snapshot();
    if !screen.capture(106).contains("FINAL_OUTPUT") {
        return false;
    }
    for y in 0..snapshot.rows {
        for x in 0..snapshot.cols {
            let actual = &screen.grid.cells[(layout.terminal_y + y) * 320 + layout.terminal_x + x];
            let expected = &snapshot.cells[y * snapshot.cols + x];
            if actual != expected {
                return false;
            }
        }
    }
    true
}
fn unchanged(f: &Fixture, tab: &TabView, expected: &[u8]) {
    let snapshot = f.snapshot();
    let now = snapshot.session().unwrap();
    assert_eq!(
        (now.id, now.pid, &now.run, now.alive),
        (tab.id, tab.pid, &tab.run, true)
    );
    assert_eq!(fs::read(f.root.join("output-input")).unwrap(), expected);
}
fn command(f: &Fixture, remote: bool) -> Command {
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg(if remote { "_bridge" } else { "attach" })
        .env("HOME", &f.root)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env_remove("XDG_STATE_HOME");
    command
}
#[test]
fn local_input_continues_with_output_blocked_then_reconstructs_latest_grid() {
    let f = Fixture::new();
    let tab = producer(&f);
    let (mut master, mut child) =
        os::spawn_command_pty(&f.root, &mut command(&f, false), 320, 106).unwrap();
    let mut screen = Terminal::new(320, 106);
    let end = Instant::now() + Duration::from_secs(4);
    while !screen.capture(106).contains("OUTPUT_") {
        assert!(
            Instant::now() < end,
            "owned UI did not display producer output"
        );
        read_ui_bytes(&mut master, &mut screen, &mut Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(200));
    master.write_all(b"typed-under-pressure;").unwrap();
    received(&f, b"typed-under-pressure;");
    master.write_all(b"END_OUTPUT;").unwrap();
    received(&f, b"typed-under-pressure;END_OUTPUT;");
    let end = Instant::now() + Duration::from_secs(4);
    while !current_grid(&f, &screen) {
        assert!(
            Instant::now() < end,
            "queued local diffs did not reconstruct final grid"
        );
        read_ui_bytes(&mut master, &mut screen, &mut Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    let cleanup = finish_ui_output(&mut master, &mut screen, &mut child);
    assert!(cleanup.windows(8).any(|w| w == b"\x1b[?1049l"));
    unchanged(&f, &tab, b"typed-under-pressure;END_OUTPUT;");
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn remote_output_backpressure_preserves_input_packets_cleanup_and_refresh_order() {
    let f = Fixture::new();
    let tab = producer(&f);
    let mut child = ChildGuard(
        command(&f, true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut input = child.0.stdin.take().unwrap();
    let mut output = child.0.stdout.take().unwrap();
    let hello = Packet::read(&mut output).unwrap();
    protocol::hello_reply(&hello, 320, 106)
        .unwrap()
        .write(&mut input)
        .unwrap();
    os::nonblock(output.as_raw_fd()).unwrap();
    let mut screen = Terminal::new(320, 106);
    let mut buffer = Vec::new();
    let mut packets = Vec::new();
    let read = |output: &mut std::process::ChildStdout,
                buffer: &mut Vec<u8>,
                screen: &mut Terminal,
                packets: &mut Vec<Packet>| {
        let mut bytes = [0; 65536];
        loop {
            match output.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => buffer.extend_from_slice(&bytes[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("remote read: {e}"),
            }
        }
        while let Some(packet) = Packet::take(buffer).unwrap() {
            if packet.tag == protocol::OUTPUT {
                screen.feed(&packet.data);
            }
            packets.push(packet);
        }
    };
    let end = Instant::now() + Duration::from_secs(4);
    while !screen.capture(106).contains("OUTPUT_") {
        assert!(Instant::now() < end);
        read(&mut output, &mut buffer, &mut screen, &mut packets);
        std::thread::sleep(Duration::from_millis(5));
    }
    // An invalid image offer still requires an exact ordered RESULT receipt.
    std::thread::sleep(Duration::from_millis(200));
    Packet::new(
        protocol::IMAGE,
        7,
        (protocol::IMAGE_LIMIT + 1).to_be_bytes(),
    )
    .write(&mut input)
    .unwrap();
    Packet::new(protocol::KEYS, 0, b"remote-under-pressure;")
        .write(&mut input)
        .unwrap();
    received(&f, b"remote-under-pressure;");
    Packet::new(protocol::KEYS, 0, b"END_OUTPUT;")
        .write(&mut input)
        .unwrap();
    received(&f, b"remote-under-pressure;END_OUTPUT;");
    let end = Instant::now() + Duration::from_secs(4);
    while !current_grid(&f, &screen)
        || !packets
            .iter()
            .any(|p| p.tag == protocol::RESULT && p.id == 7)
    {
        assert!(
            Instant::now() < end,
            "remote diffs or receipt did not arrive"
        );
        read(&mut output, &mut buffer, &mut screen, &mut packets);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        packets
            .iter()
            .filter(|p| p.tag == protocol::RESULT && p.id == 7)
            .count(),
        1
    );
    packets.clear();
    Packet::new(protocol::KEYS, 0, b"\0 R")
        .write(&mut input)
        .unwrap();
    let end = Instant::now() + Duration::from_secs(4);
    while !packets.iter().any(|p| p.tag == protocol::HELLO) {
        assert!(
            Instant::now() < end,
            "refresh did not drain and re-handshake"
        );
        read(&mut output, &mut buffer, &mut screen, &mut packets);
        std::thread::sleep(Duration::from_millis(5));
    }
    let cleanup = packets
        .iter()
        .position(|p| p.tag == protocol::OUTPUT && p.data.windows(8).any(|w| w == b"\x1b[?1049l"))
        .unwrap();
    let hello = packets
        .iter()
        .position(|p| p.tag == protocol::HELLO)
        .unwrap();
    assert!(cleanup < hello, "old cleanup must precede new handshake");
    protocol::hello_reply(&packets[hello], 320, 106)
        .unwrap()
        .write(&mut input)
        .unwrap();
    packets.clear();
    Packet::new(protocol::KEYS, 0, b"\0q")
        .write(&mut input)
        .unwrap();
    let end = Instant::now() + Duration::from_secs(4);
    loop {
        read(&mut output, &mut buffer, &mut screen, &mut packets);
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < end, "remote detach did not drain output");
        std::thread::sleep(Duration::from_millis(5));
    }
    read(&mut output, &mut buffer, &mut screen, &mut packets);
    assert!(buffer.is_empty(), "truncated remote packet at detach");
    assert!(
        packets
            .iter()
            .any(|p| p.tag == protocol::OUTPUT && p.data.windows(8).any(|w| w == b"\x1b[?1049l"))
    );
    unchanged(&f, &tab, b"remote-under-pressure;END_OUTPUT;");
}
