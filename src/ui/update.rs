//! The installer runs outside the frontend. The watch/PTY loop keeps draining.
use super::*;
use crate::install::{InstallReceipt, ManagerUpgrade, PackageSource, Store};
use std::{process::Command, sync::mpsc};

pub(super) struct Update {
    source: Vec<u8>,
    status: String,
    job: Option<mpsc::Receiver<io::Result<InstallReceipt>>>,
    owner: Owner,
}
enum Owner {
    Checking(mpsc::Receiver<(Option<ManagerUpgrade>, String)>),
    PackageManager(ManagerUpgrade),
    Local,
    Unavailable,
}
#[derive(Debug, PartialEq, Eq)]
enum UpdateInput {
    None,
    Close,
    Apply,
}
impl Update {
    fn input(&mut self, key: &Key) -> UpdateInput {
        if self.job.is_some() {
            return UpdateInput::None;
        }
        if matches!(key, Key::Bytes(b) if b == b"\x1b" || b == b"\0") {
            return UpdateInput::Close;
        }
        // The source field and Apply action become available only after this
        // modal's own probe completes. Package-manager commands are display-only.
        if !matches!(self.owner, Owner::Local) {
            return UpdateInput::None;
        }
        match key {
            Key::Bytes(b) if b == b"\r" || b == b"\n" => return UpdateInput::Apply,
            Key::Bytes(b) | Key::Paste(b) => {
                if matches!(key, Key::Bytes(_)) && (b == b"\x7f" || b == b"\x08") {
                    self.source.pop();
                    while !self.source.is_empty() && std::str::from_utf8(&self.source).is_err() {
                        self.source.pop();
                    }
                } else if b == b"\x15" {
                    self.source.clear();
                } else if matches!(key, Key::Paste(_)) || !b.starts_with(b"\x1b") {
                    let bytes = b.strip_prefix(b"\x1b[200~").unwrap_or(b);
                    let bytes = bytes.strip_suffix(b"\x1b[201~").unwrap_or(bytes);
                    self.source.extend(
                        bytes
                            .iter()
                            .filter(|b| **b >= 32 && **b != 127)
                            .take(4096usize.saturating_sub(self.source.len())),
                    );
                }
            }
            _ => {}
        }
        UpdateInput::None
    }
    fn poll_owner(&mut self) -> bool {
        let Owner::Checking(receiver) = &self.owner else {
            return false;
        };
        match receiver.try_recv() {
            Ok((Some(manager), _)) => {
                self.status = manager.detail.into();
                self.owner = Owner::PackageManager(manager);
            }
            Ok((None, source)) => {
                self.source = source.into_bytes();
                self.status = "Enter applies immediately and preserves sessions".into();
                self.owner = Owner::Local;
            }
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.owner = Owner::Unavailable;
                self.status = "Ownership check stopped. Close and reopen Update to retry.".into();
            }
        }
        true
    }
}
pub(super) struct RemoteRpc {
    id: u64,
    job: Option<mpsc::Receiver<io::Result<Vec<u8>>>>,
    output: Vec<u8>,
    offset: usize,
}
impl Ui {
    pub(super) fn tick_update_rpc(&mut self) -> io::Result<bool> {
        use crate::{
            remote_protocol::{self as protocol, Packet},
            remote_update as update,
        };
        let Some(remote) = &mut self.remote else {
            return Ok(false);
        };
        let Some(rpc) = &mut remote.update_rpc else {
            return Ok(false);
        };
        if let Some(job) = &rpc.job {
            let result = match job.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return Ok(false),
                Err(mpsc::TryRecvError::Disconnected) => Err(io::Error::other(
                    "update helper stopped; inspect update status",
                )),
            };
            rpc.job = None;
            let (success, bytes) = match result {
                Ok(bytes) => (true, bytes),
                Err(e) => (false, wire::passive(&e.to_string()).into_bytes()),
            };
            if bytes.len() > update::RESPONSE_LIMIT {
                return Err(wire::invalid("update response exceeds bound"));
            }
            rpc.output = bytes;
            let mut begin = vec![u8::from(success)];
            begin.extend((rpc.output.len() as u64).to_be_bytes());
            Packet::new(update::BEGIN, rpc.id, begin).write(&mut io::stdout().lock())?;
        }
        let end = (rpc.offset + protocol::CHUNK).min(rpc.output.len());
        if rpc.offset < end {
            Packet::new(update::DATA, rpc.id, rpc.output[rpc.offset..end].to_vec())
                .write(&mut io::stdout().lock())?;
            rpc.offset = end;
        }
        if end == rpc.output.len() {
            Packet::new(update::END, rpc.id, Vec::new()).write(&mut io::stdout().lock())?;
            remote.update_rpc = None;
        }
        Ok(true)
    }
    pub(super) fn remote_update_packet(
        &mut self,
        packet: &crate::remote_protocol::Packet,
    ) -> io::Result<bool> {
        use crate::{remote_protocol as protocol, remote_update as update};
        if packet.tag == protocol::CAPABILITIES
            && packet.id == 0
            && packet.data == update::CAPABILITY
        {
            // Legacy capability alone cannot authorize replacement.
            return Ok(true);
        }
        if packet.tag == protocol::NOTICE
            && packet.id == 0
            && packet.data == update::OWNERSHIP_PROBE
        {
            self.remote.as_mut().unwrap().update_capable = true;
            protocol::Packet::new(protocol::CAPABILITIES, 0, update::OWNERSHIP_CAPABILITY)
                .write(&mut io::stdout().lock())?;
            return Ok(true);
        }
        if !matches!(packet.tag, update::ACTIVATE | update::RESULT | update::CALL) {
            return Ok(false);
        }
        if !self.remote.as_ref().is_some_and(|r| r.update_capable) || packet.data.len() > 8192 {
            return Err(wire::invalid("invalid coordinated update packet"));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&packet.data).map_err(io::Error::other)?;
        if packet.tag == update::CALL {
            if packet.id == 0 || self.remote.as_ref().is_some_and(|r| r.update_rpc.is_some()) {
                return Err(wire::invalid("update RPC is already in progress"));
            }
            let args: Vec<String> = serde_json::from_value(value).map_err(io::Error::other)?;
            let state = self.state.clone();
            let executable = os::executable_path()?;
            let frontend = std::process::id().to_string();
            let request = serde_json::to_string(&args).map_err(io::Error::other)?;
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let result = crate::install::run(
                    Command::new(executable)
                        .arg("--state")
                        .arg(state)
                        .args(["update-coordinated-v1", &frontend, &request])
                        .stdin(std::process::Stdio::null()),
                    update::RESPONSE_LIMIT,
                    Duration::from_secs(1800),
                );
                let _ = sender.send(result);
            });
            self.remote.as_mut().unwrap().update_rpc = Some(RemoteRpc {
                id: packet.id,
                job: Some(receiver),
                output: Vec::new(),
                offset: 0,
            });
            return Ok(true);
        }
        if packet.tag == update::RESULT {
            self.notice = wire::passive(value["message"].as_str().unwrap_or("Update finished"));
            return Ok(true);
        }
        if self.update.is_some() {
            return Err(wire::invalid("an update is already in progress"));
        }
        let attempt = value["attempt"]
            .as_str()
            .filter(|s| update::token(s))
            .ok_or_else(|| wire::invalid("invalid remote update attempt"))?
            .to_owned();
        let state = self.state.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let receipt = Store::for_user("flere")?
                    .status()?
                    .ok_or_else(|| wire::invalid("update is not managed"))?;
                let activation = receipt
                    .activation
                    .as_ref()
                    .ok_or_else(|| wire::invalid("update has no activation receipt"))?;
                let after = activation
                    .after
                    .as_ref()
                    .ok_or_else(|| wire::invalid("update has no verified runtime"))?;
                let runtime = crate::install::runtime_identity(&state)?;
                if receipt.attempt != attempt
                    || activation.supervisor != "applied"
                    || after.epoch != runtime.epoch
                    || after.pid != runtime.pid
                    || after.build != runtime.build
                    || receipt.current.manifest.build != runtime.build
                {
                    return Err(wire::invalid(
                        "update attempt does not match the selected supervisor",
                    ));
                }
                Ok(receipt)
            })();
            let _ = sender.send(result);
        });
        self.update = Some(Update {
            source: Vec::new(),
            status: "Applying updated remote frontend…".into(),
            job: Some(receiver),
            owner: Owner::Local,
        });
        Ok(true)
    }
    pub(super) fn tick_update_ack(&mut self) -> bool {
        let Some((attempt, started)) = &self.update_ack else {
            return false;
        };
        let registered = wire::request(&self.state, &["frontends"])
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| {
                v["tracked"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v["pid"] == std::process::id()))
            });
        if !registered && started.elapsed() < Duration::from_secs(3) {
            return false;
        }
        let result =
            crate::install::command(Some(&self.state), &["update-ack".into(), attempt.clone()]);
        match result {
            Ok(_) => {
                if self.remote.is_some() {
                    let data = serde_json::to_vec(&serde_json::json!({"attempt":attempt,"build_id":crate::build_info::BUILD_ID})).unwrap();
                    let _ = crate::remote_protocol::Packet::new(crate::remote_update::ACK, 0, data)
                        .write(&mut io::stdout().lock());
                }
                self.update_ack = None;
                self.notice = "Updated Flere; sessions preserved and this frontend applied".into();
                true
            }
            Err(e) => {
                self.notice = format!("Update remains partial: {}", wire::passive(&e.to_string()));
                self.update_ack = None;
                true
            }
        }
    }
    pub(super) fn open_update(&mut self) {
        if let Some(remote) = &mut self.remote {
            if !remote.update_capable {
                self.notice =
                    "Upgrade the local companion manually first; coordinated updates require installation ownership reporting".into();
                return;
            }
            remote.update_counter = remote.update_counter.saturating_add(1);
            let result = crate::remote_protocol::Packet::new(
                crate::remote_update::REQUEST,
                remote.update_counter,
                Vec::new(),
            )
            .write(&mut io::stdout().lock());
            self.menu = false;
            self.notice = result
                .map(|_| "Update sources are selected in your local companion".into())
                .unwrap_or_else(|e| wire::passive(&e.to_string()));
            return;
        }
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let managed = Store::for_user("flere")
                .and_then(|store| store.status())
                .ok()
                .flatten();
            let owner = crate::install::current_manager_upgrade(managed.as_ref());
            let source = managed
                .and_then(|receipt| match receipt.source {
                    PackageSource::Public { manifest_url } => Some(manifest_url),
                    _ => None,
                })
                .unwrap_or_default();
            // Closing the modal drops this receiver. A later modal creates a
            // different channel and cannot consume a stale ownership result.
            let _ = sender.send((owner, source));
        });
        self.update = Some(Update {
            source: Vec::new(),
            status: "Checking this executable's installation owner…".into(),
            job: None,
            owner: Owner::Checking(receiver),
        });
        self.menu = false;
    }
    pub(super) fn update_key(&mut self, key: &Key) -> bool {
        let Some(mut update) = self.update.take() else {
            return false;
        };
        match update.input(key) {
            UpdateInput::Close => return true,
            UpdateInput::Apply => {
                let state = self.state.clone();
                let source = String::from_utf8_lossy(&update.source).trim().to_owned();
                let (sender, receiver) = mpsc::channel();
                update.job = Some(receiver);
                update.status = "Updating… terminal sessions remain alive".into();
                std::thread::spawn(move || {
                    let _ = sender.send(perform(&state, &source));
                });
            }
            UpdateInput::None => {}
        }
        self.update = Some(update);
        true
    }
    pub(super) fn tick_update(&mut self) -> bool {
        let Some(update) = &mut self.update else {
            return false;
        };
        if update.poll_owner() {
            return true;
        }
        let Some(job) = &update.job else {
            return false;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Err(io::Error::other(
                "Update helper stopped; inspect Install status before retrying",
            )),
        };
        update.job = None;
        match result {
            Ok(receipt)
                if receipt
                    .activation
                    .as_ref()
                    .is_some_and(|a| a.supervisor == "applied") =>
            {
                self.refresh_to = Some(receipt.current.executable);
                self.refresh_attempt = Some(receipt.attempt);
                self.refresh = true;
                update.status = "Installed; applying the new frontend…".into();
            }
            Ok(receipt) => {
                update.status = receipt
                    .activation
                    .map(|a| a.detail)
                    .unwrap_or_else(|| receipt.status)
            }
            Err(e) => update.status = wire::passive(&e.to_string()),
        }
        true
    }
    pub(super) fn draw_update(&self, c: &mut Canvas) {
        let Some(update) = &self.update else {
            return;
        };
        let w = self.layout.width.saturating_sub(2).clamp(8, 96);
        let h = self.layout.height.saturating_sub(2).clamp(6, 12);
        let (x, y) = ((self.layout.width - w) / 2, (self.layout.height - h) / 2);
        c.fill(x, y, w, h, style(TEXT, PANEL, false));
        c.border(x, y, w, h, BORDER);
        c.text(
            x + 2,
            y + 1,
            w - 4,
            "Update Flere",
            style(CYAN, PANEL, true),
        );
        if !matches!(update.owner, Owner::Local) {
            let heading = owner_heading(&update.owner);
            c.text(x + 2, y + 2, w - 4, &heading, style(MUTED, PANEL, false));
            if let Owner::PackageManager(manager) = &update.owner
                && let Some(command) = &manager.command
            {
                for (row, text) in chrome::wrap(command, w - 4, h.saturating_sub(7))
                    .iter()
                    .enumerate()
                {
                    c.text(x + 2, y + 3 + row, w - 4, text, style(TEXT, PANEL, true));
                }
            }
            for (row, text) in chrome::wrap(&update.status, w - 4, 2).iter().enumerate() {
                c.text(
                    x + 2,
                    y + h - 4 + row,
                    w - 4,
                    text,
                    style(GOLD, PANEL, false),
                );
            }
            c.text(
                x + 2,
                y + h - 2,
                w - 4,
                "Esc close · commands are not run by Flere",
                style(MUTED, PANEL, false),
            );
            return;
        }
        c.text(
            x + 2,
            y + 2,
            w - 4,
            "Package directory / HTTPS manifest; empty = registered local source",
            style(MUTED, PANEL, false),
        );
        c.text(
            x + 2,
            y + 3,
            w - 4,
            &chrome::tail(&String::from_utf8_lossy(&update.source), w - 4),
            style(TEXT, ACTIVE_BG, true),
        );
        if h > 8 {
            c.text(
                x + 2,
                y + 5,
                w - 4,
                "Updates the per-user command; keeps the previous package for recovery.",
                style(MUTED, PANEL, false),
            );
        }
        c.text(
            x + 2,
            y + h - 3,
            w - 4,
            &update.status,
            style(GOLD, PANEL, false),
        );
        c.text(
            x + 2,
            y + h - 2,
            w - 4,
            if update.job.is_some() {
                "Update is running; no input is sent to terminals"
            } else {
                "Enter update · Ctrl+U clear · Esc cancel"
            },
            style(MUTED, PANEL, false),
        );
    }
}
fn owner_heading(owner: &Owner) -> String {
    match owner {
        Owner::PackageManager(manager) if manager.verified => {
            format!("Installed with {}", manager.manager)
        }
        Owner::PackageManager(_) => "Installation ownership needs review".into(),
        Owner::Checking(_) => "Checking installation owner…".into(),
        Owner::Unavailable => "Installation owner check unavailable".into(),
        Owner::Local => unreachable!(),
    }
}
fn perform(state: &Path, source: &str) -> io::Result<InstallReceipt> {
    // Recheck immediately before launching the existing installer/developer
    // helper, in case ownership changed while its modal was open.
    if let Some(manager) = crate::install::update_manager_guard() {
        let guidance = manager.command.map_or_else(
            || manager.detail.to_owned(),
            |command| format!("Installed with {}. Run: {command}", manager.manager),
        );
        return Err(wire::invalid(&guidance));
    }
    let mut command;
    if source.is_empty() {
        let checkout = Store::for_user("flere")?.local_source()?.ok_or_else(|| {
            wire::invalid("Choose a package, or register this checkout with flere dev-source PATH")
        })?;
        command = Command::new(checkout.join("scripts/dev"));
        command
            .args(["update", "--frontend", "--state"])
            .arg(state)
            .arg("--adopt");
    } else {
        command = Command::new(os::executable_path()?);
        command
            .arg("--state")
            .arg(state)
            .args(["update", "--frontend", "--adopt"]);
        if source.starts_with("https://") {
            command.args(["--from-url", source]);
        } else {
            command.arg(source);
        }
    }
    command.stdin(std::process::Stdio::null());
    let bytes = crate::install::run(
        &mut command,
        crate::install::MAX_RECEIPT as usize,
        Duration::from_secs(1800),
    )?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checking() -> (Update, mpsc::Sender<(Option<ManagerUpgrade>, String)>) {
        let (sender, receiver) = mpsc::channel();
        (
            Update {
                source: Vec::new(),
                status: "Checking".into(),
                job: None,
                owner: Owner::Checking(receiver),
            },
            sender,
        )
    }
    fn manager() -> ManagerUpgrade {
        ManagerUpgrade {
            manager: "Homebrew",
            verified: true,
            command: Some("brew upgrade robert-cronin/flere/flere".into()),
            detail: "Run this in a shell",
        }
    }

    #[test]
    fn pending_probe_and_package_manager_panel_never_accept_apply_or_source_input() {
        for owner in [
            manager(),
            ManagerUpgrade {
                manager: "Nix",
                verified: true,
                command: None,
                detail: "Update the owning Nix configuration or profile, then reopen Flere.",
            },
            ManagerUpgrade {
                manager: "Cargo",
                verified: false,
                command: None,
                detail: "Review the ambiguous Cargo installation and reopen Flere",
            },
        ] {
            let expected_heading = if owner.verified {
                format!("Installed with {}", owner.manager)
            } else {
                "Installation ownership needs review".into()
            };
            let (mut update, sender) = checking();
            for key in [
                Key::Bytes(b"\r".to_vec()),
                Key::Paste(b"https://example.invalid/manifest.json".to_vec()),
            ] {
                assert_eq!(update.input(&key), UpdateInput::None);
            }
            assert!(update.source.is_empty());
            sender
                .send((
                    Some(owner),
                    "https://old-store.invalid/manifest.json".into(),
                ))
                .unwrap();
            assert!(update.poll_owner());
            assert!(matches!(update.owner, Owner::PackageManager(_)));
            assert_eq!(owner_heading(&update.owner), expected_heading);
            assert_eq!(update.input(&Key::Bytes(b"\r".to_vec())), UpdateInput::None);
            assert_eq!(
                update.input(&Key::Paste(b"replacement".to_vec())),
                UpdateInput::None
            );
            assert!(update.source.is_empty());
            assert_eq!(
                update.input(&Key::Bytes(b"\x1b".to_vec())),
                UpdateInput::Close
            );
        }
    }

    #[test]
    fn cancelled_modal_cannot_deliver_ownership_to_a_reopened_modal() {
        let (mut previous, previous_sender) = checking();
        assert_eq!(
            previous.input(&Key::Bytes(b"\x1b".to_vec())),
            UpdateInput::Close
        );
        drop(previous);
        let (mut current, current_sender) = checking();
        assert!(
            previous_sender
                .send((Some(manager()), String::new()))
                .is_err()
        );
        assert!(!current.poll_owner());
        assert_eq!(
            current.input(&Key::Bytes(b"\r".to_vec())),
            UpdateInput::None
        );
        current_sender
            .send((None, "https://selected.invalid/manifest.json".into()))
            .unwrap();
        assert!(current.poll_owner());
        assert_eq!(current.source, b"https://selected.invalid/manifest.json");
        assert_eq!(
            current.input(&Key::Bytes(b"\r".to_vec())),
            UpdateInput::Apply
        );
    }

    #[test]
    fn interrupted_probe_stays_closed_to_apply_until_a_new_probe() {
        let (mut update, sender) = checking();
        drop(sender);
        assert!(update.poll_owner());
        assert!(matches!(update.owner, Owner::Unavailable));
        assert_eq!(update.input(&Key::Bytes(b"\r".to_vec())), UpdateInput::None);
        assert_eq!(
            update.input(&Key::Bytes(b"\x1b".to_vec())),
            UpdateInput::Close
        );
    }
}
