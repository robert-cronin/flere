//! Atomic inbox operation. The caller establishes fresh native/user authority;
//! these scopes filter records and never grant authorization by themselves.
use super::*;

#[derive(Clone, Copy)]
pub enum MailboxScope<'a> {
    Native(Scope<'a>),
    Human { workspace: u64 },
}
impl MailboxScope<'_> {
    pub(super) fn values(self) -> (String, bool, Option<String>) {
        match self {
            Self::Native(scope) => (
                key(scope.workspace),
                false,
                scope.conversation.map(str::to_owned),
            ),
            Self::Human { workspace } => (key(workspace), true, None),
        }
    }
    fn allows(self, recipient: &str, conversation: Option<&str>) -> bool {
        match self {
            Self::Native(scope) => allowed(recipient, conversation, scope),
            Self::Human { workspace } => recipient == key(workspace) || recipient == key(0),
        }
    }
}

pub struct InboxRequest<'a> {
    pub scope: MailboxScope<'a>,
    pub include_acknowledged: bool,
    pub after: Option<&'a str>,
    pub limit: usize,
    pub acknowledge: &'a [&'a str],
    /// A native ACK-only call returns a receipt without surfacing the next page.
    pub return_messages: bool,
    pub timestamp: u64,
}
#[derive(Debug)]
pub struct InboxReceipt {
    pub messages: Vec<Value>,
    pub acknowledged: Vec<String>,
    pub pending: u64,
    pub total: u64,
    pub remaining: u64,
    pub next_after: Option<String>,
    pub changed: bool,
}

impl Store {
    pub fn inbox(&mut self, request: InboxRequest<'_>) -> Result<InboxReceipt> {
        if !(1..=32).contains(&request.limit) || request.acknowledge.len() > 4096 {
            return Err(Error::Invalid("inbox request exceeds bound"));
        }
        if request.acknowledge.iter().any(|id| !id_valid(id))
            || request.after.is_some_and(|id| !id_valid(id))
        {
            return Err(Error::Invalid("invalid message identity"));
        }
        self.transact(|tx| inbox(tx, &request))
    }
}
pub(super) fn inbox(db: &Connection, request: &InboxRequest<'_>) -> Result<InboxReceipt> {
    inbox_validated(db, request, &|_| Ok(()))
}
pub(super) fn inbox_validated(
    db: &Connection,
    request: &InboxRequest<'_>,
    validate: &dyn Fn(&Value) -> Result<()>,
) -> Result<InboxReceipt> {
    let changes_before = db.total_changes();
    let after = match request.after {
        None => 0,
        Some(id) => {
            let row: Option<(i64, String, Option<String>)> = db
                .query_row(
                    "SELECT seq,recipient,conversation FROM messages WHERE id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((seq, recipient, conversation)) = row else {
                return Err(Error::Invalid("unknown inbox cursor"));
            };
            if !request.scope.allows(&recipient, conversation.as_deref()) {
                return Err(Error::Invalid("inbox cursor outside scope"));
            }
            seq
        }
    };
    // Any validation, read or write failure below rolls back the entire call.
    for id in request.acknowledge {
        let row: Option<(String,Option<String>,String)> = db.query_row(
            "SELECT recipient,conversation,value FROM messages JOIN lifecycle USING(id) WHERE id=?",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let Some((recipient, conversation, state)) = row else {
            return Err(Error::Invalid("unknown inbox acknowledgement"));
        };
        if !request.scope.allows(&recipient, conversation.as_deref()) {
            return Err(Error::Invalid("inbox acknowledgement outside scope"));
        }
        let (base, raw): (String, String) = db.query_row(
            "SELECT original,value FROM messages JOIN lifecycle USING(id) WHERE id=?",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        validate(&checked_message(db, id, base, raw)?)?;
        let mut state: Map<String, Value> = serde_json::from_str(&state)?;
        if state.get("acknowledged").is_none_or(Value::is_null) {
            state.insert("acknowledged".into(), json!(request.timestamp));
            db.execute(
                "UPDATE lifecycle SET value=? WHERE id=?",
                params![serde_json::to_string(&state)?, id],
            )?;
        }
    }
    let (recipient, human, conversation) = request.scope.values();
    let (total, pending, following): (i64, i64, i64) = db.query_row(
        &counts_sql(human),
        params![
            recipient,
            human,
            conversation,
            after,
            request.include_acknowledged
        ],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let total = u64::try_from(total).map_err(|_| Error::Invalid("invalid inbox count"))?;
    let pending =
        u64::try_from(pending).map_err(|_| Error::Invalid("invalid inbox pending count"))?;
    let following =
        u64::try_from(following).map_err(|_| Error::Invalid("invalid inbox following count"))?;
    let mut messages = Vec::new();
    let mut surfaced = Vec::new();
    if request.return_messages {
        let mut statement=db.prepare(
            "SELECT id,original,value FROM messages JOIN lifecycle USING(id)
             WHERE ((recipient=?1 AND (?2 OR conversation IS NULL OR conversation=?3)) OR (?2 AND recipient='0000000000000000'))
               AND seq>?4 AND (?5 OR pending=1) ORDER BY seq LIMIT ?6")?;
        let mut rows = statement.query(params![
            recipient,
            human,
            conversation,
            after,
            request.include_acknowledged,
            request.limit as i64
        ])?;
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let original: String = row.get(1)?;
            let lifecycle: String = row.get(2)?;
            let mut state: Map<String, Value> = serde_json::from_str(&lifecycle)?;
            let mut changed = false;
            if !human {
                for field in ["surfaced", "native_surfaced"] {
                    if state.get(field).is_none_or(Value::is_null) {
                        state.insert(field.into(), json!(request.timestamp));
                        changed = true;
                    }
                }
            }
            let state = serde_json::to_string(&state)?;
            let record = checked_message(db, &id, original, state.clone())?;
            validate(&record)?;
            let size = serde_json::to_vec(&record)?.len();
            if !human && !messages.is_empty() && bytes + size > 32 * 1024 {
                break;
            }
            bytes += size;
            messages.push(record);
            if changed {
                surfaced.push((id, state));
            }
        }
    }
    // Only accepted page records get surfaced. Over-budget lookahead gets no receipt.
    for (id, state) in surfaced {
        db.execute(
            "UPDATE lifecycle SET value=? WHERE id=?",
            params![state, id],
        )?;
    }
    let remaining = following
        .checked_sub(messages.len() as u64)
        .ok_or(Error::Invalid("inconsistent inbox count"))?;
    let next_after = if remaining > 0 {
        messages
            .last()
            .and_then(|m| m["id"].as_str())
            .map(str::to_owned)
    } else {
        None
    };
    Ok(InboxReceipt {
        messages,
        acknowledged: request
            .acknowledge
            .iter()
            .map(|s| (*s).to_owned())
            .collect(),
        pending,
        total: if request.include_acknowledged {
            total
        } else {
            pending
        },
        remaining,
        next_after,
        changed: db.total_changes() != changes_before,
    })
}

pub(super) fn counts_sql(_human: bool) -> String {
    "SELECT count(*),coalesce(sum(pending),0),coalesce(sum(CASE WHEN seq>?4 AND (?5 OR pending=1) THEN 1 ELSE 0 END),0)
         FROM messages JOIN lifecycle USING(id)
         WHERE (recipient=?1 AND (?2 OR conversation IS NULL OR conversation=?3)) OR (?2 AND recipient='0000000000000000')".to_owned()
}
