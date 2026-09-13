//! Local trust boundary for remote tools. A remote packet may request a prompt;
//! only fresh local input can authorize a local file, listener or application.
use crate::{protocol::Packet, remote_services as service};
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

enum Kind {
    Upload,
    Forward,
    Confirm,
    Attach,
}
struct Prompt {
    request: Packet,
    value: Value,
    kind: Kind,
    title: String,
    context: String,
    text: Vec<u8>,
    input: Vec<u8>,
    paste: bool,
    displayed: Instant,
    candidate: Option<crate::drop_path::Candidate>,
    selected: usize,
}
pub enum Decision {
    Authorized(Packet),
    Cancelled(Packet),
    Text(Packet),
}
#[derive(Default)]
pub struct Gate {
    prompt: Option<Prompt>,
    last: u64,
    capture: Option<(u64, crate::drop_path::Candidate, Instant)>,
}
fn denied(request: &Packet, message: &str) -> Packet {
    let data = if request.tag == service::PORTS_REQUEST {
        serde_json::to_vec(&json!({"success":false,"message":message,"ports":[]})).unwrap()
    } else {
        service::result(false, message)
    };
    Packet::new(
        if request.tag == service::PORTS_REQUEST {
            service::PORTS_RESULT
        } else {
            service::RESULT
        },
        request.id,
        data,
    )
}
impl Gate {
    pub fn active(&self) -> bool {
        self.prompt.is_some() || self.capture.is_some()
    }
    pub fn capture(&mut self, id: u64, candidate: crate::drop_path::Candidate) -> bool {
        if self.active() {
            return false;
        }
        self.capture = Some((id, candidate, Instant::now()));
        true
    }
    pub fn reject_capture(&mut self, id: u64) {
        if self
            .capture
            .as_ref()
            .is_some_and(|(offer, _, _)| *offer == id)
        {
            self.capture = None;
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn offer(&mut self, request: Packet) -> io::Result<Option<Decision>> {
        if request.id == 0 || request.id <= self.last {
            return Ok(Some(Decision::Cancelled(denied(
                &request,
                "Stale local action",
            ))));
        }
        self.last = request.id;
        if self.prompt.is_some() {
            return Ok(Some(Decision::Cancelled(denied(
                &request,
                "Another local prompt is open",
            ))));
        }
        let value = service::decode(&request.data)?;
        let op = value
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| service::invalid("missing local action"))?;
        if self.capture.is_some() && !matches!(op, "attach" | "literal") {
            return Ok(Some(Decision::Cancelled(denied(
                &request,
                "A local attachment choice is pending",
            ))));
        }
        let mut candidate = None;
        let (kind, title, context) = match (request.tag, op) {
            (service::REQUEST, "attach" | "literal") => {
                let offer = value["offer"].as_u64();
                if value.get("path").is_some()
                    || !self
                        .capture
                        .as_ref()
                        .is_some_and(|(id, _, _)| Some(*id) == offer)
                {
                    return Ok(Some(Decision::Cancelled(denied(
                        &request,
                        "No matching local dropped path",
                    ))));
                }
                let target = value["target"]
                    .as_str()
                    .filter(|s| service::printable(s))
                    .ok_or_else(|| service::invalid("missing chat attachment target"))?;
                let (_, local, _) = self.capture.take().unwrap();
                if op == "literal" {
                    return Ok(Some(Decision::Text(Packet::new(
                        service::DROP_TEXT,
                        request.id,
                        local.original,
                    ))));
                }
                let path = match &local.path {
                    Ok(path) => path.as_str(),
                    Err(error) => error.as_str(),
                };
                let context = format!(
                    "Chat: {target} · Local: {path} · No Enter is sent; appearance depends on chat/file type"
                );
                candidate = Some(local);
                (
                    Kind::Attach,
                    "Attach local file or paste text?".into(),
                    context,
                )
            }
            (service::REQUEST, "upload") => {
                if value.get("path").is_some() {
                    return Ok(Some(Decision::Cancelled(denied(
                        &request,
                        "Remote requests cannot choose a local upload path",
                    ))));
                }
                let destination = value
                    .get("destination")
                    .and_then(Value::as_str)
                    .filter(|s| service::printable(s))
                    .ok_or_else(|| service::invalid("missing remote destination"))?;
                (
                    Kind::Upload,
                    "Upload local file".into(),
                    format!("Remote destination: {destination}"),
                )
            }
            (service::REQUEST, "download") => {
                let (name, size) = service::file_metadata(&request.data)?;
                let open = value.get("open").and_then(Value::as_bool).unwrap_or(false);
                if open && !crate::transfers::open_allowed(std::path::Path::new(&name)) {
                    return Ok(Some(Decision::Cancelled(denied(
                        &request,
                        "Open locally supports images, HTML and PDF files",
                    ))));
                }
                (
                    Kind::Confirm,
                    if open {
                        "Download and open locally"
                    } else {
                        "Download file"
                    }
                    .into(),
                    format!("Local Downloads/{name} · {size} bytes · existing files kept"),
                )
            }
            (service::PORTS_REQUEST, "forward") => {
                if value.get("local").is_some() || value.get("remote").is_some() {
                    return Ok(Some(Decision::Cancelled(denied(
                        &request,
                        "Enter forward ports locally",
                    ))));
                }
                let workspace = value
                    .get("workspace")
                    .and_then(Value::as_str)
                    .filter(|s| service::printable(s))
                    .ok_or_else(|| service::invalid("missing workspace label"))?;
                (
                    Kind::Forward,
                    "Forward a port".into(),
                    format!("Workspace: {workspace} · loopback only"),
                )
            }
            (service::PORTS_REQUEST, "open") => {
                let port = value
                    .get("local")
                    .and_then(Value::as_u64)
                    .filter(|n| (1..=65535).contains(n))
                    .ok_or_else(|| service::invalid("invalid browser port"))?;
                (
                    Kind::Confirm,
                    "Open forwarded port in browser".into(),
                    format!("http://127.0.0.1:{port}/"),
                )
            }
            // Listing and relinquishing an already-owned listener cannot acquire
            // new local resources. The Ports manager still checks exact ownership.
            (service::PORTS_REQUEST, "list" | "stop") => {
                return Ok(Some(Decision::Authorized(request)));
            }
            _ => return Err(service::invalid("unknown local action")),
        };
        self.prompt = Some(Prompt {
            request,
            value,
            kind,
            title,
            context,
            text: Vec::new(),
            input: Vec::new(),
            paste: false,
            displayed: Instant::now(),
            candidate,
            selected: 2,
        });
        Ok(None)
    }
    pub fn cancel(&mut self, id: u64) -> Option<Decision> {
        if self.prompt.as_ref().is_some_and(|p| p.request.id == id) {
            self.prompt
                .take()
                .map(|p| Decision::Cancelled(denied(&p.request, "Local action cancelled")))
        } else {
            None
        }
    }
    pub fn expired(&mut self) -> Option<Decision> {
        if self
            .capture
            .as_ref()
            .is_some_and(|(_, _, at)| at.elapsed() > Duration::from_secs(service::IDLE_SECONDS))
        {
            self.capture = None;
        }
        let id = self
            .prompt
            .as_ref()
            .filter(|p| p.displayed.elapsed() > Duration::from_secs(service::IDLE_SECONDS))
            .map(|p| p.request.id)?;
        self.cancel(id)
    }
    pub fn draw(&mut self, size: (u16, u16), out: &mut impl Write) -> io::Result<()> {
        let Some(p) = &mut self.prompt else {
            if self.capture.is_some() {
                out.write_all(b"\x1b[?2026h\x1b[0m\x1b[?25l\x1b[2J\x1b[HLOCAL - waiting for exact chat target\x1b[?2026l")?;
                out.flush()?;
            }
            return Ok(());
        };
        let width = usize::from(size.0).saturating_sub(4);
        // Dynamic text is printable and conservatively clipped by UTF-8 bytes;
        // only this fixed local renderer may write while the prompt owns the screen.
        let clip = |s: &str| {
            s.chars()
                .filter(|c| !c.is_control())
                .scan(0, |n, c| {
                    *n += c.len_utf8();
                    (*n <= width).then_some(c)
                })
                .collect::<String>()
        };
        let compact = size.1 < 12;
        let title_row = if compact { 3 } else { 4 };
        let context_row = if compact { 4 } else { 6 };
        write!(
            out,
            "\x1b[?2026h\x1b[0m\x1b[?25l\x1b[2J\x1b[H\x1b[2;3H{}\x1b[{title_row};3H{}",
            clip("LOCAL · flere-connect"),
            clip(&p.title)
        )?;
        let lines = usize::from(size.1)
            .saturating_sub(context_row + 5)
            .clamp(1, 8);
        let mut remaining = p.context.as_str();
        let mut used = 0;
        for index in 0..lines {
            let text = clip(remaining);
            if text.is_empty() {
                break;
            }
            write!(out, "\x1b[{};3H{}", context_row + index, text)?;
            remaining = &remaining[text.len()..];
            used += 1;
            if remaining.is_empty() {
                break;
            }
        }
        if !remaining.is_empty() {
            write!(out, "\x1b[{};{}H…", context_row + used - 1, width + 1)?;
        }
        let hint_row = (context_row + used + 1).min(usize::from(size.1).saturating_sub(2));
        let choices = [
            "[Attach]  Paste as text   Cancel",
            " Attach  [Paste as text]  Cancel",
            " Attach   Paste as text  [Cancel]",
        ];
        let hint = match p.kind {
            Kind::Attach => choices[p.selected],
            Kind::Upload => "Absolute LOCAL path · Enter upload · Esc cancel",
            Kind::Forward => "Remote port [local port] · Enter forward · Esc cancel",
            Kind::Confirm => "Enter to apply · Esc cancel",
        };
        write!(out, "\x1b[{hint_row};3H{}", clip(hint))?;
        if matches!(p.kind, Kind::Attach) {
            write!(
                out,
                "\x1b[{};3H{}",
                (hint_row + 2).min(usize::from(size.1)),
                clip("a Attach · t Text · Esc Cancel · arrows/Tab, Enter")
            )?;
        } else if !matches!(p.kind, Kind::Confirm) {
            write!(
                out,
                "\x1b[{};3H> {}",
                (hint_row + 2).min(usize::from(size.1)),
                clip(&String::from_utf8_lossy(&p.text))
            )?;
        }
        out.write_all(b"\x1b[?2026l")?;
        out.flush()?;
        Ok(())
    }
    pub fn displayed(&mut self) {
        if let Some(p) = &mut self.prompt {
            p.displayed = Instant::now();
        }
    }
    pub fn input(&mut self, bytes: &[u8], received: Instant) -> Option<Decision> {
        let p = self.prompt.as_mut()?;
        if received <= p.displayed {
            return None;
        }
        p.input.extend_from_slice(bytes);
        let mut decision = None;
        while !p.input.is_empty() {
            if p.input.starts_with(b"\x1b[200~") {
                p.input.drain(..6);
                p.paste = true;
                continue;
            }
            if p.input.starts_with(b"\x1b[201~") {
                p.input.drain(..6);
                p.paste = false;
                // A pasted delimiter cannot turn a trailing byte from the same
                // read (or already queued input) into a fresh local choice.
                p.input.clear();
                p.displayed = Instant::now();
                return None;
            }
            if matches!(p.kind, Kind::Attach) && !p.paste {
                let key = [
                    b"\x1b[A".as_slice(),
                    b"\x1b[D",
                    b"\x1bOA",
                    b"\x1bOD",
                    b"\x1b[B",
                    b"\x1b[C",
                    b"\x1bOB",
                    b"\x1bOC",
                ]
                .iter()
                .enumerate()
                .find(|(_, key)| p.input.starts_with(key))
                .map(|(index, key)| (index, key.len()));
                if let Some((index, len)) = key {
                    p.input.drain(..len);
                    p.selected = (p.selected + if index < 4 { 2 } else { 1 }) % 3;
                    continue;
                }
            }
            if p.input[0] == 0x1b {
                if p.input.len() == 1 {
                    break;
                }
                if p.input[1] == b'[' || p.input[1] == b'O' {
                    if let Some(end) = p.input[2..].iter().position(|b| (0x40..=0x7e).contains(b)) {
                        p.input.drain(..end + 3);
                        continue;
                    }
                    if p.input.len() > 128 {
                        p.input.clear();
                    }
                    break;
                }
                decision = Some(false);
                break;
            }
            let b = p.input.remove(0);
            if matches!(p.kind, Kind::Attach) {
                if p.paste {
                    continue;
                }
                match b {
                    b'a' => {
                        p.selected = 0;
                        decision = Some(true);
                        break;
                    }
                    b't' => {
                        p.selected = 1;
                        decision = Some(true);
                        break;
                    }
                    b'c' | 3 => {
                        decision = Some(false);
                        break;
                    }
                    b'\t' | b'j' | b'l' => p.selected = (p.selected + 1) % 3,
                    b'k' | b'h' => p.selected = (p.selected + 2) % 3,
                    b'\r' | b'\n' => {
                        decision = Some(true);
                        break;
                    }
                    _ => {}
                }
                continue;
            }
            match b {
                b'\r' | b'\n' if !p.paste => {
                    decision = Some(true);
                    break;
                }
                3 if !p.paste => {
                    decision = Some(false);
                    break;
                }
                21 if !p.paste => p.text.clear(),
                8 | 127 if !p.paste => while p.text.pop().is_some_and(|b| b & 0xc0 == 0x80) {},
                b if b >= 32
                    && b != 127
                    && !matches!(p.kind, Kind::Confirm)
                    && p.text.len() < 4096 =>
                {
                    p.text.push(b)
                }
                _ => {}
            }
        }
        decision.map(|apply| self.complete(apply))
    }
    pub fn escape(&mut self) -> Option<Decision> {
        self.prompt
            .as_ref()
            .is_some_and(|p| !p.paste && p.input == b"\x1b")
            .then(|| self.complete(false))
    }
    fn complete(&mut self, apply: bool) -> Decision {
        let mut p = self.prompt.take().unwrap();
        if !apply {
            return Decision::Cancelled(denied(&p.request, "Local action cancelled"));
        }
        let text = String::from_utf8(p.text).unwrap_or_default();
        match p.kind {
            Kind::Attach => {
                let candidate = p.candidate.take().unwrap();
                match p.selected {
                    0 => match candidate.path {
                        Ok(path) => p.value["path"] = Value::String(path),
                        Err(error) => return Decision::Cancelled(denied(&p.request, &error)),
                    },
                    1 => {
                        return Decision::Text(Packet::new(
                            service::DROP_TEXT,
                            p.request.id,
                            candidate.original,
                        ));
                    }
                    _ => {
                        return Decision::Cancelled(denied(
                            &p.request,
                            "Local attachment cancelled",
                        ));
                    }
                }
            }
            Kind::Upload => {
                if !service::printable(&text) || !std::path::Path::new(&text).is_absolute() {
                    return Decision::Cancelled(denied(
                        &p.request,
                        "Choose an absolute local upload path",
                    ));
                }
                p.value["path"] = Value::String(text);
            }
            Kind::Forward => {
                let ports = text
                    .split_whitespace()
                    .map(str::parse::<u16>)
                    .collect::<Result<Vec<_>, _>>();
                let Ok(ports) = ports else {
                    return Decision::Cancelled(denied(&p.request, "Ports must be 1–65535"));
                };
                if ports.is_empty() || ports.len() > 2 || ports.contains(&0) {
                    return Decision::Cancelled(denied(
                        &p.request,
                        "Enter remote port and optional local port (1–65535)",
                    ));
                }
                p.value["remote"] = ports[0].into();
                p.value["local"] = ports.get(1).copied().unwrap_or(ports[0]).into();
            }
            Kind::Confirm => {}
        }
        p.request.data = serde_json::to_vec(&p.value).unwrap();
        Decision::Authorized(p.request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_captured_path_needs_a_matching_offer_and_fresh_local_choice() {
        let mut gate = Gate::default();
        // Model a fresh event after the displayed boundary explicitly. Back-to-
        // back Instant::now calls can share a tick on the host running this test.
        let fresh =
            |gate: &Gate| gate.prompt.as_ref().unwrap().displayed + Duration::from_millis(1);
        let request = || json!({"op":"attach","offer":7,"target":"workspace · exact chat"});
        assert!(matches!(
            offer(&mut gate, 1, request()),
            Some(Decision::Cancelled(_))
        ));
        let candidate = || crate::drop_path::Candidate {
            original: b"\x1b[200~/does/not/exist\x1b[201~".to_vec(),
            path: Ok("/does/not/exist".into()),
        };
        assert!(gate.capture(7, candidate()));
        assert!(matches!(
            offer(
                &mut gate,
                2,
                json!({"op":"attach","offer":8,"target":"wrong"})
            ),
            Some(Decision::Cancelled(_))
        ));
        let queued = Instant::now();
        assert!(offer(&mut gate, 3, request()).is_none());
        gate.draw((60, 24), &mut Vec::new()).unwrap();
        gate.displayed();
        assert!(gate.input(b"a\r", queued).is_none());
        for byte in b"\x1b[200~a\r\x1b[201~\x1b[<0;4;4M" {
            assert!(gate.input(&[*byte], fresh(&gate)).is_none());
        }
        assert!(
            gate.input(b"\x1b[200~ignored\x1b[201~a\r", fresh(&gate))
                .is_none()
        );
        assert!(gate.active());
        let Decision::Text(packet) = gate.input(b"t\rignored-tail", fresh(&gate)).unwrap() else {
            panic!("fresh text choice")
        };
        assert_eq!(packet.tag, service::DROP_TEXT);
        assert_eq!(packet.data, candidate().original);
        assert!(gate.capture(7, candidate()));
        offer(&mut gate, 4, request());
        assert!(
            matches!(
                gate.input(b"\r", fresh(&gate)),
                Some(Decision::Cancelled(_))
            ),
            "default is Cancel"
        );
        assert!(gate.capture(7, candidate()));
        offer(&mut gate, 5, request());
        let Decision::Authorized(packet) = gate.input(b"a", fresh(&gate)).unwrap() else {
            panic!("fresh attach choice")
        };
        assert_eq!(
            service::decode(&packet.data).unwrap()["path"],
            "/does/not/exist",
            "prompt never tries to read the chosen file"
        );
    }
    fn offer(g: &mut Gate, id: u64, value: Value) -> Option<Decision> {
        g.offer(Packet::new(
            service::REQUEST,
            id,
            serde_json::to_vec(&value).unwrap(),
        ))
        .unwrap()
    }
    #[test]
    fn hostile_remote_requests_cannot_choose_upload_paths_or_authorize_queued_input() {
        let mut gate = Gate::default();
        assert!(matches!(
            offer(
                &mut gate,
                1,
                json!({"op":"upload","path":"/private/secret","destination":"/remote"})
            ),
            Some(Decision::Cancelled(_))
        ));
        assert!(!gate.active());
        let queued = Instant::now();
        assert!(offer(&mut gate, 2, json!({"op":"upload","destination":"/remote"})).is_none());
        gate.draw((60, 24), &mut Vec::new()).unwrap();
        gate.displayed();
        assert!(gate.input(b"/private/secret\r", queued).is_none());
        let local_path = if cfg!(windows) {
            r"C:\local\chosen.txt"
        } else {
            "/local/chosen.txt"
        };
        let outcome = gate
            .input(
                format!("{local_path}\rignored-tail").as_bytes(),
                Instant::now(),
            )
            .unwrap();
        let Decision::Authorized(packet) = outcome else {
            panic!("fresh local choice should apply")
        };
        assert_eq!(service::decode(&packet.data).unwrap()["path"], local_path);
        assert!(!gate.active());
        assert!(offer(&mut gate, 2, json!({"op":"download","name":"x","size":0})).is_some());
    }
    #[test]
    fn local_paste_mouse_and_terminal_replies_cannot_apply_an_action() {
        let mut gate = Gate::default();
        offer(
            &mut gate,
            1,
            json!({"op":"download","name":"report.pdf","size":123,"open":true}),
        );
        for byte in b"\x1b[200~\r\n\x1b[201~\x1b[<0;4;4M\x1b[<0;4;4m\x1b[?65;1;4c" {
            assert!(gate.input(&[*byte], Instant::now()).is_none());
        }
        assert!(gate.active());
        gate.input(b"\x1b", Instant::now());
        assert!(matches!(gate.escape(), Some(Decision::Cancelled(_))));
        assert!(!gate.active());
    }
}
