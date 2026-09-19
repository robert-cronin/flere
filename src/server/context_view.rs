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

fn record_page<T>(
    records: impl Iterator<Item = T>,
    limit: usize,
    project: impl Fn(&T) -> Value,
    cursor: impl Fn(&T) -> Value,
) -> io::Result<(Vec<Value>, usize, Option<Value>)> {
    let mut page = Vec::new();
    let mut bytes = 0;
    let mut remaining = 0;
    let mut full = false;
    let mut next_after = None;
    for record in records {
        if full || page.len() == limit {
            full = true;
            remaining += 1;
            continue;
        }
        let value = project(&record);
        let len = serde_json::to_vec(&value).map_err(io::Error::other)?.len();
        if !page.is_empty() && bytes + len > PAGE_BYTES {
            full = true;
            remaining += 1;
            continue;
        }
        bytes += len;
        next_after = Some(cursor(&record));
        page.push(value);
    }
    if remaining == 0 {
        next_after = None;
    }
    Ok((page, remaining, next_after))
}

pub(super) fn checkpoint_id(index: usize) -> String {
    format!("checkpoint:{index}")
}
fn checkpoint_index(value: &Value) -> io::Result<usize> {
    value
        .as_str()
        .and_then(|s| s.strip_prefix("checkpoint:"))
        .and_then(|s| s.parse::<usize>().ok().filter(|n| n.to_string() == s))
        .ok_or_else(|| invalid("expected a checkpoint ID returned by this workspace"))
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
        if self.record_mode() {
            return self.record_coordinate(wid, native, "inbox", args);
        }
        let agent = native.is_some();
        let include_acknowledged = match args.get("include_acknowledged") {
            None => false,
            Some(Value::Bool(value)) => *value,
            _ => return Err(invalid("include_acknowledged must be a boolean")),
        };
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
        let mut total = 0;
        let mut remaining = 0;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for (i, m) in self.coordination.messages.iter().enumerate() {
            if !recipient(m) {
                continue;
            }
            let ack_requested = m.acknowledged.is_none() && acknowledged.contains(&i);
            let is_pending = m.acknowledged.is_none() && !ack_requested;
            pending += usize::from(is_pending);
            if !is_pending && !include_acknowledged {
                continue;
            }
            total += 1;
            if after.is_some_and(|a| i <= a) {
                continue;
            }
            if full || messages.len() == limit {
                full = true;
                remaining += 1;
                continue;
            }
            let mut record = m.clone();
            if ack_requested {
                record.acknowledged.get_or_insert(timestamp);
            }
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
            json!({"messages":messages,"acknowledged":ack,"pending":pending,"total":total,
            "remaining":remaining,"next_after":next_after}),
        )
    }
}

impl Server {
    /// Decision history is a read: no coordination clone, write or status change.
    pub(super) fn coordination_decisions(
        &self,
        wid: u64,
        agent: bool,
        args: &Value,
    ) -> io::Result<Value> {
        let target = match args.get("id") {
            None => None,
            Some(Value::String(id)) => Some(id.as_str()),
            _ => return Err(invalid("id must be a decision ID")),
        };
        if target.is_some() && (args.get("after").is_some() || args.get("limit").is_some()) {
            return Err(invalid("id cannot be combined with decision pagination"));
        }
        let decisions = &self.coordination.decisions;
        let scoped = || decisions.iter().filter(|d| d.workspace == wid);
        let total = scoped().count();
        let pending = scoped().filter(|d| d.answer.is_none()).count();
        if let Some(id) = target {
            let decision = scoped()
                .find(|d| d.id == id)
                .ok_or_else(|| invalid("decision does not belong to this workspace"))?;
            return Ok(
                json!({"decisions":[decision],"total":total,"pending":pending,
                "remaining":0,"next_after":null}),
            );
        }
        let after = match args.get("after") {
            None => None,
            Some(Value::String(id)) => Some(
                decisions
                    .iter()
                    .position(|d| d.workspace == wid && d.id == *id)
                    .ok_or_else(|| invalid("decision cursor does not belong to this workspace"))?,
            ),
            _ => return Err(invalid("after must be a decision ID")),
        };
        // Preserve the existing human history view unless pagination is requested.
        if !agent && after.is_none() && args.get("limit").is_none() {
            return Ok(
                json!({"decisions":scoped().collect::<Vec<_>>(),"total":total,
                "pending":pending,"remaining":0,"next_after":null}),
            );
        }
        let limit = limit(args, 8, 32)?;
        let records = decisions
            .iter()
            .enumerate()
            .filter(|(i, d)| d.workspace == wid && after.is_none_or(|a| *i > a));
        let (page, remaining, next_after) =
            record_page(records, limit, |(_, d)| json!(d), |(_, d)| json!(d.id))?;
        Ok(json!({"decisions":page,"total":total,"pending":pending,
            "remaining":remaining,"next_after":next_after}))
    }
}

impl Server {
    pub(super) fn coordination_checkpoints(&self, wid: u64, args: &Value) -> io::Result<Value> {
        let records = &self.coordination.checkpoints;
        let lookup = |value: &Value| -> io::Result<usize> {
            let index = checkpoint_index(value)?;
            if records.get(index).is_none_or(|c| c["workspace"] != wid) {
                return Err(invalid("checkpoint does not belong to this workspace"));
            }
            Ok(index)
        };
        let total = records.iter().filter(|c| c["workspace"] == wid).count();
        if let Some(id) = args.get("id") {
            if args.get("after").is_some() || args.get("limit").is_some() {
                return Err(invalid("id cannot be combined with checkpoint pagination"));
            }
            let index = lookup(id)?;
            return Ok(
                json!({"checkpoints":[{"id":checkpoint_id(index),"checkpoint":records[index]}],
                "total":total,"remaining":0,"next_after":null}),
            );
        }
        let after = args.get("after").map(lookup).transpose()?;
        let limit = limit(args, 8, 32)?;
        let selected = records
            .iter()
            .enumerate()
            .filter(|(i, c)| c["workspace"] == wid && after.is_none_or(|a| *i > a));
        let (page, remaining, next_after) = record_page(
            selected,
            limit,
            |(i, c)| json!({"id":checkpoint_id(*i),"checkpoint":c}),
            |(i, _)| json!(checkpoint_id(*i)),
        )?;
        Ok(json!({"checkpoints":page,"total":total,"remaining":remaining,"next_after":next_after}))
    }
}
