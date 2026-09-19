use super::*;
use crate::record_store::tests::{Temp, message};

fn scope() -> Scope<'static> {
    Scope {
        workspace: 2,
        conversation: Some("a"),
    }
}
fn ids(page: &MessagePage) -> Vec<String> {
    page.messages
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect()
}
fn count(store: &Store, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}
fn source() -> Value {
    let mut old = message(1, 2, None);
    for field in STATE_FIELDS {
        old.as_object_mut().unwrap().remove(field);
    }
    old["extension"] = json!({"keep": [1, null, "界"]});
    json!({
        "version":10,"selected":2,"order":[2,3],"unknown_root":{"preserve":true},
        "workspaces":[
            {"id":2,"name":"synthetic-a","meta":{"status":"in-progress","unknown":"keep"},
             "tabs":[{"id":8,"kind":"shell","directory":"fixture"},{"id":5,"kind":"codex","conversation":"saved-conversation"}],"selected_tab":5},
            {"id":3,"name":"synthetic-b","meta":{"status":"idle"},"tabs":[],"selected_tab":null}
        ],
        "coordination":{
            "messages":[old,message(2,2,Some("a"))],
            "decisions":[{"id":"decision-a","workspace":2,"question":"Keep local?","answer":null,"unknown":[true]}],
            "checkpoints":[{"workspace":3,"kind":"note","body":"first"},{"workspace":2,"kind":"note","body":"second"},{"workspace":3,"kind":"note","body":"third"}],
            "dispatches":[{"id":"dispatch-a","workspace":2,"assignment":"preserve exact text","unknown":{"flag":false}}],
            "delivery":{"hooks":[{"synthetic":true}],"focus":[{"workspace":2}],"extension":"unchanged"},
            "extra_coordination":{"keep":null}
        }
    })
}

#[test]
fn paging_filters_counts_and_does_not_write_handling_receipts() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let mut handled = message(5, 2, Some("a"));
    handled["acknowledged"] = json!(7);
    let rows = vec![
        message(1, 2, None),
        message(2, 2, Some("a")),
        message(3, 2, Some("b")),
        message(4, 3, Some("a")),
        handled.clone(),
        message(6, 0, None),
    ];
    store.insert(&rows).unwrap();
    let page = store.messages_page(scope(), false, None, 32).unwrap();
    assert_eq!(page.messages, rows[..2]);
    assert_eq!((page.total, page.pending, page.remaining), (2, 2, 0));
    assert!(page.next_after.is_none());
    let all = store.messages_page(scope(), true, None, 32).unwrap();
    assert_eq!(
        all.messages,
        vec![rows[0].clone(), rows[1].clone(), handled]
    );
    assert_eq!((all.total, all.pending, all.remaining), (3, 2, 0));
    let no_chat = store
        .messages_page(
            Scope {
                conversation: None,
                ..scope()
            },
            true,
            None,
            32,
        )
        .unwrap();
    assert_eq!(no_chat.messages, vec![rows[0].clone()]);
    for row in &all.messages {
        assert!(row["surfaced"].is_null());
        assert!(row["native_surfaced"].is_null());
    }
    for limit in [0, 33, usize::MAX] {
        assert!(store.messages_page(scope(), false, None, limit).is_err());
    }
}

#[test]
fn cursor_survives_acknowledgement_restart_and_nonlexical_arrivals() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows = vec![
        message(20, 2, Some("a")),
        message(10, 2, None),
        message(30, 2, Some("a")),
        message(40, 2, Some("a")),
    ];
    store.insert(&rows).unwrap();
    let first = store.messages_page(scope(), false, None, 2).unwrap();
    assert_eq!(first.messages, rows[..2]);
    assert_eq!(first.remaining, 2);
    let seen = ids(&first);
    store
        .acknowledge(
            scope(),
            &seen.iter().map(String::as_str).collect::<Vec<_>>(),
            11,
        )
        .unwrap();
    let arrival = message(1, 2, Some("a"));
    store.insert(std::slice::from_ref(&arrival)).unwrap();
    drop(store);
    let store = Store::open(&dir.0).unwrap();
    let second = store
        .messages_page(scope(), false, first.next_after.as_deref(), 2)
        .unwrap();
    assert_eq!(second.messages, rows[2..]);
    assert_eq!((second.pending, second.remaining), (3, 1));
    let last = store
        .messages_page(scope(), false, second.next_after.as_deref(), 2)
        .unwrap();
    assert_eq!(last.messages, vec![arrival]);
    assert_eq!(last.remaining, 0);
    assert!(last.next_after.is_none());
    let after_last = store
        .messages_page(
            scope(),
            false,
            Some(last.messages[0]["id"].as_str().unwrap()),
            2,
        )
        .unwrap();
    assert!(after_last.messages.is_empty());
    assert_eq!(after_last.remaining, 0);
    for bad in ["bad".to_owned(), format!("{:032x}", 999)] {
        assert!(store.messages_page(scope(), true, Some(&bad), 2).is_err());
    }
    for wrong in [
        Scope {
            workspace: 3,
            ..scope()
        },
        Scope {
            conversation: Some("b"),
            ..scope()
        },
    ] {
        assert!(
            store
                .messages_page(wrong, true, Some(rows[0]["id"].as_str().unwrap()), 2)
                .is_err()
        );
    }
}

#[test]
fn serialized_byte_budget_keeps_oversized_unicode_records_whole() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let mut large = message(1, 2, Some("a"));
    large["body"] = json!(format!("{}界🦀", "\0".repeat(16000)));
    let mut next = message(2, 2, Some("a"));
    next["body"] = json!("界".repeat(5000));
    let mut third = message(3, 2, Some("a"));
    third["body"] = json!("x".repeat(16000));
    let fourth = message(4, 2, Some("a"));
    store
        .insert(&[large.clone(), next.clone(), third.clone(), fourth.clone()])
        .unwrap();
    let page = store.messages_page(scope(), false, None, 32).unwrap();
    assert_eq!(page.messages, vec![large.clone()]);
    assert_eq!(page.remaining, 3);
    assert!(serde_json::to_vec(&page.messages[0]).unwrap().len() > 32 * 1024);
    let rest = store
        .messages_page(scope(), false, page.next_after.as_deref(), 32)
        .unwrap();
    assert_eq!(rest.messages, vec![next, third, fourth]);
    assert!(
        rest.messages
            .iter()
            .map(|m| serde_json::to_vec(m).unwrap().len())
            .sum::<usize>()
            <= 32 * 1024
    );
    assert_eq!(
        store.get(large["id"].as_str().unwrap(), scope()).unwrap(),
        Some(large)
    );
}

#[test]
fn budget_stops_before_second_record_without_truncating_it() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows: Vec<_> = (1..=3)
        .map(|n| {
            let mut m = message(n, 2, Some("a"));
            m["body"] = json!("x".repeat(16384));
            m
        })
        .collect();
    store.insert(&rows).unwrap();
    let page = store.messages_page(scope(), false, None, 32).unwrap();
    assert_eq!(page.messages, rows[..1]);
    assert_eq!(page.remaining, 2);
    let next = store
        .messages_page(scope(), false, page.next_after.as_deref(), 32)
        .unwrap();
    assert_eq!(next.messages, rows[1..2]);
    assert_eq!(next.remaining, 1);
}

#[test]
fn request_lookup_is_exact_and_recovers_original_identity_after_restart() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let mut a = message(1, 2, Some("a"));
    a["from"] = json!(u64::MAX - 1);
    let mut b = a.clone();
    b["id"] = json!(format!("{:032x}", 2));
    b["chat"]["sender"]["conversation"] = json!("other");
    let mut c = a.clone();
    c["id"] = json!(format!("{:032x}", 3));
    c["from"] = json!(9);
    store.insert(&[a.clone(), b, c]).unwrap();
    drop(store);
    let store = Store::open(&dir.0).unwrap();
    assert_eq!(
        store
            .message_by_request(u64::MAX - 1, "sender", "request-1")
            .unwrap(),
        Some(a)
    );
    assert!(
        store
            .message_by_request(1, "sender", "request-1")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .message_by_request(u64::MAX - 1, "sender", "request-2")
            .unwrap()
            .is_none()
    );
    assert!(store.message_by_request(1, "", "request-1").is_err());
}

#[test]
fn bounded_page_and_lookup_do_not_decode_unselected_history() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows = vec![
        message(1, 2, Some("a")),
        message(2, 2, Some("a")),
        message(3, 2, Some("a")),
    ];
    store.insert(&rows).unwrap();
    store
        .connection
        .execute(
            "UPDATE messages SET original='{broken' WHERE id=?",
            [rows[1]["id"].as_str().unwrap()],
        )
        .unwrap();
    let page = store.messages_page(scope(), false, None, 1).unwrap();
    assert_eq!(page.messages, rows[..1]);
    assert_eq!((page.total, page.remaining), (3, 2));
    assert_eq!(
        store.message_by_request(1, "sender", "request-1").unwrap(),
        Some(rows[0].clone())
    );
    assert!(
        store
            .messages_page(scope(), false, page.next_after.as_deref(), 1)
            .is_err()
    );
    let last = store
        .messages_page(scope(), false, Some(rows[1]["id"].as_str().unwrap()), 1)
        .unwrap();
    assert_eq!(last.messages, rows[2..]);
}

#[test]
fn import_preserves_every_record_family_large_documents_and_raw_sources() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let mut expected = source();
    expected["workspaces"][0]["unknown_large"] = json!("界".repeat(100000));
    expected["coordination"]["checkpoints"][1]["body"] = json!("x".repeat(300000));
    let bytes = serde_json::to_vec_pretty(&expected).unwrap();
    let unused = b"unused fallback bytes are retained, not parsed";
    assert_eq!(
        store.import_legacy(&bytes, Some(unused)).unwrap(),
        ImportOutcome::Imported
    );
    assert_eq!(store.export_legacy_canonical().unwrap(), expected);
    assert_eq!(
        store.original_sources().unwrap(),
        (bytes.clone(), Some(unused.to_vec()))
    );
    assert!(count(&store, "document_chunks") > count(&store, "documents"));
    drop(store);
    let store = Store::open(&dir.0).unwrap();
    assert_eq!(store.export_legacy_canonical().unwrap(), expected);
    assert_eq!(
        store.original_sources().unwrap(),
        (bytes, Some(unused.to_vec()))
    );
    assert!(
        store
            .checkpoint_by_reference(2, "checkpoint:0")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store.checkpoint_by_reference(2, "checkpoint:1").unwrap(),
        Some(expected["coordination"]["checkpoints"][1].clone())
    );
    assert_eq!(
        store.checkpoint_by_reference(3, "checkpoint:2").unwrap(),
        Some(expected["coordination"]["checkpoints"][2].clone())
    );
    assert!(
        store
            .checkpoint_by_reference(3, "checkpoint:999")
            .unwrap()
            .is_none()
    );
    for reference in [
        "checkpoint:01",
        "checkpoint:-1",
        "checkpoint:+1",
        "checkpoint:1x",
        "message:1",
    ] {
        assert!(store.checkpoint_by_reference(2, reference).is_err());
    }
}

#[test]
fn legacy_versions_and_missing_or_null_inline_coordination_use_defaults_or_fallback() {
    for version in 2..=10 {
        for missing in [true, false] {
            for fallback in [
                None,
                Some(json!({"checkpoints":[{"workspace":2,"body":"retained"}],"extra":"yes"})),
            ] {
                let dir = Temp::new();
                let mut store = Store::open(&dir.0).unwrap();
                let mut original = source();
                original["version"] = json!(version);
                if missing {
                    original.as_object_mut().unwrap().remove("coordination");
                } else {
                    original["coordination"] = Value::Null;
                }
                let bytes = serde_json::to_vec(&original).unwrap();
                let fallback_bytes = fallback.as_ref().map(|v| serde_json::to_vec(v).unwrap());
                store
                    .import_legacy(&bytes, fallback_bytes.as_deref())
                    .unwrap();
                let mut expected = original;
                let mut coord = fallback.unwrap_or(json!({}));
                for field in ["messages", "decisions", "checkpoints", "dispatches"] {
                    coord
                        .as_object_mut()
                        .unwrap()
                        .entry(field)
                        .or_insert(json!([]));
                }
                coord["delivery"] = json!({"hooks":[],"focus":[]});
                expected["coordination"] = coord;
                assert_eq!(store.export_legacy_canonical().unwrap(), expected);
                assert_eq!(store.original_sources().unwrap(), (bytes, fallback_bytes));
            }
        }
    }
}

#[test]
fn same_source_retry_preserves_ack_and_empty_fallback_presence() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let expected = source();
    let bytes = serde_json::to_vec(&expected).unwrap();
    store.import_legacy(&bytes, Some(b"")).unwrap();
    let id = expected["coordination"]["messages"][0]["id"]
        .as_str()
        .unwrap();
    store.acknowledge(scope(), &[id], 55).unwrap();
    assert_eq!(
        store.import_legacy(&bytes, Some(b"")).unwrap(),
        ImportOutcome::AlreadyImported
    );
    let exported = store.export_legacy_canonical().unwrap();
    assert_eq!(exported["coordination"]["messages"][0]["acknowledged"], 55);
    assert!(store.import_legacy(&bytes, None).is_err());
    let mut different = expected;
    different["selected"] = json!(3);
    assert!(
        store
            .import_legacy(&serde_json::to_vec(&different).unwrap(), Some(b""))
            .is_err()
    );
    assert_eq!(store.export_legacy_canonical().unwrap(), exported);
    assert_eq!(store.original_sources().unwrap(), (bytes, Some(vec![])));
}

#[test]
fn nonempty_destination_and_invalid_sources_do_not_alter_existing_records() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let row = message(9, 2, Some("a"));
    store.insert(std::slice::from_ref(&row)).unwrap();
    assert!(
        store
            .import_legacy(&serde_json::to_vec(&source()).unwrap(), None)
            .is_err()
    );
    assert_eq!(
        store.get(row["id"].as_str().unwrap(), scope()).unwrap(),
        Some(row)
    );
    assert_eq!(count(&store, "import_sources"), 0);
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    for invalid in [
        json!({}),
        json!({"version":11,"workspaces":[]}),
        json!({"version":10,"workspaces":[{"id":2},{"id":2}]}),
        json!({"version":10,"workspaces":[],"coordination":{"messages":null}}),
    ] {
        assert!(
            store
                .import_legacy(&serde_json::to_vec(&invalid).unwrap(), None)
                .is_err()
        );
        assert_eq!(count(&store, "import_meta"), 0);
        assert_eq!(count(&store, "import_sources"), 0);
    }
    assert!(
        store
            .import_legacy(&vec![b' '; 8 * 1024 * 1024 + 1], None)
            .is_err()
    );
}

#[test]
fn import_failure_rolls_back_sources_records_chunks_and_completion_marker() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_dispatch BEFORE INSERT ON documents WHEN NEW.kind='dispatches' BEGIN SELECT RAISE(FAIL,'injected late import failure'); END;").unwrap();
    let bytes = serde_json::to_vec(&source()).unwrap();
    assert!(store.import_legacy(&bytes, None).is_err());
    for table in [
        "messages",
        "lifecycle",
        "workspace",
        "documents",
        "document_chunks",
        "import_sources",
        "import_meta",
        "checkpoints",
    ] {
        assert_eq!(count(&store, table), 0, "partial import in {table}");
    }
    drop(store);
    let mut store = Store::open(&dir.0).unwrap();
    assert_eq!(count(&store, "import_sources"), 0);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_dispatch")
        .unwrap();
    store.import_legacy(&bytes, None).unwrap();
    assert_eq!(store.export_legacy_canonical().unwrap(), source());
}

#[test]
fn post_import_result_appends_global_reference_and_status_atomically() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let original = source();
    let bytes = serde_json::to_vec(&original).unwrap();
    store.import_legacy(&bytes, None).unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_status BEFORE UPDATE ON workspace BEGIN SELECT RAISE(FAIL,'injected result failure'); END;").unwrap();
    assert!(
        store
            .submit_result("request-result", 2, "synthetic result")
            .is_err()
    );
    assert_eq!(store.export_legacy_canonical().unwrap(), original);
    assert_eq!(count(&store, "checkpoints"), 0);
    assert!(
        store
            .checkpoint_by_reference(2, "checkpoint:3")
            .unwrap()
            .is_none()
    );
    store
        .connection
        .execute_batch("DROP TRIGGER reject_status")
        .unwrap();
    store
        .submit_result("request-result", 2, "synthetic result")
        .unwrap();
    let new = store
        .checkpoint_by_reference(2, "checkpoint:3")
        .unwrap()
        .unwrap();
    assert_eq!(new["body"], "synthetic result");
    assert_eq!(new["workspace"], 2);
    assert!(new["time"].as_u64().is_some());
    let mut expected = original;
    expected["coordination"]["checkpoints"]
        .as_array_mut()
        .unwrap()
        .push(new);
    expected["workspaces"][0]["meta"]["status"] = json!("needs-me");
    assert_eq!(store.export_legacy_canonical().unwrap(), expected);
    assert_eq!(
        store.import_legacy(&bytes, None).unwrap(),
        ImportOutcome::AlreadyImported
    );
    assert_eq!(store.export_legacy_canonical().unwrap(), expected);
    assert!(
        store
            .submit_result("request-result", 2, "synthetic result")
            .is_err()
    );
    assert_eq!(store.export_legacy_canonical().unwrap(), expected);
}

#[test]
fn counting_history_uses_a_covering_metadata_index() {
    let dir = Temp::new();
    let store = Store::open(&dir.0).unwrap();
    let mut statement = store
        .connection
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            crate::record_store::history::COUNTS_SQL
        ))
        .unwrap();
    let plan: Vec<String> = statement
        .query_map(params![key(2), "a", 0, false], |row| row.get(3))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|step| step.contains("SEARCH messages USING COVERING INDEX recipient_order")),
        "counts may fetch historical bodies: {plan:?}"
    );
}

fn inbox_request<'a>(acknowledge: &'a [&'a str]) -> InboxRequest<'a> {
    InboxRequest {
        scope: MailboxScope::Native(scope()),
        include_acknowledged: false,
        after: None,
        limit: 8,
        acknowledge,
        return_messages: true,
        timestamp: 90,
    }
}

#[test]
fn inbox_ack_and_next_page_share_one_commit_and_retry_preserves_timestamps() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows = vec![message(1, 2, Some("a")), message(2, 2, Some("a"))];
    store.insert(&rows).unwrap();
    let ack = [rows[0]["id"].as_str().unwrap()];
    let mut request = inbox_request(&ack);
    request.limit = 1;
    let receipt = store.inbox(request).unwrap();
    assert_eq!(receipt.acknowledged, ack);
    assert_eq!(
        (receipt.pending, receipt.total, receipt.remaining),
        (1, 1, 0)
    );
    let mut expected = rows[1].clone();
    expected["surfaced"] = json!(90);
    expected["native_surfaced"] = json!(90);
    assert_eq!(receipt.messages, vec![expected.clone()]);
    drop(store);
    let mut store = Store::open(&dir.0).unwrap();
    let first = store.get(ack[0], scope()).unwrap().unwrap();
    assert_eq!(first["acknowledged"], 90);
    assert!(first["native_surfaced"].is_null());
    store.connection.execute_batch("CREATE TRIGGER reject_receipt_rewrite BEFORE UPDATE ON lifecycle BEGIN SELECT RAISE(FAIL,'retry rewrites state'); END;").unwrap();
    let mut retry = inbox_request(&ack);
    retry.timestamp = 999;
    let receipt = store.inbox(retry).unwrap();
    assert_eq!(receipt.messages, vec![expected]);
}

#[test]
fn native_ack_only_does_not_surface_or_return_the_next_page() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows = vec![message(1, 2, Some("a")), message(2, 2, Some("a"))];
    store.insert(&rows).unwrap();
    let ack = [
        rows[0]["id"].as_str().unwrap(),
        rows[0]["id"].as_str().unwrap(),
    ];
    let mut request = inbox_request(&ack);
    request.return_messages = false;
    let receipt = store.inbox(request).unwrap();
    assert!(receipt.messages.is_empty());
    assert_eq!(receipt.acknowledged, ack);
    assert_eq!(
        (receipt.pending, receipt.total, receipt.remaining),
        (1, 1, 1)
    );
    assert!(receipt.next_after.is_none());
    assert_eq!(
        store.get(rows[1]["id"].as_str().unwrap(), scope()).unwrap(),
        Some(rows[1].clone())
    );
}

#[test]
fn inbox_failure_after_ack_rolls_back_all_ack_and_surface_updates() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows: Vec<_> = (1..=3).map(|n| message(n, 2, Some("a"))).collect();
    store.insert(&rows).unwrap();
    let rejected = rows[2]["id"].as_str().unwrap();
    store.connection.execute_batch(&format!("CREATE TRIGGER fail_second_surface BEFORE UPDATE ON lifecycle WHEN NEW.id='{rejected}' BEGIN SELECT RAISE(FAIL,'injected surface failure'); END;")).unwrap();
    let ack = [rows[0]["id"].as_str().unwrap()];
    assert!(store.inbox(inbox_request(&ack)).is_err());
    for row in &rows {
        assert_eq!(
            store.get(row["id"].as_str().unwrap(), scope()).unwrap(),
            Some(row.clone())
        );
    }
    drop(store);
    let store = Store::open(&dir.0).unwrap();
    for row in rows {
        assert_eq!(
            store.get(row["id"].as_str().unwrap(), scope()).unwrap(),
            Some(row)
        );
    }
}

#[test]
fn inbox_body_decode_failure_rolls_back_an_earlier_ack() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let a = message(1, 2, Some("a"));
    let b = message(2, 2, Some("a"));
    store.insert(&[a.clone(), b.clone()]).unwrap();
    store
        .connection
        .execute(
            "UPDATE messages SET original='broken' WHERE id=?",
            [b["id"].as_str().unwrap()],
        )
        .unwrap();
    let ack = [a["id"].as_str().unwrap()];
    assert!(store.inbox(inbox_request(&ack)).is_err());
    assert_eq!(store.get(ack[0], scope()).unwrap(), Some(a));
}

#[test]
fn inbox_budget_marks_only_the_returned_records_and_preserves_oversized_body() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let mut rows: Vec<_> = (1..=3)
        .map(|n| {
            let mut m = message(n, 2, Some("a"));
            m["body"] = json!("x".repeat(16384));
            m
        })
        .collect();
    rows[0]["body"] = json!("\u{0001}".repeat(16000));
    store.insert(&rows).unwrap();
    let receipt = store.inbox(inbox_request(&[])).unwrap();
    assert_eq!(receipt.messages.len(), 1);
    assert_eq!(receipt.remaining, 2);
    assert_eq!(receipt.messages[0]["body"], rows[0]["body"]);
    assert_eq!(receipt.messages[0]["native_surfaced"], 90);
    for row in &rows[1..] {
        assert_eq!(
            store.get(row["id"].as_str().unwrap(), scope()).unwrap(),
            Some(row.clone())
        );
    }
    let mut request = inbox_request(&[]);
    request.after = receipt.next_after.as_deref();
    let second = store.inbox(request).unwrap();
    assert_eq!(second.messages.len(), 1);
    assert_eq!(second.remaining, 1);
}

#[test]
fn human_inbox_includes_user_mail_and_all_local_chats_without_native_receipts() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let rows = vec![
        message(1, 2, Some("a")),
        message(2, 2, Some("b")),
        message(3, 0, None),
        message(4, 3, None),
    ];
    store.insert(&rows).unwrap();
    let ack = [rows[2]["id"].as_str().unwrap()];
    let mut request = inbox_request(&ack);
    request.scope = MailboxScope::Human { workspace: 2 };
    request.include_acknowledged = true;
    let receipt = store.inbox(request).unwrap();
    let mut expected = rows[..3].to_vec();
    expected[2]["acknowledged"] = json!(90);
    assert_eq!(receipt.messages, expected);
    assert_eq!(
        (receipt.total, receipt.pending, receipt.remaining),
        (3, 2, 0)
    );
    let native = store.inbox(inbox_request(&[])).unwrap();
    assert_eq!(native.messages.len(), 1);
    assert_eq!(native.messages[0]["id"], rows[0]["id"]);
}

#[test]
fn inbox_rejects_mixed_scope_acks_and_wrong_scope_cursor_atomically() {
    let dir = Temp::new();
    let mut store = Store::open(&dir.0).unwrap();
    let a = message(1, 2, Some("a"));
    let b = message(2, 2, Some("b"));
    store.insert(&[a.clone(), b.clone()]).unwrap();
    let a_id = a["id"].as_str().unwrap();
    let b_id = b["id"].as_str().unwrap();
    assert!(store.inbox(inbox_request(&[a_id, b_id])).is_err());
    let ack = [a_id];
    let mut wrong = inbox_request(&ack);
    wrong.after = Some(b_id);
    assert!(store.inbox(wrong).is_err());
    assert_eq!(store.get(a_id, scope()).unwrap(), Some(a.clone()));
    let mut human = inbox_request(&[]);
    human.scope = MailboxScope::Human { workspace: 3 };
    human.after = Some(a_id);
    assert!(store.inbox(human).is_err());
}

#[test]
fn atomic_inbox_counts_use_scoped_covering_metadata_searches() {
    let dir = Temp::new();
    let store = Store::open(&dir.0).unwrap();
    for human in [false, true] {
        let mut statement = store
            .connection
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {}",
                crate::record_store::inbox::counts_sql(human)
            ))
            .unwrap();
        let plan: Vec<String> = statement
            .query_map(params![key(2), human, "a", 0, false], |r| r.get(3))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert!(
            plan.iter()
                .any(|step| step.contains("SEARCH messages USING COVERING INDEX recipient_order")),
            "inbox counts scan unrelated history: {plan:?}"
        );
    }
}
