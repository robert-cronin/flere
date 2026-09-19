//! Indexed, bounded runtime working sets. These selectors are retrieval filters;
//! the application must still establish current native ownership for every call.
use super::*;

#[derive(Clone, Copy)]
pub enum MessageSelection<'a> {
    Id(&'a str),
    Context {
        scope: MailboxScope<'a>,
        handled: bool,
        limit: usize,
    },
    Notice {
        scope: Scope<'a>,
        interrupt_only: bool,
        expired_hooks: bool,
        now: u64,
    },
    Lease(&'a str),
}
#[derive(Clone, Copy)]
pub enum DocumentSelection<'a> {
    Id(&'a str),
    Request { owner: u64, request: &'a str },
    Recent { workspace: u64, limit: usize },
    ActiveDispatch { workspace: u64 },
    Assignment { workspace: u64 },
    ContextDecisions { workspace: u64 },
}
#[derive(Debug)]
pub struct WorkingDocument {
    pub id: String,
    pub ordinal: i64,
    pub value: Value,
}

impl Store {
    pub fn select_messages(&self, selection: MessageSelection<'_>) -> Result<Vec<Value>> {
        use rusqlite::types::Value as Sql;
        self.ensure_ready()?;
        let (clause, values, order, limit): (String, Vec<Sql>, &str, usize) = match selection {
            MessageSelection::Id(id) => {
                ("messages.id=?".into(), vec![Sql::Text(id.into())], "seq", 1)
            }
            MessageSelection::Lease(lease) => (
                "json_extract(value,'$.delivery.lease')=?".into(),
                vec![Sql::Text(lease.into())],
                "seq",
                8,
            ),
            MessageSelection::Context {
                scope,
                handled,
                limit,
            } => {
                if !(1..=32).contains(&limit) {
                    return Err(Error::Invalid("invalid context message limit"));
                }
                let (recipient, human, conversation) = scope.values();
                let clause = if human {
                    "(recipient=? OR recipient='0000000000000000') AND pending=?"
                } else {
                    "recipient=? AND (conversation IS NULL OR conversation=?) AND pending=?"
                };
                let mut values = vec![Sql::Text(recipient)];
                if !human {
                    values.push(conversation.map_or(Sql::Null, Sql::Text));
                }
                values.push(Sql::Integer(i64::from(!handled)));
                (
                    clause.into(),
                    values,
                    if handled { "seq DESC" } else { "seq" },
                    limit,
                )
            }
            MessageSelection::Notice {
                scope,
                interrupt_only,
                expired_hooks,
                now,
            } => {
                let eligible = "(json_extract(value,'$.delivery.outcome') IS NULL OR json_extract(value,'$.delivery.outcome') NOT IN ('queue-prepared','queued','unknown','queue-submitted','hook-prepared','hook-emitted','mcp-returned'))";
                let clause = format!(
                    "recipient=? AND (conversation IS NULL OR conversation=?) AND pending=1 AND json_extract(value,'$.native_surfaced') IS NULL AND ({eligible} OR (? AND json_extract(value,'$.delivery.outcome')='hook-prepared' AND json_extract(value,'$.delivery.expires')<?)) AND (NOT ? OR json_extract(original,'$.intent')='interrupt')"
                );
                (
                    clause,
                    vec![
                        Sql::Text(key(scope.workspace)),
                        scope
                            .conversation
                            .map_or(Sql::Null, |s| Sql::Text(s.into())),
                        Sql::Integer(expired_hooks.into()),
                        Sql::Integer(
                            i64::try_from(now)
                                .map_err(|_| Error::Invalid("invalid notice timestamp"))?,
                        ),
                        Sql::Integer(interrupt_only.into()),
                    ],
                    "seq",
                    8,
                )
            }
        };
        let sql = format!(
            "SELECT messages.id,original,value FROM messages JOIN lifecycle USING(id) WHERE {clause} ORDER BY {order} LIMIT {limit}"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, original, state) = row?;
            checked_message(&self.connection, &id, original, state)
        })
        .collect()
    }

    pub fn message_counts(&self, scope: MailboxScope<'_>) -> Result<(u64, u64)> {
        self.ensure_ready()?;
        let (recipient, human, conversation) = scope.values();
        let clause = if human {
            "(recipient=?1 OR recipient='0000000000000000')"
        } else {
            "recipient=?1 AND (conversation IS NULL OR conversation=?2)"
        };
        let sql = format!(
            "SELECT count(*),coalesce(sum(pending),0) FROM messages JOIN lifecycle USING(id) WHERE {clause}"
        );
        let values = if human {
            vec![Some(recipient)]
        } else {
            vec![Some(recipient), conversation]
        };
        let (total, pending): (i64, i64) =
            self.connection
                .query_row(&sql, rusqlite::params_from_iter(values), |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        Ok((
            total
                .try_into()
                .map_err(|_| Error::Invalid("invalid count"))?,
            pending
                .try_into()
                .map_err(|_| Error::Invalid("invalid count"))?,
        ))
    }

    /// At most one competing reservation is needed to enforce the existing
    /// same-run/conversation handoff guard. No original body is read.
    pub fn queued_handoff(&self, workspace: u64, conversation: &str, run: &str) -> Result<bool> {
        self.ensure_ready()?;
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM messages JOIN lifecycle USING(id)
            WHERE pending=1 AND json_extract(value,'$.native_surfaced') IS NULL
            AND json_extract(value,'$.delivery.outcome') IN ('queue-prepared','queued','unknown')
            AND (json_extract(value,'$.delivery.run')=? OR (recipient=? AND json_extract(value,'$.delivery.conversation')=?)))",
            params![run,key(workspace),conversation],|r|r.get(0))?)
    }

    /// Cursor advances through metadata only; retained handled bodies never enter
    /// the delivery loop. An empty page lets the caller wrap to the beginning.
    pub fn delivery_candidates(&self, after: Option<&str>, limit: usize) -> Result<Vec<String>> {
        self.ensure_ready()?;
        if !(1..=16).contains(&limit) {
            return Err(Error::Invalid("invalid delivery page limit"));
        }
        let seq = match after {
            None => 0,
            Some(id) => {
                self.connection
                    .query_row("SELECT seq FROM messages WHERE id=?", [id], |r| {
                        r.get::<_, i64>(0)
                    })?
            }
        };
        let mut statement=self.connection.prepare("SELECT messages.id FROM messages JOIN lifecycle USING(id)
            WHERE seq>? AND recipient!='0000000000000000' AND pending=1
            AND json_extract(value,'$.native_surfaced') IS NULL
            AND (json_extract(value,'$.delivery.outcome') IS NULL OR json_extract(value,'$.delivery.outcome') NOT IN ('queue-prepared','queued','unknown','queue-submitted','hook-prepared','hook-emitted','mcp-returned'))
            ORDER BY seq LIMIT ?")?;
        Ok(statement
            .query_map(params![seq, limit as i64], |r| r.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub fn select_documents(
        &self,
        kind: DocumentKind,
        selection: DocumentSelection<'_>,
    ) -> Result<Vec<WorkingDocument>> {
        use rusqlite::types::Value as Sql;
        self.ensure_ready()?;
        let (clause, values, order, limit): (&str, Vec<Sql>, &str, usize) = match selection {
            DocumentSelection::Id(id) => ("identity=?", vec![Sql::Text(id.into())], "ordinal", 1),
            DocumentSelection::Request { owner, request } => (
                "request_owner=? AND request_id=?",
                vec![Sql::Text(key(owner)), Sql::Text(request.into())],
                "ordinal",
                1,
            ),
            DocumentSelection::Recent { workspace, limit } => {
                if !(1..=64).contains(&limit) {
                    return Err(Error::Invalid("invalid document selection limit"));
                }
                (
                    "owner=?",
                    vec![Sql::Text(key(workspace))],
                    "ordinal DESC",
                    limit,
                )
            }
            DocumentSelection::ActiveDispatch { workspace } => (
                "owner=? AND phase IN ('prepared','reserved','hosted','started')",
                vec![Sql::Text(key(workspace))],
                "ordinal DESC",
                1,
            ),
            DocumentSelection::Assignment { workspace } => (
                "owner=? AND phase NOT IN ('cancelled','blocked')",
                vec![Sql::Text(key(workspace))],
                "ordinal DESC",
                1,
            ),
            DocumentSelection::ContextDecisions { workspace } => (
                "owner=?",
                vec![Sql::Text(key(workspace))],
                "pending DESC, CASE WHEN pending=1 THEN ordinal END, CASE WHEN pending=0 THEN ordinal END DESC",
                32,
            ),
        };
        let table = kind.table();
        let sql = format!(
            "SELECT identity,ordinal FROM documents WHERE kind=? AND {clause} ORDER BY {order} LIMIT {limit}"
        );
        let mut values = values;
        values.insert(0, Sql::Text(table.into()));
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut records = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let (id, ordinal) = row?;
            let value =
                crate::record_store::legacy::read_document(&self.connection, table, ordinal)?;
            bytes += serde_json::to_vec(&value)?.len();
            if bytes > 16 * 1024 * 1024 {
                return Err(Error::Invalid("working documents exceed byte bound"));
            }
            records.push(WorkingDocument { id, ordinal, value });
        }
        Ok(records)
    }

    pub fn checkpoint_append_ids(&self, count: usize) -> Result<Vec<String>> {
        self.ensure_ready()?;
        if count > 1 {
            return Err(Error::Invalid("one checkpoint per application mutation"));
        }
        let ordinal: i64 = self.connection.query_row(
            "SELECT coalesce(max(ordinal)+1,0) FROM documents WHERE kind='checkpoints'",
            [],
            |r| r.get(0),
        )?;
        Ok((0..count)
            .map(|n| format!("checkpoint:{}", ordinal + n as i64))
            .collect())
    }

    pub fn document_counts(&self, kind: DocumentKind, workspace: u64) -> Result<(u64, u64)> {
        self.ensure_ready()?;
        let (total, pending): (i64, i64) = self.connection.query_row(
            "SELECT count(*),coalesce(sum(pending),0) FROM documents WHERE kind=? AND owner=?",
            params![kind.table(), key(workspace)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((
            total
                .try_into()
                .map_err(|_| Error::Invalid("invalid count"))?,
            pending
                .try_into()
                .map_err(|_| Error::Invalid("invalid count"))?,
        ))
    }

    /// Cold startup loads layout and current observation state, never history.
    pub fn load_layout(&self) -> Result<Value> {
        self.ensure_ready()?;
        read_layout(&self.connection)
    }
}

fn list<'a>(root: &'a Value, field: &str) -> Result<&'a Vec<Value>> {
    root[field]
        .as_array()
        .ok_or(Error::Invalid("missing working-view list"))
}
fn keyed(values: &[Value]) -> Result<std::collections::BTreeMap<String, &Value>> {
    let mut result = std::collections::BTreeMap::new();
    if values.len() > 256 {
        return Err(Error::Invalid("working view exceeds record bound"));
    }
    for value in values {
        let id = value["id"]
            .as_str()
            .ok_or(Error::Invalid("missing working-view identity"))?;
        if result.insert(id.to_owned(), value).is_some() {
            return Err(Error::Invalid("duplicate working-view identity"));
        }
    }
    Ok(result)
}
fn immutable(value: &Value, fields: &[&str]) -> Result<Map<String, Value>> {
    let mut record = value
        .as_object()
        .ok_or(Error::Invalid("working record must be an object"))?
        .clone();
    for field in fields {
        record.remove(*field);
    }
    Ok(record)
}
pub(super) fn validate_document_update(kind: &str, before: &Value, after: &Value) -> Result<()> {
    let fields = match kind {
        "decisions" => &["answer"][..],
        "dispatches" => &[
            "phase",
            "detail",
            "session",
            "run",
            "started",
            "surfaced",
            "acknowledged",
        ][..],
        _ => return Err(Error::Invalid("document is immutable")),
    };
    if immutable(before, fields)? != immutable(after, fields)? {
        return Err(Error::Invalid("immutable document evidence changed"));
    }
    if kind == "decisions" && !before["answer"].is_null() && before["answer"] != after["answer"] {
        return Err(Error::Invalid("decision already answered"));
    }
    Ok(())
}
impl Store {
    /// Commit only changes to the explicitly loaded working view. Omitted
    /// history is retained. Before-values are checked against durable rows;
    /// missing legacy defaults never overwrite unknown original evidence.
    pub fn sync_working_view(
        &mut self,
        operation: &str,
        before: &Value,
        root: &Value,
        checkpoint_ids: &[String],
    ) -> CommitOutcome<()> {
        if self.blocked {
            return CommitOutcome::Unavailable;
        }
        let changes = match self.working_changes(before, root, checkpoint_ids) {
            Ok(changes) => changes,
            Err(error) => return CommitOutcome::Unchanged(error),
        };
        self.commit_changes(operation, changes)
    }
    fn working_changes(
        &self,
        before: &Value,
        root: &Value,
        checkpoint_ids: &[String],
    ) -> Result<Vec<crate::record_store::snapshot::Change>> {
        use crate::record_store::snapshot::Change;
        let mut changes = self.layout_changes(root)?;
        let next = &root["coordination"];
        let old = keyed(list(before, "messages")?)?;
        let new = keyed(list(next, "messages")?)?;
        if old.keys().any(|id| !new.contains_key(id)) {
            return Err(Error::Invalid("working view cannot remove messages"));
        }
        for value in list(next, "messages")? {
            let id = value["id"].as_str().unwrap().to_owned();
            let prepared = PreparedMessage::new(value)?;
            if let Some(expected) = old.get(&id) {
                if immutable(expected, &STATE_FIELDS)? != immutable(value, &STATE_FIELDS)? {
                    return Err(Error::Invalid("immutable message evidence changed"));
                }
                if *expected == value {
                    continue;
                }
                let state: String = self.connection.query_row(
                    "SELECT value FROM lifecycle WHERE id=?",
                    [&id],
                    |r| r.get(0),
                )?;
                let mut actual: Value = serde_json::from_str(&state)?;
                for field in STATE_FIELDS {
                    if actual[field] != expected[field] {
                        return Err(Error::Invalid("message changed since working read"));
                    }
                    if expected[field] != value[field] {
                        actual[field] = value[field].clone();
                    }
                }
                changes.push(Change::Lifecycle {
                    id,
                    before: state,
                    after: serde_json::to_string(&actual)?,
                });
            } else {
                changes.push(Change::NewMessage(prepared));
            }
        }
        for kind in ["decisions", "dispatches"] {
            let old = keyed(list(before, kind)?)?;
            let new = keyed(list(next, kind)?)?;
            if old.keys().any(|id| !new.contains_key(id)) {
                return Err(Error::Invalid("working view cannot remove documents"));
            }
            let mut ordinal: i64 = self.connection.query_row(
                "SELECT coalesce(max(ordinal)+1,0) FROM documents WHERE kind=?",
                [kind],
                |r| r.get(0),
            )?;
            for value in list(next, kind)? {
                let id = value["id"].as_str().unwrap().to_owned();
                let bound = if old.contains_key(&id) {
                    crate::record_store::legacy::SOURCE_LIMIT
                } else {
                    MAX_RECORD
                };
                if serde_json::to_vec(value)?.len() > bound {
                    return Err(Error::Invalid("document exceeds bound"));
                }
                if let Some(expected) = old.get(&id) {
                    if *expected == value {
                        continue;
                    }
                    validate_document_update(kind, expected, value)?;
                    let n: i64 = self.connection.query_row(
                        "SELECT ordinal FROM documents WHERE kind=? AND identity=?",
                        params![kind, id],
                        |r| r.get(0),
                    )?;
                    let actual =
                        crate::record_store::legacy::read_document(&self.connection, kind, n)?;
                    // Compare the known typed fields; retain fields from legacy
                    // versions that the current application does not interpret.
                    for (field, previous) in expected
                        .as_object()
                        .ok_or(Error::Invalid("invalid expected document"))?
                    {
                        if actual[field] != *previous {
                            return Err(Error::Invalid("document changed since working read"));
                        }
                    }
                    let mut merged = actual
                        .as_object()
                        .ok_or(Error::Invalid("invalid durable document"))?
                        .clone();
                    merged.extend(
                        value
                            .as_object()
                            .ok_or(Error::Invalid("invalid document"))?
                            .clone(),
                    );
                    changes.push(Change::Document {
                        kind,
                        ordinal: n,
                        id: Some(id),
                        owner: value["workspace"].as_u64().map(key),
                        before: Some(actual),
                        after: Value::Object(merged),
                    });
                } else {
                    changes.push(Change::Document {
                        kind,
                        ordinal,
                        id: Some(id),
                        owner: value["workspace"].as_u64().map(key),
                        before: None,
                        after: value.clone(),
                    });
                    ordinal += 1;
                }
            }
        }
        let old = list(before, "checkpoints")?;
        let new = list(next, "checkpoints")?;
        if old.len() != checkpoint_ids.len()
            || old.len() > 64
            || new.len() > 65
            || !new.starts_with(old)
        {
            return Err(Error::Invalid(
                "working checkpoints must preserve their originals",
            ));
        }
        for (id, expected) in checkpoint_ids.iter().zip(old) {
            let n: i64 = self.connection.query_row(
                "SELECT ordinal FROM documents WHERE kind='checkpoints' AND identity=?",
                [id],
                |r| r.get(0),
            )?;
            if crate::record_store::legacy::read_document(&self.connection, "checkpoints", n)?
                != *expected
            {
                return Err(Error::Invalid("checkpoint changed since working read"));
            }
        }
        let ordinal: i64 = self.connection.query_row(
            "SELECT coalesce(max(ordinal)+1,0) FROM documents WHERE kind='checkpoints'",
            [],
            |r| r.get(0),
        )?;
        for (offset, value) in new[old.len()..].iter().enumerate() {
            let ordinal = ordinal + offset as i64;
            if serde_json::to_vec(value)?.len() > MAX_RECORD {
                return Err(Error::Invalid("checkpoint exceeds bound"));
            }
            changes.push(Change::Document {
                kind: "checkpoints",
                ordinal,
                id: Some(format!("checkpoint:{ordinal}")),
                owner: value["workspace"].as_u64().map(key),
                before: None,
                after: value.clone(),
            });
        }
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_store::tests::{Temp, message};
    fn initial() -> Value {
        json!({"version":10,"active":2,"workspaces":[{"id":2,"meta":{"status":"in-progress"}}],"coordination":{"messages":[],"decisions":[],"checkpoints":[],"dispatches":[],"delivery":{"hooks":[],"focus":[]}}})
    }
    fn operation(n: u64) -> String {
        format!("{n:032x}")
    }
    #[test]
    fn bounded_view_updates_preserve_omitted_history_and_cold_start_loads_no_bodies() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        store
            .import_legacy(&serde_json::to_vec(&initial()).unwrap(), None)
            .unwrap();
        for batch in 0..18 {
            let records = (0..50)
                .map(|n| {
                    let mut m = message(batch * 50 + n, 2, None);
                    m["body"] = json!("x".repeat(16384));
                    m
                })
                .collect::<Vec<_>>();
            store.insert(&records).unwrap();
        }
        let bytes: i64 = store
            .connection
            .query_row("SELECT sum(length(original)) FROM messages", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(bytes > 8 * 1024 * 1024);
        let mut view = store.load_layout().unwrap();
        assert_eq!(view["coordination"]["messages"], json!([]));
        assert!(serde_json::to_vec(&view).unwrap().len() < 1024);
        let id = operation(700);
        view["coordination"]["messages"] =
            json!(store.select_messages(MessageSelection::Id(&id)).unwrap());
        let before = view["coordination"].clone();
        view["coordination"]["messages"][0]["acknowledged"] = json!(42);
        view["coordination"]["checkpoints"] =
            json!([{"workspace":2,"body":"exact result","time":42}]);
        view["workspaces"][0]["meta"]["status"] = json!("needs-me");
        assert!(matches!(
            store.sync_working_view(&operation(9000), &before, &view, &[]),
            CommitOutcome::Committed(())
        ));
        assert_eq!(
            store
                .message_counts(MailboxScope::Human { workspace: 2 })
                .unwrap(),
            (900, 899)
        );
        assert_eq!(
            store
                .get(
                    &operation(800),
                    Scope {
                        workspace: 2,
                        conversation: None
                    }
                )
                .unwrap()
                .unwrap()["body"],
            "x".repeat(16384)
        );
        drop(store);
        let store = Store::open(&dir.0).unwrap();
        let layout = store.load_layout().unwrap();
        assert_eq!(layout["coordination"]["messages"], json!([]));
        assert_eq!(layout["coordination"]["checkpoints"], json!([]));
        assert_eq!(layout["workspaces"][0]["meta"]["status"], "needs-me");
        assert_eq!(
            store
                .document_by_id(DocumentKind::Checkpoint, 2, "checkpoint:0")
                .unwrap()["checkpoint"]["body"],
            "exact result"
        );
        assert_eq!(
            store
                .get(
                    &id,
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
    fn working_view_failure_rolls_back_receipt_result_and_status_together() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let mut root = initial();
        root["coordination"]["messages"] = json!([message(1, 2, None)]);
        store
            .import_legacy(&serde_json::to_vec(&root).unwrap(), None)
            .unwrap();
        let before = root["coordination"].clone();
        root["coordination"]["messages"][0]["acknowledged"] = json!(42);
        root["workspaces"][0]["meta"]["status"] = json!("needs-me");
        root["coordination"]["checkpoints"] = json!([{"workspace":2,"body":"must be atomic"}]);
        store.connection.execute_batch("CREATE TRIGGER reject_result BEFORE INSERT ON documents WHEN NEW.kind='checkpoints' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
        assert!(matches!(
            store.sync_working_view(&operation(9), &before, &root, &[]),
            CommitOutcome::Unchanged(_)
        ));
        drop(store);
        let store = Store::open(&dir.0).unwrap();
        assert_eq!(
            store.load_layout().unwrap()["workspaces"][0]["meta"]["status"],
            "in-progress"
        );
        assert_eq!(
            store.document_counts(DocumentKind::Checkpoint, 2).unwrap(),
            (0, 0)
        );
        assert!(
            store
                .get(
                    &operation(1),
                    Scope {
                        workspace: 2,
                        conversation: None
                    }
                )
                .unwrap()
                .unwrap()["acknowledged"]
                .is_null()
        );
    }
    #[test]
    fn selectors_keep_conversations_scoped_and_advance_past_blocked_handoffs() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        store
            .import_legacy(&serde_json::to_vec(&initial()).unwrap(), None)
            .unwrap();
        let mut records = vec![
            message(1, 2, None),
            message(2, 2, Some("chat-a")),
            message(3, 2, Some("chat-b")),
            message(4, 0, None),
        ];
        records[1]["delivery"] =
            json!({"outcome":"queued","run":"run-a","conversation":"chat-a","lease":"owned-lease"});
        store.insert(&records).unwrap();
        let scope = Scope {
            workspace: 2,
            conversation: Some("chat-a"),
        };
        let values = store
            .select_messages(MessageSelection::Context {
                scope: MailboxScope::Native(scope),
                handled: false,
                limit: 32,
            })
            .unwrap();
        assert_eq!(
            values.iter().map(|m| m["id"].clone()).collect::<Vec<_>>(),
            vec![json!(operation(1)), json!(operation(2))]
        );
        assert_eq!(
            store
                .message_counts(MailboxScope::Human { workspace: 2 })
                .unwrap(),
            (4, 4)
        );
        let notice = store
            .select_messages(MessageSelection::Notice {
                scope,
                interrupt_only: false,
                expired_hooks: false,
                now: 100,
            })
            .unwrap();
        assert_eq!(notice.len(), 1);
        assert_eq!(notice[0]["id"], operation(1));
        assert!(store.queued_handoff(2, "chat-a", "new-run").unwrap());
        assert!(!store.queued_handoff(2, "chat-b", "new-run").unwrap());
        assert_eq!(store.delivery_candidates(None, 1).unwrap(), [operation(1)]);
        assert_eq!(
            store.delivery_candidates(Some(&operation(1)), 1).unwrap(),
            [operation(3)]
        );
        assert!(
            store
                .delivery_candidates(Some(&operation(3)), 1)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .select_messages(MessageSelection::Lease("owned-lease"))
                .unwrap(),
            [records[1].clone()]
        );
    }
    #[test]
    fn working_view_retains_unknown_legacy_evidence_and_rejects_stale_or_rewritten_records() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let mut root = initial();
        let mut original = message(1, 2, None);
        original["legacy_evidence"] = json!("retain exact original");
        original.as_object_mut().unwrap().remove("native_surfaced");
        root["coordination"]["messages"] = json!([original.clone()]);
        store
            .import_legacy(&serde_json::to_vec(&root).unwrap(), None)
            .unwrap();
        let mut view = root.clone();
        view["coordination"]["messages"][0]
            .as_object_mut()
            .unwrap()
            .remove("legacy_evidence");
        view["coordination"]["messages"][0]["native_surfaced"] = Value::Null;
        let before = view["coordination"].clone();
        view["coordination"]["messages"][0]["acknowledged"] = json!(1);
        assert!(matches!(
            store.sync_working_view(&operation(2), &before, &view, &[]),
            CommitOutcome::Committed(())
        ));
        let current = store
            .get(
                &operation(1),
                Scope {
                    workspace: 2,
                    conversation: None,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(current["legacy_evidence"], original["legacy_evidence"]);
        assert!(current.get("native_surfaced").is_none());
        view["coordination"]["messages"][0]["acknowledged"] = json!(2);
        assert!(matches!(
            store.sync_working_view(&operation(3), &before, &view, &[]),
            CommitOutcome::Unchanged(_)
        ));
        view["coordination"]["messages"][0]["body"] = json!("rewritten");
        assert!(matches!(
            store.sync_working_view(&operation(4), &before, &view, &[]),
            CommitOutcome::Unchanged(_)
        ));
    }
}

/// A single read snapshot prevents mixing old header/layout with new delivery state.
pub(super) fn read_layout(db: &Connection) -> Result<Value> {
    let tx = db.unchecked_transaction()?;
    let mut root = crate::record_store::legacy::read_document(&tx, "header", 0)?;
    root["workspaces"] = json!(crate::record_store::snapshot::workspaces(&tx)?);
    root["coordination"]["delivery"] =
        crate::record_store::legacy::read_document(&tx, "delivery", 0)?;
    if serde_json::to_vec(&root)?.len() > 8 * 1024 * 1024 {
        return Err(Error::Invalid("workspace layout exceeds 8 MiB"));
    }
    tx.commit()?;
    Ok(root)
}

impl Store {
    pub fn has_notice_candidate(&self, workspace: u64) -> Result<bool> {
        self.ensure_ready()?;
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM messages JOIN lifecycle USING(id) WHERE recipient=? AND pending=1 AND json_extract(value,'$.native_surfaced') IS NULL AND (json_extract(value,'$.delivery.outcome') IS NULL OR json_extract(value,'$.delivery.outcome') NOT IN ('queue-prepared','queued','unknown','queue-submitted','hook-prepared','hook-emitted','mcp-returned')))",[key(workspace)],|r|r.get(0))?)
    }
}

#[cfg(test)]
mod final_controls {
    use super::*;
    use crate::record_store::tests::{Temp, message};
    fn initial() -> Value {
        json!({"version":10,"active":2,"workspaces":[{"id":2,"meta":{"status":"in-progress"},"tabs":[]}],"coordination":{"messages":[],"decisions":[],"checkpoints":[],"dispatches":[],"delivery":{"hooks":[],"focus":[]}}})
    }
    #[test]
    fn readonly_layout_observes_commits_without_creation_and_rejects_auxiliary_symlinks() {
        let dir = Temp::new();
        assert!(Store::read_layout(&dir.0).is_err());
        assert!(!dir.0.join("state.sqlite").exists());
        let mut store = Store::open(&dir.0).unwrap();
        let mut root = initial();
        store
            .import_legacy(&serde_json::to_vec(&root).unwrap(), None)
            .unwrap();
        assert_eq!(Store::read_layout(&dir.0).unwrap(), root);
        root["active"] = json!(0);
        root["workspaces"][0]["meta"]["status"] = json!("needs-me");
        assert!(matches!(
            store.sync_snapshot(&format!("{:032x}", 1), &root),
            CommitOutcome::Committed(())
        ));
        assert_eq!(Store::read_layout(&dir.0).unwrap(), root);
        drop(store);
        std::os::unix::fs::symlink("missing", dir.0.join("state.sqlite-wal")).unwrap();
        assert!(Store::read_layout(&dir.0).is_err());
    }
    #[test]
    fn corrupt_message_scope_or_lifecycle_never_returns_or_acknowledges_a_body() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let record = message(1, 2, Some("a"));
        store.insert(std::slice::from_ref(&record)).unwrap();
        let id = record["id"].as_str().unwrap();
        store
            .connection
            .execute(
                "UPDATE messages SET recipient=? WHERE id=?",
                params![key(3), id],
            )
            .unwrap();
        assert!(
            store
                .get(
                    id,
                    Scope {
                        workspace: 3,
                        conversation: Some("a")
                    }
                )
                .is_err()
        );
        assert!(
            store
                .acknowledge(
                    Scope {
                        workspace: 3,
                        conversation: Some("a")
                    },
                    &[id],
                    42
                )
                .is_err()
        );
        let state: String = store
            .connection
            .query_row("SELECT value FROM lifecycle WHERE id=?", [id], |r| r.get(0))
            .unwrap();
        assert!(serde_json::from_str::<Value>(&state).unwrap()["acknowledged"].is_null());
        store
            .connection
            .execute(
                "UPDATE messages SET recipient=? WHERE id=?",
                params![key(2), id],
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE lifecycle SET value=json_set(value,'$.body','forged') WHERE id=?",
                [id],
            )
            .unwrap();
        assert!(
            store
                .get(
                    id,
                    Scope {
                        workspace: 2,
                        conversation: Some("a")
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn corrupt_document_owner_is_not_a_scope_grant() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        store
            .import_legacy(&serde_json::to_vec(&initial()).unwrap(), None)
            .unwrap();
        store
            .append_document(
                DocumentKind::Decision,
                2,
                &json!({"id":"decision-a","workspace":2,"question":"synthetic","answer":null}),
                None,
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE documents SET owner=? WHERE kind='decisions'",
                [key(3)],
            )
            .unwrap();
        assert!(
            store
                .document_by_id(DocumentKind::Decision, 3, "decision-a")
                .is_err()
        );
        assert!(
            store
                .select_documents(
                    DocumentKind::Decision,
                    DocumentSelection::Recent {
                        workspace: 3,
                        limit: 8
                    }
                )
                .is_err()
        );
    }
}

impl Store {
    pub fn has_bound_mail(&self, workspace: u64) -> Result<bool> {
        self.ensure_ready()?;
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE recipient=?1 AND conversation IS NOT NULL UNION ALL SELECT 1 FROM messages WHERE sender=?1 AND sender_conversation IS NOT NULL)",[key(workspace)],|r|r.get(0))?)
    }
}

#[cfg(test)]
mod imported_document_tests {
    use super::*;
    use crate::record_store::tests::Temp;
    #[test]
    fn accepted_large_original_can_be_answered_and_remains_unchanged_in_other_mutations() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let question = "synthetic retained evidence ".repeat(9000);
        let mut root = json!({"version":10,"active":2,"workspaces":[{"id":2,"meta":{"status":"in-progress"}}],"coordination":{"messages":[],"decisions":[{"id":"decision-a","workspace":2,"question":question,"answer":null}],"checkpoints":[],"dispatches":[],"delivery":{"hooks":[],"focus":[]}}});
        store
            .import_legacy(&serde_json::to_vec(&root).unwrap(), None)
            .unwrap();
        let before = root["coordination"].clone();
        root["workspaces"][0]["meta"]["status"] = json!("needs-me");
        assert!(matches!(
            store.sync_working_view(&format!("{:032x}", 1), &before, &root, &[]),
            CommitOutcome::Committed(())
        ));
        root["coordination"]["decisions"][0]["answer"] = json!("Synthetic human answer");
        assert!(matches!(
            store.sync_working_view(&format!("{:032x}", 2), &before, &root, &[]),
            CommitOutcome::Committed(())
        ));
        let retained = store
            .document_by_id(DocumentKind::Decision, 2, "decision-a")
            .unwrap();
        assert_eq!(retained["question"], question);
        assert_eq!(retained["answer"], "Synthetic human answer");
    }
}
