//! Explicit companion actions with frozen workspace/pane ownership. File bytes
//! never enter the terminal input path; the supervisor validates every chunk.
use super::*;
use crate::{
    remote_protocol::{self as protocol, Packet},
    remote_services as service,
};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(super) enum Action {
    Upload,
    Download,
    OpenLocal,
    Ports,
}
enum Panel {
    Ports { selected: usize },
}
enum FileOp {
    BeginUpload,
    BeginDownload(bool),
    WriteUpload(usize),
    ReadDownload,
    FinishUpload,
    FinishDownload,
}
struct Transfer {
    id: u64,
    origin: panes::EditorOrigin,
    state: PathBuf,
    token: Option<String>,
    upload: bool,
    attachment: bool,
    literal: bool,
    attachment_ticket: Option<String>,
    directory: PathBuf,
    size: u64,
    offset: u64,
    ready: bool,
    ended: bool,
    pending: Option<FileOp>,
    touched: Instant,
}
impl Drop for Transfer {
    fn drop(&mut self) {
        if let Some(ticket) = &self.attachment_ticket {
            let _ = wire::request(&self.state, &["attachment-cancel", ticket]);
        }
        if let Some(token) = &self.token {
            let _ = wire::request(&self.state, &["file-cancel", token]);
        }
    }
}
#[derive(Default)]
pub(super) struct Tools {
    capable: bool,
    drop_capable: bool,
    drop_context: Option<bool>,
    last_drop: u64,
    next: u64,
    panel: Option<Panel>,
    transfer: Option<Transfer>,
    ports: Vec<Value>,
    port_status: String,
    port_origin: Option<(u64, panes::EditorOrigin)>,
}
fn send(tag: u8, id: u64, data: impl Into<Vec<u8>>) -> io::Result<()> {
    Packet::new(tag, id, data).write(&mut output::writer())
}
impl Ui {
    pub(super) fn remote_tools_open(&self) -> bool {
        self.remote_tools.panel.is_some()
    }
    pub(super) fn remote_tools_busy(&self) -> bool {
        self.remote_tools.panel.is_some() || self.remote_tools.transfer.is_some()
    }
    fn remote_origin_matches(&self, origin: &panes::EditorOrigin) -> bool {
        origin.epoch == self.snapshot.epoch
            && origin.workspace == self.snapshot.active
            && origin.tab == self.snapshot.tab
            && origin.revision == self.pane_revision()
            && origin.run
                == self
                    .snapshot
                    .session()
                    .map(|t| t.run.as_str())
                    .unwrap_or("")
    }
    fn next_remote_tool(&mut self) -> io::Result<u64> {
        self.remote_tools.next = self
            .remote_tools
            .next
            .checked_add(1)
            .ok_or_else(|| service::invalid("remote request counter exhausted"))?;
        Ok(self.remote_tools.next)
    }
    fn file_begin(
        &self,
        origin: &panes::EditorOrigin,
        upload: bool,
        path: &Path,
        size: u64,
    ) -> io::Result<Value> {
        let fields = [
            "file-begin".to_string(),
            if upload { "upload" } else { "download" }.into(),
            origin.epoch.clone(),
            origin.workspace.to_string(),
            origin.tab.to_string(),
            origin.run.clone(),
            origin.revision.to_string(),
            wire::hex(path.to_string_lossy().as_bytes()),
            size.to_string(),
        ];
        let bytes = wire::request(
            &self.state,
            &fields[..if upload { 9 } else { 8 }]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )?;
        service::decode(&bytes)
    }
    pub(super) fn open_remote_tool(&mut self, action: Action) {
        self.menu = false;
        if self.remote.is_none() || !self.remote_tools.capable {
            self.notice =
                "Connect with the updated flere-connect companion to use remote files and Ports"
                    .into();
            return;
        }
        if self.remote_tools.transfer.is_some() {
            self.notice = "A file transfer is in progress; Esc cancels it".into();
            return;
        }
        self.selection = None;
        self.smooth_scroll = None;
        self.paste_target = None;
        let origin = self.editor_origin();
        let result = (|| -> io::Result<()> {
            match action {
                Action::Ports => {
                    self.remote_tools.panel = Some(Panel::Ports { selected: 0 });
                    self.port_request(json!({"op":"list"}))?;
                }
                Action::Upload => {
                    let explorer = self.explorers.get(&self.snapshot.active).ok_or_else(|| {
                        service::invalid("Open Files and choose an upload directory")
                    })?;
                    let directory = explorer
                        .entries
                        .get(explorer.selected)
                        .filter(|(_, _, dir)| *dir)
                        .map(|(_, p, _)| p.clone())
                        .unwrap_or_else(|| explorer.root.clone());
                    self.start_upload(origin, directory)?;
                }
                Action::Download | Action::OpenLocal => {
                    let path = self
                        .explorers
                        .get(&self.snapshot.active)
                        .and_then(|e| e.entries.get(e.selected))
                        .filter(|(_, _, dir)| !dir)
                        .map(|(_, p, _)| p.clone())
                        .ok_or_else(|| service::invalid("Select a regular file in Files"))?;
                    let metadata = self.file_begin(&origin, false, &path, 0)?;
                    let token = metadata
                        .get("token")
                        .and_then(Value::as_str)
                        .ok_or_else(|| service::invalid("missing transfer token"))?
                        .to_owned();
                    let id = self.next_remote_tool()?;
                    self.remote_tools.transfer = Some(Transfer {
                        id,
                        origin,
                        state: self.state.clone(),
                        token: Some(token),
                        upload: false,
                        attachment: false,
                        literal: false,
                        attachment_ticket: None,
                        directory: PathBuf::new(),
                        size: 0,
                        offset: 0,
                        ready: false,
                        ended: false,
                        pending: Some(FileOp::BeginDownload(matches!(action, Action::OpenLocal))),
                        touched: Instant::now(),
                    });
                    self.notice = "Preparing download to local Downloads · Esc cancels".into();
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.remote_tools.transfer = None;
            self.notice = wire::passive(&error.to_string());
        }
    }
    fn port_request(&mut self, value: Value) -> io::Result<()> {
        let id = self.next_remote_tool()?;
        if matches!(value["op"].as_str(), Some("forward" | "open")) {
            self.remote_tools.port_origin = Some((id, self.editor_origin()));
        }
        send(
            service::PORTS_REQUEST,
            id,
            serde_json::to_vec(&value).map_err(io::Error::other)?,
        )
    }
    fn cancel_remote_file(&mut self, message: &str) {
        if let Some(transfer) = self.remote_tools.transfer.take() {
            let _ = send(service::CANCEL, transfer.id, message.as_bytes());
            let committing = matches!(transfer.pending, Some(FileOp::FinishUpload));
            drop(transfer);
            self.notice = if committing {
                "Cancel requested; an upload commit already started may complete".into()
            } else {
                message.into()
            };
        }
    }
    pub(super) fn remote_tools_key(&mut self, key: &Key) -> bool {
        let Some(Panel::Ports { mut selected }) = self.remote_tools.panel.take() else {
            if self.remote_tools.transfer.is_some() && matches!(key, Key::Bytes(b) if b == b"\x1b")
            {
                self.cancel_remote_file("File transfer cancelled");
                return true;
            }
            return false;
        };
        if matches!(key, Key::Bytes(b) if b == b"\x1b") {
            return true;
        }
        let mut request = None;
        if let Key::Bytes(b) = key {
            match b.as_slice() {
                b"j" | b"\x1b[B" => {
                    selected = (selected + 1).min(self.remote_tools.ports.len().saturating_sub(1))
                }
                b"k" | b"\x1b[A" => selected = selected.saturating_sub(1),
                b"a" => {
                    request = Some(
                        json!({"op":"forward","workspace":self.snapshot.workspace().map(|w|w.name.as_str()).unwrap_or("Workspace")}),
                    )
                }
                b"r" => request = Some(json!({"op":"list"})),
                b"d" | b"\r" => {
                    if let Some(row) = self.remote_tools.ports.get(selected) {
                        request = Some(
                            json!({"op":if b==b"d" {"stop"} else {"open"},"local":row["local"]}),
                        );
                    }
                }
                _ => {}
            }
        }
        self.remote_tools.panel = Some(Panel::Ports { selected });
        if let Some(request) = request
            && let Err(error) = self.port_request(request)
        {
            self.notice = error.to_string();
        }
        true
    }
    fn start_upload(&mut self, origin: panes::EditorOrigin, directory: PathBuf) -> io::Result<()> {
        let id = self.next_remote_tool()?;
        send(
            service::REQUEST,
            id,
            serde_json::to_vec(&json!({"op":"upload","destination":directory}))
                .map_err(io::Error::other)?,
        )?;
        self.remote_tools.transfer = Some(Transfer {
            id,
            origin,
            state: self.state.clone(),
            token: None,
            upload: true,
            attachment: false,
            literal: false,
            attachment_ticket: None,
            directory,
            size: 0,
            offset: 0,
            ready: false,
            ended: false,
            pending: None,
            touched: Instant::now(),
        });
        self.notice = "Preparing local upload · Esc cancels".into();
        Ok(())
    }
    fn chat_drop_focused(&self) -> bool {
        self.arcade.is_none()
            && !self.arcade_gate.pending()
            && !self.screensaver.active
            && !self.image_view()
            && !self.nav
            && !self.menu
            && self.context_menu.is_none()
            && !self.board
            && self.form.is_none()
            && self.search.is_none()
            && self.update.is_none()
            && self.tasks.is_none()
            && self.confirm.is_none()
            && self.focus == Focus::Terminal
            && self.remote_tools.panel.is_none()
            && !self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
            && !self.remote_image_busy()
    }
    pub(super) fn remote_drop_context(&mut self) -> io::Result<()> {
        if !self.remote_tools.drop_capable {
            return Ok(());
        }
        // A capture affordance only: the one-use supervisor ticket decides whether
        // the current agent is the supported native harness when a path is offered.
        let enabled = self.chat_drop_focused()
            && self.snapshot.bracketed_paste
            && self
                .snapshot
                .session()
                .is_some_and(|t| t.alive && t.kind == "agent");
        if self.remote_tools.drop_context != Some(enabled) {
            send(service::DROP_CONTEXT, 0, [u8::from(enabled)])?;
            self.remote_tools.drop_context = Some(enabled);
        }
        Ok(())
    }
    fn offer_chat_drop(&mut self, packet: &Packet) -> io::Result<()> {
        let result = (|| -> io::Result<()> {
            if !self.remote_tools.drop_capable
                || packet.id == 0
                || packet.id <= self.remote_tools.last_drop
                || !packet.data.is_empty()
            {
                return Err(service::invalid("Unnegotiated or stale local file offer"));
            }
            self.remote_tools.last_drop = packet.id;
            if self
                .screensaver
                .consume(&Key::Paste(Vec::new()), Instant::now())
            {
                return Err(service::invalid(
                    "Screensaver dismissed; drop the path again",
                ));
            }
            if !self.chat_drop_focused() || self.remote_tools.transfer.is_some() {
                return Err(service::invalid(
                    "Focus the native chat input before attaching a file",
                ));
            }
            self.flush_input();
            let origin = self.editor_origin();
            let mut literal = false;
            let ticket = String::from_utf8(
                wire::request(
                    &self.state,
                    &[
                        "attachment-check",
                        &origin.epoch,
                        &origin.workspace.to_string(),
                        &origin.tab.to_string(),
                        &origin.run,
                        &origin.revision.to_string(),
                    ],
                )
                .or_else(|_| {
                    literal = true;
                    wire::request(
                        &self.state,
                        &[
                            "literal-ticket",
                            &origin.epoch,
                            &origin.workspace.to_string(),
                            &origin.tab.to_string(),
                            &origin.run,
                            &origin.revision.to_string(),
                        ],
                    )
                })?,
            )
            .map_err(io::Error::other)?;
            let id = self.next_remote_tool()?;
            self.remote_tools.transfer = Some(Transfer {
                id,
                origin,
                state: self.state.clone(),
                token: None,
                upload: true,
                attachment: true,
                literal,
                attachment_ticket: Some(ticket),
                directory: PathBuf::new(),
                size: 0,
                offset: 0,
                ready: false,
                ended: false,
                pending: None,
                touched: Instant::now(),
            });
            let target = format!(
                "{} · {}",
                self.snapshot
                    .workspace()
                    .map(|w| w.name.as_str())
                    .unwrap_or("Workspace"),
                self.snapshot
                    .session()
                    .map(|t| t.title.as_str())
                    .unwrap_or("Chat")
            );
            send(
                service::REQUEST,
                id,
                serde_json::to_vec(
                    &json!({"op":if literal {"literal"} else {"attach"},"offer":packet.id,"target":wire::passive(&target)}),
                )
                .map_err(io::Error::other)?,
            )?;
            self.notice = "Local file choice pending · no file read yet · Esc cancels".into();
            Ok(())
        })();
        if let Err(error) = result {
            self.notice = wire::passive(&error.to_string());
            send(
                service::DROP_RESULT,
                packet.id,
                service::result(false, &self.notice),
            )?;
        }
        Ok(())
    }
    pub(super) fn remote_tools_packet(&mut self, packet: &Packet) -> io::Result<bool> {
        if packet.tag == protocol::NOTICE && packet.id == 0 && packet.data == service::DROP_PROBE {
            self.remote_tools.drop_capable = true;
            send(protocol::CAPABILITIES, 0, service::DROP_CAPABILITY)?;
            self.remote_drop_context()?;
            return Ok(true);
        }
        if packet.tag == service::DROP_OFFER {
            self.offer_chat_drop(packet)?;
            return Ok(true);
        }
        if packet.tag == protocol::CAPABILITIES
            && packet.id == 0
            && packet.data == service::CAPABILITY
        {
            self.remote_tools.capable = true;
            return Ok(true);
        }
        if packet.tag == service::PORTS_RESULT {
            if self
                .remote_tools
                .port_origin
                .as_ref()
                .is_some_and(|(id, _)| *id == packet.id)
            {
                self.remote_tools.port_origin = None;
            }
            let value = service::decode(&packet.data)?;
            self.remote_tools.ports = value
                .get("ports")
                .and_then(Value::as_array)
                .filter(|r| r.len() <= 16)
                .cloned()
                .ok_or_else(|| service::invalid("invalid forwarded port list"))?;
            self.remote_tools.port_status =
                wire::passive(value.get("message").and_then(Value::as_str).unwrap_or(""));
            return Ok(true);
        }
        if !matches!(
            packet.tag,
            service::METADATA
                | service::DROP_TEXT
                | service::READY
                | service::DATA
                | service::END
                | service::CANCEL
                | service::RESULT
        ) {
            return Ok(false);
        }
        if !self
            .remote_tools
            .transfer
            .as_ref()
            .is_some_and(|t| t.id == packet.id)
        {
            return Ok(true);
        }
        let mut transfer = self.remote_tools.transfer.take().unwrap();
        let result = (|| -> io::Result<bool> {
            if !self.remote_origin_matches(&transfer.origin)
                || (transfer.attachment && !self.chat_drop_focused())
            {
                return Err(service::invalid(
                    "File transfer cancelled: workspace or pane changed",
                ));
            }
            match packet.tag {
                service::DROP_TEXT
                    if transfer.attachment
                        && transfer.token.is_none()
                        && transfer.pending.is_none() =>
                {
                    let ticket = transfer
                        .attachment_ticket
                        .take()
                        .ok_or_else(|| service::invalid("Attachment choice already consumed"))?;
                    let message = wire::request(
                        &self.state,
                        &[
                            if transfer.literal {
                                "literal-paste"
                            } else {
                                "attachment-text"
                            },
                            &ticket,
                            &wire::hex(&packet.data),
                        ],
                    )?;
                    self.notice =
                        wire::passive(&String::from_utf8(message).map_err(io::Error::other)?);
                    send(
                        service::RESULT,
                        transfer.id,
                        service::result(true, &self.notice),
                    )?;
                    return Ok(false);
                }
                service::METADATA
                    if transfer.upload && !transfer.literal && transfer.token.is_none() =>
                {
                    let (name, size) = service::file_metadata(&packet.data)?;
                    let meta = if transfer.attachment {
                        let ticket = transfer.attachment_ticket.take().ok_or_else(|| {
                            service::invalid("Attachment choice already consumed")
                        })?;
                        let origin = &transfer.origin;
                        service::decode(&wire::request(
                            &self.state,
                            &[
                                "file-begin",
                                "attach",
                                &origin.epoch,
                                &origin.workspace.to_string(),
                                &origin.tab.to_string(),
                                &origin.run,
                                &origin.revision.to_string(),
                                &wire::hex(name.as_bytes()),
                                &size.to_string(),
                                &ticket,
                            ],
                        )?)?
                    } else {
                        self.file_begin(
                            &transfer.origin,
                            true,
                            &transfer.directory.join(name),
                            size,
                        )?
                    };
                    transfer.token = Some(
                        meta["token"]
                            .as_str()
                            .ok_or_else(|| service::invalid("missing upload token"))?
                            .into(),
                    );
                    transfer.size = size;
                    transfer.pending = Some(FileOp::BeginUpload);
                }
                service::READY if !transfer.upload && !transfer.ready && packet.data.is_empty() => {
                    transfer.ready = true
                }
                service::DATA if transfer.upload && transfer.pending.is_none() => {
                    let (offset, bytes) = service::unchunk(&packet.data)?;
                    if offset != transfer.offset || offset + bytes.len() as u64 > transfer.size {
                        return Err(service::invalid("upload chunk offset changed"));
                    }
                    let token = transfer
                        .token
                        .as_deref()
                        .ok_or_else(|| service::invalid("upload not accepted"))?;
                    wire::request(
                        &self.state,
                        &["file-write", token, &offset.to_string(), &wire::hex(bytes)],
                    )?;
                    transfer.pending = Some(FileOp::WriteUpload(bytes.len()));
                }
                service::END
                    if transfer.upload
                        && transfer.pending.is_none()
                        && transfer.offset == transfer.size
                        && packet.data.is_empty() =>
                {
                    let token = transfer
                        .token
                        .as_deref()
                        .ok_or_else(|| service::invalid("upload not accepted"))?;
                    wire::request(&self.state, &["file-finish", token])?;
                    transfer.pending = Some(FileOp::FinishUpload);
                }
                service::RESULT => {
                    let result = service::decode(&packet.data)?;
                    let success = result
                        .get("success")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| service::invalid("invalid file receipt"))?;
                    if success && (transfer.upload || !transfer.ended) {
                        return Err(service::invalid("file receipt arrived before completion"));
                    }
                    self.notice = wire::passive(
                        result
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("File transfer ended"),
                    );
                    return Ok(false);
                }
                service::CANCEL => {
                    self.notice = if matches!(transfer.pending, Some(FileOp::FinishUpload)) {
                        "Companion cancelled; an upload commit already started may complete".into()
                    } else {
                        "File transfer cancelled by companion".into()
                    };
                    return Ok(false);
                }
                _ => return Err(service::invalid("invalid file transfer state")),
            }
            transfer.touched = Instant::now();
            self.notice = format!(
                "{} {} / {} bytes · Esc cancels",
                if transfer.upload {
                    "Uploading"
                } else {
                    "Downloading"
                },
                transfer.offset,
                transfer.size
            );
            Ok(true)
        })();
        match result {
            Ok(true) => self.remote_tools.transfer = Some(transfer),
            Ok(false) => {}
            Err(error) => {
                self.notice = wire::passive(&error.to_string());
                let _ = send(service::CANCEL, packet.id, self.notice.as_bytes());
            }
        }
        Ok(true)
    }
    pub(super) fn tick_remote_tools(&mut self, can_output: bool) -> bool {
        if self
            .remote_tools
            .port_origin
            .as_ref()
            .is_some_and(|(_, origin)| !self.remote_origin_matches(origin))
        {
            let (id, _) = self.remote_tools.port_origin.take().unwrap();
            let _ = send(
                service::CANCEL,
                id,
                b"Port action cancelled: workspace or pane changed",
            );
            self.remote_tools.port_status =
                "Port action cancelled: workspace or pane changed".into();
        }
        if let Some(transfer) = &self.remote_tools.transfer
            && (!self.remote_origin_matches(&transfer.origin)
                || (transfer.attachment && !self.chat_drop_focused())
                || transfer.touched.elapsed().as_secs() >= service::IDLE_SECONDS)
        {
            self.cancel_remote_file(
                "File transfer cancelled: target changed or transfer timed out",
            );
            return true;
        }
        if !can_output {
            return false;
        }
        let Some(mut transfer) = self.remote_tools.transfer.take() else {
            return false;
        };
        let result = (|| -> io::Result<bool> {
            if let Some(operation) = transfer.pending.take() {
                let token = transfer
                    .token
                    .as_deref()
                    .ok_or_else(|| service::invalid("file operation has no token"))?;
                let response: Value =
                    serde_json::from_slice(&wire::request(&self.state, &["file-poll", token])?)
                        .map_err(io::Error::other)?;
                if response.get("pending").and_then(Value::as_bool) == Some(true) {
                    transfer.pending = Some(operation);
                    return Ok(true);
                }
                let bytes = wire::unhex(
                    response
                        .get("data")
                        .and_then(Value::as_str)
                        .ok_or_else(|| service::invalid("invalid file worker response"))?,
                )?;
                match operation {
                    FileOp::BeginUpload => send(service::READY, transfer.id, Vec::new())?,
                    FileOp::BeginDownload(open) => {
                        let (name, size) = service::file_metadata(&bytes)?;
                        transfer.size = size;
                        send(
                            service::REQUEST,
                            transfer.id,
                            serde_json::to_vec(
                                &json!({"op":"download","name":name,"size":size,"open":open}),
                            )
                            .map_err(io::Error::other)?,
                        )?;
                    }
                    FileOp::WriteUpload(length) => {
                        transfer.offset += length as u64;
                        send(service::READY, transfer.id, Vec::new())?;
                    }
                    FileOp::ReadDownload => {
                        send(
                            service::DATA,
                            transfer.id,
                            service::chunk(transfer.offset, &bytes)?,
                        )?;
                        transfer.offset += bytes.len() as u64;
                    }
                    FileOp::FinishUpload => {
                        transfer.token = None;
                        let message =
                            wire::passive(&String::from_utf8(bytes).map_err(io::Error::other)?);
                        self.notice = if transfer.attachment {
                            message
                        } else {
                            format!("Uploaded {message}")
                        };
                        send(
                            service::RESULT,
                            transfer.id,
                            service::result(true, &self.notice),
                        )?;
                        if !transfer.attachment
                            && let Some(explorer) =
                                self.explorers.get_mut(&transfer.origin.workspace)
                        {
                            explorer.load(explorer.root.clone());
                        }
                        return Ok(false);
                    }
                    FileOp::FinishDownload => {
                        transfer.token = None;
                        transfer.ended = true;
                        send(service::END, transfer.id, Vec::new())?;
                    }
                }
                transfer.touched = Instant::now();
                self.notice = format!(
                    "{} {} / {} bytes · Esc cancels",
                    if transfer.upload {
                        "Uploading"
                    } else {
                        "Downloading"
                    },
                    transfer.offset,
                    transfer.size
                );
            }
            if !transfer.upload && transfer.ready && !transfer.ended && transfer.pending.is_none() {
                let token = transfer
                    .token
                    .as_deref()
                    .ok_or_else(|| service::invalid("download has no token"))?;
                if transfer.offset == transfer.size {
                    wire::request(&self.state, &["file-finish", token])?;
                    transfer.pending = Some(FileOp::FinishDownload);
                } else {
                    wire::request(
                        &self.state,
                        &["file-read", token, &transfer.offset.to_string()],
                    )?;
                    transfer.pending = Some(FileOp::ReadDownload);
                }
            }
            Ok(true)
        })();
        match result {
            Ok(true) => {
                self.remote_tools.transfer = Some(transfer);
            }
            Ok(false) => {}
            Err(error) => {
                self.notice = wire::passive(&error.to_string());
                let _ = send(service::CANCEL, transfer.id, self.notice.as_bytes());
            }
        }
        true
    }
    pub(super) fn draw_remote_tools(&self, c: &mut Canvas) {
        let Some(panel) = &self.remote_tools.panel else {
            return;
        };
        let width = self.layout.width.saturating_sub(2).clamp(8, 100);
        let height = self.layout.height.saturating_sub(2).clamp(6, 20);
        let (x, y) = (
            (self.layout.width - width) / 2,
            (self.layout.height - height) / 2,
        );
        c.fill(x, y, width, height, style(TEXT, PANEL, false));
        c.border(x, y, width, height, CYAN);
        let hint = "a add · Enter browser · d stop · r refresh · Esc close";
        c.text(x + 2, y + 1, width - 4, "Ports", style(CYAN, PANEL, true));
        let Panel::Ports { selected } = panel;
        {
            let count = height.saturating_sub(6);
            let start = selected.saturating_sub(count.saturating_sub(1));
            for (index, row) in self
                .remote_tools
                .ports
                .iter()
                .enumerate()
                .skip(start)
                .take(count)
            {
                let bg = if index == *selected { ACTIVE_BG } else { PANEL };
                let label = format!(
                    "127.0.0.1:{} → :{} · {} · {}",
                    row["local"],
                    row["remote"],
                    row["workspace"].as_str().unwrap_or(""),
                    row["status"].as_str().unwrap_or("")
                );
                c.fill(
                    x + 1,
                    y + 3 + index - start,
                    width - 2,
                    1,
                    style(TEXT, bg, false),
                );
                c.text(
                    x + 2,
                    y + 3 + index - start,
                    width - 4,
                    &wire::passive(&label),
                    style(TEXT, bg, index == *selected),
                );
            }
            if self.remote_tools.ports.is_empty() {
                c.text(
                    x + 2,
                    y + 3,
                    width - 4,
                    "No forwarded ports. Press a to add one.",
                    style(MUTED, PANEL, false),
                );
            }
            c.text(
                x + 2,
                y + height - 3,
                width - 4,
                &self.remote_tools.port_status,
                style(MUTED, PANEL, false),
            );
        }
        c.text(
            x + 2,
            y + height - 2,
            width - 4,
            hint,
            style(MUTED, PANEL, false),
        );
    }
}
