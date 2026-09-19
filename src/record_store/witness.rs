//! Commit witnesses are storage identities, not native authorization credentials.
use super::*;

#[derive(Clone, Debug)]
pub struct OperationTicket {
    id: String,
    intent: String,
    store: String,
    file: (u64, u64),
    reconcilable: bool,
}
impl OperationTicket {
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Debug)]
pub enum CommitOutcome<T> {
    Committed(T),
    /// This ID and exact intent already committed; do not rerun the action.
    AlreadyCommitted,
    Unchanged(Error),
    Uncertain {
        ticket: OperationTicket,
        tentative: Option<T>,
        cause: Error,
    },
    /// A previous operation is unresolved. This new operation did not run.
    Unavailable,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    Committed,
    Unchanged,
}
pub struct Recovery {
    pub store: Store,
    pub resolution: Resolution,
}
#[derive(Debug)]
pub struct RecoveryFailure {
    pub ticket: OperationTicket,
    pub cause: Error,
}

impl Store {
    pub(super) fn ensure_ready(&self) -> Result<()> {
        if self.blocked {
            Err(Error::Invalid("store awaits commit reconciliation"))
        } else {
            Ok(())
        }
    }
    pub(super) fn operation_ticket(&self, id: &str, intent: String) -> Result<OperationTicket> {
        if !id_valid(id) || intent.len() > 192 * 1024 {
            return Err(Error::Invalid("invalid operation identity or intent"));
        }
        Ok(OperationTicket {
            id: id.into(),
            intent,
            store: self.identity.clone(),
            file: self.file_identity,
            reconcilable: true,
        })
    }
    pub fn inbox_witnessed(
        &mut self,
        id: &str,
        request: InboxRequest<'_>,
    ) -> CommitOutcome<InboxReceipt> {
        self.inbox_validated_witnessed(id, request, &|_| Ok(()))
    }
    /// Application validation runs within the same transaction and before any
    /// success receipt. A rejected ACK or page rolls back all earlier changes.
    pub fn inbox_validated_witnessed(
        &mut self,
        id: &str,
        request: InboxRequest<'_>,
        validate: &dyn Fn(&Value) -> Result<()>,
    ) -> CommitOutcome<InboxReceipt> {
        if self.blocked {
            return CommitOutcome::Unavailable;
        }
        if !id_valid(id)
            || !(1..=32).contains(&request.limit)
            || request.acknowledge.len() > 4096
            || request.acknowledge.iter().any(|id| !id_valid(id))
            || request.after.is_some_and(|id| !id_valid(id))
        {
            return CommitOutcome::Unchanged(Error::Invalid("invalid witnessed inbox request"));
        }
        let (workspace, human, conversation) = request.scope.values();
        let intent = match serde_json::to_string(
            &json!({"operation":"inbox","workspace":workspace,"human":human,"conversation":conversation,
            "include_acknowledged":request.include_acknowledged,"after":request.after,"limit":request.limit,"acknowledge":request.acknowledge,
            "return_messages":request.return_messages,"timestamp":request.timestamp}),
        ) {
            Ok(value) if value.len() <= 192 * 1024 => value,
            Ok(_) => {
                return CommitOutcome::Unchanged(Error::Invalid("operation intent exceeds bound"));
            }
            Err(e) => return CommitOutcome::Unchanged(e.into()),
        };
        let ticket = OperationTicket {
            id: id.into(),
            intent,
            store: self.identity.clone(),
            file: self.file_identity,
            reconcilable: true,
        };
        self.witnessed(ticket, |db| {
            crate::record_store::inbox::inbox_validated(db, &request, validate)
        })
    }
    pub(super) fn witnessed<T>(
        &mut self,
        ticket: OperationTicket,
        action: impl FnOnce(&Connection) -> Result<T>,
    ) -> CommitOutcome<T> {
        self.witnessed_with(ticket, action, |tx| tx.commit(), |tx| tx.rollback())
    }
    // Separate finalizers permit deterministic known/ambiguous error controls.
    // The public path always uses SQLite's actual commit and rollback methods.
    fn witnessed_with<T>(
        &mut self,
        mut ticket: OperationTicket,
        action: impl FnOnce(&Connection) -> Result<T>,
        commit: impl FnOnce(rusqlite::Transaction<'_>) -> rusqlite::Result<()>,
        rollback: impl FnOnce(rusqlite::Transaction<'_>) -> rusqlite::Result<()>,
    ) -> CommitOutcome<T> {
        if self.blocked {
            return CommitOutcome::Unavailable;
        }
        let tx = match self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        {
            Ok(tx) => tx,
            Err(e) => return CommitOutcome::Unchanged(e.into()),
        };
        let previous: rusqlite::Result<Option<String>> = tx
            .query_row(
                "SELECT intent FROM operations WHERE id=?",
                [&ticket.id],
                |r| r.get(0),
            )
            .optional();
        let value = match previous {
            Ok(Some(intent)) if intent == ticket.intent => {
                return match rollback(tx) {
                    Ok(()) => CommitOutcome::AlreadyCommitted,
                    Err(e) => {
                        self.blocked = true;
                        CommitOutcome::Uncertain {
                            ticket,
                            tentative: None,
                            cause: Error::Sql(e),
                        }
                    }
                };
            }
            Ok(Some(_)) => Err(Error::Invalid("operation ID belongs to a different intent")),
            Err(e) => Err(e.into()),
            Ok(None) => {
                let before = tx.total_changes();
                let result = action(&tx);
                // The action must not finish our transaction. A witness inserted
                // afterward could describe a different commit. Its absence then
                // proves nothing about earlier writes; refuse reconciliation.
                if tx.is_autocommit() {
                    self.blocked = true;
                    ticket.reconcilable = false;
                    return CommitOutcome::Uncertain {
                        ticket,
                        tentative: result.ok(),
                        cause: Error::Invalid("action ended the owned transaction"),
                    };
                }
                result.and_then(|value| {
                    if tx.total_changes() != before {
                        tx.execute(
                            "INSERT INTO operations VALUES(?,?)",
                            params![ticket.id, ticket.intent],
                        )?;
                    }
                    Ok(value)
                })
            }
        };
        match value {
            Ok(value) => match commit(tx) {
                Ok(()) => CommitOutcome::Committed(value),
                Err(e) => {
                    self.blocked = true;
                    CommitOutcome::Uncertain {
                        ticket,
                        tentative: Some(value),
                        cause: Error::CommitUncertain(e),
                    }
                }
            },
            Err(cause) => {
                if tx.is_autocommit() {
                    return CommitOutcome::Unchanged(cause);
                }
                match rollback(tx) {
                    Ok(()) => CommitOutcome::Unchanged(cause),
                    Err(e) => {
                        self.blocked = true;
                        CommitOutcome::Uncertain {
                            ticket,
                            tentative: None,
                            cause: Error::RollbackUncertain(Box::new(cause), e),
                        }
                    }
                }
            }
        }
    }
    /// Consumes the old connection. A failed recovery leaves no usable Store.
    /// Reopening verifies the same file and logical store before any fence write.
    pub fn reconcile(
        self,
        ticket: OperationTicket,
    ) -> std::result::Result<Recovery, Box<RecoveryFailure>> {
        let root = self.root.clone();
        drop(self);
        Self::recover(&root, ticket)
    }
    pub fn recover(
        root: &Path,
        ticket: OperationTicket,
    ) -> std::result::Result<Recovery, Box<RecoveryFailure>> {
        match Self::recover_inner(root, &ticket) {
            Ok(recovery) => Ok(recovery),
            Err(cause) => Err(Box::new(RecoveryFailure { ticket, cause })),
        }
    }
    fn recover_inner(root: &Path, ticket: &OperationTicket) -> Result<Recovery> {
        if !ticket.reconcilable {
            return Err(Error::Invalid(
                "operation escaped its witnessed transaction",
            ));
        }
        // Refuse missing/replaced files before open could create or initialize one.
        let metadata = fs::symlink_metadata(root.join("state.sqlite"))?;
        if !metadata.is_file() || (metadata.dev(), metadata.ino()) != ticket.file {
            return Err(Error::Invalid("recovery database file identity changed"));
        }
        let mut store = Self::open(root)?;
        if store.identity != ticket.store || store.file_identity != ticket.file {
            return Err(Error::Invalid("recovery store identity changed"));
        }
        let tx = store
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT intent FROM operations WHERE id=?",
                [&ticket.id],
                |r| r.get(0),
            )
            .optional()?;
        let resolution = match previous {
            Some(intent) if intent == ticket.intent => Resolution::Committed,
            None => Resolution::Unchanged,
            Some(_) => return Err(Error::Invalid("recovery operation intent differs")),
        };
        let fence: i64 = tx.query_row("SELECT fence FROM store_identity", [], |r| r.get(0))?;
        let next = fence
            .checked_add(1)
            .ok_or(Error::Invalid("recovery fence exhausted"))?;
        tx.execute("UPDATE store_identity SET fence=?", [next])?;
        // FULL-synchronous WAL commit forces a new write barrier. Merely reading
        // an old witness is not treated as evidence that prior WAL bytes synced.
        tx.commit().map_err(Error::CommitUncertain)?;
        Ok(Recovery { store, resolution })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record_store::tests::{Temp, message};

    fn scope() -> Scope<'static> {
        Scope {
            workspace: 2,
            conversation: Some("a"),
        }
    }
    fn fixture() -> (Temp, Store, Vec<Value>) {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let originals = vec![message(1, 2, Some("a")), message(2, 2, Some("a"))];
        store.insert(&originals).unwrap();
        (dir, store, originals)
    }
    fn ticket(store: &Store) -> OperationTicket {
        OperationTicket {
            id: format!("{:032x}", 700),
            intent: "synthetic exact inbox operation".into(),
            store: store.identity.clone(),
            file: store.file_identity,
            reconcilable: true,
        }
    }
    fn change(db: &Connection) -> Result<InboxReceipt> {
        let id = format!("{:032x}", 1);
        crate::record_store::inbox::inbox(
            db,
            &InboxRequest {
                scope: MailboxScope::Native(scope()),
                include_acknowledged: false,
                after: None,
                limit: 1,
                acknowledge: &[&id],
                return_messages: true,
                timestamp: 42,
            },
        )
    }
    fn operations(store: &Store) -> i64 {
        store
            .connection
            .query_row("SELECT count(*) FROM operations", [], |r| r.get(0))
            .unwrap()
    }
    fn unchanged(store: &Store, originals: &[Value]) {
        for m in originals {
            assert_eq!(
                store
                    .get(m["id"].as_str().unwrap(), scope())
                    .unwrap()
                    .as_ref(),
                Some(m)
            );
        }
    }
    fn committed(store: &Store, originals: &[Value]) {
        let mut expected = originals.to_vec();
        expected[0]["acknowledged"] = json!(42);
        expected[1]["surfaced"] = json!(42);
        expected[1]["native_surfaced"] = json!(42);
        unchanged(store, &expected);
    }
    fn uncertain<T>(outcome: CommitOutcome<T>) -> OperationTicket {
        match outcome {
            CommitOutcome::Uncertain { ticket, .. } => ticket,
            _ => panic!("expected uncertain outcome"),
        }
    }
    fn lost_reply(tx: rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
        tx.commit()?;
        Err(rusqlite::Error::InvalidQuery)
    }

    #[test]
    fn lost_commit_reply_blocks_reads_and_writes_then_recovers_exactly_once() {
        let (_dir, mut store, originals) = fixture();
        let ticket =
            uncertain(store.witnessed_with(ticket(&store), change, lost_reply, |tx| tx.rollback()));
        assert!(
            store
                .get(originals[0]["id"].as_str().unwrap(), scope())
                .is_err()
        );
        assert!(store.insert(&[message(3, 2, None)]).is_err());
        assert!(matches!(
            store.witnessed(ticket.clone(), |_| Ok(())),
            CommitOutcome::Unavailable
        ));
        let Recovery {
            mut store,
            resolution,
        } = store.reconcile(ticket.clone()).unwrap();
        assert_eq!(resolution, Resolution::Committed);
        committed(&store, &originals);
        assert_eq!(operations(&store), 1);
        let outcome: CommitOutcome<()> =
            store.witnessed(ticket, |_| panic!("must not replay committed action"));
        assert!(matches!(outcome, CommitOutcome::AlreadyCommitted));
        committed(&store, &originals);
    }

    #[test]
    fn failed_commit_without_commit_recovers_unchanged_and_can_retry() {
        let (_dir, mut store, originals) = fixture();
        let ticket = uncertain(store.witnessed_with(
            ticket(&store),
            change,
            |tx| {
                tx.rollback()?;
                Err(rusqlite::Error::InvalidQuery)
            },
            |tx| tx.rollback(),
        ));
        let Recovery {
            mut store,
            resolution,
        } = store.reconcile(ticket.clone()).unwrap();
        assert_eq!(resolution, Resolution::Unchanged);
        unchanged(&store, &originals);
        assert_eq!(operations(&store), 0);
        assert!(matches!(
            store.witnessed(ticket, change),
            CommitOutcome::Committed(_)
        ));
        committed(&store, &originals);
    }

    #[test]
    fn later_statement_failure_rolls_back_ack_receipts_and_witness_together() {
        let (_dir, mut store, originals) = fixture();
        store.connection.execute_batch("CREATE TRIGGER reject_surface BEFORE UPDATE ON lifecycle WHEN json_extract(NEW.value,'$.native_surfaced') IS NOT NULL BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").unwrap();
        assert!(matches!(
            store.witnessed(ticket(&store), change),
            CommitOutcome::Unchanged(_)
        ));
        unchanged(&store, &originals);
        assert_eq!(operations(&store), 0);
        assert!(!store.blocked);
    }

    #[test]
    fn rollback_error_requires_reconciliation_even_when_rollback_happened() {
        let (_dir, mut store, originals) = fixture();
        let outcome: CommitOutcome<()> = store.witnessed_with(
            ticket(&store),
            |db| {
                change(db)?;
                Err(Error::Invalid("after mutation"))
            },
            |tx| tx.commit(),
            |tx| {
                tx.rollback()?;
                Err(rusqlite::Error::InvalidQuery)
            },
        );
        let ticket = uncertain(outcome);
        let recovery = store.reconcile(ticket).unwrap();
        assert_eq!(recovery.resolution, Resolution::Unchanged);
        unchanged(&recovery.store, &originals);
    }

    #[test]
    fn reused_operation_id_with_different_intent_is_rejected() {
        let (_dir, mut store, originals) = fixture();
        let mut ticket = ticket(&store);
        assert!(matches!(
            store.witnessed(ticket.clone(), change),
            CommitOutcome::Committed(_)
        ));
        ticket.intent.push_str(" changed");
        let outcome: CommitOutcome<()> =
            store.witnessed(ticket, |_| panic!("wrong intent must not execute"));
        assert!(matches!(
            outcome,
            CommitOutcome::Unchanged(Error::Invalid(_))
        ));
        committed(&store, &originals);
        assert_eq!(operations(&store), 1);
    }

    #[test]
    fn no_op_does_not_grow_witness_history() {
        let (_dir, mut store, originals) = fixture();
        for _ in 0..20 {
            assert!(matches!(
                store.witnessed(ticket(&store), |_| Ok(())),
                CommitOutcome::Committed(())
            ));
        }
        unchanged(&store, &originals);
        assert_eq!(operations(&store), 0);
    }

    #[test]
    fn recovery_refuses_replaced_file_without_mutating_replacement() {
        let (dir, mut store, _) = fixture();
        let ticket =
            uncertain(store.witnessed_with(ticket(&store), change, lost_reply, |tx| tx.rollback()));
        drop(store);
        fs::rename(dir.0.join("state.sqlite"), dir.0.join("retained.sqlite")).unwrap();
        let replacement = Store::open(&dir.0).unwrap();
        let identity = replacement.identity.clone();
        drop(replacement);
        assert!(Store::recover(&dir.0, ticket).is_err());
        let replacement = Store::open(&dir.0).unwrap();
        assert_eq!(replacement.identity, identity);
        assert_eq!(operations(&replacement), 0);
    }

    #[test]
    fn recovery_refuses_changed_logical_identity() {
        let (dir, mut store, _) = fixture();
        let ticket =
            uncertain(store.witnessed_with(ticket(&store), change, lost_reply, |tx| tx.rollback()));
        store
            .connection
            .execute("UPDATE store_identity SET id=?", [format!("{:032x}", 333)])
            .unwrap();
        drop(store);
        assert!(Store::recover(&dir.0, ticket).is_err());
    }

    #[test]
    fn failed_recovery_fence_never_claims_success_and_retains_ticket() {
        let (dir, mut store, originals) = fixture();
        store.connection.execute_batch("CREATE TRIGGER reject_fence BEFORE UPDATE ON store_identity BEGIN SELECT RAISE(ABORT,'synthetic fence failure'); END").unwrap();
        let ticket =
            uncertain(store.witnessed_with(ticket(&store), change, lost_reply, |tx| tx.rollback()));
        let failed = match store.reconcile(ticket) {
            Err(e) => e,
            Ok(_) => panic!("fence must fail"),
        };
        let connection = Connection::open(dir.0.join("state.sqlite")).unwrap();
        connection
            .execute_batch("DROP TRIGGER reject_fence")
            .unwrap();
        drop(connection);
        let recovery = Store::recover(&dir.0, failed.ticket).unwrap();
        assert_eq!(recovery.resolution, Resolution::Committed);
        committed(&recovery.store, &originals);
    }

    #[test]
    fn action_cannot_commit_or_rollback_outside_the_witnessed_boundary() {
        for end in ["COMMIT", "ROLLBACK"] {
            let (_dir, mut store, _) = fixture();
            let ticket = uncertain(store.witnessed(ticket(&store), |db| {
                change(db)?;
                db.execute_batch(end)?;
                Ok(())
            }));
            assert!(store.blocked);
            assert!(!ticket.reconcilable);
            assert!(store.reconcile(ticket).is_err());
        }
    }

    #[test]
    fn crash_child() {
        let Some(root) = std::env::var_os("FLERE_STORAGE_CRASH_FIXTURE") else {
            return;
        };
        let point = std::env::var("FLERE_STORAGE_CRASH_POINT").unwrap();
        let mut store = Store::open(Path::new(&root)).unwrap();
        let _ = store.witnessed_with(
            ticket(&store),
            |db| {
                let receipt = change(db)?;
                if point == "before_witness" {
                    std::process::exit(23);
                }
                Ok(receipt)
            },
            |tx| {
                if point == "before_commit" {
                    std::process::exit(23);
                }
                tx.commit()?;
                std::process::exit(23);
            },
            |tx| tx.rollback(),
        );
        panic!("child did not terminate at the requested point");
    }

    #[test]
    fn process_exit_before_and_after_commit_preserves_atomic_outcome() {
        for point in ["before_witness", "before_commit", "after_commit"] {
            let (dir, store, originals) = fixture();
            let ticket = ticket(&store);
            drop(store);
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "record_store::witness::tests::crash_child",
                    "--nocapture",
                ])
                .env("FLERE_STORAGE_CRASH_FIXTURE", &dir.0)
                .env("FLERE_STORAGE_CRASH_POINT", point)
                .status()
                .unwrap();
            assert_eq!(status.code(), Some(23), "child setup failed");
            let recovery = Store::recover(&dir.0, ticket).unwrap();
            if point == "after_commit" {
                assert_eq!(recovery.resolution, Resolution::Committed);
                committed(&recovery.store, &originals);
            } else {
                assert_eq!(recovery.resolution, Resolution::Unchanged);
                unchanged(&recovery.store, &originals);
            }
        }
    }
}
