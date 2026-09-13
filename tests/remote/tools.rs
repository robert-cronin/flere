//! Real supervisor and companion transfers; every path and SSH process is disposable.
use super::*;
use crate::splits::{input as probe_input, pane, panes, probes, unchanged};
use flere::remote_services as service;
use serde_json::{Value, json};
use std::path::Path;

fn begin(
    f: &Fixture,
    snapshot: &Snapshot,
    upload: bool,
    path: &Path,
    size: usize,
) -> std::io::Result<Value> {
    let tab = snapshot.session().unwrap();
    let fields = [
        "file-begin".into(),
        if upload { "upload" } else { "download" }.into(),
        snapshot.epoch.clone(),
        snapshot.active.to_string(),
        tab.id.to_string(),
        tab.run.clone(),
        snapshot
            .split
            .as_ref()
            .map_or(0, |s| s.revision)
            .to_string(),
        wire::hex(path.to_str().unwrap().as_bytes()),
        size.to_string(),
    ];
    let bytes = wire::request(
        &f.state,
        &fields[..if upload { 9 } else { 8 }]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    )?;
    let initial: Value = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
    let data = file_wait(f, token(&initial))?;
    let mut metadata: Value = serde_json::from_slice(&data).map_err(std::io::Error::other)?;
    metadata["token"] = initial["token"].clone();
    Ok(metadata)
}
fn file_wait(f: &Fixture, token: &str) -> std::io::Result<Vec<u8>> {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let value: Value = serde_json::from_slice(&wire::request(&f.state, &["file-poll", token])?)
            .map_err(std::io::Error::other)?;
        if value["pending"] == false {
            return wire::unhex(value["data"].as_str().unwrap());
        }
        assert!(Instant::now() < until, "file worker timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn file_call(f: &Fixture, fields: &[&str]) -> std::io::Result<Vec<u8>> {
    wire::request(&f.state, fields)?;
    file_wait(f, fields[1])
}
fn no_parts(directory: &Path) {
    let until = Instant::now() + Duration::from_secs(3);
    while !partials(directory).is_empty() {
        assert!(
            Instant::now() < until,
            "private staging cleanup did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn token(value: &Value) -> &str {
    value["token"].as_str().unwrap()
}
fn partials(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".flere-transfer-")
        })
        .collect()
}
fn write(f: &Fixture, value: &Value, offset: usize, bytes: &[u8]) {
    file_call(
        f,
        &[
            "file-write",
            token(value),
            &offset.to_string(),
            &wire::hex(bytes),
        ],
    )
    .unwrap();
}

#[test]
fn file_streams_publish_once_without_overwrite_and_abort_private_parts() {
    let f = Fixture::new();
    let tabs = probes(&f, 1);
    f.send(&tabs[0], b"existing draft");
    let snapshot = panes(&f);
    let destination = f.root.join("uploaded.bin");
    let bytes = (0..service::CHUNK * 2 + 19)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<_>>();
    let upload = begin(&f, &snapshot, true, &destination, bytes.len()).unwrap();
    assert!(!destination.exists());
    let part = partials(&f.root).pop().unwrap();
    assert_eq!(
        fs::metadata(part).unwrap().permissions().mode() & 0o777,
        0o600
    );
    for (index, chunk) in bytes.chunks(service::CHUNK).enumerate() {
        write(&f, &upload, index * service::CHUNK, chunk);
        assert!(
            !destination.exists(),
            "partial upload must not appear as the requested file"
        );
    }
    file_call(&f, &["file-finish", token(&upload)]).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    no_parts(&f.root);
    assert!(file_call(&f, &["file-finish", token(&upload)]).is_err());
    assert!(begin(&f, &snapshot, true, &destination, 1).is_err());
    let download = begin(&f, &snapshot, false, &destination, 0).unwrap();
    let mut received = Vec::new();
    while received.len() < bytes.len() {
        received.extend(
            file_call(
                &f,
                &["file-read", token(&download), &received.len().to_string()],
            )
            .unwrap(),
        );
    }
    file_call(&f, &["file-finish", token(&download)]).unwrap();
    assert_eq!(received, bytes);

    let colliding = f.root.join("collision.txt");
    let upload = begin(&f, &snapshot, true, &colliding, 3).unwrap();
    write(&f, &upload, 0, b"new");
    fs::write(&colliding, b"owner-created meanwhile").unwrap();
    assert!(file_call(&f, &["file-finish", token(&upload)]).is_err());
    assert_eq!(fs::read(colliding).unwrap(), b"owner-created meanwhile");
    no_parts(&f.root);

    let abandoned = f.root.join("abandoned.txt");
    let upload = begin(&f, &snapshot, true, &abandoned, 5).unwrap();
    write(&f, &upload, 0, b"ab");
    assert!(file_call(&f, &["file-write", token(&upload), "0", &wire::hex(b"abc")]).is_err());
    no_parts(&f.root);
    assert!(!abandoned.exists());
    let upload = begin(&f, &snapshot, true, &abandoned, 5).unwrap();
    write(&f, &upload, 0, b"ab");
    f.req(&["file-cancel", token(&upload)]);
    no_parts(&f.root);
    assert!(!abandoned.exists());
    let empty = f.root.join("empty.txt");
    let upload = begin(&f, &snapshot, true, &empty, 0).unwrap();
    file_call(&f, &["file-finish", token(&upload)]).unwrap();
    assert_eq!(fs::read(empty).unwrap(), b"");
    assert_eq!(probe_input(&f, &tabs[0]), b"existing draft");
    unchanged(&f, &tabs);
}

#[test]
fn file_tokens_reject_changed_pane_origin_source_and_symlink_targets() {
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    pane(&f, "split-right", Some("move"));
    let snapshot = panes(&f);
    let destination = f.root.join("never-published.txt");
    let upload = begin(&f, &snapshot, true, &destination, 4).unwrap();
    write(&f, &upload, 0, b"ab");
    pane(&f, "focus", Some("0"));
    pane(&f, "focus", Some("1"));
    assert_eq!(panes(&f).tab, snapshot.tab);
    assert!(
        begin(&f, &snapshot, true, &destination, 4).is_err(),
        "returning to a tab cannot revive its old pane revision"
    );
    assert!(file_call(&f, &["file-write", token(&upload), "2", &wire::hex(b"cd")]).is_err());
    assert!(!destination.exists());
    no_parts(&f.root);
    let source = f.root.join("source.txt");
    fs::write(&source, vec![b'a'; service::CHUNK + 10]).unwrap();
    let download = begin(&f, &panes(&f), false, &source, 0).unwrap();
    assert_eq!(
        file_call(&f, &["file-read", token(&download), "0"])
            .unwrap()
            .len(),
        service::CHUNK
    );
    fs::write(&source, b"replacement").unwrap();
    assert!(
        file_call(
            &f,
            &["file-read", token(&download), &service::CHUNK.to_string()]
        )
        .is_err()
    );
    let link = f.root.join("source-link.txt");
    std::os::unix::fs::symlink(&source, &link).unwrap();
    assert!(begin(&f, &panes(&f), false, &link, 0).is_err());
    assert!(begin(&f, &panes(&f), true, &link, 1).is_err());
    let dir_link = f.root.join("directory-link");
    std::os::unix::fs::symlink(&f.root, &dir_link).unwrap();
    assert!(begin(&f, &panes(&f), true, &dir_link.join("no.txt"), 1).is_err());
    for tab in &tabs {
        assert!(probe_input(&f, tab).is_empty());
    }
    unchanged(&f, &tabs);
}

fn bridge_key(bridge: &mut Bridge, bytes: &[u8]) {
    bridge.send(protocol::KEYS, 0, bytes);
}
fn click_bytes(x: usize, y: usize) -> Vec<u8> {
    format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).into_bytes()
}
fn file_row(screen: &Terminal, label: &str) -> (usize, usize) {
    find_file_row(screen, label)
        .unwrap_or_else(|| panic!("file {label} missing: {}", screen.capture(100)))
}
fn find_file_row(screen: &Terminal, label: &str) -> Option<(usize, usize)> {
    // Restrict to visible Files entry rows. A completion notice can contain the
    // same path while the asynchronous directory refresh still shows only '..'.
    for y in 5..screen.grid.rows.saturating_sub(5) {
        let row = screen.grid.cells[y * screen.grid.cols..(y + 1) * screen.grid.cols]
            .iter()
            .map(|c| c.text.chars().next().unwrap_or(' '))
            .collect::<String>();
        if let Some(x) = row.find(label) {
            return Some((row[..x].chars().count(), y));
        }
    }
    None
}

#[test]
fn remote_file_packets_are_owned_and_never_become_native_input() {
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    let folder = f.root.join("aaa-upload-folder");
    fs::create_dir(&folder).unwrap();
    let mut bridge = Bridge::new(&f, 60, 24);
    bridge.send(protocol::CAPABILITIES, 0, service::CAPABILITY);
    bridge_key(&mut bridge, b"\0l");
    bridge.wait_screen("aaa-upload-folder");
    let (x, y) = file_row(&bridge.screen, "aaa-upload-folder");
    bridge_key(&mut bridge, &click_bytes(x, y));
    bridge_key(&mut bridge, b"\0 6");
    let request = bridge.until(|p, _| p.tag == service::REQUEST);
    assert_eq!(
        service::decode(&request.data).unwrap()["destination"],
        folder.to_str().unwrap()
    );
    assert!(
        service::decode(&request.data)
            .unwrap()
            .get("path")
            .is_none()
    );
    bridge.send(
        service::METADATA,
        request.id,
        service::metadata("incoming.txt", 4).unwrap(),
    );
    bridge.until(|p, _| p.tag == service::READY && p.id == request.id);
    bridge.send(
        service::DATA,
        request.id + 10,
        service::chunk(0, b"XX").unwrap(),
    );
    bridge.send(service::DATA, request.id, service::chunk(0, b"ab").unwrap());
    // Ownership changes while bytes are in flight; returning does not authorize completion.
    let current = panes(&f);
    let other = tabs.iter().find(|t| t.id != current.tab).unwrap();
    f.req(&[
        "focus-exact",
        &current.epoch,
        &current.active.to_string(),
        &other.id.to_string(),
        &other.run,
    ]);
    bridge.until(|p, _| p.tag == service::CANCEL && p.id == request.id);
    bridge.send(service::DATA, request.id, service::chunk(2, b"cd").unwrap());
    bridge.send(service::END, request.id, Vec::new());
    bridge_key(&mut bridge, b"\0 9");
    let ports = bridge.until(|p, _| p.tag == service::PORTS_REQUEST);
    assert_eq!(service::decode(&ports.data).unwrap()["op"], "list");
    bridge.send(
        service::PORTS_RESULT,
        ports.id,
        serde_json::to_vec(&json!({"success":true,"message":"No forwards","ports":[]})).unwrap(),
    );
    bridge.wait_screen("No forwards");
    assert!(!folder.join("incoming.txt").exists());
    no_parts(&folder);
    for tab in &tabs {
        assert!(probe_input(&f, tab).is_empty());
    }
    unchanged(&f, &tabs);
}

struct CompanionUi {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
}
impl CompanionUi {
    fn attach(f: &Fixture, args: &[&str]) -> Self {
        let mut command = Command::new("/usr/bin/env");
        command
            .args(["-u", "FLERE"])
            .arg(companion_binary())
            .args(args)
            .env("HOME", &f.root)
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CACHE_HOME");
        let (master, child) = os::spawn_command_pty(&f.root, &mut command, 180, 42).unwrap();
        let mut ui = Self {
            master,
            child,
            screen: Terminal::new(180, 42),
        };
        drain_pty(&mut ui.master, &mut ui.screen, "PANE_READY_");
        ui
    }
    fn key(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 80);
    }
    fn wait(&mut self, text: &str) {
        drain_pty(&mut self.master, &mut self.screen, text);
    }
    fn file(&mut self, name: &str) {
        wait_current_ui(&mut self.master, &mut self.screen, |s| {
            find_file_row(s, name).is_some()
        });
        let (x, y) = file_row(&self.screen, name);
        self.key(&click_bytes(x, y));
        assert!(
            self.screen.grid.cells[y * self.screen.grid.cols + x]
                .style
                .bg
                == flere::terminal::Color::Rgb(14, 42, 55),
            "click {x},{y} must select exact file {name}: {}",
            self.screen.capture(100)
        );
    }
    fn finish(mut self) {
        finish_ui(&mut self.master, &mut self.screen, &mut self.child);
    }
    fn artifact(&self, name: &str) {
        let Some(root) = std::env::var_os("FLERE_TEST_REMOTE_TOOLS_ARTIFACTS") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let frame = flere::screenshot::Frame {
            width: self.screen.grid.cols,
            height: self.screen.grid.rows,
            cells: self.screen.grid.cells.clone(),
            layers: Vec::new(),
            cursor: None,
        };
        fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
        fs::write(root.join(format!("{name}.txt")), self.screen.capture(100)).unwrap();
    }
}
impl Drop for CompanionUi {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            os::hangup(self.master.as_raw_fd(), &mut self.child);
        }
    }
}

#[test]
fn companion_saved_connection_transfers_files_and_reconnects_without_replaying_drafts() {
    let f = Fixture::new();
    let tabs = probes(&f, 1);
    let folder = f.root.join("aaa-upload-folder");
    fs::create_dir(&folder).unwrap();
    let local = f.root.join("local");
    fs::create_dir(&local).unwrap();
    let payload = (0..service::CHUNK + 31)
        .map(|i| (i % 239) as u8)
        .collect::<Vec<_>>();
    let upload = local.join("new-data.bin");
    fs::write(&upload, &payload).unwrap();
    let ssh = f.root.join("fixture-ssh");
    fs::write(
        &ssh,
        r#"#!/usr/bin/python3
import os,sys,shlex,pathlib,json,socket
if sys.argv[1]=='-N':
 args=sys.argv[1:];pathlib.Path('forward-argv').write_text(json.dumps(args))
 address=args[args.index('-L')+1].split(':');assert address[0]==address[2]=='127.0.0.1'
 s=socket.socket();s.bind((address[0],int(address[1])));s.listen(8)
 while True:
  c,_=s.accept();c.close()
assert sys.argv[1:4]==['-T','--','fixture-host']
args=shlex.split(sys.argv[4]);assert args.pop(0)=='exec'
with pathlib.Path('ssh-invocations').open('a') as f:f.write(json.dumps(args)+'\n')
os.execv(args[0],args)
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    let output = Command::new(companion_binary())
        .args([
            "connections",
            "save",
            "work",
            "fixture-host",
            "--remote",
            env!("CARGO_BIN_EXE_flere"),
            "--state",
            f.state.to_str().unwrap(),
            "--ssh",
            ssh.to_str().unwrap(),
        ])
        .env("HOME", &f.root)
        .env_remove("XDG_CONFIG_HOME")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !f.root.join("ssh-invocations").exists(),
        "saving cannot initiate SSH"
    );
    let mut ui = CompanionUi::attach(&f, &["--connection", "work"]);
    ui.key(b"kept draft");
    ui.key(b"\0l");
    ui.file("aaa-upload-folder");
    ui.key(b"\0 6");
    ui.wait("Upload local file");
    ui.artifact("upload-local-prompt");
    ui.key(upload.to_str().unwrap().as_bytes());
    ui.key(b"\r");
    ui.wait("Uploaded");
    assert_eq!(fs::read(folder.join("new-data.bin")).unwrap(), payload);
    // Completion refreshes Explorer; reselect the explicit destination before retrying.
    ui.file("aaa-upload-folder");
    ui.key(b"\0 6");
    ui.wait("Upload local file");
    ui.key(upload.to_str().unwrap().as_bytes());
    ui.key(b"\r");
    ui.wait("destination exists");
    assert_eq!(fs::read(folder.join("new-data.bin")).unwrap(), payload);
    // Enter the chosen directory, select the completed upload and download through the same bridge.
    ui.file("aaa-upload-folder");
    ui.key(b"\r");
    ui.wait("new-data.bin");
    ui.file("new-data.bin");
    ui.key(b"\0 7");
    ui.wait("LOCAL · flere-connect");
    ui.wait("Download file");
    assert!(!f.root.join("Downloads/new-data.bin").exists());
    ui.key(b"\r");
    ui.wait("Downloaded");
    assert_eq!(
        fs::read(f.root.join("Downloads/new-data.bin")).unwrap(),
        payload
    );
    no_parts(&folder);
    assert!(partials(&f.root.join("Downloads")).is_empty());
    // The same companion owns the local entry form and only its own SSH listener.
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let local_port = listener.local_addr().unwrap().port();
    drop(listener);
    ui.key(b"\0 9");
    ui.wait("No forwarded ports");
    ui.key(b"a");
    ui.wait("LOCAL · flere-connect");
    ui.wait("Forward a port");
    ui.key(format!("4317 {local_port}\r").as_bytes());
    ui.wait("listening");
    ui.artifact("ports-listening");
    let argv: Vec<String> =
        serde_json::from_slice(&fs::read(f.root.join("forward-argv")).unwrap()).unwrap();
    assert!(
        argv.windows(2)
            .any(|v| v == ["-L", &format!("127.0.0.1:{local_port}:127.0.0.1:4317")])
    );
    ui.key(b"d");
    ui.wait("No forwarded ports");
    assert!(std::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, local_port)).is_err());
    ui.key(b"\x1b");
    assert_eq!(probe_input(&f, &tabs[0]), b"kept draft");
    unchanged(&f, &tabs);
    ui.finish();
    let saved = fs::read_to_string(f.root.join(".config/flere-connect/connections.json")).unwrap();
    assert!(!saved.contains("kept draft") && !saved.contains("new-data.bin"));
    let mut ui = CompanionUi::attach(&f, &["reconnect"]);
    assert_eq!(probe_input(&f, &tabs[0]), b"kept draft");
    ui.key(b" + resumed");
    wait_current_ui(&mut ui.master, &mut ui.screen, |_| {
        probe_input(&f, &tabs[0]) == b"kept draft + resumed"
    });
    unchanged(&f, &tabs);
    ui.finish();
    assert_eq!(
        fs::read_to_string(f.root.join("ssh-invocations"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

#[test]
fn unsolicited_remote_tools_cannot_read_local_files_or_paint_over_local_consent() {
    let f = Fixture::new();
    let ssh = f.root.join("hostile-ssh");
    fs::write(f.root.join("secret.txt"), b"LOCAL_SECRET_MUST_NOT_LEAVE").unwrap();
    fs::write(
        &ssh,
        r#"#!/usr/bin/python3
import sys,struct,json,pathlib,os
def send(tag,id,data):
 if isinstance(data,dict):data=json.dumps(data).encode()
 body=bytes([tag])+struct.pack('>Q',id)+data
 sys.stdout.buffer.write(struct.pack('>I',len(body))+body);sys.stdout.buffer.flush()
def read():
 n=struct.unpack('>I',sys.stdin.buffer.read(4))[0];b=sys.stdin.buffer.read(n)
 return b[0],struct.unpack('>Q',b[1:9])[0],b[9:]
send(1,0,os.environ['FIXTURE_VERSION'].encode()+b'a'*32)
while read()[0]!=1:pass
send(32,1,{'op':'upload','path':str(pathlib.Path('secret.txt').resolve()),'destination':'/remote'})
while True:
 tag,id,data=read()
 if id==1 and tag==38:
  pathlib.Path('rejected-upload').write_bytes(data);break
 assert tag not in (33,35),'unsolicited local file data leaked'
send(32,2,{'op':'download','name':'report.pdf','size':3,'open':True})
send(16,0,b'REMOTE_OUTPUT_MUST_NOT_COVER_LOCAL_PROMPT')
while True:
 tag,id,data=read()
 if id==2 and tag==38:
  pathlib.Path('cancelled-open').write_bytes(data);break
 assert tag!=34,'download accepted without fresh local input'
send(16,0,b'\x1b[2J\x1b[HSAFE_REMOTE_REPAINT')
while True:read()
"#,
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new("/usr/bin/env");
    command
        .args(["-u", "FLERE"])
        .arg(companion_binary())
        .args(["fixture-host", "--ssh", ssh.to_str().unwrap()])
        .env("HOME", &f.root)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_CACHE_HOME")
        .env(
            "FIXTURE_VERSION",
            std::str::from_utf8(protocol::VERSION).unwrap(),
        );
    let (master, child) = os::spawn_command_pty(&f.root, &mut command, 60, 24).unwrap();
    let mut ui = CompanionUi {
        master,
        child,
        screen: Terminal::new(60, 24),
    };
    ui.wait("Download and open locally");
    pump_ui_bytes(&mut ui.master, &mut ui.screen, 120);
    ui.artifact("local-consent-narrow");
    assert!(!ui.screen.capture(100).contains("REMOTE_OUTPUT_MUST_NOT"));
    assert!(!f.root.join("Downloads").exists());
    let rejected: Value =
        serde_json::from_slice(&fs::read(f.root.join("rejected-upload")).unwrap()).unwrap();
    assert_eq!(rejected["success"], false);
    // Pasted Enter and complete click gesture are not consent; Escape cancels.
    ui.key(b"\x1b[200~\r\n\x1b[201~\x1b[<0;4;4M\x1b[<0;4;4m");
    assert!(!f.root.join("cancelled-open").exists());
    ui.key(b"\x1b");
    ui.wait("SAFE_REMOTE_REPAINT");
    assert!(!f.root.join("Downloads").exists());
    assert_eq!(
        fs::read(f.root.join("secret.txt")).unwrap(),
        b"LOCAL_SECRET_MUST_NOT_LEAVE"
    );
}
