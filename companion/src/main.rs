//! Optional foreground SSH/clipboard companion. SSH owns authentication and host trust.
#[path = "../../src/avatar.rs"]
#[allow(
    dead_code,
    reason = "Shared badge contract contains server-only encoding and image framing"
)]
mod avatar;
mod avatars;
mod bootstrap;
#[path = "../../src/build_info.rs"]
mod build_info;
#[cfg(target_os = "linux")]
#[path = "../../src/clipboard_owner.rs"]
mod clipboard_owner;
mod connections;
mod coordinated;
#[path = "../../src/diagnostics.rs"]
#[allow(
    dead_code,
    reason = "Shared diagnostics includes server-only timing helpers"
)]
mod diagnostics;
mod drop_path;
#[path = "../../src/image_preview.rs"]
mod image_preview;
mod input;
mod keyboard;
mod local_tools;
mod os;
mod ports;
mod preview;
#[allow(
    dead_code,
    reason = "Shared protocol includes server-only framing APIs and message tags"
)]
#[path = "../../src/remote_protocol.rs"]
mod protocol;
#[path = "../../src/remote_files.rs"]
mod remote_files;
#[allow(
    dead_code,
    reason = "Shared tools framing includes UI-only encoding helpers"
)]
#[path = "../../src/remote_services.rs"]
mod remote_services;
#[path = "../../src/remote_update.rs"]
mod remote_update;
mod screenshot;
#[path = "../../src/screenshot_transfer.rs"]
mod screenshot_transfer;
mod transfers;
mod update;
use protocol as remote_protocol;
#[allow(
    dead_code,
    reason = "Shared Sixel helpers include test/reference encoding utilities"
)]
mod sixel;
use protocol::Packet;
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};
struct Ssh(Child);
impl Drop for Ssh {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
#[derive(Default)]
struct Display {
    keyboard: keyboard::Output,
}
impl Drop for Display {
    fn drop(&mut self) {
        let _ = self.keyboard.reset(&mut io::stdout());
        let _=io::stdout().write_all(b"\x1b[?2026l\x1b[0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[?2004l\x1b[?7h\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}
fn keyboard_modal(
    display: &mut Display,
    active: bool,
    queue: &mut VecDeque<Packet>,
) -> io::Result<()> {
    if display.keyboard.modal(active, &mut io::stdout())? {
        queue.push_back(Packet::new(protocol::KEYS, 0, b"\x1b[O"));
    }
    Ok(())
}
enum Event {
    Input(Vec<u8>, Instant),
    Remote(Packet),
    Failure(String),
    Closed(&'static str),
}
struct Image {
    id: u64,
    bytes: Vec<u8>,
    offset: usize,
    ready: bool,
    ended: bool,
}
fn quote(value: &str) -> io::Result<String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(io::Error::other(
            "SSH command paths must be nonempty and contain no controls",
        ));
    }
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}
fn notice(queue: &mut VecDeque<Packet>, message: &str) {
    queue.push_back(Packet::new(protocol::NOTICE, 0, message.as_bytes()));
}
fn offer(
    queue: &mut VecDeque<Packet>,
    pending: &mut Option<Image>,
    counter: &mut u64,
    bytes: Vec<u8>,
) {
    if pending.is_some() {
        notice(queue, "An image paste is already in progress");
        return;
    }
    if !(33..=protocol::IMAGE_LIMIT as usize).contains(&bytes.len())
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        notice(
            queue,
            "Clipboard image must be PNG and no larger than 20 MiB",
        );
        return;
    }
    *counter += 1;
    queue.push_back(Packet::new(
        protocol::IMAGE,
        *counter,
        (bytes.len() as u64).to_be_bytes(),
    ));
    *pending = Some(Image {
        id: *counter,
        bytes,
        offset: 0,
        ready: false,
        ended: false,
    });
}
fn actions(
    actions: Vec<input::Action>,
    queue: &mut VecDeque<Packet>,
    pending: &mut Option<Image>,
    counter: &mut u64,
    viewer: &mut preview::Viewer,
    local: &mut local_tools::Gate,
) {
    for action in actions {
        match action {
            input::Action::Drop(candidate) => {
                if pending.is_some() || viewer.active() {
                    notice(
                        queue,
                        "Finish the image transfer or close Preview before attaching a file",
                    );
                    return;
                }
                let Some(id) = counter.checked_add(1) else {
                    notice(queue, "Attachment counter exhausted");
                    return;
                };
                *counter = id;
                if local.capture(id, candidate) {
                    queue.push_back(Packet::new(remote_services::DROP_OFFER, id, Vec::new()));
                }
                // Bytes in the same input event cannot confirm the prompt or submit a draft.
                return;
            }
            input::Action::Bytes(bytes) => queue.push_back(Packet::new(protocol::KEYS, 0, bytes)),
            input::Action::Terminal(bytes) => {
                viewer.terminal(&bytes);
                if viewer.capable() && !viewer.advertised {
                    viewer.advertised = true;
                    queue.push_back(Packet::new(
                        protocol::CAPABILITIES,
                        0,
                        protocol::PREVIEW_CAP,
                    ));
                }
            }
            input::Action::Screenshot(path) => {
                let result = std::str::from_utf8(&path)
                    .map_err(io::Error::other)
                    .and_then(|path| screenshot::read_owned(std::path::Path::new(path)));
                match result {
                    Ok(bytes) => offer(queue, pending, counter, bytes),
                    Err(error) => notice(queue, &format!("Screenshot paste: {error}")),
                }
            }
            input::Action::Clipboard(original) if viewer.active() => {
                queue.push_back(Packet::new(protocol::KEYS, 0, original))
            }
            input::Action::Clipboard(original) => match os::image() {
                Ok(Some(bytes)) => offer(queue, pending, counter, bytes),
                Ok(None) => queue.push_back(Packet::new(protocol::KEYS, 0, original)),
                Err(error) => notice(queue, &format!("Clipboard: {error}")),
            },
        }
    }
}
fn local_decision(
    decision: local_tools::Decision,
    files: &mut transfers::Transfers,
    ports: &mut ports::Ports,
    connection: &connections::Connection,
    queue: &mut VecDeque<Packet>,
    dimensions: (u16, u16),
) {
    match decision {
        local_tools::Decision::Authorized(packet) if packet.tag == remote_services::REQUEST => {
            files.authorized_packet(packet, queue)
        }
        local_tools::Decision::Authorized(packet) => queue.push_back(Packet::new(
            remote_services::PORTS_RESULT,
            packet.id,
            ports.request(connection, &packet.data),
        )),
        local_tools::Decision::Cancelled(packet) | local_tools::Decision::Text(packet) => {
            queue.push_back(packet)
        }
    }
    queue.push_back(Packet::new(
        protocol::RESIZE,
        0,
        protocol::size_bytes(dimensions.0, dimensions.1),
    ));
}
fn run() -> io::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_os = "linux")]
    if args.first().is_some_and(|s| s == "_clipboard") {
        let path = PathBuf::from(
            args.get(1)
                .ok_or_else(|| io::Error::other("missing clipboard image"))?,
        );
        return clipboard_owner::serve(screenshot::read_owned(&path)?, &path);
    }
    if args.is_empty() || args[0] == "--help" || bootstrap::help(&args) {
        println!(
            "flere ssh HOST [--from-url HTTPS_MANIFEST | --package LOCAL_CORE_PACKAGE]\nflere-connect ssh HOST [options]\nflere-connect HOST [--remote PATH] [--state REMOTE_DIR] [--image LOCAL_PNG]\nflere-connect connections save NAME HOST [options]\nflere-connect connections list | remove NAME\nflere-connect --connection NAME\nflere-connect reconnect [NAME]\nflere-connect --build-info\n\n`ssh` discovers a compatible remote core, installs when absent and starts the supervisor only.\nThe direct HOST form attaches to an already-running supervisor. OpenSSH owns authentication and host trust.\nPaste a screenshot with Ctrl+V or the terminal's paste shortcut.\nText paste stays ordinary text. In Explorer, Enter/p previews PNG/JPEG images in a Sixel-capable terminal with reported cell geometry (for example Windows Terminal 1.22+). Click visible image paths in terminal text to preview. Detach with Ctrl+Space, q; remote chats remain alive.\n--image explicitly pastes one local PNG after connecting and is never saved for reconnect.\n--ssh PATH selects the installed SSH executable (default: ssh)."
        );
        return Ok(());
    }
    if matches!(args[0].as_str(), "--version" | "--build-info") && args.len() != 1 {
        return Err(io::Error::other(
            "--version and --build-info must be used alone",
        ));
    }
    if args[0] == "--version" {
        println!(
            "flere-connect {} ({}; build {})",
            env!("CARGO_PKG_VERSION"),
            build_info::TARGET,
            build_info::BUILD_ID
        );
        return Ok(());
    }
    if args[0] == "--build-info" {
        println!("{}", build_info::json());
        return Ok(());
    }
    if args[0] == "_launcher-check" && update::command(&args)? {
        return Ok(());
    }
    if update::launcher(&args)? || update::command(&args)? {
        return Ok(());
    }
    let bootstrap_source = bootstrap::arguments(&mut args)?;
    let Some(args) = connections::prepare(&args)? else {
        return Ok(());
    };
    let requested = connections::Connection::parse(&args)?;
    let connection = if let Some(source) = bootstrap_source {
        let selection = if requested.remote_explicit {
            bootstrap::Selection::ExplicitExecutable
        } else {
            bootstrap::Selection::Automatic
        };
        match bootstrap::prepare(&bootstrap::Request::new(requested, selection, source))? {
            bootstrap::Outcome::Ready(resolved) => {
                eprintln!(
                    "Connected to Flere {} (supervisor {}; epoch {}; bridge {}).",
                    resolved.runtime.build.package_version,
                    resolved.runtime.pid,
                    resolved.runtime.epoch,
                    resolved.bridge.package_version
                );
                resolved.connection
            }
            bootstrap::Outcome::UpdateRequired(offer) => {
                return Err(io::Error::other(format!(
                    "{}\nHost: {}; command: {}; state: {}; bridge version: {}; supervisor: {}.\nUpdate the selected core and local companion to a compatible release, then reconnect.",
                    offer.detail,
                    offer.connection.host,
                    offer.connection.remote,
                    offer.connection.state.as_deref().unwrap_or("default"),
                    offer.bridge.package_version,
                    offer.runtime.map_or_else(
                        || "stopped".into(),
                        |runtime| format!("{} (epoch {})", runtime.pid, runtime.epoch)
                    )
                )));
            }
        }
    } else {
        requested
    };
    let host = &connection.host;
    let executable = &connection.remote;
    let state = connection.state.as_deref();
    let ssh = &connection.ssh;
    let image = args[1..]
        .chunks(2)
        .find(|pair| pair[0] == "--image")
        .map(|pair| PathBuf::from(&pair[1]));
    let log_root = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("Flere/logs"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
            .map(|p| p.join("flere-connect/diagnostics"))
    };
    if let Some(root) = &log_root
        && let Err(e) = diagnostics::init(root, "connect")
    {
        eprintln!("Flere diagnostics unavailable: {e}");
    }
    diagnostics::record(
        "ssh-start",
        "using installed SSH; native authentication remains interactive",
    );
    // OpenSSH's remote command uses a shell; only quoted fixed argv is composed here.
    let mut remote = format!("exec {}", quote(executable)?);
    if let Some(state) = state {
        remote.push_str(&format!(" --state {}", quote(state)?));
    }
    remote.push_str(" _bridge");
    let mut child = Ssh(Command::new(ssh)
        .args(["-T", "--", host, &remote])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?);
    let mut reader = child.0.stdout.take().unwrap();
    let mut writer = child.0.stdin.take().unwrap();
    // SSH's native authentication/host-key dialogue completes before altering console modes.
    let hello = Packet::read(&mut reader)?;
    let size = os::size();
    let mut screenshot_protocol = hello.data.starts_with(protocol::VERSION)
        || hello.data.starts_with(protocol::SCREENSHOT_VERSION);
    let mut tools_protocol = hello.data.starts_with(protocol::VERSION);
    let mut avatar_protocol = hello.data.starts_with(protocol::VERSION)
        || hello.data.starts_with(protocol::SCREENSHOT_VERSION)
        || hello.data.starts_with(protocol::SIZED_VERSION)
        || hello.data.starts_with(protocol::AVATAR_VERSION);
    let mut sized_icons = hello.data.starts_with(protocol::VERSION)
        || hello.data.starts_with(protocol::SCREENSHOT_VERSION)
        || hello.data.starts_with(protocol::SIZED_VERSION);
    let reply = protocol::hello_reply(&hello, size.0, size.1)?;
    let mut update_pending = std::env::var_os("FLERE_CONNECT_UPDATE_ACK").is_some();
    let mut update_protocol = std::str::from_utf8(&hello.data[..protocol::VERSION.len()])
        .map_err(io::Error::other)?
        .to_owned();
    if let Err(error) = connections::remember(&connection.args()) {
        eprintln!("Could not remember connection for reconnect: {error}");
    }
    diagnostics::record(
        "handshake",
        &format!("connected size={}x{}", size.0, size.1),
    );
    let _console = os::Console::enter()?;
    let mut display = Display::default();
    reply.write(&mut writer)?;
    let (events, rx) = mpsc::sync_channel(32);
    let output_events = events.clone();
    std::thread::spawn(move || {
        loop {
            match Packet::read(&mut reader) {
                Ok(packet) => {
                    if output_events.send(Event::Remote(packet)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = output_events.send(if e.kind() == io::ErrorKind::UnexpectedEof {
                        Event::Closed("remote-output-eof")
                    } else {
                        Event::Failure(e.to_string())
                    });
                    break;
                }
            }
        }
    });
    let input_events = events.clone();
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        let mut bytes = [0; 8192];
        loop {
            match input.read(&mut bytes) {
                Ok(0) => {
                    let _ = input_events.send(Event::Closed("local-input-eof"));
                    break;
                }
                Ok(n) => {
                    if input_events
                        .send(Event::Input(bytes[..n].to_vec(), Instant::now()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(e) => {
                    let _ = input_events.send(Event::Failure(e.to_string()));
                    break;
                }
            }
        }
    });
    let (tx, outgoing) = mpsc::sync_channel::<Packet>(8);
    std::thread::spawn(move || {
        while let Ok(packet) = outgoing.recv() {
            if let Err(error) = packet.write(&mut writer) {
                let _ = events.send(Event::Failure(error.to_string()));
                break;
            }
        }
    });
    let mut queue = VecDeque::new();
    let mut files = transfers::Transfers::default();
    let mut ports = ports::Ports::default();
    let mut local = local_tools::Gate::default();
    let mut drop_capable = false;
    let mut coordinated = coordinated::Coordinator::default();
    coordinated.resume(&connection)?;
    if tools_protocol {
        queue.push_back(Packet::new(
            protocol::CAPABILITIES,
            0,
            remote_update::CAPABILITY,
        ));
        queue.push_back(Packet::new(
            protocol::CAPABILITIES,
            0,
            remote_services::CAPABILITY,
        ));
        queue.push_back(Packet::new(
            protocol::NOTICE,
            0,
            remote_services::DROP_PROBE,
        ));
    }
    let mut screenshots = screenshot_transfer::Receiver::default();
    if screenshot_protocol {
        queue.push_back(Packet::new(
            protocol::CAPABILITIES,
            0,
            protocol::SCREENSHOT_CAP,
        ));
    }
    let mut input = input::Input::default();
    let mut viewer = preview::Viewer::default();
    let mut avatars = avatars::Avatars::default();
    let mut avatars_advertised = false;
    let mut avatar_cell = None;
    io::stdout().write_all(preview::QUERY)?;
    io::stdout().flush()?;
    let mut pending = None;
    let mut counter = 0;
    let mut dimensions = size;
    let mut checked = Instant::now();
    let mut typed = Instant::now();
    if let Some(path) = image {
        offer(
            &mut queue,
            &mut pending,
            &mut counter,
            os::image_file(&path)?,
        );
    }
    loop {
        keyboard_modal(
            &mut display,
            local.active() || coordinated.active(),
            &mut queue,
        )?;
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Event::Input(bytes, received)) => {
                typed = Instant::now();
                if coordinated.discard_input(received) {
                    continue;
                }
                if coordinated.active() {
                    coordinated.input(&bytes, received, &connection, &mut queue, dimensions)?;
                    coordinated.draw(dimensions, &mut io::stdout())?;
                    input.reset();
                } else if local.active() {
                    if let Some(decision) = local.input(&bytes, received) {
                        local_decision(
                            decision,
                            &mut files,
                            &mut ports,
                            &connection,
                            &mut queue,
                            dimensions,
                        );
                        input.reset();
                    } else {
                        local.draw(dimensions, &mut io::stdout())?;
                    }
                } else {
                    actions(
                        input.feed(&bytes),
                        &mut queue,
                        &mut pending,
                        &mut counter,
                        &mut viewer,
                        &mut local,
                    );
                    if local.active() {
                        input.reset();
                        avatars.clear(&mut io::stdout())?;
                        viewer.restart(&mut io::stdout())?;
                        local.draw(dimensions, &mut io::stdout())?;
                    }
                }
            }
            Ok(Event::Remote(packet)) => {
                if packet.tag == protocol::OUTPUT && packet.id == 0 && update_pending {
                    update_pending = false;
                    match update::connected(&update_protocol) {
                        Ok(()) => coordinated.connection_ready(),
                        Err(error) => {
                            let message = format!("Companion update remains partial: {error}");
                            coordinated.failed(&message);
                            notice(&mut queue, &message);
                        }
                    }
                }
                if coordinated.packet(&packet, &mut queue, dimensions)? {
                    continue;
                }
                match packet.tag {
                    protocol::CAPABILITIES
                        if packet.id == 0
                            && packet.data == remote_services::DROP_CAPABILITY
                            && tools_protocol =>
                    {
                        drop_capable = true;
                        diagnostics::record("chat-drop", "capability confirmed");
                    }
                    remote_services::DROP_CONTEXT
                        if drop_capable
                            && packet.id == 0
                            && packet.data.len() == 1
                            && packet.data[0] <= 1 =>
                    {
                        if packet.data[0] == 1 {
                            input.enable_drop();
                        } else {
                            input.disable_drop();
                        }
                        diagnostics::record(
                            "chat-drop-context",
                            if packet.data[0] == 1 {
                                "capture"
                            } else {
                                "literal"
                            },
                        );
                    }
                    protocol::CAPABILITIES if packet.id == 0 && packet.data.len() <= 1024 => {}
                    remote_services::DROP_RESULT if tools_protocol => {
                        local.reject_capture(packet.id);
                        queue.push_back(Packet::new(
                            protocol::RESIZE,
                            0,
                            protocol::size_bytes(dimensions.0, dimensions.1),
                        ));
                    }
                    remote_update::REQUEST if tools_protocol => {
                        if local.active() {
                            queue.push_back(Packet::new(remote_update::RESULT, packet.id,
                            serde_json::to_vec(&serde_json::json!({"message":"Finish the current local action before updating"})).unwrap()));
                        } else {
                            coordinated.offer(&packet, &connection, &mut queue)?;
                            input.reset();
                            avatars.clear(&mut io::stdout())?;
                            viewer.restart(&mut io::stdout())?;
                            coordinated.draw(dimensions, &mut io::stdout())?;
                        }
                    }
                    protocol::OUTPUT if packet.id == 0 => {
                        let visible = !local.active() && !coordinated.active();
                        if visible && avatars_advertised {
                            avatars.begin_output();
                        }
                        display
                            .keyboard
                            .write(&packet.data, visible, &mut io::stdout())?;
                        if visible
                            && let Some((id, error)) =
                                viewer.paint(dimensions, &mut io::stdout(), os::decode_preview)?
                        {
                            queue.push_back(Packet::new(
                                protocol::PREVIEW_ERROR,
                                id,
                                error.into_bytes(),
                            ));
                        }
                    }
                    protocol::AVATAR_IMAGE
                    | protocol::AVATAR_LAYOUT
                    | protocol::AVATAR_LAYOUT_SIZED
                    | protocol::AVATAR_CLEAR
                    | protocol::PREVIEW_BEGIN
                    | protocol::PREVIEW_DATA
                    | protocol::PREVIEW_END
                    | protocol::PREVIEW_CLEAR
                        if local.active() || coordinated.active() => {}
                    remote_services::REQUEST | remote_services::PORTS_REQUEST
                        if coordinated.active() =>
                    {
                        queue.push_back(Packet::new(
                            if packet.tag == remote_services::PORTS_REQUEST {
                                remote_services::PORTS_RESULT
                            } else {
                                remote_services::RESULT
                            },
                            packet.id,
                            remote_services::result(false, "An update is in progress"),
                        ));
                    }
                    remote_services::REQUEST | remote_services::PORTS_REQUEST if tools_protocol => {
                        if let Some(decision) = local.offer(packet)? {
                            local_decision(
                                decision,
                                &mut files,
                                &mut ports,
                                &connection,
                                &mut queue,
                                dimensions,
                            );
                        } else {
                            input.reset();
                            avatars.clear(&mut io::stdout())?;
                            viewer.restart(&mut io::stdout())?;
                            local.draw(dimensions, &mut io::stdout())?;
                            local.displayed();
                        }
                    }
                    remote_services::CANCEL if tools_protocol && local.active() => {
                        if let Some(decision) = local.cancel(packet.id) {
                            local_decision(
                                decision,
                                &mut files,
                                &mut ports,
                                &connection,
                                &mut queue,
                                dimensions,
                            );
                            input.reset();
                        } else {
                            files.packet(packet, &mut queue);
                        }
                    }
                    remote_services::READY
                    | remote_services::DATA
                    | remote_services::END
                    | remote_services::CANCEL
                    | remote_services::RESULT
                        if tools_protocol =>
                    {
                        files.packet(packet, &mut queue)
                    }
                    protocol::SCREENSHOT_BEGIN
                    | protocol::SCREENSHOT_DATA
                    | protocol::SCREENSHOT_END
                    | protocol::SCREENSHOT_CANCEL
                        if screenshot_protocol =>
                    {
                        let result = screenshots.packet(&packet).and_then(|image| {
                            image.map(|bytes| screenshot::copy(&bytes)).transpose()
                        });
                        match result {
                            Ok(Some(path)) => {
                                input.image_path(path.to_string_lossy().as_bytes());
                                queue.push_back(Packet::new(
                                    protocol::SCREENSHOT_RESULT,
                                    packet.id,
                                    [1],
                                ));
                            }
                            Ok(None) => {}
                            Err(error) => {
                                screenshots.cancel();
                                let mut message = vec![0];
                                message.extend(
                                    error
                                        .to_string()
                                        .chars()
                                        .take(200)
                                        .collect::<String>()
                                        .as_bytes(),
                                );
                                queue.push_back(Packet::new(
                                    protocol::SCREENSHOT_RESULT,
                                    packet.id,
                                    message,
                                ));
                            }
                        }
                    }
                    protocol::AVATAR_IMAGE if packet.id == 0 && avatars_advertised => {
                        avatars.receive(&packet.data)?;
                    }
                    protocol::AVATAR_LAYOUT if packet.id == 0 && avatars_advertised => {
                        avatars.frame(&packet.data, dimensions)?;
                        avatars.paint(viewer.cell(), &mut io::stdout(), os::decode_preview)?;
                    }
                    protocol::AVATAR_LAYOUT_SIZED
                        if packet.id == 0 && avatars_advertised && sized_icons =>
                    {
                        avatars.sized_frame(&packet.data, dimensions)?;
                        avatars.paint(viewer.cell(), &mut io::stdout(), os::decode_preview)?;
                    }
                    protocol::AVATAR_CLEAR
                        if packet.id == 0 && packet.data.is_empty() && avatars_advertised =>
                    {
                        avatars.clear(&mut io::stdout())?;
                    }
                    protocol::PREVIEW_BEGIN
                    | protocol::PREVIEW_DATA
                    | protocol::PREVIEW_END
                    | protocol::PREVIEW_CLEAR => {
                        if packet.tag == protocol::PREVIEW_BEGIN {
                            avatars.clear(&mut io::stdout())?;
                        }
                        let ended = packet.tag == protocol::PREVIEW_END;
                        if packet.tag != protocol::PREVIEW_DATA {
                            diagnostics::record(
                                "preview",
                                &format!("tag={} id={}", packet.tag, packet.id),
                            );
                        }
                        viewer.packet(packet, &mut io::stdout())?;
                        if ended
                            && let Some((id, error)) =
                                viewer.paint(dimensions, &mut io::stdout(), os::decode_preview)?
                        {
                            queue.push_back(Packet::new(
                                protocol::PREVIEW_ERROR,
                                id,
                                error.into_bytes(),
                            ));
                        }
                    }
                    protocol::READY if packet.data.is_empty() => {
                        if let Some(p) = pending.as_mut().filter(|p| p.id == packet.id) {
                            p.ready = true;
                        }
                    }
                    protocol::RESULT => {
                        if packet.data.first().is_none_or(|b| *b > 1) {
                            return Err(io::Error::other("invalid remote transfer result"));
                        }
                        if pending.as_ref().is_some_and(|p| p.id == packet.id) {
                            pending = None;
                            queue.retain(|p| p.id != packet.id);
                        }
                    }
                    protocol::HELLO => {
                        display.keyboard.reset(&mut io::stdout())?;
                        diagnostics::record(
                            "refresh-handshake",
                            "discarding uncertain input and transfers",
                        );
                        // Refresh never replays an uncertain paste or input queued before its challenge.
                        avatar_protocol = packet.data.starts_with(protocol::VERSION)
                            || packet.data.starts_with(protocol::SCREENSHOT_VERSION)
                            || packet.data.starts_with(protocol::SIZED_VERSION)
                            || packet.data.starts_with(protocol::AVATAR_VERSION);
                        sized_icons = packet.data.starts_with(protocol::VERSION)
                            || packet.data.starts_with(protocol::SCREENSHOT_VERSION)
                            || packet.data.starts_with(protocol::SIZED_VERSION);
                        screenshot_protocol = packet.data.starts_with(protocol::VERSION)
                            || packet.data.starts_with(protocol::SCREENSHOT_VERSION);
                        screenshots = screenshot_transfer::Receiver::default();
                        files.reset();
                        local.reset();
                        input.disable_drop();
                        drop_capable = false;
                        coordinated.refreshed();
                        tools_protocol = packet.data.starts_with(protocol::VERSION);
                        if !tools_protocol {
                            ports = ports::Ports::default();
                        }
                        avatar_cell = None;
                        let reply = protocol::hello_reply(&packet, dimensions.0, dimensions.1)?;
                        update_protocol =
                            std::str::from_utf8(&packet.data[..protocol::VERSION.len()])
                                .map_err(io::Error::other)?
                                .to_owned();
                        avatars.clear(&mut io::stdout())?;
                        avatars = avatars::Avatars::default();
                        avatars_advertised = false;
                        pending = None;
                        queue.clear();
                        input.reset();
                        viewer.restart(&mut io::stdout())?;
                        queue.push_back(reply);
                        if tools_protocol {
                            queue.push_back(Packet::new(
                                protocol::CAPABILITIES,
                                0,
                                remote_update::CAPABILITY,
                            ));
                            queue.push_back(Packet::new(
                                protocol::CAPABILITIES,
                                0,
                                remote_services::CAPABILITY,
                            ));
                            queue.push_back(Packet::new(
                                protocol::NOTICE,
                                0,
                                remote_services::DROP_PROBE,
                            ));
                        }
                        if screenshot_protocol {
                            queue.push_back(Packet::new(
                                protocol::CAPABILITIES,
                                0,
                                protocol::SCREENSHOT_CAP,
                            ));
                        }
                        if viewer.advertised {
                            queue.push_back(Packet::new(
                                protocol::CAPABILITIES,
                                0,
                                protocol::PREVIEW_CAP,
                            ));
                        }
                    }
                    _ => return Err(io::Error::other("unsupported remote packet or direction")),
                }
            }
            Ok(Event::Closed(reason)) => {
                diagnostics::record("connection-closed", reason);
                break;
            }
            Ok(Event::Failure(message)) => return Err(io::Error::other(message)),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        keyboard_modal(
            &mut display,
            local.active() || coordinated.active(),
            &mut queue,
        )?;
        if avatar_protocol
            && viewer.capable()
            && (!avatars_advertised || (sized_icons && avatar_cell != viewer.cell()))
        {
            let mut capability = protocol::AVATAR_CAP.to_vec();
            if sized_icons {
                let (w, h) = viewer.cell().unwrap();
                capability = protocol::AVATAR_SIZE_CAP.to_vec();
                capability.extend(avatar::cell_bytes((w, h))?);
                diagnostics::record(
                    "project-icons",
                    &format!("cell={}x{} pixels; height-driven widths", w, h),
                );
            }
            queue.push_back(Packet::new(protocol::CAPABILITIES, 0, capability));
            avatars_advertised = true;
        }
        if avatar_cell != viewer.cell() {
            if avatar_cell.is_some() && avatars_advertised {
                avatars.clear(&mut io::stdout())?;
                queue.push_back(Packet::new(
                    protocol::RESIZE,
                    0,
                    protocol::size_bytes(dimensions.0, dimensions.1),
                ));
            }
            avatar_cell = viewer.cell();
        }
        if avatars_advertised && !viewer.active() && !local.active() && !coordinated.active() {
            avatars.paint(viewer.cell(), &mut io::stdout(), os::decode_preview)?;
        }
        if typed.elapsed() > Duration::from_millis(30) {
            if let Some(decision) = local.escape().or_else(|| local.expired()) {
                local_decision(
                    decision,
                    &mut files,
                    &mut ports,
                    &connection,
                    &mut queue,
                    dimensions,
                );
                input.reset();
            }
            coordinated.escape(&mut queue, dimensions);
            if !local.active() && !coordinated.active() {
                actions(
                    input.timeout(),
                    &mut queue,
                    &mut pending,
                    &mut counter,
                    &mut viewer,
                    &mut local,
                );
            }
        }
        if checked.elapsed() > Duration::from_millis(200) {
            let size = os::size();
            if size != dimensions {
                diagnostics::record("resize", &format!("{}x{}", size.0, size.1));
                avatars.clear(&mut io::stdout())?;
                dimensions = size;
                if viewer.active() {
                    viewer.invalidate();
                    io::stdout().write_all(b"\x1b[2J")?;
                }
                io::stdout().write_all(preview::QUERY)?;
                io::stdout().flush()?;
                if local.active() {
                    local.draw(size, &mut io::stdout())?;
                }
                queue.push_back(Packet::new(
                    protocol::RESIZE,
                    0,
                    protocol::size_bytes(size.0, size.1),
                ));
            }
            checked = Instant::now();
        }
        files.tick(&mut queue);
        if coordinated.tick(&connection, &mut queue)? {
            break;
        }
        if coordinated.active() {
            coordinated.draw(dimensions, &mut io::stdout())?;
        }
        if let Some(update) = ports.tick() {
            queue.push_back(Packet::new(remote_services::PORTS_RESULT, 0, update));
        }
        if queue.len() < 8
            && let Some(p) = pending.as_mut().filter(|p| p.ready && !p.ended)
        {
            if p.offset < p.bytes.len() {
                let end = (p.offset + protocol::CHUNK).min(p.bytes.len());
                let mut data = (p.offset as u64).to_be_bytes().to_vec();
                data.extend_from_slice(&p.bytes[p.offset..end]);
                queue.push_back(Packet::new(protocol::DATA, p.id, data));
                p.offset = end;
            } else {
                queue.push_back(Packet::new(protocol::END, p.id, Vec::new()));
                p.ended = true;
            }
        }
        if queue.iter().map(|p| p.data.len()).sum::<usize>() > 1024 * 1024 {
            return Err(io::Error::other(
                "SSH input stalled; detached without retrying queued input",
            ));
        }
        while let Some(packet) = queue.pop_front() {
            match tx.try_send(packet) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(packet)) => {
                    queue.push_front(packet);
                    break;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    return Err(io::Error::other("SSH writer disconnected"));
                }
            }
        }
    }
    avatars.clear(&mut io::stdout())?;
    viewer.clear(&mut io::stdout())?;
    diagnostics::record(
        "disconnect",
        &format!("ssh_status={:?}", child.0.try_wait()?),
    );
    Ok(())
}
fn main() {
    let result = run().and_then(|_| update::finish_restart());
    match &result {
        Ok(()) => diagnostics::record("exit", "normal"),
        Err(error) => {
            diagnostics::error("exit-error", error);
            eprintln!("flere-connect: {error}");
        }
    }
    if let Some(path) = diagnostics::path() {
        eprintln!("Flere diagnostic log: {}", path.display());
    }
    if result.is_err() {
        std::process::exit(1);
    }
}
