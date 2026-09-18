//! One remote UI owns one ordered SSH connection and its pending image upload.
use super::*;
use crate::{attachments::Upload, remote_protocol as protocol};
use protocol::Packet;
pub(super) struct Connection {
    pub size: (u16, u16),
    pub buffer: Vec<u8>,
    last_image: u64,
    pub browser_capable: bool,
    pub browser_input: u64,
    pub browser_serial: u64,
    pub preview_capable: bool,
    pub screenshot_capable: bool,
    pub update_capable: bool,
    pub update_counter: u64,
    pub update_rpc: Option<super::update::RemoteRpc>,
    pub screenshot_counter: u64,
    pub screenshot: Option<super::screenshot::Export>,
    pub avatar_capable: bool,
    pub avatar_cell: Option<(usize, usize)>,
    pub avatar_damage: bool,
    pub force_redraw: bool,
    pub preview_counter: u64,
    pub preview: Option<super::image_preview::ImagePreview>,
    pub preview_clear: Option<u64>,
    pending: Option<Pending>,
}
impl Drop for Connection {
    fn drop(&mut self) {
        // Ui drops before DisplayGuard, including on errors/refresh: remove graphics
        // before the alternate screen is left, never repaint into the user's shell.
        if self.avatar_capable {
            let _ =
                Packet::new(protocol::AVATAR_CLEAR, 0, Vec::new()).write(&mut io::stdout().lock());
        }
        let id = self
            .preview_clear
            .take()
            .or_else(|| self.preview.as_ref().and_then(|p| p.clear_id()));
        if let Some(id) = id {
            let _ = Packet::new(protocol::PREVIEW_CLEAR, id, Vec::new())
                .write(&mut io::stdout().lock());
        }
    }
}
struct Pending {
    id: u64,
    state: PathBuf,
    token: String,
    epoch: String,
    workspace: u64,
    session: u64,
    run: String,
    upload: Option<Upload>,
    started: Instant,
}
impl Drop for Pending {
    fn drop(&mut self) {
        drop(self.upload.take());
        let _ = wire::request(&self.state, &["image-cancel", &self.token]);
    }
}
impl Connection {
    pub(super) fn input_busy(&self) -> bool {
        self.pending.is_some()
            || self.screenshot.is_some()
            || self.preview.is_some()
            || !self.buffer.is_empty()
    }
    pub fn handshake(input: &mut fs::File) -> io::Result<Self> {
        let mut challenge = protocol::VERSION.to_vec();
        challenge.extend_from_slice(os::nonce()?.as_bytes());
        Packet::new(protocol::HELLO, 0, &challenge[..]).write(&mut io::stdout().lock())?;
        let mut discarded = 0;
        let size = loop {
            // Read exactly one frame from the event loop's unbuffered file;
            // capabilities following HELLO remain visible to its poll call.
            let hello = Packet::read(input)?;
            if hello.tag == protocol::HELLO {
                if hello.id != 0 || !hello.data.starts_with(&challenge) {
                    return Err(wire::invalid(
                        "incompatible or stale Flere remote companion",
                    ));
                }
                break protocol::dimensions(&hello.data[challenge.len()..])?;
            }
            // Refresh can leave old keys/chunks already queued in SSH. Discard a bounded
            // tail before the fresh challenge reply; never replay it into a native chat.
            discarded += hello.data.len() + 13;
            if discarded > 1024 * 1024 {
                return Err(wire::invalid(
                    "remote handshake exceeded pending-input bound",
                ));
            }
        };
        Ok(Self {
            size,
            buffer: Vec::new(),
            last_image: 0,
            browser_capable: false,
            browser_input: 0,
            browser_serial: 0,
            preview_capable: false,
            screenshot_capable: false,
            update_capable: false,
            update_counter: 0,
            update_rpc: None,
            screenshot_counter: 0,
            screenshot: None,
            avatar_capable: false,
            avatar_cell: None,
            avatar_damage: false,
            force_redraw: false,
            preview_counter: 0,
            preview: None,
            preview_clear: None,
            pending: None,
        })
    }
}
pub(super) fn emit(remote: bool, bytes: &[u8]) -> io::Result<()> {
    let started = Instant::now();
    let mut stdout = io::stdout().lock();
    if remote {
        for chunk in bytes.chunks(protocol::CHUNK) {
            Packet::new(protocol::OUTPUT, 0, chunk).write(&mut stdout)?;
        }
    } else {
        stdout.write_all(bytes)?;
        stdout.flush()?;
    }
    crate::diagnostics::slow("terminal-write", started);
    Ok(())
}
impl Ui {
    pub(super) fn remote_image_busy(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.pending.is_some())
    }
    fn remote_result(&mut self, id: u64, success: bool, message: &str) -> io::Result<()> {
        self.notice = message.into();
        let mut data = vec![u8::from(success)];
        data.extend_from_slice(wire::passive(message).as_bytes());
        Packet::new(protocol::RESULT, id, data).write(&mut io::stdout().lock())
    }
    pub(super) fn remote_cancel_changed(&mut self) -> io::Result<bool> {
        let Some(remote) = &mut self.remote else {
            return Ok(false);
        };
        if remote.pending.as_ref().is_some_and(|p| {
            self.nav
                || self.arcade.is_some()
                || self.menu
                || self.context_menu.is_some()
                || self.board
                || self.form.is_some()
                || self.confirm.is_some()
                || self.focus != Focus::Terminal
                || remote.preview.is_some()
                || self
                    .previews
                    .contains_key(&(self.snapshot.active, self.snapshot.tab))
                || p.epoch != self.snapshot.epoch
                || p.workspace != self.snapshot.active
                || p.session != self.snapshot.tab
                || !self.snapshot.session().is_some_and(|s| s.run == p.run)
                || p.started.elapsed() > Duration::from_secs(110)
        }) {
            let p = remote.pending.take().unwrap();
            let id = p.id;
            drop(p);
            self.remote_result(
                id,
                false,
                "Image paste cancelled: target changed or transfer timed out",
            )?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn remote_packet(&mut self, packet: Packet) -> io::Result<()> {
        if self.remote_update_packet(&packet)? {
            return Ok(());
        }
        if self.remote_tools_packet(&packet)? {
            return Ok(());
        }
        match packet.tag {
            protocol::NOTICE if packet.id == 0 && packet.data == crate::browser_links::PROBE => {
                self.remote.as_mut().unwrap().browser_capable = true;
                Packet::new(protocol::CAPABILITIES, 0, crate::browser_links::CAPABILITY)
                    .write(&mut io::stdout().lock())?;
            }
            crate::browser_links::INPUT if self.remote.as_ref().unwrap().browser_capable => {
                let remote = self.remote.as_mut().unwrap();
                if packet.id == 0 || packet.id <= remote.browser_input {
                    return Err(wire::invalid("stale browser input"));
                }
                remote.browser_input = packet.id;
                self.feed_input(&packet.data);
            }
            protocol::CAPABILITIES if packet.id == 0 && packet.data == protocol::SCREENSHOT_CAP => {
                self.remote.as_mut().unwrap().screenshot_capable = true;
            }
            protocol::SCREENSHOT_RESULT => self.screenshot_result(packet)?,
            protocol::CAPABILITIES
                if packet.id == 0 && packet.data == protocol::AVATAR_DAMAGE_CAP =>
            {
                let r = self.remote.as_mut().unwrap();
                r.avatar_damage = true;
                r.force_redraw = true;
            }
            protocol::CAPABILITIES
                if packet.id == 0 && packet.data.starts_with(protocol::AVATAR_SIZE_CAP) =>
            {
                let cell = crate::avatar::cell(&packet.data[protocol::AVATAR_SIZE_CAP.len()..])?;
                let r = self.remote.as_mut().unwrap();
                r.avatar_capable = true;
                r.avatar_cell = Some(cell);
                crate::diagnostics::record(
                    "project-icons",
                    &format!("cell={}x{} pixels; height-driven widths", cell.0, cell.1),
                );
                r.force_redraw = true;
            }
            protocol::CAPABILITIES if packet.id == 0 && packet.data == protocol::AVATAR_CAP => {
                self.remote.as_mut().unwrap().avatar_capable = true;
            }
            protocol::CAPABILITIES if packet.id == 0 && packet.data == protocol::PREVIEW_CAP => {
                self.remote.as_mut().unwrap().preview_capable = true;
            }
            protocol::PREVIEW_ERROR => self.image_error(packet)?,
            protocol::KEYS if packet.id == 0 => {
                self.feed_input(&packet.data);
            }
            protocol::NOTICE if packet.id == 0 && packet.data.len() <= 1024 => {
                self.notice =
                    wire::passive(std::str::from_utf8(&packet.data).map_err(io::Error::other)?);
            }
            protocol::RESIZE if packet.id == 0 => {
                let r = self.remote.as_mut().unwrap();
                r.size = protocol::dimensions(&packet.data)?;
                r.force_redraw = true;
            }
            protocol::IMAGE => {
                let r = self.remote.as_mut().unwrap();
                if packet.id == 0 || packet.id <= r.last_image {
                    return Err(wire::invalid("duplicate/out-of-order image request"));
                }
                r.last_image = packet.id;
                if self
                    .screensaver
                    .consume(&Key::Paste(Vec::new()), Instant::now())
                {
                    return self.remote_result(
                        packet.id,
                        false,
                        "Screensaver dismissed; paste the image again",
                    );
                }
                if r.pending.is_some() {
                    return self.remote_result(
                        packet.id,
                        false,
                        "An image paste is already in progress",
                    );
                }
                let result = (|| -> io::Result<Pending> {
                    if packet.data.len() != 8 {
                        return Err(wire::invalid("invalid image offer"));
                    }
                    if self.image_view()
                        || self.arcade.is_some()
                        || self.arcade_gate.pending()
                        || self.nav
                        || self.menu
                        || self.context_menu.is_some()
                        || self.board
                        || self.form.is_some()
                        || self.search.is_some()
                        || self.update.is_some()
                        || self.tasks.is_some()
                        || self.remote_tools_busy()
                        || self.confirm.is_some()
                        || self.focus != Focus::Terminal
                        || self
                            .previews
                            .contains_key(&(self.snapshot.active, self.snapshot.tab))
                    {
                        return Err(wire::invalid(
                            "Focus the native chat input before pasting an image",
                        ));
                    }
                    self.flush_input();
                    let t = self
                        .snapshot
                        .session()
                        .ok_or_else(|| wire::invalid("No active chat for image paste"))?;
                    let token = String::from_utf8(wire::request(
                        &self.state,
                        &[
                            "image-ticket",
                            &self.snapshot.epoch,
                            &self.snapshot.active.to_string(),
                            &t.id.to_string(),
                            &t.run,
                        ],
                    )?)
                    .map_err(io::Error::other)?;
                    let mut p = Pending {
                        id: packet.id,
                        state: self.state.clone(),
                        token,
                        epoch: self.snapshot.epoch.clone(),
                        workspace: self.snapshot.active,
                        session: t.id,
                        run: t.run.clone(),
                        upload: None,
                        started: Instant::now(),
                    };
                    p.upload = Some(Upload::new(u64::from_be_bytes(
                        packet.data.as_slice().try_into().unwrap(),
                    ))?);
                    Ok(p)
                })();
                match result {
                    Ok(p) => {
                        self.remote.as_mut().unwrap().pending = Some(p);
                        self.notice = "Receiving clipboard image…".into();
                        Packet::new(protocol::READY, packet.id, Vec::new())
                            .write(&mut io::stdout().lock())?;
                    }
                    Err(e) => return self.remote_result(packet.id, false, &e.to_string()),
                }
            }
            protocol::DATA | protocol::END | protocol::CANCEL => {
                let r = self.remote.as_mut().unwrap();
                // Tail chunks for a rejected/cancelled offer cannot begin another transfer.
                if r.pending.as_ref().is_none_or(|p| p.id != packet.id) {
                    if packet.id > 0 && packet.id <= r.last_image {
                        return Ok(());
                    }
                    return Err(wire::invalid("image data has no matching offer"));
                }
                if packet.tag == protocol::CANCEL {
                    r.pending.take();
                    return self.remote_result(packet.id, false, "Image paste cancelled");
                }
                if packet.tag == protocol::DATA {
                    let result = if packet.data.len() > 8 {
                        let offset = u64::from_be_bytes(packet.data[..8].try_into().unwrap());
                        r.pending
                            .as_mut()
                            .unwrap()
                            .upload
                            .as_mut()
                            .unwrap()
                            .append(offset, &packet.data[8..])
                    } else {
                        Err(wire::invalid("empty image chunk"))
                    };
                    if let Err(e) = result {
                        r.pending.take();
                        return self.remote_result(packet.id, false, &e.to_string());
                    }
                    let received = r
                        .pending
                        .as_ref()
                        .unwrap()
                        .upload
                        .as_ref()
                        .unwrap()
                        .received();
                    self.notice =
                        format!("Receiving image · {} KiB · Esc cancels", received / 1024);
                } else {
                    if !packet.data.is_empty() {
                        return Err(wire::invalid("image end has unexpected data"));
                    }
                    let mut pending = r.pending.take().unwrap();
                    let result = pending.upload.take().unwrap().finish().and_then(|path| {
                        wire::request(
                            &self.state,
                            &[
                                "image-paste",
                                &pending.token,
                                &wire::hex(path.to_string_lossy().as_bytes()),
                            ],
                        )
                        .and_then(|b| String::from_utf8(b).map_err(io::Error::other))
                    });
                    drop(pending);
                    match result {
                        Ok(message) => self.remote_result(packet.id, true, &message)?,
                        Err(error) => self.remote_result(packet.id, false, &error.to_string())?,
                    }
                }
            }
            protocol::CAPABILITIES if packet.id == 0 && packet.data.len() <= 1024 => {}
            _ => return Err(wire::invalid("unsupported remote message or direction")),
        }
        self.remote_cancel_changed().map(|_| ())
    }
    pub(super) fn remote_escape(&mut self) -> bool {
        let pending = self.remote.as_mut().and_then(|r| r.pending.take());
        if let Some(p) = pending {
            let id = p.id;
            drop(p);
            let _ = self.remote_result(id, false, "Image paste cancelled");
            true
        } else {
            false
        }
    }
}
