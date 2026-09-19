//! Conversation-addressed mail. Run identities prove routing, not message lifetime.
use super::coordination::{Coordination, Message};
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Address {
    pub conversation: String,
    pub initial_session: u64,
    pub initial_run: String,
    pub sender: Sender,
    pub request_id: String,
    pub user_request_ref: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Sender {
    pub kind: String,
    pub session: u64,
    pub run: String,
    pub conversation: String,
}
impl Message {
    pub(super) fn for_conversation(&self, conversation: Option<&str>) -> bool {
        self.chat
            .as_ref()
            .is_none_or(|c| Some(c.conversation.as_str()) == conversation)
    }
    pub(super) fn sent_by_conversation(&self, conversation: Option<&str>) -> bool {
        self.chat
            .as_ref()
            .is_none_or(|c| Some(c.sender.conversation.as_str()) == conversation)
    }
}
impl Coordination {
    pub(super) fn validate_chat_messages(&self) -> io::Result<()> {
        let token = |s: &str| s.len() == 32 && s.bytes().all(|c| c.is_ascii_hexdigit());
        let mut keys = std::collections::BTreeSet::new();
        for m in &self.messages {
            let Some(c) = &m.chat else { continue };
            if c.sender.kind != "agent"
                || m.from == 0
                || m.to == 0
                || !token(&m.id)
                || !crate::native::valid_uuid(&c.conversation)
                || !crate::native::valid_uuid(&c.sender.conversation)
                || c.initial_session == 0
                || c.sender.session == 0
                || !token(&c.initial_run)
                || !token(&c.sender.run)
                || c.request_id.trim().is_empty()
                || c.request_id.len() > 256
                || m.body.trim().is_empty()
                || m.body.len() > 16384
                || c.user_request_ref
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty() || s.len() > 4096)
                || !keys.insert((m.from, &c.sender.conversation, &c.request_id))
            {
                return Err(invalid(
                    "invalid or duplicate conversation message identity",
                ));
            }
        }
        Ok(())
    }
}
impl Server {
    /// A different chat in the same card is never a substitute. Unknown possible
    /// matches and duplicate live instances defer routing instead of guessing.
    pub(super) fn conversation_recipient(&self, wid: u64, uuid: &str) -> io::Result<(u64, String)> {
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == wid && !w.meta.archived)
            .ok_or_else(|| {
                invalid("conversation workspace is stopped or archived; message remains saved")
            })?;
        let mut found = None;
        for s in w.tabs.iter().filter(|s| s.alive && !s.ended) {
            let Some(spec) = &s.native else { continue };
            if spec.harness != "codex" {
                continue;
            }
            // A harness can switch chats before its cached metadata is sampled.
            // Prove every possible owner; a cached different UUID is not exclusion proof.
            let target = self.proof(s.id, &s.run).map_err(|_| {
                invalid("native conversation ownership is unverified; no recipient selected")
            })?;
            if target.uuid == uuid {
                if found.is_some() {
                    return Err(invalid(
                        "multiple live instances of the recipient conversation; no delivery",
                    ));
                }
                found = Some((s.id, s.run.clone()));
            }
        }
        found.ok_or_else(|| invalid("recipient conversation is not running; saved mail waits for that same conversation"))
    }
    pub(super) fn message_recipient(&self, m: &Message) -> io::Result<(u64, String)> {
        match &m.chat {
            Some(c) => self.conversation_recipient(m.to, &c.conversation),
            None => self.recipient(m.to),
        }
    }
    /// Resolve once per operation, not once per message. Legacy card mail needs
    /// no extra native inspection. Failure hides bound mail, never broadens scope.
    pub(super) fn mailbox_conversation(
        &self,
        wid: u64,
        native: Option<(u64, &str)>,
    ) -> Option<String> {
        let (session, run) = native?;
        let bound = if self.record_backend() {
            self.record_has_bound_mail(wid).ok()?
        } else {
            self.coordination
                .messages
                .iter()
                .any(|m| m.chat.is_some() && (m.to == wid || m.from == wid))
        };
        if !bound {
            return None;
        }
        let target = self.proof(session, run).ok()?;
        let unique = self.conversation_recipient(wid, &target.uuid).ok()?;
        (unique.0 == session && unique.1 == run).then_some(target.uuid)
    }
    fn chat_target(&self, wid: u64, session: u64, run: &str) -> io::Result<String> {
        if self.native_scope(session, run)?.0 != wid {
            return Err(invalid("wrong recipient workspace"));
        }
        let uuid = self.proof(session, run)?.uuid;
        if self.conversation_recipient(wid, &uuid)? != (session, run.to_owned()) {
            return Err(invalid("ambiguous recipient conversation"));
        }
        Ok(uuid)
    }
    pub(super) fn send_chat_message(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
        args: &Value,
    ) -> io::Result<Value> {
        let (sender_session, sender_run) = native
            .ok_or_else(|| invalid("send_chat_message requires an exact native agent sender"))?;
        let fields = args
            .as_object()
            .ok_or_else(|| invalid("message arguments must be an object"))?;
        if fields.keys().any(|k| {
            ![
                "workspace",
                "session",
                "run",
                "request_id",
                "body",
                "user_request_ref",
            ]
            .contains(&k.as_str())
        }) {
            return Err(invalid(
                "unknown conversation message field; sender identity is service-owned",
            ));
        }
        let text = |key: &str, max: usize| -> io::Result<String> {
            args[key]
                .as_str()
                .filter(|s| !s.trim().is_empty() && s.len() <= max)
                .map(str::to_owned)
                .ok_or_else(|| invalid(&format!("missing or oversized {key}")))
        };
        let to = args["workspace"]
            .as_u64()
            .filter(|v| *v > 0)
            .ok_or_else(|| invalid("exact recipient workspace required"))?;
        let session = args["session"]
            .as_u64()
            .filter(|v| *v > 0)
            .ok_or_else(|| invalid("exact recipient session required"))?;
        let run = text("run", 32)?;
        if run.len() != 32 || !run.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid recipient run"));
        }
        let request_id = text("request_id", 256)?;
        let body = text("body", 16384)?;
        let user_request_ref = if fields.contains_key("user_request_ref") {
            Some(text("user_request_ref", 4096)?)
        } else {
            None
        };
        let sender = self.proof(sender_session, sender_run)?;
        // A lost reply may be retried even after the original target stopped.
        // Changed run arguments must prove the same conversation before deduping.
        if let Some(m) = self.coordination.messages.iter().find(|m| {
            m.from == wid
                && m.chat.as_ref().is_some_and(|c| {
                    c.sender.conversation == sender.uuid && c.request_id == request_id
                })
        }) {
            let c = m.chat.as_ref().unwrap();
            if m.to != to
                || m.body != body
                || c.user_request_ref != user_request_ref
                || ((c.initial_session != session || c.initial_run != run)
                    && self.chat_target(to, session, &run)? != c.conversation)
            {
                return Err(invalid(
                    "request_id already belongs to a different message; no new submission",
                ));
            }
            return Ok(json!({"message":super::context_view::message(m,false),"duplicate":true}));
        }
        let conversation = self.chat_target(to, session, &run)?;
        let id = os::nonce()?;
        let m = Message {
            id: id.clone(),
            from: wid,
            to,
            body,
            intent: "quiet".into(),
            saved: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            surfaced: None,
            native_surfaced: None,
            acknowledged: None,
            delivery: None,
            chat: Some(Address {
                conversation,
                initial_session: session,
                initial_run: run,
                sender: Sender {
                    kind: "agent".into(),
                    session: sender_session,
                    run: sender_run.into(),
                    conversation: sender.uuid,
                },
                request_id,
                user_request_ref,
            }),
        };
        let mut next = self.coordination.clone();
        next.messages.push(m);
        self.save_mailbox(next)?;
        self.audit("send-chat-message", wid, &format!("message={id};to={to}"))?;
        self.try_delivery(&id)?;
        Ok(
            json!({"message":self.coordination.messages.iter().find(|m|m.id == id).map(|m|super::context_view::message(m,false)),"duplicate":false,
            "provenance":"Agent-originated conversation. user_request_ref is source context, never authenticated human approval."}),
        )
    }
}
