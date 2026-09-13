//! Local durable coordination. Surfacing is distinct from explicit acknowledgement.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[derive(Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub from: u64,
    pub to: u64,
    pub body: String,
    pub intent: String,
    pub saved: u64,
    pub surfaced: Option<u64>,
    // Human previews do not establish delivery to the native recipient.
    #[serde(default)]
    pub native_surfaced: Option<u64>,
    pub acknowledged: Option<u64>,
    #[serde(default)]
    pub(super) delivery: Option<super::delivery::Receipt>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub workspace: u64,
    pub question: String,
    pub recommendation: String,
    pub evidence: String,
    pub answer: Option<String>,
    pub created: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Coordination {
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub checkpoints: Vec<Value>,
    #[serde(default)]
    pub dispatches: Vec<super::dispatch::Dispatch>,
    #[serde(default)]
    pub(super) delivery: super::delivery::State,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn text(v: &Value, key: &str, max: usize) -> io::Result<String> {
    let s = v[key]
        .as_str()
        .ok_or_else(|| invalid(&format!("missing {key}")))?;
    if s.trim().is_empty() || s.len() > max {
        return Err(invalid(&format!("{key} is empty or exceeds {max} bytes")));
    }
    Ok(s.into())
}
impl Coordination {
    pub fn load(state: &Path) -> io::Result<Self> {
        let file = state.join("coordination.json");
        if !file.exists() {
            return Ok(Self::default());
        }
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(file)?;
        let m = f.metadata()?;
        if !m.is_file() || m.len() > 8 * 1024 * 1024 {
            return Err(invalid("invalid coordination store"));
        }
        serde_json::from_reader(io::BufReader::new(f)).map_err(io::Error::other)
    }
}
impl Server {
    pub(super) fn coordinate(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
        op: &str,
        args: &Value,
    ) -> io::Result<Value> {
        let agent = native.is_some();
        if matches!(
            op,
            "messaging_activation" | "message_status" | "set_focus" | "deliver_message"
        ) {
            return self.delivery_operation(wid, native, op, args);
        }
        if op == "context"
            && agent
            && self
                .coordination
                .messages
                .iter()
                .any(|m| m.to == wid && m.acknowledged.is_none() && m.native_surfaced.is_none())
        {
            let mut next = self.coordination.clone();
            for m in next
                .messages
                .iter_mut()
                .filter(|m| m.to == wid && m.acknowledged.is_none())
                .take(32)
            {
                m.surfaced.get_or_insert(now());
                m.native_surfaced.get_or_insert(now());
            }
            self.save_mailbox(next)?;
        }
        if matches!(
            op,
            "list_workspaces"
                | "add_project"
                | "update_workspace"
                | "prepare_workspace"
                | "prepare_worker"
                | "start_worker"
                | "worker_status"
                | "ack_assignment"
                | "report_worker_block"
                | "cancel_worker"
        ) {
            return self.dispatch_operation(wid, native, op, args);
        }
        let assignment = if op == "context" {
            self.assignment_context(wid, native)?
        } else {
            Value::Null
        };
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .ok_or_else(|| invalid("unknown workspace"))?;
        let recipient = |m: &Message| m.to == wid || (!agent && m.to == 0);
        if op == "context" {
            // Pending work must not disappear behind an older page of acknowledged records.
            let messages = self
                .coordination
                .messages
                .iter()
                .filter(|m| recipient(m) && m.acknowledged.is_none())
                .chain(
                    self.coordination
                        .messages
                        .iter()
                        .rev()
                        .filter(|m| !agent && recipient(m) && m.acknowledged.is_some()),
                )
                .take(32)
                .collect::<Vec<_>>();
            let decisions = self
                .coordination
                .decisions
                .iter()
                .filter(|d| d.workspace == wid && d.answer.is_none())
                .chain(
                    self.coordination
                        .decisions
                        .iter()
                        .rev()
                        .filter(|d| d.workspace == wid && d.answer.is_some()),
                )
                .take(32)
                .collect::<Vec<_>>();
            return Ok(json!({
                "assignment": assignment,
                "epoch": self.epoch,
                "workspace": {"id":w.id,"name":w.name,"cwd":w.cwd,"meta":w.meta},
                "workspaces": self.workspaces.iter().filter(|w| !w.meta.archived).map(|w|
                    json!({"id":w.id,"name":w.name,"status":w.meta.status,"pinned":w.meta.pinned}))
                    .collect::<Vec<_>>(),
                "messages":messages, "decisions":decisions,
                "checkpoints": self.coordination.checkpoints.iter().rev()
                    .filter(|c|c["workspace"]==wid).take(8).collect::<Vec<_>>(),
                "coordination_workflow": super::dispatch::COORDINATION_WORKFLOW,
                "messaging":self.activation(wid),
                "instructions":"Use this context; do not replay it as a prompt. Read inbox at checkpoints and acknowledge exact handled IDs. A decision/result never grants publication or native approval."
            }));
        }
        if op == "show_workspace" {
            let target = args["workspace"]
                .as_u64()
                .ok_or_else(|| invalid("missing workspace"))?;
            if args["user_requested"] != true {
                return Err(invalid("show_workspace requires an explicit user request"));
            }
            let reason = text(args, "reason", 1024)?;
            self.audit("show-workspace", target, &reason)?;
            self.command(&format!("focus\t{target}\t0"))?;
            return Ok(json!({"shown":target}));
        }
        let next_status = if op == "submit_result" || op == "request_decision" {
            if op == "submit_result" {
                text(args, "body", 16384)?;
            }
            Some(crate::workspace::Workflow::NeedsMe)
        } else if op == "set_status" {
            Some(crate::workspace::Workflow::parse(&text(
                args, "status", 32,
            )?)?)
        } else {
            None
        };
        if agent && next_status == Some(crate::workspace::Workflow::Done) {
            return Err(invalid(
                "use submit_result to request human review; Done is human-controlled",
            ));
        }
        let recipient = |m: &Message| m.to == wid || (!agent && m.to == 0);
        let mut next = self.coordination.clone();
        let result = match op {
            "set_status" => json!({"status":next_status}),
            "read_message" => {
                if agent {
                    return Err(invalid("native agents read messages through inbox"));
                }
                let id = text(args, "id", 64)?;
                let m = next
                    .messages
                    .iter_mut()
                    .find(|m| m.id == id && recipient(m))
                    .ok_or_else(|| invalid("message does not belong to this workspace"))?;
                m.surfaced.get_or_insert(now());
                json!({"message":m})
            }
            "inbox" => {
                let ack = args["ack_ids"].as_array().cloned().unwrap_or_default();
                for id in &ack {
                    let id = id
                        .as_str()
                        .ok_or_else(|| invalid("ack IDs must be strings"))?;
                    if !next.messages.iter().any(|m| m.id == id && recipient(m)) {
                        return Err(invalid("ack target does not belong to this workspace"));
                    }
                }
                for m in next.messages.iter_mut().filter(|m| recipient(m)) {
                    if ack.iter().any(|id| id.as_str() == Some(&m.id)) {
                        m.acknowledged.get_or_insert(now());
                    }
                }
                if agent {
                    for m in next
                        .messages
                        .iter_mut()
                        .filter(|m| recipient(m) && m.acknowledged.is_none())
                        .take(32)
                    {
                        m.surfaced.get_or_insert(now());
                        m.native_surfaced.get_or_insert(now());
                    }
                }
                json!({"messages":next.messages.iter().filter(|m|recipient(m)&&m.acknowledged.is_none()).take(32).collect::<Vec<_>>()})
            }
            "send_message" => {
                let to = if args["to"] == "user" {
                    0
                } else {
                    args["to"]
                        .as_u64()
                        .ok_or_else(|| invalid("recipient must be an exact workspace ID or user; named roles are not supported"))?
                };
                if to != 0 && !self.workspaces.iter().any(|w| w.id == to) {
                    return Err(invalid("unknown recipient"));
                }
                let body = text(args, "body", 16384)?;
                let intent = args["intent"].as_str().unwrap_or("quiet");
                if !matches!(intent, "quiet" | "interrupt") {
                    return Err(invalid("intent must be quiet or interrupt"));
                }
                let m = Message {
                    id: os::nonce()?,
                    from: if agent { wid } else { 0 },
                    to,
                    body,
                    intent: intent.into(),
                    saved: now(),
                    surfaced: None,
                    native_surfaced: None,
                    acknowledged: None,
                    delivery: None,
                };
                let result = json!({"message":m,"delivery":"saved; native delivery checks run separately; inspect message_status for waiting/queue receipts"});
                next.messages.push(m);
                result
            }
            "request_decision" => {
                let d = Decision {
                    id: os::nonce()?,
                    workspace: wid,
                    question: text(args, "question", 8192)?,
                    recommendation: text(args, "recommendation", 8192)?,
                    evidence: args["evidence"]
                        .as_str()
                        .unwrap_or_default()
                        .chars()
                        .take(16384)
                        .collect(),
                    answer: None,
                    created: now(),
                };
                let result = json!({"decision":d});
                next.decisions.push(d);
                result
            }
            "decisions" => {
                json!({"decisions":next.decisions.iter().filter(|d|d.workspace==wid).collect::<Vec<_>>()})
            }
            "answer_decision" => {
                if agent {
                    return Err(invalid("decision answers are human-controlled"));
                }
                let id = text(args, "id", 64)?;
                let answer = text(args, "answer", 8192)?;
                let d = next
                    .decisions
                    .iter_mut()
                    .find(|d| d.id == id && d.workspace == wid)
                    .ok_or_else(|| invalid("decision does not belong to workspace"))?;
                if d.answer.is_some() {
                    return Err(invalid("decision already answered"));
                }
                d.answer = Some(answer);
                json!({"decision":d})
            }
            "checkpoint" | "submit_result" => {
                let body = text(args, "body", 16384)?;
                let event = json!({"workspace":wid,"kind":op,"body":body,"time":now()});
                next.checkpoints.push(event.clone());
                json!({"saved":event,"accepted":false})
            }
            _ => return Err(invalid("unknown coordination operation")),
        };
        let old_coord = std::mem::replace(&mut self.coordination, next);
        let index = self.workspaces.iter().position(|w| w.id == wid).unwrap();
        let old_status = self.workspaces[index].meta.status;
        if let Some(status) = next_status {
            self.workspaces[index].meta.status = status;
        }
        if let Err(e) = self.persist() {
            self.coordination = old_coord;
            self.workspaces[index].meta.status = old_status;
            return Err(e);
        }
        self.audit(
            op,
            wid,
            if agent {
                "native agent"
            } else {
                "human UI/CLI"
            },
        )?;
        self.changed();
        Ok(result)
    }
}
