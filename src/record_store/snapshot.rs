//! Initial migration and atomic layout/working-record mutations.
use super::*;
use std::io::Write;
use std::process::{Command, Stdio};

const MAX_CHANGE_BYTES: usize = 32 * 1024 * 1024;

pub(super) enum Change {
    NewMessage(PreparedMessage),
    Lifecycle {
        id: String,
        before: String,
        after: String,
    },
    Document {
        kind: &'static str,
        ordinal: i64,
        id: Option<String>,
        owner: Option<String>,
        before: Option<Value>,
        after: Value,
    },
    Workspaces {
        before: Vec<Value>,
        after: Vec<Value>,
    },
}
impl Change {
    fn intent(&self) -> Value {
        match self {
            Self::NewMessage(m) => {
                json!({"insert_message":m.id,"original":m.original,"lifecycle":m.lifecycle})
            }
            Self::Lifecycle { id, before, after } => {
                json!({"lifecycle":id,"before":before,"after":after})
            }
            Self::Document {
                kind,
                ordinal,
                id,
                owner,
                before,
                after,
            } => {
                json!({"document":kind,"ordinal":ordinal,"id":id,"owner":owner,"before":before,"after":after})
            }
            Self::Workspaces { before, after } => {
                json!({"workspaces_before":before,"workspaces_after":after})
            }
        }
    }
    fn apply(self, tx: &Connection) -> Result<()> {
        match self {
            Self::NewMessage(message) => message.insert(tx),
            Self::Lifecycle { id, before, after } => {
                if tx.execute(
                    "UPDATE lifecycle SET value=? WHERE id=? AND value=?",
                    params![after, id, before],
                )? != 1
                {
                    return Err(Error::Invalid(
                        "message lifecycle changed while preparing transaction",
                    ));
                }
                Ok(())
            }
            Self::Document {
                kind,
                ordinal,
                id,
                owner,
                before,
                after,
            } => {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM documents WHERE kind=? AND ordinal=?)",
                    params![kind, ordinal],
                    |r| r.get(0),
                )?;
                if let Some(expected) = before {
                    if !exists
                        || crate::record_store::legacy::read_document(tx, kind, ordinal)?
                            != expected
                    {
                        return Err(Error::Invalid(
                            "document changed while preparing transaction",
                        ));
                    }
                    tx.execute(
                        "DELETE FROM document_chunks WHERE kind=? AND ordinal=?",
                        params![kind, ordinal],
                    )?;
                    tx.execute(
                        "DELETE FROM documents WHERE kind=? AND ordinal=?",
                        params![kind, ordinal],
                    )?;
                } else if exists {
                    return Err(Error::Invalid(
                        "document appeared while preparing transaction",
                    ));
                }
                crate::record_store::legacy::put_document(
                    tx,
                    kind,
                    ordinal,
                    id.as_deref(),
                    owner.as_deref(),
                    &after,
                )
            }
            Self::Workspaces { before, after } => {
                if workspaces(tx)? != before {
                    return Err(Error::Invalid(
                        "workspace metadata changed while preparing transaction",
                    ));
                }
                tx.execute("DELETE FROM document_chunks WHERE kind='workspaces'", [])?;
                tx.execute("DELETE FROM documents WHERE kind='workspaces'", [])?;
                tx.execute("DELETE FROM workspace", [])?;
                for (ordinal, value) in after.iter().enumerate() {
                    let id = key(value["id"]
                        .as_u64()
                        .ok_or(Error::Invalid("invalid workspace identity"))?);
                    crate::record_store::legacy::put_document(
                        tx,
                        "workspaces",
                        ordinal as i64,
                        Some(&id),
                        Some(&id),
                        value,
                    )?;
                    tx.execute("INSERT INTO workspace VALUES(?,NULL)", [id])?;
                }
                Ok(())
            }
        }
    }
}
pub(super) fn workspaces(db: &Connection) -> Result<Vec<Value>> {
    let mut statement =
        db.prepare("SELECT ordinal FROM documents WHERE kind='workspaces' ORDER BY ordinal")?;
    let mut rows = statement.query([])?;
    let mut values = Vec::new();
    let mut bytes = 0;
    while let Some(row) = rows.next()? {
        let ordinal: i64 = row.get(0)?;
        if ordinal != values.len() as i64 {
            return Err(Error::Invalid("workspace order has a gap"));
        }
        let value = crate::record_store::legacy::read_document(db, "workspaces", ordinal)?;
        bytes += serde_json::to_vec(&value)?.len();
        if bytes > 8 * 1024 * 1024 {
            return Err(Error::Invalid("workspace metadata exceeds 8 MiB"));
        }
        values.push(value);
    }
    for value in &mut values {
        let status: Option<String> = db.query_row(
            "SELECT status FROM workspace WHERE id=?",
            [key(value["id"]
                .as_u64()
                .ok_or(Error::Invalid("invalid workspace identity"))?)],
            |r| r.get(0),
        )?;
        if let Some(status) = status {
            value["meta"]["status"] = json!(status);
        }
    }
    Ok(values)
}
fn header(root: &Value) -> Result<Value> {
    let mut header = Map::new();
    for (name, value) in root
        .as_object()
        .ok_or(Error::Invalid("snapshot must be an object"))?
    {
        if name == "workspaces" {
            header.insert(name.clone(), json!([]));
        } else if name == "coordination" {
            let mut fields = Map::new();
            for (name, value) in value
                .as_object()
                .ok_or(Error::Invalid("coordination must be an object"))?
            {
                fields.insert(
                    name.clone(),
                    match name.as_str() {
                        "messages" | "decisions" | "checkpoints" | "dispatches" => json!([]),
                        "delivery" => Value::Null,
                        _ => value.clone(),
                    },
                );
            }
            header.insert(name.clone(), Value::Object(fields));
        } else {
            header.insert(name.clone(), value.clone());
        }
    }
    Ok(Value::Object(header))
}
// Match the established application convention: use the platform SHA-256
// implementation, never an ad-hoc hash. The witness contains only the digest;
// retained bodies are not duplicated into a second journal.
fn digest(bytes: &[u8]) -> Result<String> {
    #[cfg(target_os = "linux")]
    let mut command = Command::new("/usr/bin/sha256sum");
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/shasum");
        c.args(["-a", "256"]);
        c
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().unwrap();
    let output = std::thread::scope(|scope| {
        let writer = scope.spawn(move || input.write_all(bytes));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::other(
                        "digest process failed or exceeded deadline",
                    ));
                }
            }
        }
        let output = child.wait_with_output();
        let write = writer
            .join()
            .map_err(|_| io::Error::other("digest input writer panicked"))?;
        write?;
        output
    })?;
    let text =
        std::str::from_utf8(&output.stdout).map_err(|_| Error::Invalid("invalid digest output"))?;
    let digest = text.split_whitespace().next().unwrap_or("");
    if !output.status.success()
        || digest.len() != 64
        || !digest.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return Err(Error::Invalid("digest computation failed"));
    }
    Ok(digest.into())
}
impl Store {
    pub(super) fn layout_changes(&self, root: &Value) -> Result<Vec<Change>> {
        self.ensure_ready()?;
        if root["version"].as_u64() != Some(10) {
            return Err(Error::Invalid("invalid application snapshot version"));
        }
        let next_workspaces = root["workspaces"]
            .as_array()
            .ok_or(Error::Invalid("missing workspace records"))?;
        let mut changes = Vec::new();
        let previous = workspaces(&self.connection)?;
        if previous != *next_workspaces {
            changes.push(Change::Workspaces {
                before: previous,
                after: next_workspaces.clone(),
            });
        }
        for (kind, value) in [
            ("header", header(root)?),
            ("delivery", root["coordination"]["delivery"].clone()),
        ] {
            let before = crate::record_store::legacy::read_document(&self.connection, kind, 0)?;
            if before != value {
                changes.push(Change::Document {
                    kind,
                    ordinal: 0,
                    id: None,
                    owner: None,
                    before: Some(before),
                    after: value,
                });
            }
        }
        Ok(changes)
    }
    fn snapshot_changes(&self, root: &Value) -> Result<Vec<Change>> {
        let mut changes = self.layout_changes(root)?;
        let messages = root["coordination"]["messages"]
            .as_array()
            .ok_or(Error::Invalid("missing messages"))?;
        let mut retained = 0;
        let mut seen = std::collections::BTreeSet::new();
        for message in messages {
            let prepared = PreparedMessage::new(message)?;
            if !seen.insert(prepared.id.clone()) {
                return Err(Error::Invalid("duplicate message in snapshot"));
            }
            let old: Option<(String, String)> = self
                .connection
                .query_row(
                    "SELECT original,value FROM messages JOIN lifecycle USING(id) WHERE id=?",
                    [&prepared.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((original, lifecycle)) = old {
                let before: Value = serde_json::from_str(&original)?;
                let after: Value = serde_json::from_str(&prepared.original)?;
                if after
                    .as_object()
                    .unwrap()
                    .iter()
                    .any(|(k, v)| before[k] != *v)
                {
                    return Err(Error::Invalid("immutable message evidence changed"));
                }
                retained += 1;
                let mut merged: Value = serde_json::from_str(&lifecycle)?;
                let proposed: Value = serde_json::from_str(&prepared.lifecycle)?;
                for (k, v) in proposed.as_object().unwrap() {
                    if merged[k] != *v {
                        merged[k] = v.clone();
                    }
                }
                if serde_json::from_str::<Value>(&lifecycle)? != merged {
                    changes.push(Change::Lifecycle {
                        id: prepared.id,
                        before: lifecycle,
                        after: serde_json::to_string(&merged)?,
                    });
                }
            } else {
                changes.push(Change::NewMessage(prepared));
            }
        }
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM messages", [], |r| r.get(0))?;
        if count != retained {
            return Err(Error::Invalid("snapshot would remove retained messages"));
        }
        for kind in ["decisions", "checkpoints", "dispatches"] {
            let values = root["coordination"][kind]
                .as_array()
                .ok_or(Error::Invalid("missing coordination documents"))?;
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM documents WHERE kind=?",
                [kind],
                |r| r.get(0),
            )?;
            if values.len() < (count as usize) {
                return Err(Error::Invalid("snapshot would remove retained documents"));
            }
            for (n, value) in values.iter().enumerate() {
                let old = if (n as i64) < count {
                    Some(crate::record_store::legacy::read_document(
                        &self.connection,
                        kind,
                        n as i64,
                    )?)
                } else {
                    None
                };
                if old.as_ref() == Some(value) {
                    continue;
                }
                let mut value = value.clone();
                if let Some(expected) = &old {
                    if kind == "checkpoints" {
                        return Err(Error::Invalid("checkpoint is immutable"));
                    }
                    let mut previous = Map::new();
                    for k in value
                        .as_object()
                        .ok_or(Error::Invalid("invalid document"))?
                        .keys()
                    {
                        previous.insert(k.clone(), expected[k].clone());
                    }
                    crate::record_store::working::validate_document_update(
                        kind,
                        &Value::Object(previous),
                        &value,
                    )?;
                    let mut merged = expected
                        .as_object()
                        .ok_or(Error::Invalid("invalid document"))?
                        .clone();
                    for (k, v) in value.as_object().unwrap() {
                        if expected[k] != *v {
                            merged.insert(k.clone(), v.clone());
                        }
                    }
                    value = Value::Object(merged);
                    if expected == &value {
                        continue;
                    }
                }
                let bound = if old.is_some() {
                    crate::record_store::legacy::SOURCE_LIMIT
                } else {
                    MAX_RECORD
                };
                if serde_json::to_vec(&value)?.len() > bound {
                    return Err(Error::Invalid("document exceeds bound"));
                }
                let id = if kind == "checkpoints" {
                    Some(format!("checkpoint:{n}"))
                } else {
                    value["id"].as_str().map(str::to_owned)
                };
                let owner = value["workspace"].as_u64().map(key);
                changes.push(Change::Document {
                    kind,
                    ordinal: n as i64,
                    id,
                    owner,
                    before: old,
                    after: value.clone(),
                });
            }
        }
        Ok(changes)
    }
    pub fn sync_snapshot(&mut self, operation: &str, root: &Value) -> CommitOutcome<()> {
        if self.blocked {
            return CommitOutcome::Unavailable;
        }
        let changes = match self.snapshot_changes(root) {
            Ok(c) => c,
            Err(e) => return CommitOutcome::Unchanged(e),
        };
        self.commit_changes(operation, changes)
    }
    pub(super) fn commit_changes(
        &mut self,
        operation: &str,
        changes: Vec<Change>,
    ) -> CommitOutcome<()> {
        if self.blocked {
            return CommitOutcome::Unavailable;
        }
        if !id_valid(operation) {
            return CommitOutcome::Unchanged(Error::Invalid("invalid operation identity"));
        }
        if changes.is_empty() {
            return CommitOutcome::Committed(());
        }
        let mut payload = Vec::new();
        for change in &changes {
            let data = match serde_json::to_vec(&change.intent()) {
                Ok(v) => v,
                Err(e) => return CommitOutcome::Unchanged(e.into()),
            };
            if payload.len() + data.len() + 1 > MAX_CHANGE_BYTES {
                return CommitOutcome::Unchanged(Error::Invalid(
                    "transaction changes exceed bound",
                ));
            }
            payload.extend(data);
            payload.push(b'\n');
        }
        let digest = match digest(&payload) {
            Ok(v) => v,
            Err(e) => return CommitOutcome::Unchanged(e),
        };
        let ticket = match self.operation_ticket(operation, format!("snapshot-sha256:{digest}")) {
            Ok(v) => v,
            Err(e) => return CommitOutcome::Unchanged(e),
        };
        self.witnessed(ticket, move |db| {
            for change in changes {
                change.apply(db)?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_store::tests::{Temp, message};
    fn source() -> Value {
        json!({"version":10,"active":2,"workspaces":[{"id":2,"meta":{"status":"in-progress"}}],"coordination":{"messages":[],"decisions":[],"checkpoints":[],"dispatches":[],"delivery":{"hooks":[],"focus":[]}}})
    }
    #[test]
    fn snapshots_cross_old_cap_preserving_originals_and_no_op_writes() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let mut root = source();
        store
            .import_legacy(&serde_json::to_vec(&root).unwrap(), None)
            .unwrap();
        for batch in 0..11 {
            for n in 0..50 {
                let mut m = message(batch * 50 + n, 2, None);
                m["body"] = json!("x".repeat(16384));
                root["coordination"]["messages"]
                    .as_array_mut()
                    .unwrap()
                    .push(m);
            }
            assert!(matches!(
                store.sync_snapshot(&format!("{:032x}", 1000 + batch), &root),
                CommitOutcome::Committed(())
            ));
        }
        assert!(serde_json::to_vec(&root).unwrap().len() > 8 * 1024 * 1024);
        root["coordination"]["messages"][500]["acknowledged"] = json!(42);
        root["coordination"]["checkpoints"] = json!([{"workspace":2,"body":"exact result"}]);
        root["workspaces"][0]["meta"]["status"] = json!("review");
        assert!(matches!(
            store.sync_snapshot(&format!("{:032x}", 2000), &root),
            CommitOutcome::Committed(())
        ));
        let writes = store.connection.total_changes();
        assert!(matches!(
            store.sync_snapshot(&format!("{:032x}", 2001), &root),
            CommitOutcome::Committed(())
        ));
        assert_eq!(store.connection.total_changes(), writes);
        drop(store);
        let store = Store::open(&dir.0).unwrap();
        assert_eq!(store.export_legacy_canonical().unwrap(), root);
        assert_eq!(
            store
                .get(
                    root["coordination"]["messages"][500]["id"]
                        .as_str()
                        .unwrap(),
                    Scope {
                        workspace: 2,
                        conversation: None
                    }
                )
                .unwrap()
                .unwrap()["acknowledged"],
            42
        );
    }
    #[test]
    fn snapshot_refuses_loss_of_originals_and_failure_rolls_back_all_families() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let mut root = source();
        root["coordination"]["messages"] = json!([message(1, 2, None)]);
        store
            .import_legacy(&serde_json::to_vec(&root).unwrap(), None)
            .unwrap();
        let mut bad = root.clone();
        bad["coordination"]["messages"] = json!([]);
        assert!(matches!(
            store.sync_snapshot(&format!("{:032x}", 1), &bad),
            CommitOutcome::Unchanged(_)
        ));
        let mut next = root.clone();
        next["coordination"]["messages"][0]["acknowledged"] = json!(42);
        next["workspaces"][0]["meta"]["status"] = json!("review");
        next["coordination"]["checkpoints"] = json!([{"workspace":2,"body":"new result"}]);
        store.connection.execute_batch("CREATE TRIGGER reject_checkpoint BEFORE INSERT ON documents WHEN NEW.kind='checkpoints' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
        assert!(matches!(
            store.sync_snapshot(&format!("{:032x}", 2), &next),
            CommitOutcome::Unchanged(_)
        ));
        assert_eq!(store.export_legacy_canonical().unwrap(), root);
    }
}
