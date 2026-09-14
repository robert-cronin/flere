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

fn shift_mouse_boundary(bytes: &[u8], boundary: &[u8]) {
    let boundary = bytes
        .windows(boundary.len())
        .position(|w| w == boundary)
        .unwrap();
    let request = bytes[..boundary].windows(5).rposition(|w| w == b"\x1b[>0s");
    assert!(
        request.is_some(),
        "terminal Shift policy was not set before mouse boundary"
    );
    assert!(
        !bytes[request.unwrap()..boundary]
            .windows(5)
            .any(|w| w == b"\x1b[>1s")
    );
}

fn shift_mouse_read(master: &mut fs::File, needle: &[u8]) -> Vec<u8> {
    let mut screen = Terminal::new(100, 30);
    let mut bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !bytes.windows(needle.len()).any(|w| w == needle) {
        assert!(
            Instant::now() < deadline,
            "missing mouse setup/cleanup bytes"
        );
        assert!(bytes.len() < 1024 * 1024, "excessive fixture output");
        read_ui_bytes(master, &mut screen, &mut bytes);
        std::thread::sleep(Duration::from_millis(5));
    }
    bytes
}

#[test]
fn shift_mouse_policy_precedes_capture_locally_remotely_and_with_an_older_peer() {
    let f = Fixture::new();
    f.new_workspace("Shift gestures");
    // This peer models old core startup: it enables capture without setting the
    // Shift policy, then disconnects on one literal key without remote cleanup.
    let ssh = f.root.join("shift-ssh");
    fs::write(
        &ssh,
        r#"#!/usr/bin/python3
import os,struct,sys
def send(tag,data):
 b=bytes([tag])+bytes(8)+data
 sys.stdout.buffer.write(struct.pack('>I',len(b))+b);sys.stdout.buffer.flush()
def read():
 n=struct.unpack('>I',sys.stdin.buffer.read(4))[0]
 return sys.stdin.buffer.read(n)
send(1,os.environ['FIXTURE_VERSION'].encode()+b'a'*32)
assert read()[0]==1
send(16,b'\x1b[?1049h\x1b[?1000h\x1b[?1006hSHIFT_READY')
while True:
 b=read()
 if b[0]==2 and b[9:]==b'x':break
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    for companion in [false, true] {
        // Model an inherited per-terminal Shift-reporting request. Switching
        // alternate screens alone must not be mistaken for restoring this mode.
        let mut command = Command::new("/bin/sh");
        fixture_home(&mut command, &f.root).args([
            "-c",
            "printf '\\033[>1s'; exec \"$@\"",
            "shift-fixture",
            "/usr/bin/env",
            "-u",
            "FLERE",
        ]);
        let ready: &[u8] = if companion {
            command
                .arg(companion_binary())
                .args(["fixture-host", "--ssh", ssh.to_str().unwrap()])
                .env(
                    "FIXTURE_VERSION",
                    std::str::from_utf8(protocol::VERSION).unwrap(),
                );
            b"SHIFT_READY"
        } else {
            command
                .arg(env!("CARGO_BIN_EXE_flere"))
                .arg("--state")
                .arg(&f.state)
                .arg("attach");
            b"\x1b[?1006h"
        };
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, 100, 30).unwrap();
        let startup = shift_mouse_read(&mut master, ready);
        master
            .write_all(if companion { b"x" } else { b"\0q" })
            .unwrap();
        let cleanup = shift_mouse_read(&mut master, b"\x1b[?1049l");
        assert!(child.wait().unwrap().success());
        assert!(startup.windows(5).any(|w| w == b"\x1b[>1s"));
        shift_mouse_boundary(&startup, b"\x1b[?1000h");
        shift_mouse_boundary(&cleanup, b"\x1b[?1049l");
    }
    let mut bridge = Bridge::start(&f, 100, 30, std::env::var("PATH").unwrap_or_default());
    let hello = bridge.until(|p, _| p.tag == protocol::HELLO);
    protocol::hello_reply(&hello, 100, 30)
        .unwrap()
        .write(bridge.input.as_mut().unwrap())
        .unwrap();
    let startup = bridge
        .until(|p, _| p.tag == protocol::OUTPUT && p.data.windows(8).any(|w| w == b"\x1b[?1000h"));
    bridge.input.take();
    let cleanup = bridge
        .until(|p, _| p.tag == protocol::OUTPUT && p.data.windows(8).any(|w| w == b"\x1b[?1049l"));
    bridge.disconnect();
    shift_mouse_boundary(&startup.data, b"\x1b[?1000h");
    shift_mouse_boundary(&cleanup.data, b"\x1b[?1049l");
    assert!(
        f.snapshot().session().unwrap().alive,
        "UI detach stopped the shell"
    );
}
