//! Small default coordination views; full records remain durable and retrievable.
use super::*;
use serde_json::{Value, json};

pub(super) const PAGE_BYTES: usize = 32 * 1024;

pub(super) fn detail(args: &Value) -> io::Result<bool> {
    match args.get("detail") {
        None => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(invalid("detail must be a boolean")),
    }
}

pub(super) fn limit(args: &Value, default: usize, maximum: usize) -> io::Result<usize> {
    match args.get("limit") {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .filter(|n| *n > 0 && *n <= maximum as u64)
            .map(|n| n as usize)
            .ok_or_else(|| invalid("limit is outside the supported range")),
    }
}

pub(super) fn message(m: &coordination::Message, body: bool) -> Value {
    if body {
        return json!(m);
    }
    let mut value = json!({"id":m.id,"from":m.from,"to":m.to,"intent":m.intent,
        "saved":m.saved,"surfaced":m.surfaced,"native_surfaced":m.native_surfaced,
        "acknowledged":m.acknowledged,"delivery":m.delivery,
        "body_bytes":m.body.len(),"summary":true});
    if let Some(chat) = &m.chat {
        let mut address = json!(chat);
        address.as_object_mut().unwrap().remove("user_request_ref");
        value["chat"] = address;
    }
    value
}

pub(super) fn workspace(w: &Workspace) -> Value {
    json!({"id":w.id,"name":w.name,"cwd":w.cwd,
        "meta":{"status":w.meta.status,"pinned":w.meta.pinned,
            "archived":w.meta.archived,"project":w.meta.project,
            "issue":w.meta.issue,"pr":w.meta.pr,"branch":w.meta.branch,
            "operation":w.meta.operation,"base_sha":w.meta.base_sha},
        "detail":false})
}

impl Server {
    pub(super) fn coordination_inbox(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
        args: &Value,
    ) -> io::Result<Value> {
        let agent = native.is_some();
        let conversation = self.mailbox_conversation(wid, native);
        let recipient = |m: &coordination::Message| {
            (m.to == wid && (!agent || m.for_conversation(conversation.as_deref())))
                || (!agent && m.to == 0)
        };
        let ack = match args.get("ack_ids") {
            None => Vec::new(),
            Some(Value::Array(ids)) => ids.clone(),
            _ => return Err(invalid("ack_ids must be an array")),
        };
        let mut acknowledged = Vec::new();
        for id in &ack {
            let id = id
                .as_str()
                .ok_or_else(|| invalid("acknowledgement must be a message ID"))?;
            let i = self
                .coordination
                .messages
                .iter()
                .position(|m| m.id == id && recipient(m))
                .ok_or_else(|| {
                    invalid("message is outside your workspace or native conversation")
                })?;
            acknowledged.push(i);
        }
        let after = match args.get("after") {
            None => None,
            Some(Value::String(id)) => Some(
                self.coordination
                    .messages
                    .iter()
                    .position(|m| m.id == *id && recipient(m))
                    .ok_or_else(|| invalid("inbox cursor does not belong to this conversation"))?,
            ),
            _ => return Err(invalid("after must be a message ID")),
        };
        let limit = limit(args, if agent { 8 } else { 32 }, 32)?;
        // ACK-only is a receipt, not an implicit body read of the next page.
        let mut full =
            agent && !ack.is_empty() && args.get("after").is_none() && args.get("limit").is_none();
        let mut messages = Vec::new();
        let mut surfaced = Vec::new();
        let mut bytes = 0;
        let mut pending = 0;
        let mut remaining = 0;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for (i, m) in self.coordination.messages.iter().enumerate() {
            if !recipient(m) || m.acknowledged.is_some() || acknowledged.contains(&i) {
                continue;
            }
            pending += 1;
            if after.is_some_and(|a| i <= a) {
                continue;
            }
            if full || messages.len() == limit {
                full = true;
                remaining += 1;
                continue;
            }
            let mut record = m.clone();
            if agent {
                record.surfaced.get_or_insert(timestamp);
                record.native_surfaced.get_or_insert(timestamp);
            }
            let value = json!(record);
            let len = serde_json::to_vec(&value).map_err(io::Error::other)?.len();
            if agent && !messages.is_empty() && bytes + len > PAGE_BYTES {
                full = true;
                remaining += 1;
                continue;
            }
            if agent && (m.surfaced.is_none() || m.native_surfaced.is_none()) {
                surfaced.push(i);
            }
            bytes += len;
            messages.push(value);
        }
        // Repeated reads of already surfaced records do not clone or persist the store.
        if !surfaced.is_empty()
            || acknowledged
                .iter()
                .any(|i| self.coordination.messages[*i].acknowledged.is_none())
        {
            let mut next = self.coordination.clone();
            for i in acknowledged {
                next.messages[i].acknowledged.get_or_insert(timestamp);
            }
            for i in surfaced {
                next.messages[i].surfaced.get_or_insert(timestamp);
                next.messages[i].native_surfaced.get_or_insert(timestamp);
            }
            self.save_mailbox(next)?;
            self.audit(
                "inbox",
                wid,
                if agent {
                    "native agent"
                } else {
                    "human UI/CLI"
                },
            )?;
        }
        let next_after = if remaining > 0 {
            messages.last().map(|m| m["id"].clone())
        } else {
            None
        };
        Ok(
            json!({"messages":messages,"acknowledged":ack,"pending":pending,
            "remaining":remaining,"next_after":next_after}),
        )
    }
}
