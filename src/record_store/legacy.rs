use super::*;
pub(super) const SOURCE_LIMIT: usize = 8 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;
type DocumentIndex = (
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
    Option<String>,
    Option<String>,
);
const ARRAYS: [&str; 4] = ["messages", "decisions", "checkpoints", "dispatches"];
#[derive(Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    Imported,
    AlreadyImported,
}

fn canonical(workspaces: &[u8], fallback: Option<&[u8]>) -> Result<Value> {
    if workspaces.len() > SOURCE_LIMIT || fallback.is_some_and(|b| b.len() > SOURCE_LIMIT) {
        return Err(Error::Invalid("legacy source exceeds bound"));
    }
    let mut root: Value = serde_json::from_slice(workspaces)?;
    if !root["version"]
        .as_u64()
        .is_some_and(|v| (2..=10).contains(&v))
    {
        return Err(Error::Invalid("unsupported legacy version"));
    }
    let workspaces = root["workspaces"]
        .as_array()
        .ok_or(Error::Invalid("missing legacy workspaces"))?;
    let mut ids = std::collections::BTreeSet::new();
    for workspace in workspaces {
        let id = workspace["id"]
            .as_u64()
            .filter(|n| *n != 0 && *n != u64::MAX)
            .ok_or(Error::Invalid("invalid workspace identity"))?;
        if !ids.insert(id) {
            return Err(Error::Invalid("duplicate workspace identity"));
        }
    }
    let mut coordination = match root.get("coordination") {
        Some(value) if !value.is_null() => value.clone(),
        _ => match fallback {
            Some(bytes) => serde_json::from_slice(bytes)?,
            None => json!({}),
        },
    };
    let object = coordination
        .as_object_mut()
        .ok_or(Error::Invalid("invalid legacy coordination"))?;
    for name in ARRAYS {
        let value = object.entry(name).or_insert_with(|| json!([]));
        if !value.is_array() {
            return Err(Error::Invalid("invalid coordination list"));
        }
    }
    object
        .entry("delivery")
        .or_insert_with(|| json!({"hooks":[],"focus":[]}));
    root["coordination"] = coordination;
    Ok(root)
}
fn write_source(db: &Connection, kind: &str, bytes: &[u8]) -> Result<()> {
    for part in 0..bytes.len().div_ceil(CHUNK).max(1) {
        let first = part * CHUNK;
        let last = (first + CHUNK).min(bytes.len());
        db.execute(
            "INSERT INTO import_sources VALUES(?,?,?)",
            params![kind, part as i64, &bytes[first..last]],
        )?;
    }
    Ok(())
}
fn read_source(db: &Connection, kind: &str) -> Result<Option<Vec<u8>>> {
    let mut statement =
        db.prepare("SELECT part,value FROM import_sources WHERE kind=? ORDER BY part")?;
    let mut rows = statement.query([kind])?;
    let mut bytes = Vec::new();
    let mut next = 0;
    while let Some(row) = rows.next()? {
        let part: i64 = row.get(0)?;
        let data: Vec<u8> = row.get(1)?;
        if part != next || data.len() > CHUNK || bytes.len() + data.len() > SOURCE_LIMIT {
            return Err(Error::Invalid("invalid retained source chunks"));
        }
        bytes.extend_from_slice(&data);
        next += 1;
    }
    Ok((next != 0).then_some(bytes))
}
pub(super) fn put_document(
    db: &Connection,
    kind: &str,
    ordinal: i64,
    identity: Option<&str>,
    owner: Option<&str>,
    value: &Value,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > SOURCE_LIMIT {
        return Err(Error::Invalid("legacy document exceeds bound"));
    }
    db.execute(
        "INSERT INTO documents(kind,ordinal,identity,owner,pending,phase,request_owner,request_id) VALUES(?,?,?,?,?,?,?,?)",
        params![
            kind,
            ordinal,
            identity,
            owner,
            i64::from(kind == "decisions" && value["answer"].is_null()),
            if kind == "dispatches" {value["phase"].as_str()} else {None},
            if kind == "dispatches" {value["coordinator"].as_u64().map(key)} else {None},
            if kind == "dispatches" {value["request_id"].as_str()} else {None}
        ],
    )?;
    for (part, data) in bytes.chunks(CHUNK).enumerate() {
        db.execute(
            "INSERT INTO document_chunks VALUES(?,?,?,?)",
            params![kind, ordinal, part as i64, data],
        )?;
    }
    Ok(())
}
pub(super) fn read_document(db: &Connection, kind: &str, ordinal: i64) -> Result<Value> {
    let mut statement = db.prepare(
        "SELECT part,value FROM document_chunks WHERE kind=? AND ordinal=? ORDER BY part",
    )?;
    let mut rows = statement.query(params![kind, ordinal])?;
    let mut bytes = Vec::new();
    let mut next = 0;
    while let Some(row) = rows.next()? {
        let part: i64 = row.get(0)?;
        let data: Vec<u8> = row.get(1)?;
        if part != next || data.len() > CHUNK || bytes.len() + data.len() > SOURCE_LIMIT {
            return Err(Error::Invalid("invalid legacy document chunks"));
        }
        bytes.extend_from_slice(&data);
        next += 1;
    }
    if next == 0 {
        return Err(Error::Invalid("missing legacy document"));
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    if matches!(
        kind,
        "decisions" | "dispatches" | "checkpoints" | "workspaces"
    ) {
        let row:DocumentIndex=db.query_row(
            "SELECT identity,owner,pending,phase,request_owner,request_id FROM documents WHERE kind=? AND ordinal=?",params![kind,ordinal],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
        let (id, owner) = if kind == "workspaces" {
            let id = value["id"].as_u64().map(key);
            (id.clone(), id)
        } else {
            (
                if kind == "checkpoints" {
                    Some(format!("checkpoint:{ordinal}"))
                } else {
                    value["id"].as_str().map(str::to_owned)
                },
                value["workspace"].as_u64().map(key),
            )
        };
        let expected = (
            id,
            owner,
            i64::from(kind == "decisions" && value["answer"].is_null()),
            if kind == "dispatches" {
                value["phase"].as_str().map(str::to_owned)
            } else {
                None
            },
            if kind == "dispatches" {
                value["coordinator"].as_u64().map(key)
            } else {
                None
            },
            if kind == "dispatches" {
                value["request_id"].as_str().map(str::to_owned)
            } else {
                None
            },
        );
        if row != expected {
            return Err(Error::Invalid(
                "document index differs from original evidence",
            ));
        }
    }
    Ok(value)
}
pub(super) fn document_array(db: &Connection, kind: &str) -> Result<Vec<Value>> {
    let mut statement =
        db.prepare("SELECT ordinal FROM documents WHERE kind=? ORDER BY ordinal")?;
    let mut rows = statement.query([kind])?;
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        let ordinal: i64 = row.get(0)?;
        if ordinal != values.len() as i64 {
            return Err(Error::Invalid("legacy document order has a gap"));
        }
        values.push(read_document(db, kind, ordinal)?);
    }
    Ok(values)
}
pub(super) fn export(db: &Connection) -> Result<Value> {
    let mut root = read_document(db, "header", 0)?;
    let mut workspaces = document_array(db, "workspaces")?;
    for workspace in &mut workspaces {
        let override_status: Option<String> = db.query_row(
            "SELECT status FROM workspace WHERE id=?",
            [key(workspace["id"]
                .as_u64()
                .ok_or(Error::Invalid("invalid workspace identity"))?)],
            |r| r.get(0),
        )?;
        if let Some(status) = override_status {
            workspace["meta"]["status"] = json!(status);
        }
    }
    root["workspaces"] = json!(workspaces);
    let mut statement =
        db.prepare("SELECT original,value FROM messages JOIN lifecycle USING(id) ORDER BY seq")?;
    let mut rows = statement.query([])?;
    let mut messages = Vec::new();
    while let Some(row) = rows.next()? {
        messages.push(join(row.get(0)?, row.get(1)?)?);
    }
    root["coordination"]["messages"] = json!(messages);
    for kind in ["decisions", "checkpoints", "dispatches"] {
        root["coordination"][kind] = json!(document_array(db, kind)?);
    }
    root["coordination"]["delivery"] = read_document(db, "delivery", 0)?;
    Ok(root)
}
impl Store {
    /// Structural import model. Before production use, the owning application
    /// must additionally run its existing metadata/chat/dispatch/hook validators.
    /// Source bytes are never changed; both originals are retained in chunks.
    pub fn import_legacy(
        &mut self,
        workspaces: &[u8],
        fallback: Option<&[u8]>,
    ) -> Result<ImportOutcome> {
        let expected = canonical(workspaces, fallback)?;
        self.transact(|tx| {
            let complete:Option<String>=tx.query_row("SELECT value FROM import_meta WHERE key='complete'",[],|r|r.get(0)).optional()?;
            if complete.is_some() {
                if read_source(tx,"workspaces")?.as_deref()==Some(workspaces)&&read_source(tx,"coordination")?.as_deref()==fallback {
                    return Ok(ImportOutcome::AlreadyImported);
                }
                return Err(Error::Invalid("import already belongs to different source bytes"));
            }
            let occupied:i64=tx.query_row("SELECT (SELECT count(*) FROM messages)+(SELECT count(*) FROM lifecycle)+(SELECT count(*) FROM checkpoints)+(SELECT count(*) FROM workspace)+(SELECT count(*) FROM documents)+(SELECT count(*) FROM document_chunks)+(SELECT count(*) FROM import_sources)+(SELECT count(*) FROM import_meta)+(SELECT count(*) FROM operations)",[],|r|r.get(0))?;
            if occupied!=0 {return Err(Error::Invalid("import requires an empty store"));}
            write_source(tx,"workspaces",workspaces)?;
            if let Some(bytes)=fallback {write_source(tx,"coordination",bytes)?;}
            let mut header=expected.clone();header["workspaces"]=json!([]);
            for kind in ARRAYS {header["coordination"][kind]=json!([]);}
            header["coordination"]["delivery"]=Value::Null;
            put_document(tx,"header",0,None,None,&header)?;
            for (ordinal,workspace) in expected["workspaces"].as_array().unwrap().iter().enumerate() {
                let identity=key(workspace["id"].as_u64().unwrap());
                put_document(tx,"workspaces",ordinal as i64,Some(&identity),Some(&identity),workspace)?;
                tx.execute("INSERT INTO workspace VALUES(?,NULL)",[identity])?;
            }
            for message in expected["coordination"]["messages"].as_array().unwrap() {PreparedMessage::new(message)?.insert(tx)?;}
            for kind in ["decisions","checkpoints","dispatches"] {
                for (ordinal,record) in expected["coordination"][kind].as_array().unwrap().iter().enumerate() {
                    let identity=if kind=="checkpoints"{Some(format!("checkpoint:{ordinal}"))}else{record["id"].as_str().map(str::to_owned)};
                    let owner=record["workspace"].as_u64().map(key);
                    put_document(tx,kind,ordinal as i64,identity.as_deref(),owner.as_deref(),record)?;
                }
            }
            put_document(tx,"delivery",0,None,None,&expected["coordination"]["delivery"])?;
            if export(tx)?!=expected {return Err(Error::Invalid("import verification differs from source"));}
            if read_source(tx,"workspaces")?.as_deref()!=Some(workspaces)||read_source(tx,"coordination")?.as_deref()!=fallback {
                return Err(Error::Invalid("retained source verification failed"));
            }
            tx.execute("INSERT INTO import_meta VALUES('complete','1')",[])?;
            Ok(ImportOutcome::Imported)
        })
    }
    /// Explicit whole-store export for migration verification, not routine agent context.
    /// Streaming large-store export remains an integration obligation.
    pub fn export_legacy_canonical(&self) -> Result<Value> {
        self.ensure_ready()?;
        let tx = self.connection.unchecked_transaction()?;
        export(&tx)
    }
    pub fn original_sources(&self) -> Result<(Vec<u8>, Option<Vec<u8>>)> {
        self.ensure_ready()?;
        let tx = self.connection.unchecked_transaction()?;
        Ok((
            read_source(&tx, "workspaces")?.ok_or(Error::Invalid("missing original source"))?,
            read_source(&tx, "coordination")?,
        ))
    }
    pub fn checkpoint_by_reference(&self, workspace: u64, id: &str) -> Result<Option<Value>> {
        let text = id
            .strip_prefix("checkpoint:")
            .ok_or(Error::Invalid("invalid checkpoint reference"))?;
        let index: u64 = text
            .parse()
            .map_err(|_| Error::Invalid("invalid checkpoint reference"))?;
        if index.to_string() != text {
            return Err(Error::Invalid("noncanonical checkpoint reference"));
        }
        self.ensure_ready()?;
        let tx = self.connection.unchecked_transaction()?;
        let ordinal: Option<i64> = tx
            .query_row(
                "SELECT ordinal FROM documents WHERE kind='checkpoints' AND identity=? AND owner=?",
                params![id, key(workspace)],
                |r| r.get(0),
            )
            .optional()?;
        ordinal
            .map(|n| read_document(&tx, "checkpoints", n))
            .transpose()
    }
}
