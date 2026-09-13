use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
enum TicketKind {
    Image,
    File,
    Literal,
}
#[derive(Clone)]
pub(super) struct Ticket {
    token: String,
    kind: TicketKind,
    epoch: String,
    workspace: u64,
    session: u64,
    run: String,
    revision: u64,
    dimensions: (usize, usize),
    expires: Instant,
}
impl Ticket {
    pub(super) fn matches(&self, token: &str) -> bool {
        self.token == token
    }
}
impl Server {
    fn literal_target(
        &self,
        epoch: &str,
        workspace: u64,
        id: u64,
        run: &str,
        revision: u64,
    ) -> io::Result<()> {
        self.validate_pane_origin(epoch, workspace, id, run, revision, false)?;
        if !self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .any(|t| t.id == id && t.run == run && t.alive && !t.ended)
        {
            return Err(invalid("literal paste requires a live owned terminal"));
        }
        Ok(())
    }
    pub(super) fn attachment_target(
        &self,
        epoch: &str,
        workspace: u64,
        id: u64,
        run: &str,
        revision: u64,
    ) -> io::Result<()> {
        self.validate_pane_origin(epoch, workspace, id, run, revision, false)?;
        let target = self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .find(|t| t.id == id && t.run == run)
            .ok_or_else(|| invalid("attachment chat no longer exists"))?;
        if !target.alive
            || target.ended
            || !target.term.bracketed_paste
            || !target.native.as_ref().is_some_and(|n| n.harness == "codex")
        {
            return Err(invalid(
                "file paste requires a live native Codex tab accepting bracketed paste",
            ));
        }
        Ok(())
    }
    fn take_file_ticket(&mut self, token: &str) -> io::Result<Ticket> {
        self.take_ticket(token, TicketKind::File)
    }
    fn take_ticket(&mut self, token: &str, kind: TicketKind) -> io::Result<Ticket> {
        self.retain_attachment_tickets();
        let index = self
            .attachment_tickets
            .iter()
            .position(|t| t.token == token)
            .ok_or_else(|| invalid("attachment choice expired, consumed or selection changed"))?;
        // Consume before validation or queueing: an uncertain result must never replay.
        let ticket = self.attachment_tickets.remove(index);
        if ticket.kind != kind || ticket.expires <= Instant::now() {
            return Err(invalid("invalid or expired paste choice"));
        }
        let validate = if kind == TicketKind::Literal {
            Self::literal_target
        } else {
            Self::attachment_target
        };
        validate(
            self,
            &ticket.epoch,
            ticket.workspace,
            ticket.session,
            &ticket.run,
            ticket.revision,
        )?;
        Ok(ticket)
    }
    pub(super) fn consume_attachment_ticket(
        &mut self,
        token: &str,
        epoch: &str,
        workspace: u64,
        id: u64,
        run: &str,
        revision: u64,
    ) -> io::Result<()> {
        let ticket = self.take_file_ticket(token)?;
        if ticket.epoch != epoch
            || ticket.workspace != workspace
            || ticket.session != id
            || ticket.run != run
            || ticket.revision != revision
        {
            return Err(invalid(
                "attachment choice belongs to a different chat origin",
            ));
        }
        Ok(())
    }
    pub(super) fn paste_attachment(
        &mut self,
        epoch: &str,
        workspace: u64,
        id: u64,
        run: &str,
        revision: u64,
        bytes: &[u8],
    ) -> io::Result<()> {
        self.attachment_target(epoch, workspace, id, run, revision)?;
        if self.session(id, run)?.input.len() + bytes.len() > 131072 {
            return Err(invalid(
                "native input queue full; attachment was not pasted",
            ));
        }
        self.audit(
            "attachment-paste",
            id,
            &format!("run={run};bytes={}", bytes.len()),
        )?;
        self.session(id, run)?.input.extend(bytes);
        Ok(())
    }
    pub(super) fn attachment_command(&mut self, p: &[&str]) -> io::Result<Vec<u8>> {
        match p {
            [
                "attachment-check" | "literal-ticket",
                epoch,
                workspace,
                id,
                run,
                revision,
            ] => {
                let workspace = workspace.parse().map_err(io::Error::other)?;
                let id = id.parse().map_err(io::Error::other)?;
                let revision = revision.parse().map_err(io::Error::other)?;
                let kind = if p[0] == "literal-ticket" {
                    TicketKind::Literal
                } else {
                    TicketKind::File
                };
                if kind == TicketKind::Literal {
                    self.literal_target(epoch, workspace, id, run, revision)?;
                } else {
                    self.attachment_target(epoch, workspace, id, run, revision)?;
                }
                self.retain_attachment_tickets();
                if self.attachment_tickets.len() >= 8 {
                    return Err(invalid("too many pending attachment choices"));
                }
                let token = os::nonce()?;
                let target = self.session(id, run)?;
                let dimensions = (target.term.grid.cols, target.term.grid.rows);
                self.attachment_tickets.push(Ticket {
                    token: token.clone(),
                    kind,
                    epoch: (*epoch).into(),
                    workspace,
                    session: id,
                    run: (*run).into(),
                    revision,
                    dimensions,
                    expires: Instant::now() + Duration::from_secs(120),
                });
                Ok(token.into_bytes())
            }
            ["attachment-text" | "literal-paste", token, data] => {
                let literal = p[0] == "literal-paste";
                let ticket = self.take_ticket(
                    token,
                    if literal {
                        TicketKind::Literal
                    } else {
                        TicketKind::File
                    },
                )?;
                if data.len() > 8204 * 2 {
                    return Err(invalid("attachment text exceeds 8 KiB"));
                }
                let bytes = wire::unhex(data)?;
                let body = bytes
                    .strip_prefix(b"\x1b[200~")
                    .and_then(|b| b.strip_suffix(b"\x1b[201~"))
                    .ok_or_else(|| invalid("attachment text must be a single bracketed paste"))?;
                let text = std::str::from_utf8(body).map_err(io::Error::other)?;
                if text.is_empty() || text.chars().any(char::is_control) {
                    return Err(invalid(
                        "attachment text must be printable UTF-8 without nested escapes",
                    ));
                }
                if literal {
                    let target = self.session(ticket.session, &ticket.run)?;
                    let paste = if target.term.bracketed_paste {
                        bytes.as_slice()
                    } else {
                        body
                    };
                    if target.input.len() + paste.len() > 131072 {
                        return Err(invalid("terminal input queue full; text was not pasted"));
                    }
                    self.audit(
                        "literal-paste",
                        ticket.session,
                        &format!("run={};bytes={}", ticket.run, paste.len()),
                    )?;
                    self.session(ticket.session, &ticket.run)?
                        .input
                        .extend(paste);
                    return Ok(b"Literal text paste queued without Enter".to_vec());
                }
                self.paste_attachment(
                    &ticket.epoch,
                    ticket.workspace,
                    ticket.session,
                    &ticket.run,
                    ticket.revision,
                    &bytes,
                )?;
                Ok(b"Text paste queued for the chat draft without Enter".to_vec())
            }
            ["attachment-cancel", token] => {
                self.attachment_tickets
                    .retain(|t| t.kind == TicketKind::Image || t.token != *token);
                Ok(b"Attachment choice cancelled".to_vec())
            }
            _ => Err(invalid("invalid attachment command")),
        }
    }
    pub(super) fn retain_attachment_tickets(&mut self) {
        let active = self.active;
        let selected = self
            .workspaces
            .iter()
            .find(|w| w.id == active)
            .map_or(0, |w| w.selected);
        let revision = self.pane_revision(active);
        let dimensions = self
            .workspaces
            .iter()
            .find(|w| w.id == active)
            .and_then(|w| w.tabs.iter().find(|t| t.id == selected))
            .map(|t| (t.term.grid.cols, t.term.grid.rows));
        self.attachment_tickets.retain(|t| {
            t.workspace == active
                && t.session == selected
                && t.expires > Instant::now()
                && t.revision == revision
                && Some(t.dimensions) == dimensions
        });
    }
    fn image_target(&mut self, epoch: &str, workspace: u64, id: u64, run: &str) -> io::Result<()> {
        if epoch != self.epoch
            || self.active != workspace
            || !self
                .workspaces
                .iter()
                .any(|w| w.id == workspace && w.selected == id)
        {
            return Err(invalid(
                "image paste target changed; paste again in the intended chat",
            ));
        }
        let s = self.session(id, run)?;
        if !s.alive
            || s.ended
            || !s.term.bracketed_paste
            || !s.native.as_ref().is_some_and(|n| n.harness == "codex")
        {
            return Err(invalid(
                "image paste requires a live native Codex tab accepting bracketed paste",
            ));
        }
        Ok(())
    }
    pub(super) fn image_ticket(
        &mut self,
        epoch: &str,
        workspace: u64,
        id: u64,
        run: &str,
    ) -> io::Result<Vec<u8>> {
        self.image_target(epoch, workspace, id, run)?;
        self.retain_attachment_tickets();
        if self.attachment_tickets.len() >= 8 {
            return Err(invalid("too many pending image pastes"));
        }
        let token = os::nonce()?;
        let revision = self.pane_revision(workspace);
        let target = self.session(id, run)?;
        let dimensions = (target.term.grid.cols, target.term.grid.rows);
        self.attachment_tickets.push(Ticket {
            token: token.clone(),
            kind: TicketKind::Image,
            epoch: epoch.into(),
            workspace,
            session: id,
            run: run.into(),
            revision,
            dimensions,
            expires: Instant::now() + Duration::from_secs(120),
        });
        Ok(token.into_bytes())
    }
    pub(super) fn image_paste(&mut self, token: &str, path: &Path) -> io::Result<Vec<u8>> {
        let index = self
            .attachment_tickets
            .iter()
            .position(|t| t.kind == TicketKind::Image && t.token == token)
            .ok_or_else(|| invalid("image paste ticket expired, consumed or selection changed"))?;
        // Consume before validation: callers must not replay uncertain paste outcomes.
        let t = self.attachment_tickets.remove(index);
        if t.expires <= Instant::now() {
            return Err(invalid("image paste ticket expired"));
        }
        self.image_target(&t.epoch, t.workspace, t.session, &t.run)?;
        crate::attachments::validate(path)?;
        let value = path
            .to_str()
            .ok_or_else(|| invalid("image path must be UTF-8"))?;
        if value.chars().any(char::is_control) {
            return Err(invalid("image path contains controls"));
        }
        let bytes = [
            b"\x1b[200~".as_slice(),
            value.as_bytes(),
            b"\x1b[201~".as_slice(),
        ]
        .concat();
        if self.session(t.session, &t.run)?.input.len() + bytes.len() > 131072 {
            return Err(invalid("native input queue full; image was not pasted"));
        }
        self.audit(
            "image-paste",
            t.session,
            &format!("run={};bytes={}", t.run, bytes.len()),
        )?;
        self.session(t.session, &t.run)?.input.extend(bytes);
        Ok(b"Image paste queued; check the attachment in the chat".to_vec())
    }
}
