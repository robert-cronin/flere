//! Bounded indexed access to retained coordination documents.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentKind {
    Decision,
    Checkpoint,
    Dispatch,
}
impl DocumentKind {
    pub(super) fn table(self) -> &'static str {
        match self {
            Self::Decision => "decisions",
            Self::Checkpoint => "checkpoints",
            Self::Dispatch => "dispatches",
        }
    }
}
#[derive(Debug)]
pub struct DocumentPage {
    /// Checkpoints include the stable global reference, matching existing history.
    pub records: Vec<Value>,
    pub total: u64,
    pub pending: u64,
    pub remaining: u64,
    pub next_after: Option<String>,
}
fn ordinal(db: &Connection, kind: DocumentKind, workspace: u64, id: &str) -> Result<i64> {
    db.query_row(
        "SELECT ordinal FROM documents WHERE kind=? AND owner=? AND identity=?",
        params![kind.table(), key(workspace), id],
        |r| r.get(0),
    )
    .optional()?
    .ok_or(Error::Invalid(
        "document identity is outside this workspace",
    ))
}
fn record_value(kind: DocumentKind, id: &str, record: Value) -> Value {
    if kind == DocumentKind::Checkpoint {
        json!({"id":id,"checkpoint":record})
    } else {
        record
    }
}
impl Store {
    pub fn document_by_id(&self, kind: DocumentKind, workspace: u64, id: &str) -> Result<Value> {
        self.ensure_ready()?;
        let tx = self.connection.unchecked_transaction()?;
        let n = ordinal(&tx, kind, workspace, id)?;
        Ok(record_value(
            kind,
            id,
            crate::record_store::legacy::read_document(&tx, kind.table(), n)?,
        ))
    }
    pub fn documents_page(
        &self,
        kind: DocumentKind,
        workspace: u64,
        after: Option<&str>,
        limit: usize,
    ) -> Result<DocumentPage> {
        self.ensure_ready()?;
        if !(1..=32).contains(&limit) {
            return Err(Error::Invalid("document page exceeds bound"));
        }
        let tx = self.connection.unchecked_transaction()?;
        let after = after
            .map(|id| ordinal(&tx, kind, workspace, id))
            .transpose()?
            .unwrap_or(-1);
        let (total,pending,following):(i64,i64,i64)=tx.query_row(
            "SELECT count(*),coalesce(sum(pending),0),coalesce(sum(ordinal>?3),0) FROM documents WHERE kind=?1 AND owner=?2",
            params![kind.table(),key(workspace),after], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let mut statement=tx.prepare("SELECT ordinal,identity FROM documents WHERE kind=? AND owner=? AND ordinal>? ORDER BY ordinal LIMIT ?")?;
        let mut rows =
            statement.query(params![kind.table(), key(workspace), after, limit as i64])?;
        let mut records = Vec::new();
        let mut bytes = 0usize;
        let mut last = None;
        while let Some(row) = rows.next()? {
            let n: i64 = row.get(0)?;
            let id: String = row.get(1)?;
            let value = record_value(
                kind,
                &id,
                crate::record_store::legacy::read_document(&tx, kind.table(), n)?,
            );
            let size = serde_json::to_vec(&value)?.len();
            if !records.is_empty() && bytes + size > 32 * 1024 {
                break;
            }
            records.push(value);
            bytes += size;
            last = Some(id);
        }
        let remaining = u64::try_from(following)
            .map_err(|_| Error::Invalid("invalid document count"))?
            .checked_sub(records.len() as u64)
            .ok_or(Error::Invalid("document count underflow"))?;
        Ok(DocumentPage {
            records,
            total: total as u64,
            pending: pending as u64,
            remaining,
            next_after: if remaining > 0 { last } else { None },
        })
    }
    /// Append one record and optional workspace status as one logical mutation.
    /// Application domain/native validation must precede this storage operation.
    pub fn append_document(
        &mut self,
        kind: DocumentKind,
        workspace: u64,
        value: &Value,
        status: Option<&str>,
    ) -> Result<String> {
        if value["workspace"].as_u64() != Some(workspace) || !value.is_object() {
            return Err(Error::Invalid("document owner differs"));
        }
        if serde_json::to_vec(value)?.len() > MAX_RECORD {
            return Err(Error::Invalid("document exceeds bound"));
        }
        self.transact(|tx| {
            let next: i64 = tx.query_row(
                "SELECT coalesce(max(ordinal)+1,0) FROM documents WHERE kind=?",
                [kind.table()],
                |r| r.get(0),
            )?;
            let identity = if kind == DocumentKind::Checkpoint {
                format!("checkpoint:{next}")
            } else {
                value["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .ok_or(Error::Invalid("missing document identity"))?
                    .to_owned()
            };
            crate::record_store::legacy::put_document(
                tx,
                kind.table(),
                next,
                Some(&identity),
                Some(&key(workspace)),
                value,
            )?;
            if let Some(status) = status {
                let rows = tx.execute(
                    "UPDATE workspace SET status=? WHERE id=?",
                    params![status, key(workspace)],
                )?;
                if rows != 1 {
                    return Err(Error::Invalid("unknown workspace for status update"));
                }
            }
            Ok(identity)
        })
    }
    /// Compare-and-set mutable fields without replacing original evidence.
    pub fn update_document(
        &mut self,
        kind: DocumentKind,
        workspace: u64,
        id: &str,
        expected: &Value,
        next: &Value,
    ) -> Result<()> {
        if kind == DocumentKind::Checkpoint {
            return Err(Error::Invalid("checkpoints are immutable"));
        }
        if serde_json::to_vec(next)?.len() > MAX_RECORD {
            return Err(Error::Invalid("document exceeds bound"));
        }
        let mutable = match kind {
            DocumentKind::Decision => &["answer"][..],
            DocumentKind::Dispatch => &[
                "phase",
                "detail",
                "session",
                "run",
                "started",
                "surfaced",
                "acknowledged",
            ][..],
            DocumentKind::Checkpoint => unreachable!(),
        };
        let mut old_original = expected
            .as_object()
            .ok_or(Error::Invalid("expected document must be an object"))?
            .clone();
        let mut new_original = next
            .as_object()
            .ok_or(Error::Invalid("next document must be an object"))?
            .clone();
        for field in mutable {
            old_original.remove(*field);
            new_original.remove(*field);
        }
        if old_original != new_original {
            return Err(Error::Invalid("immutable document evidence changed"));
        }
        if kind == DocumentKind::Decision
            && !expected["answer"].is_null()
            && expected["answer"] != next["answer"]
        {
            return Err(Error::Invalid("decision already answered"));
        }
        self.transact(|tx| {
            let n = ordinal(tx, kind, workspace, id)?;
            if crate::record_store::legacy::read_document(tx, kind.table(), n)? != *expected {
                return Err(Error::Invalid("document compare-and-set failed"));
            }
            if expected == next {
                return Ok(());
            }
            tx.execute(
                "DELETE FROM document_chunks WHERE kind=? AND ordinal=?",
                params![kind.table(), n],
            )?;
            tx.execute(
                "DELETE FROM documents WHERE kind=? AND ordinal=?",
                params![kind.table(), n],
            )?;
            crate::record_store::legacy::put_document(
                tx,
                kind.table(),
                n,
                Some(id),
                Some(&key(workspace)),
                next,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_store::tests::Temp;
    fn fixture() -> (Temp, Store) {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let source = json!({"version":10,"active":2,"workspaces":[{"id":2,"meta":{"status":"in-progress"}},{"id":3,"meta":{}}],"coordination":{}});
        store
            .import_legacy(&serde_json::to_vec(&source).unwrap(), None)
            .unwrap();
        (dir, store)
    }
    #[test]
    fn scoped_history_is_bounded_and_references_survive_growth_and_reopen() {
        let (dir, mut store) = fixture();
        for n in 0..70 {
            store
                .append_document(
                    DocumentKind::Checkpoint,
                    if n % 2 == 0 { 2 } else { 3 },
                    &json!({"workspace":if n%2==0 {2}else{3},"body":format!("exact-{n}")}),
                    None,
                )
                .unwrap();
        }
        let first = store
            .documents_page(DocumentKind::Checkpoint, 2, None, 32)
            .unwrap();
        assert_eq!(
            (first.total, first.remaining, first.records.len()),
            (35, 3, 32)
        );
        assert_eq!(first.next_after.as_deref(), Some("checkpoint:62"));
        drop(store);
        let mut store = Store::open(&dir.0).unwrap();
        store
            .append_document(
                DocumentKind::Checkpoint,
                2,
                &json!({"workspace":2,"body":"late"}),
                None,
            )
            .unwrap();
        let next = store
            .documents_page(DocumentKind::Checkpoint, 2, first.next_after.as_deref(), 32)
            .unwrap();
        assert_eq!((next.total, next.remaining, next.records.len()), (36, 0, 4));
        assert_eq!(next.records[0]["id"], "checkpoint:64");
        assert_eq!(next.records[3]["checkpoint"]["body"], "late");
        assert!(
            store
                .document_by_id(DocumentKind::Checkpoint, 3, "checkpoint:64")
                .is_err()
        );
        assert!(
            store
                .documents_page(DocumentKind::Checkpoint, 3, Some("checkpoint:64"), 8)
                .is_err()
        );
        assert!(
            store
                .documents_page(DocumentKind::Checkpoint, 2, None, 33)
                .is_err()
        );
    }
    #[test]
    fn result_status_commit_and_rollback_together() {
        let (_dir, mut store) = fixture();
        store.connection.execute_batch("CREATE TRIGGER fail_status BEFORE UPDATE ON workspace BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
        assert!(
            store
                .append_document(
                    DocumentKind::Checkpoint,
                    2,
                    &json!({"workspace":2,"body":"exact result"}),
                    Some("review")
                )
                .is_err()
        );
        assert_eq!(
            store
                .documents_page(DocumentKind::Checkpoint, 2, None, 8)
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            store.export_legacy_canonical().unwrap()["workspaces"][0]["meta"]["status"],
            "in-progress"
        );
        store
            .connection
            .execute_batch("DROP TRIGGER fail_status")
            .unwrap();
        assert_eq!(
            store
                .append_document(
                    DocumentKind::Checkpoint,
                    2,
                    &json!({"workspace":2,"body":"exact result"}),
                    Some("review")
                )
                .unwrap(),
            "checkpoint:0"
        );
        let exported = store.export_legacy_canonical().unwrap();
        assert_eq!(
            exported["coordination"]["checkpoints"][0]["body"],
            "exact result"
        );
        assert_eq!(exported["workspaces"][0]["meta"]["status"], "review");
    }
    #[test]
    fn decision_answer_preserves_evidence_and_rejects_stale_or_rewritten_originals() {
        let (_dir, mut store) = fixture();
        let original = json!({"id":"decision-1","workspace":2,"question":"Which limit?","recommendation":"8","evidence":"exact evidence","answer":null});
        store
            .append_document(DocumentKind::Decision, 2, &original, None)
            .unwrap();
        assert_eq!(
            store
                .documents_page(DocumentKind::Decision, 2, None, 8)
                .unwrap()
                .pending,
            1
        );
        let mut answered = original.clone();
        answered["answer"] = json!("5");
        store
            .update_document(
                DocumentKind::Decision,
                2,
                "decision-1",
                &original,
                &answered,
            )
            .unwrap();
        assert_eq!(
            store
                .documents_page(DocumentKind::Decision, 2, None, 8)
                .unwrap()
                .pending,
            0
        );
        assert!(
            store
                .update_document(
                    DocumentKind::Decision,
                    2,
                    "decision-1",
                    &original,
                    &answered
                )
                .is_err()
        );
        let mut wrong = answered.clone();
        wrong["evidence"] = json!("replacement summary");
        assert!(
            store
                .update_document(DocumentKind::Decision, 2, "decision-1", &answered, &wrong)
                .is_err()
        );
        store
            .update_document(
                DocumentKind::Decision,
                2,
                "decision-1",
                &answered,
                &answered,
            )
            .unwrap();
        assert_eq!(
            store
                .document_by_id(DocumentKind::Decision, 2, "decision-1")
                .unwrap(),
            answered
        );
    }
    #[test]
    fn dispatch_updates_keep_assignment_request_and_run_evidence() {
        let (_dir, mut store) = fixture();
        let original = json!({"id":"dispatch-1","workspace":2,"request_id":"exact-request","assignment":"exact assignment","user_request":"keep local","phase":"prepared","run":"","session":0});
        store
            .append_document(DocumentKind::Dispatch, 2, &original, None)
            .unwrap();
        let mut next = original.clone();
        next["phase"] = json!("started");
        next["run"] = json!("verified-run");
        next["session"] = json!(9);
        store
            .update_document(DocumentKind::Dispatch, 2, "dispatch-1", &original, &next)
            .unwrap();
        let mut wrong = next.clone();
        wrong["assignment"] = json!("summary");
        assert!(
            store
                .update_document(DocumentKind::Dispatch, 2, "dispatch-1", &next, &wrong)
                .is_err()
        );
        assert_eq!(
            store
                .documents_page(DocumentKind::Dispatch, 2, None, 8)
                .unwrap()
                .records,
            vec![next]
        );
    }
    #[test]
    fn page_byte_limit_preserves_whole_oversized_first_record() {
        let (_dir, mut store) = fixture();
        let large = json!({"workspace":2,"body":"\0".repeat(12000)});
        store
            .append_document(DocumentKind::Checkpoint, 2, &large, None)
            .unwrap();
        store
            .append_document(
                DocumentKind::Checkpoint,
                2,
                &json!({"workspace":2,"body":"later"}),
                None,
            )
            .unwrap();
        let page = store
            .documents_page(DocumentKind::Checkpoint, 2, None, 8)
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0]["checkpoint"], large);
        assert_eq!(page.remaining, 1);
        assert_eq!(page.next_after.as_deref(), Some("checkpoint:0"));
    }
}
