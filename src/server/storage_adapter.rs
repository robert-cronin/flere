//! Contract comparison between legacy JSON behavior and record transactions.
//! Uses the application's current argument contract and fresh mailbox scope;
//! comparisons below do not claim to exercise native process ownership proof.
use super::*;
use crate::record_store::{CommitOutcome, InboxRequest, MailboxScope, Resolution, Scope, Store};
use serde_json::{Value, json};

fn storage_inbox(
    server: &mut Server,
    storage: &mut Storage,
    wid: u64,
    native: Option<(u64, &str)>,
    args: &Value,
    timestamp: u64,
) -> io::Result<Value> {
    let agent = native.is_some();
    let conversation = server.mailbox_conversation(wid, native);
    let scope = if agent {
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
        Some(Value::Array(a)) => a
            .iter()
            .map(|id| {
                id.as_str()
                    .ok_or_else(|| invalid("acknowledgement must be a message ID"))
            })
            .collect::<io::Result<Vec<_>>>()?,
        _ => return Err(invalid("ack_ids must be an array")),
    };
    let after = match args.get("after") {
        None => None,
        Some(Value::String(id)) => Some(id.as_str()),
        _ => return Err(invalid("after must be a message ID")),
    };
    let limit = super::context_view::limit(args, if agent { 8 } else { 32 }, 32)?;
    let request = InboxRequest {
        scope,
        include_acknowledged,
        after,
        limit,
        acknowledge: &ids,
        return_messages: !(agent
            && !ids.is_empty()
            && args.get("after").is_none()
            && args.get("limit").is_none()),
        timestamp,
    };
    let operation = os::nonce()?;
    let reply = storage.run(server, move |store| {
        store.inbox_witnessed(&operation, request)
    })?;
    Ok(
        json!({"messages":reply.messages,"acknowledged":reply.acknowledged,"pending":reply.pending,"total":reply.total,"remaining":reply.remaining,"next_after":reply.next_after}),
    )
}

/// Owns the connection between jobs. Unresolved outcomes take it out of service;
/// they are never mapped to an ordinary rollback followed by more writes.
struct Storage {
    store: Option<Store>,
    unresolved: Option<String>,
}
impl Storage {
    fn run<T: Send>(
        &mut self,
        server: &mut Server,
        action: impl FnOnce(&mut Store) -> CommitOutcome<T> + Send,
    ) -> io::Result<T> {
        let mut store = self
            .store
            .take()
            .ok_or_else(|| invalid("storage unavailable pending recovery"))?;
        // If starting/joining the job fails, retain the unavailable state. In
        // particular a panic is not evidence that the transaction rolled back.
        self.unresolved = Some("storage job did not return a verified outcome".into());
        let (returned, result) = server.with_durable_job(move || {
            let outcome = action(&mut store);
            let result = match outcome {
                CommitOutcome::Committed(value) => (Some(store), Ok(value)),
                CommitOutcome::Unchanged(error) => (
                    Some(store),
                    Err(io::Error::other(format!("storage unchanged: {error:?}"))),
                ),
                CommitOutcome::AlreadyCommitted => (
                    Some(store),
                    Err(io::Error::other(
                        "operation already committed; retrieve its current result",
                    )),
                ),
                CommitOutcome::Uncertain {
                    ticket,
                    tentative,
                    cause,
                } => match store.reconcile(ticket) {
                    Ok(recovery) => match (recovery.resolution, tentative) {
                        (Resolution::Committed, Some(value)) => (Some(recovery.store), Ok(value)),
                        (Resolution::Committed, None) => (
                            Some(recovery.store),
                            Err(io::Error::other(
                                "operation already committed; retrieve its current result",
                            )),
                        ),
                        (Resolution::Unchanged, _) => (
                            Some(recovery.store),
                            Err(io::Error::other(format!(
                                "storage unchanged after recovery: {cause:?}"
                            ))),
                        ),
                    },
                    Err(error) => (
                        None,
                        Err(io::Error::other(format!(
                            "storage outcome unresolved for {}: {:?}",
                            error.ticket.id(),
                            error.cause
                        ))),
                    ),
                },
                CommitOutcome::Unavailable => (
                    None,
                    Err(io::Error::other("storage unavailable pending recovery")),
                ),
            };
            Ok(result)
        })?;
        self.store = returned;
        self.unresolved = if self.store.is_none() {
            result.as_ref().err().map(ToString::to_string)
        } else {
            None
        };
        result
    }
}

struct Fixture {
    root: PathBuf,
    state: PathBuf,
    database: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(&os::nonce().unwrap()[..12]);
        let state = root.join("s");
        let database = root.join("db");
        private_state(&state).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&database).unwrap();
        Self {
            root,
            state,
            database,
        }
    }
    fn seeded(&self) -> Server {
        let mut server = Server::load(&self.state).unwrap();
        for id in 1..=3 {
            server.workspaces.push(Workspace {
                id,
                name: format!("synthetic-{id}"),
                cwd: self.root.clone(),
                tabs: Vec::new(),
                selected: 0,
                split: None,
                meta: CardMeta::default(),
            });
        }
        server.active = 2;
        server.next = 4;
        for n in 1..=16_u64 {
            let chat = (n.is_multiple_of(4)).then(|| super::chat_messages::Address {
                conversation: "00000000-1234-5678-9012-000000000002".into(),
                initial_session: 8,
                initial_run: format!("{:032x}", 8),
                sender: super::chat_messages::Sender {
                    kind: "agent".into(),
                    session: 7,
                    run: format!("{:032x}", 7),
                    conversation: "00000000-1234-5678-9012-000000000001".into(),
                },
                request_id: format!("request-{n}"),
                user_request_ref: Some("synthetic local-only constraint".into()),
            });
            server
                .coordination
                .messages
                .push(super::coordination::Message {
                    id: format!("{n:032x}"),
                    from: 1,
                    to: if n == 3 {
                        0
                    } else if n == 7 {
                        3
                    } else {
                        2
                    },
                    body: if n == 9 {
                        "\u{0001}".repeat(8000)
                    } else if n.is_multiple_of(3) {
                        "界".repeat(5000)
                    } else {
                        format!("synthetic message {n}")
                    },
                    intent: "quiet".into(),
                    saved: 100,
                    surfaced: None,
                    native_surfaced: None,
                    acknowledged: if n == 5 { Some(101) } else { None },
                    delivery: None,
                    chat,
                });
        }
        server.coordination.validate_chat_messages().unwrap();
        server.persist().unwrap();
        server
    }
    fn import(&self) -> Store {
        // Run actual saved-state domain validation before the structural import.
        // Fixture files are immutable between these reads; production needs a
        // single captured byte image under its existing ownership/lock boundary.
        let validated = Server::load(&self.state).unwrap();
        assert!(validated.workspaces.iter().all(|w| w.tabs.is_empty()));
        let mut store = Store::open(&self.database).unwrap();
        store
            .import_legacy(
                &fs::read(self.state.join("workspaces.v2.json")).unwrap(),
                None,
            )
            .unwrap();
        store
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn timestamp(before: &Value, after: &Value) -> u64 {
    for (a, b) in before["messages"]
        .as_array()
        .unwrap()
        .iter()
        .zip(after["messages"].as_array().unwrap())
    {
        for field in ["acknowledged", "surfaced", "native_surfaced"] {
            if a[field] != b[field] {
                return b[field].as_u64().unwrap();
            }
        }
    }
    999
}
fn compare(server: &mut Server, storage: &mut Storage, native: bool, args: Value) {
    let caller = native.then_some((99, "synthetic-unverified-native"));
    // The absence of process proof must hide conversation-bound mail in both paths.
    let before = json!(server.coordination);
    let existing = server.coordination_inbox(2, caller, &args);
    let after = json!(server.coordination);
    let candidate = storage_inbox(
        server,
        storage,
        2,
        caller,
        &args,
        timestamp(&before, &after),
    );
    match (existing, candidate) {
        (Ok(a), Ok(b)) => assert_eq!(a, b, "reply differs for {args}"),
        (Err(_), Err(_)) => assert_eq!(before, after, "failed existing request mutated state"),
        (a, b) => panic!("different acceptance for {args}: {a:?} versus {b:?}"),
    }
    assert_eq!(
        storage
            .store
            .as_ref()
            .unwrap()
            .export_legacy_canonical()
            .unwrap()["coordination"],
        after,
        "durable records differ for {args}"
    );
}

#[test]
fn native_and_human_inbox_contract_matches_existing_server_over_mixed_sequences() {
    for native in [false, true] {
        let fixture = Fixture::new();
        let mut server = fixture.seeded();
        let mut storage = Storage {
            store: Some(fixture.import()),
            unresolved: None,
        };
        let id = |n: u64| format!("{n:032x}");
        for args in [
            json!({}),
            json!({"limit":1}),
            json!({"ack_ids":[id(1)]}),
            json!({"ack_ids":[id(1)]}),
            json!({"ack_ids":[id(2)],"limit":3}),
            json!({"include_acknowledged":true,"limit":32}),
            json!({"after":id(2),"limit":2}),
            json!({"ack_ids":[id(6)],"after":id(6),"limit":2}),
            json!({"ack_ids":[id(3)]}),
            json!({"ack_ids":[id(4)]}),
            json!({"ack_ids":[id(10),id(7)]}),
            json!({"after":id(7)}),
            json!({"ack_ids":[id(1),id(1)],"include_acknowledged":true,"limit":2}),
            json!({"limit":0}),
            json!({"limit":33}),
            json!({"limit":"1"}),
            json!({"include_acknowledged":1}),
            json!({"ack_ids":null}),
            json!({"ack_ids":[9]}),
            json!({"after":false}),
        ] {
            compare(&mut server, &mut storage, native, args);
        }
        let mut seed = 74u64;
        for _ in 0..40 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let n = (seed >> 32) % 18 + 1;
            let mut args =
                json!({"limit":(seed%8)+1,"include_acknowledged":seed.is_multiple_of(2)});
            if seed.is_multiple_of(3) {
                args["ack_ids"] = json!([id(n)]);
            }
            if seed.is_multiple_of(5) {
                args["after"] = json!(id(n));
            }
            compare(&mut server, &mut storage, native, args);
        }
        drop(storage);
        let reopened = Store::open(&fixture.database).unwrap();
        assert_eq!(
            reopened.export_legacy_canonical().unwrap()["coordination"],
            json!(server.coordination)
        );
    }
}

#[test]
fn actual_domain_validator_rejects_bad_native_provenance_before_import() {
    let fixture = Fixture::new();
    let _server = fixture.seeded();
    let path = fixture.state.join("workspaces.v2.json");
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["coordination"]["messages"][3]["chat"]["sender"]["kind"] = json!("human");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Server::load(&fixture.state).is_err());
    assert!(!fixture.database.join("state.sqlite").exists());
}
