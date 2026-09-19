use super::*;
const PAGE_BYTES: usize = 32 * 1024;
// Cover this query with metadata indexes so exact counts do not fetch message bodies.
pub(super) const COUNTS_SQL: &str = "SELECT count(*),coalesce(sum(pending),0),coalesce(sum(CASE WHEN seq>?3 AND (?4 OR pending=1) THEN 1 ELSE 0 END),0)
             FROM messages JOIN lifecycle USING(id) WHERE recipient=?1 AND (conversation IS NULL OR conversation=?2)";
#[derive(Debug)]
pub struct MessagePage {
    pub messages: Vec<Value>,
    pub total: u64,
    pub pending: u64,
    pub remaining: u64,
    pub next_after: Option<String>,
}
impl Store {
    /// Native scope is supplied by fresh caller ownership proof. This read does
    /// not itself claim surfacing or handling and includes no user-wide mailbox.
    pub fn messages_page(
        &self,
        scope: Scope<'_>,
        include_acknowledged: bool,
        after: Option<&str>,
        limit: usize,
    ) -> Result<MessagePage> {
        self.ensure_ready()?;
        if !(1..=32).contains(&limit) {
            return Err(Error::Invalid("invalid page limit"));
        }
        let tx = self.connection.unchecked_transaction()?;
        let after = match after {
            None => 0,
            Some(id) => {
                if !id_valid(id) {
                    return Err(Error::Invalid("invalid message cursor"));
                }
                let row: Option<(i64, String, Option<String>)> = tx
                    .query_row(
                        "SELECT seq,recipient,conversation FROM messages WHERE id=?",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                let Some((seq, recipient, conversation)) = row else {
                    return Err(Error::Invalid("unknown message cursor"));
                };
                if !allowed(&recipient, conversation.as_deref(), scope) {
                    return Err(Error::Invalid("wrong message cursor scope"));
                }
                seq
            }
        };
        let (total, pending, following): (i64, i64, i64) = tx.query_row(
            COUNTS_SQL,
            params![
                key(scope.workspace),
                scope.conversation,
                after,
                include_acknowledged
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let total = u64::try_from(total).map_err(|_| Error::Invalid("invalid message count"))?;
        let pending =
            u64::try_from(pending).map_err(|_| Error::Invalid("invalid pending count"))?;
        let following =
            u64::try_from(following).map_err(|_| Error::Invalid("invalid following count"))?;
        let mut statement=tx.prepare(
            "SELECT original,value,messages.id FROM messages JOIN lifecycle USING(id)
             WHERE recipient=?1 AND (conversation IS NULL OR conversation=?2) AND seq>?3 AND (?4 OR pending=1)
             ORDER BY seq LIMIT ?5")?;
        let mut rows = statement.query(params![
            key(scope.workspace),
            scope.conversation,
            after,
            include_acknowledged,
            limit as i64
        ])?;
        let mut messages = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let value = checked_message(&tx, &row.get::<_, String>(2)?, row.get(0)?, row.get(1)?)?;
            let size = serde_json::to_vec(&value)?.len();
            if !messages.is_empty() && bytes + size > PAGE_BYTES {
                break;
            }
            bytes += size;
            messages.push(value);
        }
        let remaining = following - messages.len() as u64;
        let next_after = if remaining > 0 {
            messages
                .last()
                .and_then(|m| m["id"].as_str())
                .map(str::to_owned)
        } else {
            None
        };
        // Drop the read transaction, never write a new receipt on a history read.
        Ok(MessagePage {
            messages,
            total: if include_acknowledged { total } else { pending },
            pending,
            remaining,
            next_after,
        })
    }
    /// Locate the original request before resolving a potentially stopped target.
    /// The caller still checks body/recipient/ref and changed target-run proof.
    pub fn message_by_request(
        &self,
        sender: u64,
        conversation: &str,
        request_id: &str,
    ) -> Result<Option<Value>> {
        self.ensure_ready()?;
        if conversation.is_empty()
            || conversation.len() > 64
            || request_id.trim().is_empty()
            || request_id.len() > 256
        {
            return Err(Error::Invalid("invalid sender request identity"));
        }
        let row:Option<(String,String,String)>=self.connection.query_row(
            "SELECT messages.id,original,value FROM messages JOIN lifecycle USING(id) WHERE sender=? AND sender_conversation=? AND request_id=?",
            params![key(sender),conversation,request_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        row.map(|(id, base, state)| checked_message(&self.connection, &id, base, state))
            .transpose()
    }
}
