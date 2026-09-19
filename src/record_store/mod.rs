//! Bounded transactional records for private workspace and coordination state.
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Map, Value, json};
use std::{
    fs, io,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Io(io::Error),
    Json(serde_json::Error),
    Sql(rusqlite::Error),
    /// A commit error must never be treated as proof that no change committed.
    CommitUncertain(rusqlite::Error),
    RollbackUncertain(Box<Error>, rusqlite::Error),
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

mod documents;
mod history;
mod snapshot;
pub use documents::{DocumentKind, DocumentPage};
mod witness;
pub use witness::{CommitOutcome, OperationTicket, Recovery, RecoveryFailure, Resolution};
mod inbox;
pub use inbox::{InboxReceipt, InboxRequest, MailboxScope};
mod legacy;
pub use history::MessagePage;
mod working;
pub use legacy::ImportOutcome;
pub use working::{DocumentSelection, MessageSelection, WorkingDocument};

pub struct Store {
    connection: Connection,
    root: PathBuf,
    identity: String,
    file_identity: (u64, u64),
    blocked: bool,
}
#[derive(Clone, Copy)]
pub struct Scope<'a> {
    pub workspace: u64,
    pub conversation: Option<&'a str>,
}
const STATE_FIELDS: [&str; 4] = ["surfaced", "native_surfaced", "acknowledged", "delivery"];
const MAX_RECORD: usize = 128 * 1024;
const MAX_TRANSACTION: usize = 1024 * 1024;
fn key(value: u64) -> String {
    format!("{value:016x}")
}
fn id_valid(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|c| c.is_ascii_hexdigit())
}
fn allowed(workspace: &str, conversation: Option<&str>, scope: Scope<'_>) -> bool {
    workspace == key(scope.workspace)
        && (conversation.is_none() || conversation == scope.conversation)
}
fn join(base: String, state: String) -> Result<Value> {
    if base.len() + state.len() > MAX_RECORD {
        return Err(Error::Invalid("record exceeds bound"));
    }
    let mut base: Map<String, Value> = serde_json::from_str(&base)?;
    let state: Map<String, Value> = serde_json::from_str(&state)?;
    if state.keys().any(|k| !STATE_FIELDS.contains(&k.as_str())) {
        return Err(Error::Invalid("unexpected lifecycle field"));
    }
    base.extend(state);
    Ok(Value::Object(base))
}

fn checked_message(db: &Connection, id: &str, base: String, state: String) -> Result<Value> {
    let value = join(base, state)?;
    let parsed = PreparedMessage::new(&value)?;
    let actual:(String,Option<String>,String,Option<String>,Option<String>)=db.query_row(
        "SELECT recipient,conversation,sender,sender_conversation,request_id FROM messages WHERE id=?",[id],
        |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    if parsed.id != id
        || actual
            != (
                parsed.recipient,
                parsed.conversation,
                parsed.sender,
                parsed.sender_conversation,
                parsed.request_id,
            )
    {
        return Err(Error::Invalid(
            "message index differs from original evidence",
        ));
    }
    Ok(value)
}
fn validate_message(db: &Connection, id: &str) -> Result<()> {
    let (base, state): (String, String) = db.query_row(
        "SELECT original,value FROM messages JOIN lifecycle USING(id) WHERE id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    checked_message(db, id, base, state).map(|_| ())
}
struct PreparedMessage {
    id: String,
    recipient: String,
    conversation: Option<String>,
    sender: String,
    sender_conversation: Option<String>,
    request_id: Option<String>,
    original: String,
    lifecycle: String,
}
impl PreparedMessage {
    fn new(record: &Value) -> Result<Self> {
        let mut base = record
            .as_object()
            .ok_or(Error::Invalid("message must be an object"))?
            .clone();
        let id = record["id"]
            .as_str()
            .filter(|s| id_valid(s))
            .ok_or(Error::Invalid("invalid message id"))?
            .to_owned();
        let recipient = key(record["to"]
            .as_u64()
            .ok_or(Error::Invalid("missing recipient"))?);
        let sender = key(record["from"]
            .as_u64()
            .ok_or(Error::Invalid("missing sender"))?);
        let body = record["body"]
            .as_str()
            .ok_or(Error::Invalid("missing body"))?;
        if body.is_empty() || body.len() > 16384 {
            return Err(Error::Invalid("invalid body size"));
        }
        let mut state = Map::new();
        for field in STATE_FIELDS {
            if let Some(value) = base.remove(field) {
                if field != "delivery" && !value.is_null() && value.as_u64().is_none() {
                    return Err(Error::Invalid("invalid lifecycle timestamp"));
                }
                state.insert(field.into(), value);
            }
        }
        let original = serde_json::to_string(&base)?;
        let lifecycle = serde_json::to_string(&state)?;
        if original.len() + lifecycle.len() > MAX_RECORD {
            return Err(Error::Invalid("message exceeds bound"));
        }
        Ok(Self {
            id,
            recipient,
            sender,
            conversation: record["chat"]["conversation"].as_str().map(str::to_owned),
            sender_conversation: record["chat"]["sender"]["conversation"]
                .as_str()
                .map(str::to_owned),
            request_id: record["chat"]["request_id"].as_str().map(str::to_owned),
            original,
            lifecycle,
        })
    }
    fn insert(&self, db: &Connection) -> Result<()> {
        db.execute("INSERT INTO messages(id,recipient,conversation,sender,sender_conversation,request_id,original) VALUES(?,?,?,?,?,?,?)",
            params![self.id,self.recipient,self.conversation,self.sender,self.sender_conversation,self.request_id,self.original])?;
        db.execute(
            "INSERT INTO lifecycle(id,value) VALUES(?,?)",
            params![self.id, self.lifecycle],
        )?;
        Ok(())
    }
}

impl Store {
    /// Read current layout without creating a database, schema or journal.
    /// The caller validates that the private root belongs to the current user.
    pub fn read_layout(root: &Path) -> Result<Value> {
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.mode() & 0o077 != 0 {
            return Err(Error::Invalid(
                "store directory must be private and regular",
            ));
        }
        for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
            match fs::symlink_metadata(root.join(name)) {
                Ok(m) if !m.is_file() || m.uid() != meta.uid() || m.mode() & 0o077 != 0 => {
                    return Err(Error::Invalid("unexpected database file"));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound && name != "state.sqlite" => (),
                Err(e) => return Err(e.into()),
                _ => (),
            }
        }
        let db = Connection::open_with_flags(
            root.join("state.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        db.busy_timeout(std::time::Duration::from_millis(100))?;
        db.set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            (2 * MAX_RECORD) as i32,
        )?;
        db.pragma_update(None, "trusted_schema", false)?;
        db.pragma_update(None, "query_only", true)?;
        db.pragma_update(None, "temp_store", "MEMORY")?;
        db.pragma_update(None, "cache_size", -2048)?;
        let version: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != 6 {
            return Err(Error::Invalid("unknown store version"));
        }
        working::read_layout(&db)
    }
    pub fn open(root: &Path) -> Result<Self> {
        let meta = fs::symlink_metadata(root)?;
        if !meta.is_dir() || meta.permissions().mode() & 0o077 != 0 {
            return Err(Error::Invalid(
                "store directory must be private and regular",
            ));
        }
        for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
            match fs::symlink_metadata(root.join(name)) {
                Ok(m)
                    if !m.is_file()
                        || m.uid() != meta.uid()
                        || m.permissions().mode() & 0o077 != 0 =>
                {
                    return Err(Error::Invalid("unexpected database file"));
                }
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        let path = root.join("state.sqlite");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file.sync_all()?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.set_limit(
            rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
            (2 * MAX_RECORD) as i32,
        )?;
        connection.pragma_update(None, "trusted_schema", false)?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "temp_store", "MEMORY")?;
        connection.pragma_update(None, "cache_size", -2048)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if !matches!(version, 0 | 6) {
            return Err(Error::Invalid("unknown store version"));
        }
        if version == 0 {
            let tables: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |r| r.get(0),
            )?;
            if tables != 0 {
                return Err(Error::Invalid("unrecognized database"));
            }
        }
        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if mode != "wal" {
            return Err(Error::Invalid("WAL is unavailable"));
        }
        connection.pragma_update(None, "synchronous", "FULL")?;
        if version == 0 {
            let tx = connection.transaction()?;
            tx.execute_batch("
                CREATE TABLE messages(seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
                  recipient TEXT NOT NULL, conversation TEXT, sender TEXT NOT NULL,
                  sender_conversation TEXT, request_id TEXT, original TEXT NOT NULL,
                  UNIQUE(sender,sender_conversation,request_id));
                CREATE INDEX recipient_order ON messages(recipient,seq,conversation,id);
                CREATE TABLE lifecycle(id TEXT PRIMARY KEY REFERENCES messages(id), value TEXT NOT NULL CHECK(json_valid(value)),
                  pending INTEGER GENERATED ALWAYS AS (CASE WHEN json_type(value,'$.acknowledged') IS NULL OR json_type(value,'$.acknowledged')='null' THEN 1 ELSE 0 END) STORED);
                CREATE TABLE checkpoints(id TEXT PRIMARY KEY, body TEXT NOT NULL);
                CREATE TABLE workspace(id TEXT PRIMARY KEY, status TEXT);
                CREATE TABLE documents(kind TEXT NOT NULL, ordinal INTEGER NOT NULL, identity TEXT, owner TEXT, pending INTEGER NOT NULL, phase TEXT, request_owner TEXT, request_id TEXT,
                  PRIMARY KEY(kind,ordinal), UNIQUE(kind,identity));
                CREATE INDEX document_owner_order ON documents(kind,owner,ordinal);
                CREATE UNIQUE INDEX dispatch_request ON documents(kind,request_owner,request_id);
                CREATE INDEX dispatch_phase ON documents(kind,owner,phase,ordinal);
                CREATE INDEX lifecycle_lease ON lifecycle(json_extract(value,'$.delivery.lease'));
                CREATE INDEX document_pending_order ON documents(kind,owner,pending,ordinal);
                CREATE TABLE document_chunks(kind TEXT NOT NULL, ordinal INTEGER NOT NULL, part INTEGER NOT NULL, value BLOB NOT NULL,
                  PRIMARY KEY(kind,ordinal,part), FOREIGN KEY(kind,ordinal) REFERENCES documents(kind,ordinal));
                CREATE TABLE import_sources(kind TEXT NOT NULL, part INTEGER NOT NULL, value BLOB NOT NULL, PRIMARY KEY(kind,part));
                CREATE TABLE import_meta(key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE store_identity(id TEXT PRIMARY KEY, fence INTEGER NOT NULL CHECK(typeof(fence)='integer' AND fence>=0));
                INSERT INTO store_identity VALUES(lower(hex(randomblob(16))),0);
                CREATE TABLE operations(id TEXT PRIMARY KEY, intent TEXT NOT NULL);
                PRAGMA user_version=6;
            ")?;
            tx.commit().map_err(Error::CommitUncertain)?;
            fs::File::open(root)?.sync_all()?;
        }
        let identity: String =
            connection.query_row("SELECT id FROM store_identity", [], |row| row.get(0))?;
        let count: i64 =
            connection.query_row("SELECT count(*) FROM store_identity", [], |row| row.get(0))?;
        if count != 1 || !id_valid(&identity) {
            return Err(Error::Invalid("invalid store identity"));
        }
        let file = fs::metadata(root.join("state.sqlite"))?;
        Ok(Self {
            connection,
            root: fs::canonicalize(root)?,
            identity,
            file_identity: (file.dev(), file.ino()),
            blocked: false,
        })
    }
    // Mutation closures never commit themselves. Preserve rollback failure as
    // uncertainty instead of relying on Transaction's best-effort Drop cleanup.
    fn transact<T>(
        &mut self,
        action: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        self.ensure_ready()?;
        let tx = self.connection.transaction()?;
        match action(&tx) {
            Ok(value) => {
                if let Err(error) = tx.commit() {
                    self.blocked = true;
                    return Err(Error::CommitUncertain(error));
                }
                Ok(value)
            }
            Err(cause) => {
                if tx.is_autocommit() {
                    // SQLite may have already rolled the failed transaction back.
                    return Err(cause);
                }
                match tx.rollback() {
                    Ok(()) => Err(cause),
                    Err(error) => {
                        self.blocked = true;
                        Err(Error::RollbackUncertain(Box::new(cause), error))
                    }
                }
            }
        }
    }
    /// Input has already passed native authorization; this layer preserves records.
    pub fn insert(&mut self, records: &[Value]) -> Result<()> {
        let mut prepared = Vec::new();
        let mut bytes = 0;
        for record in records {
            let record = PreparedMessage::new(record)?;
            bytes += record.original.len() + record.lifecycle.len();
            if bytes > MAX_TRANSACTION {
                return Err(Error::Invalid("message transaction exceeds bound"));
            }
            prepared.push(record);
        }
        self.transact(|tx| {
            for record in &prepared {
                record.insert(tx)?;
            }
            Ok(())
        })
    }
    pub fn get(&self, id: &str, scope: Scope<'_>) -> Result<Option<Value>> {
        self.ensure_ready()?;
        if !id_valid(id) {
            return Err(Error::Invalid("invalid message id"));
        }
        let row: Option<(String, Option<String>, String, String)> = self.connection.query_row(
            "SELECT recipient,conversation,original,value FROM messages JOIN lifecycle USING(id) WHERE id=?",
            [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        match row {
            Some((recipient, conversation, base, state))
                if allowed(&recipient, conversation.as_deref(), scope) =>
            {
                Ok(Some(checked_message(&self.connection, id, base, state)?))
            }
            _ => Ok(None),
        }
    }
    pub fn acknowledge(&mut self, scope: Scope<'_>, ids: &[&str], timestamp: u64) -> Result<()> {
        if ids.iter().any(|id| !id_valid(id)) {
            return Err(Error::Invalid("invalid message id"));
        }
        if ids.len() > 4096 {
            return Err(Error::Invalid("acknowledgements exceed bound"));
        }
        self.transact(|tx| {
        for id in ids {
            let row: Option<(String,Option<String>,String)> = tx.query_row(
                "SELECT recipient,conversation,value FROM messages JOIN lifecycle USING(id) WHERE id=?",
                [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let Some((recipient, conversation, state)) = row else {
                return Err(Error::Invalid("unknown acknowledgement"));
            };
            if !allowed(&recipient, conversation.as_deref(), scope) {
                return Err(Error::Invalid("wrong acknowledgement scope"));
            }
            validate_message(tx,id)?;
            let mut state: Value = serde_json::from_str(&state)?;
            if state["acknowledged"].is_null() {
                state["acknowledged"] = json!(timestamp);
                tx.execute(
                    "UPDATE lifecycle SET value=? WHERE id=?",
                    params![serde_json::to_string(&state)?, id],
                )?;
            }
        }
            Ok(())
        })
    }
    pub fn submit_result(&mut self, id: &str, workspace: u64, body: &str) -> Result<()> {
        if id.len() > 64 || body.is_empty() || body.len() > 16384 {
            return Err(Error::Invalid("invalid result"));
        }
        self.transact(|tx| {
        tx.execute("INSERT INTO checkpoints VALUES(?,?)", params![id, body])?;
        let ordinal:i64=tx.query_row("SELECT coalesce(max(ordinal)+1,0) FROM documents WHERE kind='checkpoints'",[],|r|r.get(0))?;
        let reference=format!("checkpoint:{ordinal}");
        let owner=key(workspace);
        let time=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        legacy::put_document(tx,"checkpoints",ordinal,Some(&reference),Some(&owner),&json!({"workspace":workspace,"kind":"submit_result","body":body,"time":time}))?;
        tx.execute("INSERT INTO workspace VALUES(?,'needs-me') ON CONFLICT(id) DO UPDATE SET status='needs-me'",[key(workspace)])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{DirBuilderExt, symlink},
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };
    pub(super) struct Temp(pub(super) PathBuf);
    impl Temp {
        pub(super) fn new() -> Self {
            let parent = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp/storage-rust-tests");
            fs::create_dir_all(&parent).unwrap();
            let path = parent.join(format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    pub(super) fn message(n: u64, to: u64, conversation: Option<&str>) -> Value {
        json!({"id":format!("{n:032x}"),"from":1,"to":to,"body":"exact synthetic \\ evidence\n界",
            "intent":"quiet","saved":1700000000u64,"surfaced":null,"native_surfaced":null,"acknowledged":null,"delivery":null,
            "chat":conversation.map(|c|json!({"conversation":c,"sender":{"conversation":"sender"},"request_id":format!("request-{n}"),"user_request_ref":"local-only"}))})
    }
    #[test]
    fn exact_records_scope_and_unsigned_workspace_survive_reopen() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let m = message(1, u64::MAX - 1, Some("conversation"));
        let scope = Scope {
            workspace: u64::MAX - 1,
            conversation: Some("conversation"),
        };
        store.insert(std::slice::from_ref(&m)).unwrap();
        assert_eq!(
            store.get(m["id"].as_str().unwrap(), scope).unwrap(),
            Some(m.clone())
        );
        assert!(
            store
                .get(
                    m["id"].as_str().unwrap(),
                    Scope {
                        workspace: 2,
                        ..scope
                    }
                )
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get(
                    m["id"].as_str().unwrap(),
                    Scope {
                        conversation: Some("different"),
                        ..scope
                    }
                )
                .unwrap()
                .is_none()
        );
        drop(store);
        let reopened = Store::open(&dir.0).unwrap();
        assert_eq!(
            reopened.get(m["id"].as_str().unwrap(), scope).unwrap(),
            Some(m)
        );
    }
    #[test]
    fn ack_is_atomic_scoped_idempotent_and_does_not_rewrite_original() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let a = message(1, 2, Some("a"));
        let b = message(2, 2, Some("b"));
        let id = a["id"].as_str().unwrap();
        let other = b["id"].as_str().unwrap();
        let scope = Scope {
            workspace: 2,
            conversation: Some("a"),
        };
        store.insert(&[a.clone(), b.clone()]).unwrap();
        assert!(store.acknowledge(scope, &[id, other], 9).is_err());
        assert_eq!(store.get(id, scope).unwrap(), Some(a.clone()));
        store.connection.execute_batch("CREATE TRIGGER immutable_evidence BEFORE UPDATE ON messages BEGIN SELECT RAISE(FAIL,'original changed'); END;").unwrap();
        store.acknowledge(scope, &[id], 10).unwrap();
        store.acknowledge(scope, &[id], 99).unwrap();
        let mut expected = a;
        expected["acknowledged"] = json!(10);
        assert_eq!(
            store.get(expected["id"].as_str().unwrap(), scope).unwrap(),
            Some(expected)
        );
    }
    #[test]
    fn request_uniqueness_rolls_back_entire_insert_batch() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let a = message(1, 2, Some("a"));
        store.insert(std::slice::from_ref(&a)).unwrap();
        let b = message(2, 2, Some("a"));
        let mut duplicate = a;
        duplicate["id"] = json!(format!("{:032x}", 3));
        assert!(store.insert(&[b.clone(), duplicate]).is_err());
        assert!(
            store
                .get(
                    b["id"].as_str().unwrap(),
                    Scope {
                        workspace: 2,
                        conversation: Some("a")
                    }
                )
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn result_and_status_rollback_together() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_status BEFORE INSERT ON workspace BEGIN SELECT RAISE(FAIL,'injected metadata failure'); END;").unwrap();
        assert!(
            store
                .submit_result("result", 2, "exact synthetic result")
                .is_err()
        );
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM checkpoints", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        store
            .connection
            .execute_batch("DROP TRIGGER reject_status")
            .unwrap();
        store
            .submit_result("result", 2, "exact synthetic result")
            .unwrap();
        assert_eq!(
            store
                .connection
                .query_row("SELECT status FROM workspace WHERE id=?", [key(2)], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "needs-me"
        );
    }
    #[test]
    fn retained_history_beyond_old_cap_still_acknowledges_exactly() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        for start in (0..640).step_by(16) {
            let batch: Vec<_> = (start..start + 16)
                .map(|n| {
                    let mut m = message(n, 2, Some("a"));
                    m["body"] = json!("s".repeat(16384));
                    m
                })
                .collect();
            store.insert(&batch).unwrap();
        }
        store
            .connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        assert!(fs::metadata(dir.0.join("state.sqlite")).unwrap().len() > 8 * 1024 * 1024);
        let id = format!("{:032x}", 639);
        let scope = Scope {
            workspace: 2,
            conversation: Some("a"),
        };
        store.acknowledge(scope, &[&id], 42).unwrap();
        drop(store);
        let reopened = Store::open(&dir.0).unwrap();
        for n in 0..640 {
            let mut m = message(n, 2, Some("a"));
            m["body"] = json!("s".repeat(16384));
            if n == 639 {
                m["acknowledged"] = json!(42);
            }
            assert_eq!(
                reopened.get(m["id"].as_str().unwrap(), scope).unwrap(),
                Some(m)
            );
        }
    }
    #[test]
    fn oversized_database_record_is_rejected_without_decoding() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let m = message(1, 2, Some("a"));
        store.insert(std::slice::from_ref(&m)).unwrap();
        store
            .connection
            .set_limit(
                rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
                (4 * MAX_RECORD) as i32,
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE messages SET original=?",
                ["x".repeat(3 * MAX_RECORD)],
            )
            .unwrap();
        store
            .connection
            .set_limit(
                rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
                (2 * MAX_RECORD) as i32,
            )
            .unwrap();
        let error = store
            .get(
                m["id"].as_str().unwrap(),
                Scope {
                    workspace: 2,
                    conversation: Some("a"),
                },
            )
            .unwrap_err();
        assert!(
            matches!(error, Error::Sql(rusqlite::Error::SqliteFailure(e,_)) if e.code == rusqlite::ErrorCode::TooBig)
        );
    }
    #[test]
    fn sqlite_full_preserves_preexisting_records_and_reports_failure() {
        let dir = Temp::new();
        let mut store = Store::open(&dir.0).unwrap();
        let original = message(1, 2, Some("a"));
        store.insert(std::slice::from_ref(&original)).unwrap();
        let pages: i64 = store
            .connection
            .pragma_query_value(None, "page_count", |r| r.get(0))
            .unwrap();
        store
            .connection
            .pragma_update(None, "max_page_count", pages)
            .unwrap();
        let mut large = message(2, 2, Some("a"));
        large["body"] = json!("s".repeat(16384));
        let error = store.insert(&[large]).unwrap_err();
        assert!(
            matches!(error,Error::Sql(rusqlite::Error::SqliteFailure(e,_)) if e.code==rusqlite::ErrorCode::DiskFull)
        );
        let scope = Scope {
            workspace: 2,
            conversation: Some("a"),
        };
        assert_eq!(
            store.get(original["id"].as_str().unwrap(), scope).unwrap(),
            Some(original.clone())
        );
        assert!(store.get(&format!("{:032x}", 2), scope).unwrap().is_none());
        drop(store);
        let reopened = Store::open(&dir.0).unwrap();
        assert_eq!(
            reopened
                .get(original["id"].as_str().unwrap(), scope)
                .unwrap(),
            Some(original)
        );
    }
    #[test]
    fn rejects_unknown_schema_and_database_or_auxiliary_symlinks() {
        let dir = Temp::new();
        let store = Store::open(&dir.0).unwrap();
        store
            .connection
            .pragma_update(None, "user_version", 99)
            .unwrap();
        drop(store);
        assert!(matches!(
            Store::open(&dir.0),
            Err(Error::Invalid("unknown store version"))
        ));
        let dir = Temp::new();
        let target = dir.0.join("retained");
        fs::write(&target, b"synthetic").unwrap();
        symlink(&target, dir.0.join("state.sqlite")).unwrap();
        assert!(Store::open(&dir.0).is_err());
        assert_eq!(fs::read(target).unwrap(), b"synthetic");
        let dir = Temp::new();
        let target = dir.0.join("retained");
        fs::write(&target, b"synthetic").unwrap();
        symlink(&target, dir.0.join("state.sqlite-wal")).unwrap();
        assert!(Store::open(&dir.0).is_err());
        assert_eq!(fs::read(target).unwrap(), b"synthetic");
    }
}

#[cfg(test)]
mod contract_tests;
