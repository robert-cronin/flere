use super::*;
#[path = "../splits/screenshot.rs"]
mod split_screenshot;
use flere::remote_protocol::{self as protocol, Packet};
use std::{process::ChildStdin, sync::mpsc};
mod drop;
mod drop_backend;
mod environment;
use environment::fixture_home;
mod handshake;
mod hyperlinks;
#[path = "../image_gallery/mod.rs"]
mod image_gallery;
mod tools;
mod update;
struct Bridge {
    child: Child,
    input: Option<ChildStdin>,
    packets: mpsc::Receiver<Packet>,
    screen: Terminal,
    allow_cat_pixels: bool,
    allow_ui_clipboard: bool,
}
impl Bridge {
    fn new(f: &Fixture, width: u16, height: u16) -> Self {
        Self::with_path(f, width, height, std::env::var("PATH").unwrap_or_default())
    }
    fn with_path(f: &Fixture, width: u16, height: u16, path: String) -> Self {
        let mut bridge = Self::start(f, width, height, path);
        let hello = bridge.packets.recv_timeout(Duration::from_secs(3)).unwrap();
        let mut bytes = Vec::new();
        protocol::hello_reply(&hello, width, height)
            .unwrap()
            .write(&mut bytes)
            .unwrap();
        // A network stream may split anywhere, including inside a length prefix.
        for byte in bytes {
            bridge.input.as_mut().unwrap().write_all(&[byte]).unwrap();
        }
        bridge.wait_screen("FLERE");
        bridge
    }
    fn start(f: &Fixture, width: u16, height: u16, path: String) -> Self {
        let mut child = fixture_home(&mut outer_ui_command(), &f.root)
            .arg("--state")
            .arg(&f.state)
            .arg("_bridge")
            .env("PATH", path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (tx, packets) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(packet) = Packet::read(&mut output) {
                if tx.send(packet).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input: Some(input),
            packets,
            screen: Terminal::new(width as usize, height as usize),
            allow_cat_pixels: false,
            allow_ui_clipboard: false,
        }
    }
    fn send(&mut self, tag: u8, id: u64, data: impl Into<Vec<u8>>) {
        Packet::new(tag, id, data)
            .write(self.input.as_mut().unwrap())
            .unwrap();
    }
    fn until(&mut self, mut pred: impl FnMut(&Packet, &Terminal) -> bool) -> Packet {
        let end = Instant::now() + Duration::from_secs(4);
        loop {
            let packet = self
                .packets
                .recv_timeout(end.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|e| {
                    panic!(
                        "remote packet timeout: {e}; screen {}",
                        self.screen.capture(100)
                    )
                });
            if packet.tag == protocol::OUTPUT {
                assert!(
                    self.allow_ui_clipboard || !packet.data.windows(5).any(|w| w == b"\x1b]52;"),
                    "native clipboard OSC leaked"
                );
                assert!(
                    !packet.data.windows(2).any(|w| w == b"\x1bP")
                        || (self.allow_cat_pixels
                            && packet
                                .data
                                .windows(b"\x1bP7;1q\"1;1;".len())
                                .any(|w| w == b"\x1bP7;1q\"1;1;")),
                    "native graphics DCS leaked"
                );
                self.screen.feed(&packet.data);
            }
            if pred(&packet, &self.screen) {
                return packet;
            }
        }
    }
    fn wait_screen(&mut self, text: &str) {
        if !self.screen.capture(100).contains(text) {
            self.until(|_, s| s.capture(100).contains(text));
        }
    }
    fn offer(&mut self, id: u64, len: usize) {
        self.send(protocol::IMAGE, id, (len as u64).to_be_bytes());
        let p =
            self.until(|p, _| p.id == id && matches!(p.tag, protocol::READY | protocol::RESULT));
        assert_eq!(p.tag, protocol::READY, "{:?}", p);
    }
    fn chunk(&mut self, id: u64, offset: u64, bytes: &[u8]) {
        let mut data = offset.to_be_bytes().to_vec();
        data.extend_from_slice(bytes);
        self.send(protocol::DATA, id, data);
    }
    fn result(&mut self, id: u64, success: bool) {
        let p = self.until(|p, _| p.tag == protocol::RESULT && p.id == id);
        assert_eq!(
            p.data[0],
            u8::from(success),
            "{}",
            String::from_utf8_lossy(&p.data)
        );
    }
    fn disconnect(&mut self) {
        self.input.take();
        let end = Instant::now() + Duration::from_secs(3);
        while self.child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < end, "remote UI did not exit on SSH EOF");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.input.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
fn native(f: &Fixture) -> (u64, TabView) {
    use serde_json::json;
    f.new_workspace("Remote image chat");
    let wid = f.snapshot().active;
    let script = f.root.join("image-native.py");
    fs::write(&script, include_str!("../fixtures/image_native.py")).unwrap();
    fs::write(
        f.state.join("harnesses.json"),
        serde_json::to_vec(&json!([{"name":"codex","command":["/usr/bin/python3",script]}]))
            .unwrap(),
    )
    .unwrap();
    f.req(&["native", &wid.to_string(), "codex", ""]);
    let t = f.snapshot().session().unwrap().clone();
    f.wait_text(&t, "IMAGE_NATIVE_DRAFT");
    (wid, t)
}
fn png() -> Vec<u8> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNg+H8HAALeAdxNj6zLAAAAAElFTkSuQmCC").unwrap()
}
fn input(f: &Fixture) -> Vec<u8> {
    fs::read(f.root.join("image-input")).unwrap()
}
fn wait_input(f: &Fixture, len: usize) -> Vec<u8> {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        let b = input(f);
        if b.len() >= len {
            return b;
        }
        assert!(Instant::now() < end, "native input did not arrive");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn parts(f: &Fixture) -> usize {
    fs::read_dir(f.root.join(".cache/flere/attachments"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|s| s == "part"))
        .count()
}
#[test]
fn remote_ui_image_paste_is_private_exact_and_never_submits() {
    for (width, height) in [(60, 24), (236, 54)] {
        let f = Fixture::new();
        let other = f.new_workspace("other");
        let other_wid = f.snapshot().active;
        let (wid, t) = native(&f);
        let mut b = Bridge::new(&f, width, height);
        b.wait_screen("IMAGE_NATIVE_DRAFT");
        let image = png();
        b.offer(1, image.len());
        b.chunk(1, 0, &image[..20]);
        assert!(input(&f).is_empty());
        b.send(protocol::END, 1, Vec::new());
        b.result(1, false);
        assert_eq!(parts(&f), 0);
        b.offer(2, image.len());
        b.chunk(2, 0, &image[..20]);
        b.chunk(2, 20, &image[20..]);
        b.send(protocol::END, 2, Vec::new());
        b.result(2, true);
        let received = wait_input(&f, 13);
        assert!(received.starts_with(b"\x1b[200~") && received.ends_with(b"\x1b[201~"));
        assert!(!received.contains(&b'\r') && !received.contains(&b'\n'));
        let path = PathBuf::from(std::str::from_utf8(&received[6..received.len() - 6]).unwrap());
        assert_eq!(fs::read(&path).unwrap(), image);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(path.starts_with(f.root.join(".cache/flere/attachments")));
        b.send(protocol::END, 2, Vec::new()); // Repeated completion is not a repeated native paste.
        b.send(protocol::KEYS, 0, b"\x1b[200~text draft\x1b[201~");
        let mut expected = received;
        expected.extend_from_slice(b"\x1b[200~text draft\x1b[201~");
        assert_eq!(wait_input(&f, expected.len()), expected);
        b.offer(3, image.len());
        f.req(&["focus", &other_wid.to_string(), &other.id.to_string()]);
        // Cancellation may arrive before the next paint; consume its result first.
        b.result(3, false);
        if b.screen.capture(100).contains("IMAGE_NATIVE_DRAFT") {
            b.until(|p, s| {
                p.tag == protocol::OUTPUT && !s.capture(100).contains("IMAGE_NATIVE_DRAFT")
            });
        }
        f.req(&["focus", &wid.to_string(), &t.id.to_string()]);
        b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("IMAGE_NATIVE_DRAFT"));
        b.chunk(3, 0, &image); // Returning to the same chat does not revive the cancelled transfer.
        b.send(protocol::END, 3, Vec::new());
        assert_eq!(input(&f), expected);
        b.offer(4, image.len());
        b.send(protocol::KEYS, 0, b"\x1b");
        b.result(4, false);
        assert_eq!(input(&f), expected);
        assert_eq!(parts(&f), 0);
        b.offer(5, image.len());
        b.chunk(5, 1, &image);
        b.result(5, false);
        assert_eq!(parts(&f), 0);
        b.offer(6, image.len());
        b.chunk(6, 0, &image[..10]);
        b.disconnect();
        assert_eq!(parts(&f), 0);
        assert_eq!(input(&f), expected);
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
    }
}
#[test]
fn image_tickets_reject_wrong_native_identity_duplicate_and_refresh() {
    let f = Fixture::new();
    let shell = f.new_workspace("shell");
    let shell_wid = f.snapshot().active;
    let (wid, t) = native(&f);
    let epoch = f.snapshot().epoch;
    for (ep, w, id, run) in [
        ("wrong", wid, t.id, t.run.as_str()),
        (epoch.as_str(), wid, t.id, "wrong"),
        (epoch.as_str(), shell_wid, shell.id, shell.run.as_str()),
    ] {
        assert!(
            wire::request(
                &f.state,
                &["image-ticket", ep, &w.to_string(), &id.to_string(), run]
            )
            .is_err()
        );
    }
    let ticket = String::from_utf8(f.req(&[
        "image-ticket",
        &epoch,
        &wid.to_string(),
        &t.id.to_string(),
        &t.run,
    ]))
    .unwrap();
    assert!(
        wire::request(
            &f.state,
            &[
                "image-paste",
                &ticket,
                &wire::hex(
                    f.root
                        .join("not-an-attachment.png")
                        .to_str()
                        .unwrap()
                        .as_bytes()
                )
            ]
        )
        .is_err()
    );
    assert!(wire::request(&f.state, &["image-paste", &ticket, ""]).is_err());
    assert!(input(&f).is_empty());
    let ticket = String::from_utf8(f.req(&[
        "image-ticket",
        &epoch,
        &wid.to_string(),
        &t.id.to_string(),
        &t.run,
    ]))
    .unwrap();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let end = Instant::now() + Duration::from_secs(4);
    loop {
        if wire::request(&f.state, &["refresh-status"])
            .is_ok_and(|s| String::from_utf8_lossy(&s).contains("Refreshed"))
        {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(wire::request(&f.state, &["image-paste", &ticket, ""]).is_err());
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().epoch, epoch);
    assert!(input(&f).is_empty());
}
#[test]
fn remote_protocol_rejects_bad_handshake_and_oversized_frames_without_input() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    for bytes in [vec![0xff, 0xff, 0xff, 0xff], {
        let mut b = Vec::new();
        Packet::new(protocol::HELLO, 0, b"wrong-version")
            .write(&mut b)
            .unwrap();
        b
    }] {
        let mut c = outer_ui_command()
            .arg("--state")
            .arg(&f.state)
            .arg("_bridge")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut out = c.stdout.take().unwrap();
        assert_eq!(Packet::read(&mut out).unwrap().tag, protocol::HELLO);
        c.stdin.take().unwrap().write_all(&bytes).unwrap();
        assert!(!c.wait().unwrap().success());
        assert!(input(&f).is_empty());
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    }
}

#[test]
fn remote_upload_rejects_bounds_bad_images_competing_clients_and_cleans_crash_partials() {
    let f = Fixture::new();
    native(&f);
    let mut b = Bridge::new(&f, 100, 30);
    b.wait_screen("IMAGE_NATIVE_DRAFT");
    b.send(
        protocol::IMAGE,
        1,
        (protocol::IMAGE_LIMIT + 1).to_be_bytes(),
    );
    b.result(1, false);
    b.offer(2, png().len());
    let mut other = Bridge::new(&f, 100, 30);
    other.wait_screen("IMAGE_NATIVE_DRAFT");
    other.send(protocol::IMAGE, 1, (png().len() as u64).to_be_bytes());
    other.result(1, false);
    assert_eq!(parts(&f), 1);
    b.send(protocol::CANCEL, 2, Vec::new());
    b.result(2, false);
    assert_eq!(parts(&f), 0);
    for (id, bytes) in [
        (3, vec![0; png().len()]),
        (4, {
            let mut bytes = png();
            bytes[16..20].copy_from_slice(&20_000_001u32.to_be_bytes());
            bytes
        }),
    ] {
        b.offer(id, bytes.len());
        b.chunk(id, 0, &bytes);
        b.send(protocol::END, id, Vec::new());
        b.result(id, false);
        assert_eq!(parts(&f), 0);
    }
    b.offer(5, png().len());
    b.send(protocol::KEYS, 0, b"\0"); // Navigation cancels even without a native target change.
    b.result(5, false);
    assert_eq!(parts(&f), 0);
    assert!(input(&f).is_empty());
    // A killed attachment cannot run Drop. The next exclusive upload removes its nonce partial.
    other.offer(2, png().len());
    other.child.kill().unwrap();
    other.child.wait().unwrap();
    assert_eq!(parts(&f), 1);
    b.send(protocol::KEYS, 0, b"\0");
    b.offer(6, png().len());
    assert_eq!(parts(&f), 1);
    b.send(protocol::CANCEL, 6, Vec::new());
    b.result(6, false);
    assert_eq!(parts(&f), 0);
    // Completed drafts consume a bounded cache; rejection does not evict them.
    let retained = f.root.join(".cache/flere/attachments/retained-draft.png");
    let file = fs::File::create(&retained).unwrap();
    file.set_len(256 * 1024 * 1024).unwrap();
    b.send(protocol::IMAGE, 7, (png().len() as u64).to_be_bytes());
    b.result(7, false);
    assert_eq!(file.metadata().unwrap().len(), 256 * 1024 * 1024);
    assert_eq!(parts(&f), 0);
    assert!(input(&f).is_empty());
}

#[test]
fn remote_refresh_handshake_discards_inflight_chunks_without_native_replay() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    let mut b = Bridge::new(&f, 100, 30);
    b.wait_screen("IMAGE_NATIVE_DRAFT");
    b.offer(1, png().len());
    b.send(protocol::KEYS, 0, b"\0 R");
    // These frames may already be in the pipe or UI buffer when the UI execs.
    b.chunk(1, 0, &png());
    b.send(protocol::END, 1, Vec::new());
    let hello = b.until(|p, _| p.tag == protocol::HELLO);
    let reply = protocol::hello_reply(&hello, 100, 30).unwrap();
    b.send(reply.tag, reply.id, reply.data);
    b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("IMAGE_NATIVE_DRAFT"));
    assert_eq!(parts(&f), 0);
    assert!(input(&f).is_empty());
    b.send(protocol::KEYS, 0, b"draft after refresh");
    assert_eq!(wait_input(&f, 19), b"draft after refresh");
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
}

fn companion_binary() -> &'static PathBuf {
    static BINARY: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BINARY.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("companion");
        let output = Command::new(env!("CARGO"))
            .args(["build", "--offline", "--manifest-path"])
            .arg(root.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(root.join("target"))
            .env_remove("RUSTFLAGS")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        root.join("target/debug/flere-connect")
    })
}

#[test]
fn companion_pty_explicit_image_preserves_text_and_reconnects_after_refresh() {
    companion_pty_image_text_refresh(false);
}

#[cfg(target_os = "linux")]
#[test]
fn companion_pty_linux_clipboard_gestures_preserve_text_and_reconnect_after_refresh() {
    companion_pty_image_text_refresh(true);
}

fn companion_pty_image_text_refresh(clipboard_gestures: bool) {
    // Linux's optional helper can be replaced by an owned executable fixture.
    // macOS reads the real native pasteboard, so its clipboard implementation is
    // covered by the private named-pasteboard unit test instead. --image exercises
    // the complete portable transport here without accessing the general clipboard.
    assert!(!clipboard_gestures || cfg!(target_os = "linux"));
    let binary = companion_binary();
    for (width, height) in [(60, 24), (236, 54)] {
        let f = Fixture::new();
        let (_, t) = native(&f);
        let ssh = f.root.join("fixture-ssh");
        fs::write(&ssh, "#!/usr/bin/python3\nimport os,sys,shlex,pathlib,json\nassert sys.argv[1:4]==['-T','--','fixture-host']\nargs=shlex.split(sys.argv[4]);assert args.pop(0)=='exec'\npathlib.Path('ssh-argv').write_text(json.dumps(args))\nos.execv(args[0],args)\n").unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
        let remote = f.root.join("rail hand'candidate");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_flere"), &remote).unwrap();
        if clipboard_gestures {
            let clipboard = f.root.join("wl-paste");
            fs::write(&clipboard, "#!/usr/bin/python3\nimport pathlib,sys\np=pathlib.Path('clipboard.png')\nif not p.exists():sys.exit(1)\nsys.stdout.buffer.write(p.read_bytes())\n").unwrap();
            fs::set_permissions(&clipboard, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(f.root.join("clipboard.png"), png()).unwrap();
        let mut command = Command::new("/usr/bin/env");
        fixture_home(&mut command, &f.root)
            .args(["-u", "FLERE"])
            .arg(binary)
            .arg("fixture-host")
            .arg("--remote")
            .arg(&remote)
            .arg("--state")
            .arg(&f.state)
            .arg("--ssh")
            .arg(&ssh)
            .env("PATH", &f.root);
        if !clipboard_gestures {
            command.arg("--image").arg(f.root.join("clipboard.png"));
        }
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "IMAGE_NATIVE_DRAFT");
        // Physical terminal query replies may arrive a byte at a time; they are not native input.
        for byte in b"\x1b[?65;1;4;6c\x1b[6;20;10t" {
            master.write_all(&[*byte]).unwrap();
        }
        if clipboard_gestures {
            master.write_all(b"\x16").unwrap();
        }
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).ends_with(b"\x1b[201~")
        });
        drain_pty(&mut master, &mut screen, "Image paste queued");
        let received = input(&f);
        assert!(received.starts_with(b"\x1b[200~"));
        let path = PathBuf::from(std::str::from_utf8(&received[6..received.len() - 6]).unwrap());
        assert_eq!(fs::read(path).unwrap(), png());
        assert!(!received.contains(&b'\r') && !received.contains(&b'\n'));
        // A text paste retains literal controls inside the bracketed paste.
        master
            .write_all(b"\x1b[200~text\x16inside\x1b[201~")
            .unwrap();
        let mut expected = received;
        expected.extend_from_slice(b"\x1b[200~text\x16inside\x1b[201~");
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).len() == expected.len()
        });
        assert_eq!(input(&f), expected);
        if clipboard_gestures {
            // Empty terminal paste is the other supported Linux image gesture.
            master.write_all(b"\x1b[200~\x1b[201~").unwrap();
            wait_current_ui(&mut master, &mut screen, |_| {
                input(&f).len() > expected.len() && input(&f).ends_with(b"\x1b[201~")
            });
            expected = input(&f);
            fs::remove_file(f.root.join("clipboard.png")).unwrap();
            master.write_all(b"\x16").unwrap(); // No image: preserve the native control.
            expected.push(0x16);
            wait_current_ui(&mut master, &mut screen, |_| {
                input(&f).len() == expected.len()
            });
            assert_eq!(input(&f), expected);
        }
        // A new screen model only sees the bridge's post-refresh full paint.
        screen = Terminal::new(width as usize, height as usize);
        master.write_all(b"\0 R").unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            wire::request(&f.state, &["refresh-status"])
                .is_ok_and(|s| String::from_utf8_lossy(&s).contains("Refreshed"))
        });
        // Wait for the re-executed bridge's full paint, not a cached pre-refresh cell.
        drain_pty(&mut master, &mut screen, "IMAGE_NATIVE_DRAFT");
        master.write_all(b"after refresh").unwrap();
        expected.extend_from_slice(b"after refresh");
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).len() == expected.len()
        });
        assert_eq!(input(&f), expected);
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
        assert_eq!(f.snapshot().session().unwrap().run, t.run);
        finish_ui(&mut master, &mut screen, &mut child);
        assert_eq!(input(&f), expected);
        assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
        let logs: String = fs::read_dir(f.root.join(".local/state/flere-connect/diagnostics"))
            .unwrap()
            .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert!(
            logs.contains("handshake")
                && logs.contains("refresh-handshake")
                && logs.contains("connection-closed")
        );
        assert!(
            !logs.contains("after refresh")
                && !logs.contains("text\\x16inside")
                && !logs.contains("IMAGE_NATIVE_DRAFT")
        );
        let remote_logs: String = fs::read_dir(f.state.join("diagnostics"))
            .unwrap()
            .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
            .collect();
        assert!(remote_logs.contains("refresh-exec") && remote_logs.contains("leaving"));
        assert!(!remote_logs.contains("IMAGE_NATIVE_DRAFT"));
    }
}

fn select_preview(b: &mut Bridge, f: &Fixture) {
    let mut entries: Vec<_> = fs::read_dir(&f.root)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            (
                e.file_name().to_string_lossy().to_lowercase(),
                e.file_type().unwrap().is_dir(),
            )
        })
        .collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let index = 1 + entries.iter().position(|e| e.0 == "preview.png").unwrap();
    // The explorer loads asynchronously. Wait for the listing before moving,
    // then confirm the selected-file detail instead of merely a visible row.
    b.send(protocol::KEYS, 0, b"\0l");
    b.wait_screen("preview.png");
    b.send(protocol::KEYS, 0, vec![b'j'; index]);
    b.wait_screen("preview.png ·");
}
#[test]
fn remote_image_preview_transfer_modal_resize_target_and_detach_preserve_native() {
    for (width, height) in [(236, 54), (60, 24)] {
        let f = Fixture::new();
        let (wid, t) = native(&f);
        fs::write(f.root.join("preview.png"), png()).unwrap();
        let mut b = Bridge::new(&f, width, height);
        b.wait_screen("IMAGE_NATIVE_DRAFT");
        b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
        select_preview(&mut b, &f);
        b.send(protocol::KEYS, 0, b"\r");
        let begin = b.until(|p, _| p.tag == protocol::PREVIEW_BEGIN);
        assert_eq!(begin.id, 1);
        assert_eq!(begin.data, (png().len() as u64).to_be_bytes());
        let mut image = Vec::new();
        b.until(|p, _| {
            if p.tag == protocol::PREVIEW_DATA {
                assert_eq!(p.id, 1);
                assert_eq!(
                    u64::from_be_bytes(p.data[..8].try_into().unwrap()),
                    image.len() as u64
                );
                image.extend_from_slice(&p.data[8..]);
            }
            p.tag == protocol::PREVIEW_END
        });
        assert_eq!(image, png());
        assert!(b.screen.capture(100).contains("IMAGE PREVIEW"));
        b.send(protocol::KEYS, 0, b"typing\x1b[200~paste\x1b[201~\x1b[A");
        b.send(protocol::RESIZE, 0, protocol::size_bytes(80, 28));
        b.screen = Terminal::new(80, 28);
        b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("IMAGE PREVIEW"));
        assert!(input(&f).is_empty());
        b.send(protocol::KEYS, 0, b"q");
        let clear = b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR);
        assert_eq!(clear.id, 1);
        b.until(|p, s| p.tag == protocol::OUTPUT && !s.capture(100).contains("IMAGE PREVIEW"));
        b.send(protocol::KEYS, 0, b"p");
        b.until(|p, _| p.tag == protocol::PREVIEW_END && p.id == 2);
        b.send(protocol::PREVIEW_ERROR, 2, b"Cannot decode this image");
        b.wait_screen("Cannot decode this image");
        b.send(protocol::KEYS, 0, b"\x1b");
        b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR && p.id == 2);
        b.send(protocol::KEYS, 0, b"p");
        b.until(|p, _| p.tag == protocol::PREVIEW_END && p.id == 3);
        f.req(&[
            "focus",
            &wid.to_string(),
            &f.snapshot().workspace().unwrap().tabs[0].id.to_string(),
        ]);
        b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR && p.id == 3);
        b.until(|p, s| p.tag == protocol::OUTPUT && !s.capture(100).contains("IMAGE PREVIEW"));
        assert!(input(&f).is_empty());
        b.send(protocol::KEYS, 0, b"p");
        b.until(|p, _| p.tag == protocol::PREVIEW_END && p.id == 4);
        b.disconnect();
        b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR && p.id == 4);
        b.until(|p, _| p.tag == protocol::OUTPUT && p.data.windows(8).any(|w| w == b"\x1b[?1049l"));
        let now = f.snapshot();
        let current = now
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .find(|s| s.id == t.id)
            .unwrap();
        assert_eq!(current.pid, t.pid);
        assert_eq!(current.run, t.run);
        assert!(input(&f).is_empty());
        assert!(!f.root.join(".cache/flere/attachments").exists());
    }
}
#[test]
fn remote_image_preview_requires_capability_and_rejects_files_without_native_input() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    let path = f.root.join("preview.png");
    fs::write(&path, png()).unwrap();
    let mut b = Bridge::new(&f, 100, 30);
    select_preview(&mut b, &f);
    b.send(protocol::KEYS, 0, b"p");
    b.wait_screen("Image preview unavailable:");
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    fs::write(&path, b"\x1bPmalicious non-image\x1b\\").unwrap();
    b.send(protocol::KEYS, 0, b"p");
    b.wait_screen("PNG and JPEG");
    fs::File::create(&path)
        .unwrap()
        .set_len(protocol::IMAGE_LIMIT + 1)
        .unwrap();
    b.send(protocol::KEYS, 0, b"p");
    b.wait_screen("20 MiB");
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink("/dev/zero", &path).unwrap();
    b.send(protocol::KEYS, 0, b"p");
    b.wait_screen(&format!("os error {}", libc::ELOOP));
    assert!(input(&f).is_empty());
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
}

#[test]
fn remote_image_preview_cancels_midstream_and_ignores_stale_errors() {
    let f = Fixture::new();
    let (_, t) = native(&f);
    let mut bytes = png();
    bytes.resize(10 * 1024 * 1024, 0);
    let path = f.root.join("preview.png");
    fs::write(&path, &bytes).unwrap();
    let mut b = Bridge::new(&f, 100, 30);
    b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
    select_preview(&mut b, &f);
    b.send(protocol::KEYS, 0, b"p");
    b.until(|p, _| p.tag == protocol::PREVIEW_DATA);
    b.send(protocol::KEYS, 0, b"q");
    let mut ended = false;
    b.until(|p, _| {
        ended |= p.tag == protocol::PREVIEW_END;
        p.tag == protocol::PREVIEW_CLEAR
    });
    assert!(!ended, "cancelled preview finished unexpectedly");
    fs::write(&path, png()).unwrap();
    b.send(protocol::KEYS, 0, b"p");
    b.until(|p, _| p.tag == protocol::PREVIEW_END && p.id == 2);
    b.send(protocol::PREVIEW_ERROR, 1, b"stale error");
    b.send(protocol::RESIZE, 0, protocol::size_bytes(90, 30));
    b.screen = Terminal::new(90, 30);
    b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("IMAGE PREVIEW"));
    assert!(!b.screen.capture(100).contains("stale error"));
    // q then p can be coalesced by SSH: clear and restore the same modal in order.
    b.send(protocol::KEYS, 0, b"qp");
    b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR && p.id == 2);
    b.screen = Terminal::new(90, 30);
    b.until(|p, _| p.tag == protocol::PREVIEW_END && p.id == 3);
    assert!(b.screen.capture(100).contains("IMAGE PREVIEW"));
    b.send(protocol::KEYS, 0, b"\x1b");
    b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR && p.id == 3);
    assert!(input(&f).is_empty());
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
}

#[test]
fn remote_clicked_wrapped_image_path_preserves_draft_and_cannot_target_another_run() {
    for (width, height) in [(209, 49), (60, 24)] {
        let f = Fixture::new();
        let path = f.root.join("a comparison image.png");
        fs::write(&path, png()).unwrap();
        let (wid, t) = native(&f);
        let mut b = Bridge::new(&f, width, height);
        flere::workspace::atomic_write(
            &f.root.join("image-display"),
            format!("\r\n({})\r\nDO_NOT_SUBMIT", path.display()).as_bytes(),
        )
        .unwrap();
        b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
        b.wait_screen("DO_NOT_SUBMIT");
        let l = flere::ui::Layout::new(width as usize, height as usize);
        let (x, y) = (l.terminal_x + 4, l.terminal_y + 1);
        b.send(
            protocol::KEYS,
            0,
            format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1),
        );
        b.until(|p, _| p.tag == protocol::PREVIEW_END);
        assert!(input(&f).is_empty());
        b.send(protocol::KEYS, 0, b"q");
        b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR);
        b.until(|p, s| p.tag == protocol::OUTPUT && !s.capture(100).contains("IMAGE PREVIEW"));
        // A pressed path is invalidated when focus changes to another exact target.
        b.send(protocol::KEYS, 0, format!("\x1b[<0;{};{}M", x + 1, y + 1));
        let other = f.new_workspace("Other target");
        b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("Other target"));
        b.send(protocol::KEYS, 0, format!("\x1b[<0;{};{}m", x + 1, y + 1));
        b.send(protocol::KEYS, 0, b"\0");
        b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("NAV"));
        assert!(!b.screen.capture(100).contains("IMAGE PREVIEW"));
        assert_eq!(f.snapshot().session().unwrap().run, other.run);
        let snapshot = f.snapshot();
        let original = snapshot
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .unwrap()
            .tabs
            .iter()
            .find(|s| s.id == t.id)
            .unwrap();
        assert_eq!((original.pid, &original.run), (t.pid, &t.run));
        assert!(input(&f).is_empty());
    }
}

#[test]
fn bridge_diagnostics_keep_disconnect_errors_without_terminal_payloads() {
    let f = Fixture::new();
    native(&f);
    let mut b = Bridge::new(&f, 100, 30);
    b.send(255, 0, b"SECRET_PAYLOAD_MUST_NOT_BE_LOGGED");
    let until = Instant::now() + Duration::from_secs(3);
    while b.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    let logs: String = fs::read_dir(f.state.join("diagnostics"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert!(logs.contains("connected") && logs.contains("exit-error"));
    assert!(!logs.contains("SECRET_PAYLOAD") && !logs.contains("IMAGE_NATIVE_DRAFT"));
    assert!(input(&f).is_empty());
}

#[test]
fn remote_avatar_layouts_follow_cards_overlays_resize_and_detach_without_native_input() {
    use flere::avatar::Layout;
    for (width, height) in [(209, 49), (60, 24)] {
        let f = Fixture::new();
        let (wid, t) = native(&f);
        fs::write(f.root.join("preview.png"), png()).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&f.root)
                .status()
                .unwrap()
                .success()
        );
        fs::create_dir(f.root.join(".flere")).unwrap();
        let mut icon = png();
        icon[16..20].copy_from_slice(&442u32.to_be_bytes());
        icon[20..24].copy_from_slice(&491u32.to_be_bytes());
        // Valid header plus trailing bytes exercises ordered multi-packet delivery.
        icon.resize(protocol::CHUNK * 2 + 100, 0);
        fs::write(f.root.join(".flere/icon.png"), &icon).unwrap();
        f.req(&[
            "metadata",
            &wid.to_string(),
            &flere::wire::hex(br#"{"issue":"https://github.com/example/demo-project/issues/417"}"#),
        ]);
        let mut b = Bridge::new(&f, width, height);
        b.wait_screen("IMAGE_NATIVE_DRAFT");
        b.send(protocol::CAPABILITIES, 0, protocol::AVATAR_CAP);
        b.send(protocol::CAPABILITIES, 0, protocol::PREVIEW_CAP);
        let packet = b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT && !Layout::decode(&p.data).unwrap().badges.is_empty()
        });
        let layout = Layout::decode(&packet.data).unwrap();
        assert_eq!((layout.width, layout.height), (width, height));
        assert!(layout.badges.iter().all(|b| b.key == "repo-1"));
        let mut received = Vec::new();
        while received.len() < icon.len() {
            let packet = b.until(|p, _| p.tag == protocol::AVATAR_IMAGE);
            let (key, total, offset, bytes) = flere::avatar::read_chunk(&packet.data).unwrap();
            assert_eq!(key, "repo-1");
            assert_eq!(total, icon.len());
            assert_eq!(offset, received.len());
            received.extend_from_slice(bytes);
        }
        assert_eq!(received, icon); // linked issue owner never selects a profile image.
        // Sidebar logos keep a fixed three-cell gutter across pixel-cell metrics.
        let capability = [protocol::AVATAR_SIZE_CAP, &protocol::size_bytes(10, 25)].concat();
        b.send(protocol::CAPABILITIES, 0, capability);
        let packet = b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && !Layout::decode_sized(&p.data).unwrap().badges.is_empty()
        });
        assert!(
            Layout::decode_sized(&packet.data)
                .unwrap()
                .badges
                .iter()
                .all(|b| b.columns == 3)
        );
        // Same-grid font changes send a fresh frame without shifting sidebar titles.
        b.send(
            protocol::CAPABILITIES,
            0,
            [protocol::AVATAR_SIZE_CAP, &protocol::size_bytes(16, 25)].concat(),
        );
        let packet = b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && !Layout::decode_sized(&p.data).unwrap().badges.is_empty()
        });
        assert!(
            Layout::decode_sized(&packet.data)
                .unwrap()
                .badges
                .iter()
                .all(|b| b.x == 4 && b.columns == 3)
        );
        // Board cards retain their existing height-driven two-cell logo at this
        // font size; the fixed sidebar slot must not change the board renderer.
        b.send(protocol::KEYS, 0, b"\0Bll");
        let left = flere::ui::Layout::new(width as usize, height as usize).left as u16;
        let packet = b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && Layout::decode_sized(&p.data)
                    .unwrap()
                    .badges
                    .iter()
                    .any(|b| b.x >= left)
        });
        let board = Layout::decode_sized(&packet.data).unwrap();
        assert!(
            board
                .badges
                .iter()
                .filter(|b| b.x >= left)
                .all(|b| b.columns == 2)
        );
        assert!(
            board
                .badges
                .iter()
                .filter(|b| b.x < left)
                .all(|b| b.x == 4 && b.columns == 3)
        );
        b.send(protocol::KEYS, 0, b"\x1b");
        b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && Layout::decode_sized(&p.data)
                    .unwrap()
                    .badges
                    .iter()
                    .all(|b| b.x < left)
        });
        // Menu hides badges; returning restores the same project without touching native input.
        // Narrow Windows fonts used to be sent through size_bytes(), clamping
        // a 7-pixel cell to 10 and reserving only two columns for these logos.
        b.send(
            protocol::CAPABILITIES,
            0,
            [
                protocol::AVATAR_SIZE_CAP,
                &flere::avatar::cell_bytes((7, 19)).unwrap(),
            ]
            .concat(),
        );
        let frame = b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && !Layout::decode_sized(&p.data).unwrap().badges.is_empty()
        });
        assert!(
            Layout::decode_sized(&frame.data)
                .unwrap()
                .badges
                .iter()
                .all(|b| b.columns == 3)
        );
        b.send(protocol::KEYS, 0, b"\0 ");
        b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && Layout::decode_sized(&p.data).unwrap().badges.is_empty()
        });
        b.send(protocol::KEYS, 0, b"\x1b");
        b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && !Layout::decode_sized(&p.data).unwrap().badges.is_empty()
        });
        // Escape closes the action menu but leaves NAV active; leave it explicitly.
        b.send(protocol::KEYS, 0, b"\0");
        // Same-grid resize requests a complete repaint after a client cell/font change.
        b.screen = Terminal::new(width as usize, height as usize);
        b.send(protocol::RESIZE, 0, protocol::size_bytes(width, height));
        b.until(|p, s| p.tag == protocol::AVATAR_LAYOUT_SIZED && s.capture(100).contains("FLERE"));
        select_preview(&mut b, &f);
        b.send(protocol::KEYS, 0, b"p");
        b.until(|p, _| {
            p.tag == protocol::AVATAR_LAYOUT_SIZED
                && Layout::decode_sized(&p.data).unwrap().badges.is_empty()
        });
        b.until(|p, _| p.tag == protocol::PREVIEW_END);
        b.send(protocol::KEYS, 0, b"q");
        b.until(|p, _| p.tag == protocol::PREVIEW_CLEAR);
        // Narrow Files covers the sidebar; wide Files retains it.
        let packet = b.until(|p, _| p.tag == protocol::AVATAR_LAYOUT_SIZED);
        assert_eq!(
            Layout::decode_sized(&packet.data)
                .unwrap()
                .badges
                .is_empty(),
            width < 100
        );
        assert!(input(&f).is_empty());
        b.disconnect();
        b.until(|p, _| p.tag == protocol::AVATAR_CLEAR);
        b.until(|p, _| p.tag == protocol::OUTPUT && p.data.windows(8).any(|w| w == b"\x1b[?1049l"));
        let now = f.snapshot();
        let current = now
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .find(|s| s.id == t.id)
            .unwrap();
        assert_eq!((current.pid, &current.run), (t.pid, &t.run));
        assert!(input(&f).is_empty());
    }
}

#[test]
fn remote_cat_pixels_are_negotiated_bounded_and_cleared_with_overlays() {
    let f = Fixture::new();
    let t = f.new_workspace("Pixel cat");
    f.send(&t, b"echo PIXEL_CAT_DRAFT");
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":33,"pet":true}"#,
    )
    .unwrap();
    let mut b = Bridge::new(&f, 188, 39);
    b.wait_screen(" PET / ");
    // Plain/older attachments have the full-body Unicode fallback, never the old face.
    assert!(!b.screen.capture(100).contains("(="));
    b.allow_cat_pixels = true;
    b.send(
        protocol::CAPABILITIES,
        0,
        [protocol::AVATAR_SIZE_CAP, &[0, 10, 0, 25]].concat(),
    );
    let has_pixels =
        |p: &Packet| p.tag == protocol::OUTPUT && p.data.windows(2).any(|s| s == b"\x1bP");
    let first = b.until(|p, _| has_pixels(p));
    assert!(
        first
            .data
            .windows(b"\x1b[33;158H".len())
            .any(|s| s == b"\x1b[33;158H")
    );
    assert!(
        first
            .data
            .windows(b"\x1bP7;1q\"1;1;290;100".len())
            .any(|s| s == b"\x1bP7;1q\"1;1;290;100")
    );
    let next = b.until(|p, _| has_pixels(p));
    assert_ne!(first.data, next.data);
    // An unchanged animation frame updates its private rectangle, not the entire screen.
    let steady = b.until(|p, _| has_pixels(p));
    assert!(!steady.data.windows(4).any(|s| s == b"\x1b[2J"));
    b.send(protocol::KEYS, 0, b"\x1b[<0;55;10M\x1b[<32;75;11M");
    let selection = b.until(|p, _| has_pixels(p));
    assert!(b.screen.capture(100).contains(" PET / "));
    assert!(!selection.data.windows(4).any(|s| s == b"\x1b[2J"));
    // Cancel the selection before release: this bridge helper deliberately
    // rejects clipboard OSC, including legitimate user-requested copies.
    b.send(protocol::KEYS, 0, b"\x1b");
    b.until(|p, _| has_pixels(p));
    b.send(protocol::KEYS, 0, b"\x1b[<0;75;11m");
    b.until(|p, _| has_pixels(p));
    b.send(protocol::KEYS, 0, b"\0 ");
    let hidden =
        b.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("FLERE ACTIONS"));
    assert!(!has_pixels(&hidden));
    assert!(hidden.data.windows(4).any(|s| s == b"\x1b[2J"));
    b.send(protocol::KEYS, 0, b"\x1b");
    b.until(|p, _| has_pixels(p));
    b.send(protocol::KEYS, 0, b"M");
    let still = b.until(|p, _| has_pixels(p));
    assert!(still.data.len() < protocol::MAX);
    b.send(
        protocol::CAPABILITIES,
        0,
        [protocol::AVATAR_SIZE_CAP, &[0, 16, 0, 25]].concat(),
    );
    b.until(|p, _| {
        p.data
            .windows(b"\x1bP7;1q\"1;1;464;100".len())
            .any(|s| s == b"\x1bP7;1q\"1;1;464;100")
    });
    b.send(protocol::KEYS, 0, b"o");
    let off = b.until(|p, s| p.tag == protocol::OUTPUT && !s.capture(100).contains(" PET / "));
    assert!(!has_pixels(&off));
    assert!(off.data.windows(4).any(|s| s == b"\x1b[2J"));
    b.disconnect();
    assert!(f.capture(&t).contains("echo PIXEL_CAT_DRAFT"));
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
}

#[test]
fn remote_pixel_pet_gestures_capture_hold_release_and_resize() {
    let f = Fixture::new();
    let t = f.new_workspace("Pixel physics");
    f.send(&t, b"echo PIXEL_PHYSICS_DRAFT");
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":33,"pet":true,"pet_kind":"robot","reduced_motion":true}"#,
    )
    .unwrap();
    let mut b = Bridge::new(&f, 188, 39);
    b.allow_cat_pixels = true;
    b.send(
        protocol::CAPABILITIES,
        0,
        [protocol::AVATAR_SIZE_CAP, &[0, 10, 0, 25]].concat(),
    );
    let pixels = |p: &Packet| p.tag == protocol::OUTPUT && p.data.windows(2).any(|s| s == b"\x1bP");
    b.until(|p, _| pixels(p));
    b.send(protocol::KEYS, 0, b"\x1b[<0;158;31M\x1b[<0;158;31m");
    b.until(|p, _| pixels(p));
    assert!(!b.screen.grid.line(38).contains("Esc back"));
    b.send(protocol::KEYS, 0, b"\0O");
    b.wait_screen("PET Ready");
    b.send(protocol::KEYS, 0, b"\x1b[<0;173;34M\x1b[<0;173;34m");
    b.wait_screen("PET Happy");
    // Selecting robot again leaves its position fixed; create a ball while quiet.
    b.send(protocol::KEYS, 0, b"t");
    b.wait_screen("PET Chasing toy");
    b.send(protocol::KEYS, 0, b"\x1b[<0;165;33M");
    b.wait_screen("PET Ball held");
    b.send(protocol::KEYS, 0, b"\x1b[<32;177;32M");
    b.until(|p, _| pixels(p));
    b.send(protocol::KEYS, 0, b"\x1b[<0;177;32m");
    b.wait_screen("PET Chasing toy");
    // Sprite hold is time-based even with reduced motion enabled.
    b.send(protocol::KEYS, 0, b"n");
    b.wait_screen("PET Napping");
    b.send(protocol::KEYS, 0, b"\x1b[<0;173;35M");
    b.wait_screen("PET Dangling");
    b.send(protocol::KEYS, 0, b"\x1b[<32;165;31M");
    let held = b.until(|p, _| pixels(p));
    assert!(held.data.len() < protocol::MAX);
    // Changing cell dimensions invalidates the old pointer geometry safely.
    b.send(
        protocol::CAPABILITIES,
        0,
        [protocol::AVATAR_SIZE_CAP, &[0, 16, 0, 25]].concat(),
    );
    b.until(|p, s| pixels(p) && !s.grid.line(38).contains("Dangling"));
    b.send(
        protocol::KEYS,
        0,
        b"\x1b[<0;20;3m\x1b[200~NO_PIXEL_NATIVE_INPUT\x1b[201~",
    );
    b.until(|p, _| pixels(p));
    b.disconnect();
    assert!(f.capture(&t).contains("PIXEL_PHYSICS_DRAFT"));
    assert!(!f.capture(&t).contains("NO_PIXEL_NATIVE_INPUT"));
    assert_eq!(
        (
            f.snapshot().session().unwrap().pid,
            &f.snapshot().session().unwrap().run
        ),
        (t.pid, &t.run)
    );
}

#[test]
fn remote_card_links_copy_only_after_explicit_activation_without_native_input() {
    let f = Fixture::new();
    let t = f.new_workspace("Remote details");
    let wid = f.snapshot().active;
    let mut meta = f.snapshot().workspace().unwrap().meta.clone();
    meta.pr = "https://github.com/demo/fixture/pull/193".into();
    f.req(&[
        "metadata",
        &wid.to_string(),
        &wire::hex(&serde_json::to_vec(&meta).unwrap()),
    ]);
    f.send(&t, b"echo REMOTE_DETAILS_DRAFT");
    f.wait_text(&t, "REMOTE_DETAILS_DRAFT");
    let empty_bin = f.root.join("empty-bin");
    fs::create_dir(&empty_bin).unwrap();
    let mut b = Bridge::with_path(&f, 160, 35, empty_bin.to_string_lossy().into_owned());
    b.send(protocol::KEYS, 0, b"\0h?");
    b.wait_screen("Install gh");
    assert!(b.screen.capture(100).contains("Copy PR"));
    assert!(!b.screen.capture(100).contains("Open PR"));
    b.allow_ui_clipboard = true;
    b.send(protocol::KEYS, 0, b"p");
    let packet =
        b.until(|p, _| p.tag == protocol::OUTPUT && p.data.windows(7).any(|s| s == b"\x1b]52;c;"));
    let expected = b"aHR0cHM6Ly9naXRodWIuY29tL2RlbW8vZml4dHVyZS9wdWxsLzE5Mw==";
    assert!(packet.data.windows(expected.len()).any(|s| s == expected));
    b.send(protocol::KEYS, 0, b"\x1b");
    b.until(|p, s| p.tag == protocol::OUTPUT && !s.capture(100).contains("Details · Remote"));
    b.send(protocol::KEYS, 0, b"\r_STILL_NATIVE");
    b.wait_screen("REMOTE_DETAILS_DRAFT_STILL_NATIVE");
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
}

#[test]
fn screenshot_exports_the_full_flere_frame_and_waits_for_local_receipt() {
    use flere::screenshot::{CELL_H, CELL_W};
    for (width, height) in [(100, 32), (236, 54)] {
        let f = Fixture::new();
        let (_, tab) = native(&f);
        let mut bridge = Bridge::new(&f, width, height);
        bridge.wait_screen("IMAGE_NATIVE_DRAFT");
        bridge.send(protocol::CAPABILITIES, 0, protocol::SCREENSHOT_CAP);
        bridge.send(protocol::KEYS, 0, b"\0s");
        let mut receiver = flere::screenshot_transfer::Receiver::default();
        let mut id = None;
        let png = loop {
            let packet = bridge.until(|packet, _| {
                matches!(
                    packet.tag,
                    protocol::SCREENSHOT_BEGIN
                        | protocol::SCREENSHOT_DATA
                        | protocol::SCREENSHOT_END
                )
            });
            assert!(
                !bridge
                    .screen
                    .capture(100)
                    .contains("Flere screenshot copied")
            );
            if packet.tag == protocol::SCREENSHOT_BEGIN {
                id = Some(packet.id);
            }
            if let Some(png) = receiver.packet(&packet).unwrap() {
                break png;
            }
        };
        let (w, h, pixels) = flere::screenshot::decode(&png).unwrap();
        assert_eq!((w, h), (width as usize * CELL_W, height as usize * CELL_H));
        // All three columns and the bottom status row belong to the screenshot.
        // A centre-buffer crop cannot pass these dimensions and edge colours.
        let pixel = |x, y| &pixels[(y * w + x) * 4..(y * w + x) * 4 + 3];
        let centre = pixel(w / 2, h / 2);
        assert_ne!(pixel(0, h / 2), centre);
        assert_ne!(pixel(w - 1, h / 2), centre);
        assert_ne!(pixel(w / 2, h - 1), centre);
        assert_eq!(input(&f), b"");
        assert_eq!(f.snapshot().session().unwrap().run, tab.run);
        bridge.send(protocol::SCREENSHOT_RESULT, id.unwrap(), [1]);
        bridge.wait_screen("Flere screenshot copied");
        // Screenshot keys leave NAV without submitting input or losing the draft.
        bridge.send(protocol::KEYS, 0, b"after screenshot");
        assert_eq!(wait_input(&f, 16), b"after screenshot");
        if width == 236
            && let Some(path) = std::env::var_os("FLERE_SCREENSHOT_ARTIFACT")
        {
            fs::write(path, &png).unwrap();
        }
    }
}
#[path = "../arcade/remote.rs"]
mod arcade;
