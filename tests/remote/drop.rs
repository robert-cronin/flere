//! Local path consent and exact remote chat completion, with no real clipboard or model.
use super::*;
use flere::remote_services as service;
use serde_json::Value;

fn logs(f: &Fixture) -> String {
    fs::read_dir(f.root.join(".local/state/flere-connect/diagnostics"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| fs::read_to_string(e.path()).ok())
        .collect()
}
fn companion(f: &Fixture, width: u16, height: u16) -> (fs::File, os::Process, Terminal) {
    let ssh = f.root.join("fixture-ssh");
    fs::write(&ssh, "#!/usr/bin/python3\nimport os,sys,shlex\na=shlex.split(sys.argv[4]);assert a.pop(0)=='exec';os.execv(a[0],a)\n").unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new("/usr/bin/env");
    command.args(["-u", "FLERE"]).arg(companion_binary());
    fixture_home(&mut command, &f.root)
        .arg("fixture-host")
        .args(["--remote", env!("CARGO_BIN_EXE_flere")])
        .arg("--state")
        .arg(&f.state)
        .arg("--ssh")
        .arg(ssh)
        .env_remove("FLERE");
    let (mut master, child) = os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
    let mut screen = Terminal::new(width as usize, height as usize);
    drain_pty(&mut master, &mut screen, "IMAGE_NATIVE_DRAFT");
    wait_current_ui(&mut master, &mut screen, |_| {
        logs(f).contains("chat-drop-context") && logs(f).contains("capture")
    });
    (master, child, screen)
}
fn candidate(master: &mut fs::File, screen: &mut Terminal, text: &str) {
    // A repeated path has the same prompt title. Wait for the prior modal to
    // clear so its stale paint cannot authorize input before the new prompt.
    wait_current_ui(master, screen, |s| {
        !s.capture(100).contains("Attach local file or paste text?")
    });
    // A coalesced Enter is not fresh consent and must never submit the chat.
    master
        .write_all(&[b"\x1b[200~", text.as_bytes(), b"\x1b[201~\r"].concat())
        .unwrap();
    wait_current_ui(master, screen, |s| {
        let text = s.capture(100);
        text.contains("Attach local file or paste text?")
            && text.contains("a Attach · t Text · Esc Cancel · arrows/Tab, Enter")
    });
}
fn offer(bridge: &mut Bridge, id: u64) -> Packet {
    bridge.send(service::DROP_OFFER, id, Vec::new());
    bridge.until(|p, _| p.tag == service::REQUEST)
}
fn negotiated(bridge: &mut Bridge) {
    bridge.send(protocol::NOTICE, 0, service::DROP_PROBE);
    bridge.until(|p, _| p.tag == protocol::CAPABILITIES && p.data == service::DROP_CAPABILITY);
}

#[test]
fn local_drop_choices_preserve_bytes_and_attach_private_files_without_enter() {
    for (width, height) in [(60, 24), (180, 42)] {
        let f = Fixture::new();
        let (_, tab) = native(&f);
        let path = f.root.join("report ü with spaces.pdf");
        let data = (0..service::CHUNK * 2 + 19)
            .map(|i| (i % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&path, &data).unwrap();
        let (mut master, mut child, mut screen) = companion(&f, width, height);
        let quoted = format!("'{}'", path.display());
        candidate(&mut master, &mut screen, &quoted);
        assert!(input(&f).is_empty());
        master.write_all(b"\r").unwrap(); // Default Cancel.
        drain_pty(&mut master, &mut screen, "cancelled");
        assert!(input(&f).is_empty());
        assert!(!f.root.join(".cache/flere/chat-attachments").exists());

        candidate(&mut master, &mut screen, &quoted);
        master.write_all(b"t\r").unwrap();
        let expected = [b"\x1b[200~", quoted.as_bytes(), b"\x1b[201~"].concat();
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).len() == expected.len()
        });
        assert_eq!(input(&f), expected);
        assert!(!f.root.join(".cache/flere/chat-attachments").exists());

        candidate(&mut master, &mut screen, &quoted);
        master.write_all(b"a\r").unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).len() > expected.len() && input(&f).ends_with(b"\x1b[201~")
        });
        let received = input(&f);
        let tail = &received[expected.len()..];
        assert!(tail.starts_with(b"\x1b[200~") && !tail.contains(&b'\r') && !tail.contains(&b'\n'));
        let remote = PathBuf::from(std::str::from_utf8(&tail[6..tail.len() - 6]).unwrap());
        assert!(remote.starts_with(f.root.join(".cache/flere/chat-attachments")));
        assert_eq!(remote.file_name(), path.file_name());
        assert_eq!(fs::read(&remote).unwrap(), data);
        assert_eq!(
            fs::metadata(&remote).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drain_pty(&mut master, &mut screen, "File path paste queued");
        let multiline = b"\x1b[200~/ordinary\n/path text\x1b[201~";
        master.write_all(multiline).unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).len() == received.len() + multiline.len()
        });
        assert_eq!(input(&f), [received, multiline.to_vec()].concat());
        let received = input(&f);
        let literal = b"/unbracketed/path stays literal";
        master.write_all(literal).unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            input(&f).len() == received.len() + literal.len()
        });
        assert_eq!(input(&f), [received, literal.to_vec()].concat());
        assert_eq!(f.snapshot().session().unwrap().run, tab.run);
        assert_eq!(f.snapshot().session().unwrap().pid, tab.pid);
        finish_ui(&mut master, &mut screen, &mut child);
        assert!(
            remote.exists(),
            "detaching must retain a path already in the draft"
        );
    }
}

#[test]
fn bridge_drop_negotiation_and_prompt_tokens_cannot_retarget() {
    let f = Fixture::new();
    let (_, tab) = native(&f);
    let mut bridge = Bridge::new(&f, 100, 30);
    // A legacy companion receives no unsolicited new capabilities.
    bridge.send(protocol::NOTICE, 0, b"legacy-marker".as_slice());
    bridge.until(|packet, screen| {
        assert_ne!(packet.tag, protocol::CAPABILITIES);
        screen.capture(100).contains("legacy-marker")
    });
    bridge.send(service::DROP_OFFER, 1, Vec::new());
    bridge.until(|p, _| p.tag == service::DROP_RESULT && p.id == 1);
    assert!(input(&f).is_empty());
    negotiated(&mut bridge);
    bridge.send(
        protocol::CAPABILITIES,
        0,
        b"future-optional-capability".as_slice(),
    );
    let request = offer(&mut bridge, 2);
    let value: Value = serde_json::from_slice(&request.data).unwrap();
    assert_eq!(value["op"], "attach");
    assert!(value.get("path").is_none());
    // Change away and back without waiting for the bridge to paint either state.
    let initial = f.snapshot();
    f.req(&["tab", &initial.active.to_string()]);
    f.req(&[
        "focus-exact",
        &initial.epoch,
        &initial.active.to_string(),
        &tab.id.to_string(),
        &tab.run,
    ]);
    bridge.send(
        service::DROP_TEXT,
        request.id,
        b"\x1b[200~/must-not-arrive\x1b[201~".as_slice(),
    );
    bridge.until(|p, _| p.id == request.id && matches!(p.tag, service::CANCEL | service::RESULT));
    assert!(input(&f).is_empty());
    // Drain the post-command watch frame; the prior cancelled prompt can still
    // be the bridge's last snapshot even though the supervisor already returned.
    bridge.until(|p, s| p.tag == protocol::OUTPUT && s.capture(100).contains("IMAGE_NATIVE_DRAFT"));
    let request = offer(&mut bridge, 3);
    bridge.send(
        service::METADATA,
        request.id,
        service::metadata("image.png", png().len() as u64).unwrap(),
    );
    bridge.until(|p, _| p.id == request.id && p.tag == service::READY);
    bridge.send(
        service::DATA,
        request.id,
        service::chunk(0, &png()).unwrap(),
    );
    bridge.until(|p, _| p.id == request.id && p.tag == service::READY);
    bridge.send(service::END, request.id, Vec::new());
    let result = bridge.until(|p, _| p.id == request.id && p.tag == service::RESULT);
    assert_eq!(service::decode(&result.data).unwrap()["success"], true);
    let received = wait_input(&f, 12);
    let path = std::str::from_utf8(&received[6..received.len() - 6]).unwrap();
    assert_eq!(fs::read(path).unwrap(), png());
    bridge.send(service::END, request.id, Vec::new());
    bridge.send(protocol::NOTICE, 0, b"duplicate-end-drained".as_slice());
    bridge.wait_screen("duplicate-end-drained");
    assert_eq!(input(&f), received);
    let snapshot = f.snapshot();
    let shell = snapshot
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.kind == "shell")
        .unwrap()
        .clone();
    f.req(&[
        "focus-exact",
        &snapshot.epoch,
        &snapshot.active.to_string(),
        &shell.id.to_string(),
        &shell.run,
    ]);
    bridge.until(|p, _| p.tag == service::DROP_CONTEXT && p.data == [0]);
    // Simulate an offer already in flight when the native-chat hint became stale.
    let request = offer(&mut bridge, 4);
    assert_eq!(service::decode(&request.data).unwrap()["op"], "literal");
    bridge.send(
        service::DROP_TEXT,
        request.id,
        b"\x1b[200~/literal-raced-shell-path\x1b[201~".as_slice(),
    );
    let result = bridge.until(|p, _| p.tag == service::RESULT && p.id == request.id);
    assert_eq!(service::decode(&result.data).unwrap()["success"], true);
    f.wait_text(&shell, "/literal-raced-shell-path");
    assert_eq!(input(&f), received);
}

#[test]
fn new_companion_against_legacy_v6_core_keeps_path_paste_literal() {
    let f = Fixture::new();
    let ssh = f.root.join("legacy-ssh");
    fs::write(
        &ssh,
        r#"#!/usr/bin/python3
import sys,struct,pathlib
r=sys.stdin.buffer;w=sys.stdout.buffer
key=pathlib.Path('legacy-input');key.write_bytes(b'')
def send(tag,data=b''):
 b=bytes([tag])+bytes(8)+data;w.write(struct.pack('>I',len(b))+b);w.flush()
send(1,b'flere-remote-v6'+b'a'*32)
while True:
 h=r.read(4)
 if not h:break
 n=struct.unpack('>I',h)[0];b=r.read(n);tag=b[0];data=b[9:]
 assert tag in (1,2,3,8,9),('new operation without negotiation',tag)
 if tag==2:
  with key.open('ab') as f:f.write(data)
  send(16,b'\x1b[2J\x1b[HLEGACY INPUT RECORDED')
 elif tag==8:send(16,b'\x1b[2J\x1b[HLEGACY CORE READY')
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(companion_binary());
    fixture_home(&mut command, &f.root)
        .arg("fixture-host")
        .arg("--ssh")
        .arg(ssh)
        .env_remove("FLERE");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 80, 24).unwrap();
    let mut screen = Terminal::new(80, 24);
    drain_pty(&mut master, &mut screen, "LEGACY CORE READY");
    let text = b"\x1b[200~'/local/report.pdf'\x1b[201~";
    master.write_all(text).unwrap();
    drain_pty(&mut master, &mut screen, "LEGACY INPUT RECORDED");
    assert_eq!(fs::read(f.root.join("legacy-input")).unwrap(), text);
    assert!(!logs(&f).contains("capability confirmed"));
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn context_hints_keep_shell_editor_and_remote_forms_on_their_normal_text_path() {
    let f = Fixture::new();
    let (wid, native_tab) = native(&f);
    let initial = f.snapshot();
    let shell = initial
        .workspace()
        .unwrap()
        .tabs
        .iter()
        .find(|t| t.kind == "shell")
        .unwrap()
        .clone();
    let (mut master, mut child, mut screen) = companion(&f, 120, 36);
    f.req(&[
        "focus-exact",
        &initial.epoch,
        &wid.to_string(),
        &shell.id.to_string(),
        &shell.run,
    ]);
    wait_current_ui(&mut master, &mut screen, |_| logs(&f).contains("literal"));
    master
        .write_all(b"\x1b[200~/ordinary-shell-path\x1b[201~")
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.capture(&shell).contains("/ordinary-shell-path")
    });
    assert!(!screen.capture(100).contains("Attach local file"));
    assert!(input(&f).is_empty());
    let path = f.root.join("editor.txt");
    fs::write(&path, b"EDITOR_START\n").unwrap();
    f.req(&[
        "open",
        &wid.to_string(),
        &wire::hex(path.to_str().unwrap().as_bytes()),
    ]);
    let editor = f.snapshot().session().unwrap().clone();
    assert_eq!(editor.kind, "editor");
    drain_pty(&mut master, &mut screen, "EDITOR_START");
    master
        .write_all(b"\x1b[200~/ordinary-editor-path\x1b[201~")
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.capture(&editor).contains("/ordinary-editor-path")
    });
    assert!(!screen.capture(100).contains("Attach local file"));
    master.write_all(b"\0n").unwrap();
    drain_pty(&mut master, &mut screen, "New worktree");
    master
        .write_all(b"\x1b[200~/ordinary-form-path\x1b[201~")
        .unwrap();
    drain_pty(&mut master, &mut screen, "/ordinary-form-path");
    assert!(!screen.capture(100).contains("LOCAL"));
    assert!(input(&f).is_empty());
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        !s.capture(100).contains("New worktree")
    });
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().session().is_some_and(|t| t.id == editor.id)
    });
    finish_ui(&mut master, &mut screen, &mut child);
    assert!(input(&f).is_empty());
    assert!(
        f.snapshot()
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .any(|t| t.id == native_tab.id && t.run == native_tab.run && t.pid == native_tab.pid)
    );
}
