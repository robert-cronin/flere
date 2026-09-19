use super::*;
use crate::record_store::{
    DocumentKind as Kind, DocumentSelection as Docs, InboxRequest, MailboxScope,
    MessageSelection as Mail, Scope, WorkingDocument,
};

#[derive(Default)]
struct View {
    messages: Vec<Value>,
    decisions: Vec<WorkingDocument>,
    checkpoints: Vec<WorkingDocument>,
    dispatches: Vec<WorkingDocument>,
    context: Value,
}
impl View {
    fn document(&mut self, kind: Kind, rows: Vec<WorkingDocument>) {
        let target = match kind {
            Kind::Decision => &mut self.decisions,
            Kind::Checkpoint => &mut self.checkpoints,
            Kind::Dispatch => &mut self.dispatches,
        };
        for row in rows {
            if !target.iter().any(|r| r.id == row.id) {
                target.push(row);
            }
        }
        target.sort_by_key(|r| r.ordinal);
    }
    fn mail(&mut self, rows: Vec<Value>) {
        for row in rows {
            if !self.messages.iter().any(|r| r["id"] == row["id"]) {
                self.messages.push(row);
            }
        }
    }
}
impl Storage {
    fn run<T: Send>(
        &mut self,
        server: &mut Server,
        action: impl FnOnce(&mut Store) -> CommitOutcome<T> + Send,
    ) -> io::Result<T> {
        self.ready()?;
        let mut store = self.store.take().unwrap();
        self.unresolved = Some("record job did not return a verified outcome".into());
        let (returned, result) = server.with_durable_job(move || {
            Ok(match action(&mut store) {
                CommitOutcome::Committed(value) => (Some(store), Ok(value)),
                CommitOutcome::Unchanged(error) => (Some(store), Err(failure(error))),
                CommitOutcome::AlreadyCommitted => (
                    Some(store),
                    Err(invalid("operation already committed; retrieve its result")),
                ),
                CommitOutcome::Unavailable => (
                    None,
                    Err(invalid("record storage unavailable pending recovery")),
                ),
                CommitOutcome::Uncertain {
                    ticket,
                    tentative,
                    cause,
                } => match store.reconcile(ticket) {
                    Ok(recovered) => match (recovered.resolution, tentative) {
                        (Resolution::Committed, Some(value)) => (Some(recovered.store), Ok(value)),
                        (Resolution::Committed, None) => (
                            Some(recovered.store),
                            Err(invalid("operation committed; retrieve its result")),
                        ),
                        (Resolution::Unchanged, _) => (Some(recovered.store), Err(failure(cause))),
                    },
                    Err(error) => (None, Err(failure(error))),
                },
            })
        })?;
        self.store = returned;
        self.unresolved = if self.store.is_none() {
            Some("record outcome unresolved; restart requires recovery".into())
        } else {
            None
        };
        result
    }
}
impl Server {
    fn records(&self) -> io::Result<&Store> {
        self.storage
            .as_ref()
            .ok_or_else(|| invalid("record storage unavailable"))?
            .ready()
    }
    fn with_record_view<T>(
        &mut self,
        view: View,
        action: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        self.records()?;
        let mut next = coordination::Coordination {
            messages: serde_json::from_value(json!(view.messages)).map_err(failure)?,
            decisions: serde_json::from_value(json!(
                view.decisions.iter().map(|r| &r.value).collect::<Vec<_>>()
            ))
            .map_err(failure)?,
            checkpoints: view.checkpoints.iter().map(|r| r.value.clone()).collect(),
            dispatches: serde_json::from_value(json!(
                view.dispatches.iter().map(|r| &r.value).collect::<Vec<_>>()
            ))
            .map_err(failure)?,
            delivery: self.coordination.delivery.clone(),
        };
        next.validate_dispatches()?;
        next.validate_chat_messages()?;
        let before = json!(next);
        std::mem::swap(&mut self.coordination, &mut next);
        let storage = self.storage.as_mut().unwrap();
        storage.before = before;
        storage.active_view = true;
        storage.checkpoint_ids = view.checkpoints.into_iter().map(|r| r.id).collect();
        storage.context_fields = view.context;
        let result = action(self);
        // Keep current observations, but never retain historical bodies between
        // operations. A failed commit leaves the last published layout intact.
        self.coordination.messages.clear();
        self.coordination.decisions.clear();
        self.coordination.checkpoints.clear();
        self.coordination.dispatches.clear();
        let storage = self.storage.as_mut().unwrap();
        storage.active_view = false;
        storage.before = json!(self.coordination);
        storage.checkpoint_ids.clear();
        storage.context_fields = Value::Null;
        result
    }
    pub(in crate::server) fn complete_record_context(&self, result: &mut Value) {
        if let Some(storage) = &self.storage
            && let Some(fields) = storage.context_fields.as_object()
        {
            result.as_object_mut().unwrap().extend(fields.clone());
            result["decisions_remaining"] = json!(
                result["decisions_total"]
                    .as_u64()
                    .unwrap()
                    .saturating_sub(result["decisions"].as_array().unwrap().len() as u64)
            );
        }
    }
    pub(in crate::server) fn record_coordinate(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
        op: &str,
        args: &Value,
    ) -> io::Result<Value> {
        if !self.workspaces.iter().any(|w| w.id == wid) {
            return Err(invalid("unknown workspace"));
        }
        self.records()?;
        if op == "inbox" {
            return self.record_inbox(wid, native, args);
        }
        if matches!(op, "decisions" | "checkpoints") {
            return self.record_history(wid, native.is_some(), op, args);
        }
        let mut view = View::default();
        match op {
            "context" => {
                let conversation = self.mailbox_conversation(wid, native);
                let scope = if native.is_some() {
                    MailboxScope::Native(Scope {
                        workspace: wid,
                        conversation: conversation.as_deref(),
                    })
                } else {
                    MailboxScope::Human { workspace: wid }
                };
                let store = self.records()?;
                view.mail(
                    store
                        .select_messages(Mail::Context {
                            scope,
                            handled: false,
                            limit: 32,
                        })
                        .map_err(failure)?,
                );
                if native.is_none() {
                    let mut handled = store
                        .select_messages(Mail::Context {
                            scope,
                            handled: true,
                            limit: 32,
                        })
                        .map_err(failure)?;
                    handled.reverse();
                    view.mail(handled);
                }
                view.document(
                    Kind::Decision,
                    store
                        .select_documents(Kind::Decision, Docs::ContextDecisions { workspace: wid })
                        .map_err(failure)?,
                );
                view.document(
                    Kind::Checkpoint,
                    store
                        .select_documents(
                            Kind::Checkpoint,
                            Docs::Recent {
                                workspace: wid,
                                limit: 8,
                            },
                        )
                        .map_err(failure)?,
                );
                view.document(
                    Kind::Dispatch,
                    store
                        .select_documents(Kind::Dispatch, Docs::Assignment { workspace: wid })
                        .map_err(failure)?,
                );
                let (messages, pending) = store.message_counts(scope).map_err(failure)?;
                let (decisions, pending_decisions) = store
                    .document_counts(Kind::Decision, wid)
                    .map_err(failure)?;
                let (checkpoints, _) = store
                    .document_counts(Kind::Checkpoint, wid)
                    .map_err(failure)?;
                view.context = json!({"messages_total":messages,"pending_messages":pending,"decisions_total":decisions,"pending_decisions":pending_decisions,"checkpoints_total":checkpoints,"latest_checkpoint_id":view.checkpoints.last().map(|r|r.id.clone())});
            }
            "read_message" | "message_status" | "deliver_message" => {
                if let Some(id) = args["id"].as_str() {
                    view.mail(
                        self.records()?
                            .select_messages(Mail::Id(id))
                            .map_err(failure)?,
                    );
                }
            }
            "send_chat_message" => {
                if let Some((session, run)) = native {
                    let sender = self.proof(session, run)?;
                    if let Some(request) = args["request_id"].as_str()
                        && let Some(message) = self
                            .records()?
                            .message_by_request(wid, &sender.uuid, request)
                            .map_err(failure)?
                    {
                        view.mail(vec![message]);
                    }
                }
            }
            "answer_decision" => {
                if let Some(id) = args["id"].as_str() {
                    view.document(
                        Kind::Decision,
                        self.records()?
                            .select_documents(Kind::Decision, Docs::Id(id))
                            .map_err(failure)?,
                    );
                }
            }
            "prepare_worker" => {
                if let Some(request) = args["request_id"].as_str() {
                    view.document(
                        Kind::Dispatch,
                        self.records()?
                            .select_documents(
                                Kind::Dispatch,
                                Docs::Request {
                                    owner: wid,
                                    request,
                                },
                            )
                            .map_err(failure)?,
                    );
                }
                if let Some(workspace) = args["workspace"].as_u64() {
                    view.document(
                        Kind::Dispatch,
                        self.records()?
                            .select_documents(Kind::Dispatch, Docs::ActiveDispatch { workspace })
                            .map_err(failure)?,
                    );
                }
            }
            "list_workspaces" => {
                if args["detail"] == true
                    && let Some(workspace) = args["workspace"].as_u64()
                {
                    view.document(
                        Kind::Dispatch,
                        self.records()?
                            .select_documents(
                                Kind::Dispatch,
                                Docs::Recent {
                                    workspace,
                                    limit: 64,
                                },
                            )
                            .map_err(failure)?,
                    );
                }
            }
            "start_worker"
            | "worker_status"
            | "ack_assignment"
            | "report_worker_block"
            | "cancel_worker" => {
                if let Some(id) = args["dispatch_id"].as_str() {
                    view.document(
                        Kind::Dispatch,
                        self.records()?
                            .select_documents(Kind::Dispatch, Docs::Id(id))
                            .map_err(failure)?,
                    );
                }
            }
            _ => {}
        }
        self.with_record_view(view, |server| server.coordinate(wid, native, op, args))
    }
    fn record_inbox(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
        args: &Value,
    ) -> io::Result<Value> {
        let conversation = self.mailbox_conversation(wid, native);
        let scope = if native.is_some() {
            MailboxScope::Native(Scope {
                workspace: wid,
                conversation: conversation.as_deref(),
            })
        } else {
            MailboxScope::Human { workspace: wid }
        };
        let include_acknowledged = match args.get("include_acknowledged") {
            None => false,
            Some(Value::Bool(v)) => *v,
            _ => return Err(invalid("include_acknowledged must be a boolean")),
        };
        let ids = match args.get("ack_ids") {
            None => Vec::new(),
            Some(Value::Array(ids)) => ids
                .iter()
                .map(|v| {
                    v.as_str()
                        .ok_or_else(|| invalid("acknowledgement must be a message ID"))
                })
                .collect::<io::Result<Vec<_>>>()?,
            _ => return Err(invalid("ack_ids must be an array")),
        };
        let after = optional_text(args, "after")?;
        let limit =
            super::super::context_view::limit(args, if native.is_some() { 8 } else { 32 }, 32)?;
        let operation = os::nonce()?;
        let request = InboxRequest {
            scope,
            include_acknowledged,
            after,
            limit,
            acknowledge: &ids,
            return_messages: !(native.is_some()
                && !ids.is_empty()
                && args.get("after").is_none()
                && args.get("limit").is_none()),
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        };
        let mut storage = self.storage.take().unwrap();
        let result = storage.run(self, |store| {
            store.inbox_validated_witnessed(&operation, request, &validate_mail)
        });
        self.storage = Some(storage);
        let reply = result?;
        if reply.changed {
            self.changed();
            self.audit(
                "inbox",
                wid,
                if native.is_some() {
                    "native agent"
                } else {
                    "human UI/CLI"
                },
            )?;
        }
        Ok(
            json!({"messages":reply.messages,"acknowledged":reply.acknowledged,"pending":reply.pending,"total":reply.total,"remaining":reply.remaining,"next_after":reply.next_after}),
        )
    }
    fn record_history(&self, wid: u64, agent: bool, op: &str, args: &Value) -> io::Result<Value> {
        let kind = if op == "decisions" {
            Kind::Decision
        } else {
            Kind::Checkpoint
        };
        let id = optional_text(args, "id")?;
        if id.is_some() && (args.get("after").is_some() || args.get("limit").is_some()) {
            return Err(invalid("id cannot be combined with pagination"));
        }
        let store = self.records()?;
        if let Some(id) = id {
            let record = store.document_by_id(kind, wid, id).map_err(failure)?;
            let (total, pending) = store.document_counts(kind, wid).map_err(failure)?;
            let mut result = json!({op:[record],"total":total,"remaining":0,"next_after":null});
            if op == "decisions" {
                result["pending"] = json!(pending);
            }
            return Ok(result);
        }
        let limit = super::super::context_view::limit(
            args,
            if agent || op == "checkpoints" { 8 } else { 32 },
            32,
        )?;
        let page = store
            .documents_page(kind, wid, optional_text(args, "after")?, limit)
            .map_err(failure)?;
        let mut result = json!({op:page.records,"total":page.total,"remaining":page.remaining,"next_after":page.next_after});
        if op == "decisions" {
            result["pending"] = json!(page.pending);
        }
        Ok(result)
    }
}
fn optional_text<'a>(args: &'a Value, key: &str) -> io::Result<Option<&'a str>> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        _ => Err(invalid("record identity must be a string")),
    }
}

impl Server {
    pub(in crate::server) fn record_message_view<T>(
        &mut self,
        id: &str,
        action: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut view = View::default();
        view.mail(
            self.records()?
                .select_messages(Mail::Id(id))
                .map_err(failure)?,
        );
        self.with_record_view(view, action)
    }
    pub(in crate::server) fn record_lease_view<T>(
        &mut self,
        lease: &str,
        action: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut view = View::default();
        view.mail(
            self.records()?
                .select_messages(Mail::Lease(lease))
                .map_err(failure)?,
        );
        self.with_record_view(view, action)
    }
    pub(in crate::server) fn record_notice_view<T>(
        &mut self,
        wid: u64,
        native: (u64, &str),
        interrupt_only: bool,
        expired_hooks: bool,
        extra: Option<&str>,
        action: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        let conversation = self.mailbox_conversation(wid, Some(native));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut view = View::default();
        view.mail(
            self.records()?
                .select_messages(Mail::Notice {
                    scope: Scope {
                        workspace: wid,
                        conversation: conversation.as_deref(),
                    },
                    interrupt_only,
                    expired_hooks,
                    now,
                })
                .map_err(failure)?,
        );
        if let Some(id) = extra {
            view.mail(
                self.records()?
                    .select_messages(Mail::Id(id))
                    .map_err(failure)?,
            );
        }
        self.with_record_view(view, action)
    }
    pub(in crate::server) fn record_worker_view<T>(
        &mut self,
        id: &str,
        action: impl FnOnce(&mut Self) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut view = View::default();
        view.document(
            Kind::Dispatch,
            self.records()?
                .select_documents(Kind::Dispatch, Docs::Id(id))
                .map_err(failure)?,
        );
        self.with_record_view(view, action)
    }
    pub(in crate::server) fn record_queued_handoff(
        &self,
        workspace: u64,
        conversation: &str,
        run: &str,
    ) -> io::Result<bool> {
        self.records()?
            .queued_handoff(workspace, conversation, run)
            .map_err(failure)
    }
    pub(in crate::server) fn record_delivery_candidates(
        &mut self,
    ) -> io::Result<Vec<(usize, String)>> {
        let after = self
            .storage
            .as_ref()
            .and_then(|s| s.delivery_after.as_deref());
        let mut ids = self
            .records()?
            .delivery_candidates(after, 16)
            .map_err(failure)?;
        if ids.is_empty() && after.is_some() {
            ids = self
                .records()?
                .delivery_candidates(None, 16)
                .map_err(failure)?;
        }
        Ok(ids.into_iter().enumerate().collect())
    }
    pub(in crate::server) fn advance_record_delivery(&mut self, id: &str) {
        self.storage.as_mut().unwrap().delivery_after = Some(id.into());
    }
    pub(in crate::server) fn record_backend(&self) -> bool {
        self.storage.as_ref().is_some_and(|s| !s.legacy_view)
    }
    pub(in crate::server) fn record_has_notice_candidate(&self, wid: u64) -> io::Result<bool> {
        self.records()?.has_notice_candidate(wid).map_err(failure)
    }
    pub(in crate::server) fn record_has_bound_mail(&self, wid: u64) -> io::Result<bool> {
        self.records()?.has_bound_mail(wid).map_err(failure)
    }
}
fn validate_mail(value: &Value) -> crate::record_store::Result<()> {
    let message: coordination::Message = serde_json::from_value(value.clone())?;
    let records = coordination::Coordination {
        messages: vec![message],
        ..Default::default()
    };
    records
        .validate_chat_messages()
        .map_err(crate::record_store::Error::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_provenance_validation_rolls_back_other_acknowledgements_in_the_call() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(&os::nonce().unwrap()[..12]);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .unwrap();
        let mut store = Store::open(&root).unwrap();
        let good = json!({"id":format!("{:032x}",1),"from":1,"to":2,"body":"synthetic exact record","intent":"quiet","saved":1,"surfaced":null,"native_surfaced":null,"acknowledged":null,"delivery":null});
        let mut bad = good.clone();
        bad["id"] = json!(format!("{:032x}", 2));
        bad["chat"] = json!({"conversation":"22222222-aaaa-aaaa-aaaa-222222222222","initial_session":2,"initial_run":format!("{:032x}",3),"request_id":"synthetic-request","sender":{"kind":"human","session":1,"run":format!("{:032x}",4),"conversation":"11111111-aaaa-aaaa-aaaa-111111111111"}});
        store.insert(&[good.clone(), bad]).unwrap();
        let id = good["id"].as_str().unwrap();
        let ack = [id];
        let outcome = store.inbox_validated_witnessed(
            &os::nonce().unwrap(),
            InboxRequest {
                scope: MailboxScope::Human { workspace: 2 },
                include_acknowledged: true,
                after: None,
                limit: 8,
                acknowledge: &ack,
                return_messages: true,
                timestamp: 42,
            },
            &validate_mail,
        );
        assert!(matches!(outcome, CommitOutcome::Unchanged(_)));
        assert!(
            store
                .get(
                    id,
                    Scope {
                        workspace: 2,
                        conversation: None
                    }
                )
                .unwrap()
                .unwrap()["acknowledged"]
                .is_null()
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
