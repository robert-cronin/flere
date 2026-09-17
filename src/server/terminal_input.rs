//! Explicit agent input uses the same PTY queue as frontend keys. Tickets are
//! transient, single-use observations, not permission grants or replay records.
use super::*;
use serde::Deserialize;
use serde_json::{Value, json};

const LIMIT: usize = 65536;
const LIFETIME: Duration = Duration::from_secs(120);

pub(super) struct Ticket {
    token: String,
    actor: (u64, String),
    target: Target,
    created: Instant,
}

#[derive(Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Target {
    expected_epoch: String,
    workspace: u64,
    session: u64,
    run: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {
    target: Target,
    #[serde(default = "default_lines")]
    lines: usize,
}
fn default_lines() -> usize {
    80
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    target: Target,
    ticket: String,
    actions: Vec<Action>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    Text(String),
    Paste(String),
    Key(String),
    Hex(String),
}

fn key(name: &str, app_cursor: bool) -> io::Result<Vec<u8>> {
    if name.len() == 1 && name.is_ascii() {
        return Ok(name.as_bytes().to_vec());
    }
    let original = name;
    let name = name.to_ascii_lowercase();
    if let Some(rest) = name.strip_prefix("alt+") {
        if rest.starts_with("alt+") {
            return Err(invalid("repeated Alt modifier"));
        }
        let mut bytes = vec![27];
        bytes.extend(key(&original[4..], app_cursor)?);
        return Ok(bytes);
    }
    if let Some(rest) = name.strip_prefix("ctrl+") {
        let byte = match rest {
            "space" | "@" => 0,
            "?" => 127,
            _ if rest.len() == 1 && rest.as_bytes()[0].is_ascii_lowercase() => {
                rest.as_bytes()[0] - b'a' + 1
            }
            "[" => 27,
            "\\" => 28,
            "]" => 29,
            "^" => 30,
            "_" => 31,
            _ => {
                return Err(invalid(
                    "unknown Ctrl key; use hex for other terminal encodings",
                ));
            }
        };
        return Ok(vec![byte]);
    }
    let sequence = match name.as_str() {
        "enter" => "\r",
        "tab" => "\t",
        "shift+tab" => "\x1b[Z",
        "escape" | "esc" => "\x1b",
        "space" => " ",
        "backspace" => "\x7f",
        "up" if app_cursor => "\x1bOA",
        "down" if app_cursor => "\x1bOB",
        "right" if app_cursor => "\x1bOC",
        "left" if app_cursor => "\x1bOD",
        "home" if app_cursor => "\x1bOH",
        "end" if app_cursor => "\x1bOF",
        "up" => "\x1b[A",
        "down" => "\x1b[B",
        "right" => "\x1b[C",
        "left" => "\x1b[D",
        "home" => "\x1b[H",
        "end" => "\x1b[F",
        "insert" => "\x1b[2~",
        "delete" => "\x1b[3~",
        "pageup" => "\x1b[5~",
        "pagedown" => "\x1b[6~",
        "f1" => "\x1bOP",
        "f2" => "\x1bOQ",
        "f3" => "\x1bOR",
        "f4" => "\x1bOS",
        "f5" => "\x1b[15~",
        "f6" => "\x1b[17~",
        "f7" => "\x1b[18~",
        "f8" => "\x1b[19~",
        "f9" => "\x1b[20~",
        "f10" => "\x1b[21~",
        "f11" => "\x1b[23~",
        "f12" => "\x1b[24~",
        _ => {
            return Err(invalid(
                "unknown key; use text or hex for other terminal encodings",
            ));
        }
    };
    Ok(sequence.as_bytes().to_vec())
}

fn encode(actions: Vec<Action>, term: &Terminal) -> io::Result<Vec<u8>> {
    if actions.is_empty() || actions.len() > 256 {
        return Err(invalid("provide 1 to 256 terminal actions"));
    }
    let mut bytes = Vec::new();
    for action in actions {
        match action {
            Action::Text(text) => bytes.extend(text.as_bytes()),
            Action::Hex(hex) => bytes.extend(wire::unhex(&hex)?),
            Action::Key(name) => bytes.extend(key(&name, term.app_cursor)?),
            Action::Paste(text) => {
                if text.chars().any(|c| {
                    c.is_control() && !(term.bracketed_paste && matches!(c, '\n' | '\r' | '\t'))
                }) {
                    return Err(invalid(
                        "literal paste contains controls or requires bracketed paste; use explicit text/hex for raw input",
                    ));
                }
                if term.bracketed_paste {
                    bytes.extend(b"\x1b[200~");
                }
                bytes.extend(text.as_bytes());
                if term.bracketed_paste {
                    bytes.extend(b"\x1b[201~");
                }
            }
        }
        if bytes.len() > LIMIT {
            return Err(invalid("terminal input exceeds 64 KiB"));
        }
    }
    if bytes.is_empty() {
        return Err(invalid("terminal input is empty"));
    }
    Ok(bytes)
}

impl Server {
    fn terminal_target(&mut self, target: &Target) -> io::Result<&mut Session> {
        if target.expected_epoch != self.epoch {
            return Err(invalid("stale terminal supervisor epoch"));
        }
        self.workspaces
            .iter_mut()
            .find(|w| w.id == target.workspace)
            .and_then(|w| {
                w.tabs
                    .iter_mut()
                    .find(|s| s.id == target.session && s.run == target.run)
            })
            .ok_or_else(|| invalid("stale terminal workspace/session/run"))
    }

    pub(super) fn terminal_operation(
        &mut self,
        actor: (u64, &str),
        op: &str,
        args: &Value,
    ) -> io::Result<Value> {
        self.terminal_tickets
            .retain(|t| t.created.elapsed() < LIFETIME);
        if op == "inspect_terminal" {
            let request: Inspect =
                serde_json::from_value(args.clone()).map_err(io::Error::other)?;
            if !(1..=200).contains(&request.lines) {
                return Err(invalid("lines must be between 1 and 200"));
            }
            let session = self.terminal_target(&request.target)?;
            let alive = session.alive && !session.ended;
            let mut result = json!({"epoch":request.target.expected_epoch,
                "workspace":request.target.workspace,"session":session.id,"run":session.run,
                "pid":session.child.id(),"alive":alive,"kind":session.kind,"title":session.title,
                "text":session.term.capture(request.lines),"pending_input_bytes":session.input.len(),
                "bracketed_paste":session.term.bracketed_paste,"app_cursor":session.term.app_cursor,
                "ticket":null});
            self.terminal_tickets.retain(|t| {
                !(t.actor.0 == actor.0 && t.actor.1 == actor.1 && t.target == request.target)
            });
            if alive {
                if self.terminal_tickets.len() >= 256 {
                    return Err(invalid(
                        "terminal inspection ticket capacity reached; wait for expiry",
                    ));
                }
                let token = os::nonce()?;
                result["ticket"] = json!(token);
                result["expires_in_seconds"] = json!(LIFETIME.as_secs());
                self.terminal_tickets.push(Ticket {
                    token,
                    actor: (actor.0, actor.1.into()),
                    target: request.target,
                    created: Instant::now(),
                });
            }
            return Ok(result);
        }
        let request: Send = serde_json::from_value(args.clone()).map_err(io::Error::other)?;
        let index = self.terminal_tickets.iter().position(|t| {
            t.token == request.ticket && t.actor.0 == actor.0 && t.actor.1 == actor.1
                && t.target == request.target
        }).ok_or_else(|| invalid("unknown, expired, consumed or mismatched terminal ticket; inspect the target before new input"))?;
        // Consume before any possible side effect. Refresh/restart drops tickets;
        // neither a lost response nor a repeated request can replay these bytes.
        self.terminal_tickets.remove(index);
        let session = self.terminal_target(&request.target)?;
        let bytes = encode(request.actions, &session.term)?;
        // Identity, liveness, queue capacity and audit use the frontend's route.
        self.audit(
            "agent-terminal-input",
            request.target.session,
            &format!(
                "actor={};actor_run={};workspace={};run={};bytes={}",
                actor.0,
                actor.1,
                request.target.workspace,
                request.target.run,
                bytes.len()
            ),
        )?;
        self.command(&format!(
            "input\t{}\t{}\t{}",
            request.target.session,
            request.target.run,
            wire::hex(&bytes)
        ))?;
        Ok(
            json!({"workspace":request.target.workspace,"session":request.target.session,
            "run":request.target.run,"queued_bytes":bytes.len(),"outcome":"queued",
            "detail":"Input queued to the terminal; execution and completion are not established. Inspect the target before continuing. Do not replay after an uncertain reply."}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_modes_and_literal_paste_preserve_bytes() {
        let mut term = Terminal::new(80, 24);
        let actions = || {
            vec![
                Action::Text("λ".into()),
                Action::Key("Ctrl+C".into()),
                Action::Key("Alt+Enter".into()),
                Action::Key("Up".into()),
                Action::Key("F12".into()),
                Action::Hex("00ff".into()),
            ]
        };
        assert_eq!(
            encode(actions(), &term).unwrap(),
            b"\xce\xbb\x03\x1b\r\x1b[A\x1b[24~\x00\xff"
        );
        term.app_cursor = true;
        assert_eq!(key("Up", term.app_cursor).unwrap(), b"\x1bOA");
        assert!(encode(vec![Action::Paste("a\nb".into())], &term).is_err());
        term.bracketed_paste = true;
        assert_eq!(
            encode(vec![Action::Paste("a\nb".into())], &term).unwrap(),
            b"\x1b[200~a\nb\x1b[201~"
        );
        assert!(encode(vec![Action::Paste("\x1b[201~".into())], &term).is_err());
        assert!(encode(vec![Action::Hex("xz".into())], &term).is_err());
        assert!(encode(vec![Action::Text("a".repeat(LIMIT + 1))], &term).is_err());
    }
}
