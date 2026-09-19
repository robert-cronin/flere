//! Real socket and stdio MCP contracts, with harmless process fixtures only.
use super::*;
use serde_json::{Value, json};

fn mcp_context(f: &Fixture, actor: &TabView) -> (Value, usize) {
    mcp_context_args(f, actor, json!({}))
}
fn mcp_context_args(f: &Fixture, actor: &TabView, args: Value) -> (Value, usize) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args(["--state", f.state.to_str().unwrap(), "mcp"])
        .env("FLERE_SESSION", actor.id.to_string())
        .env("FLERE_RUN", &actor.run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        json!({"jsonrpc":"2.0","id":1,
        "method":"tools/call","params":{"name":"get_context","arguments":args}})
    )
    .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    (serde_json::from_str(text).unwrap(), text.len())
}

#[test]
fn context_budget_inventory_pages_and_full_metadata_cas() {
    let f = Fixture::new();
    let (wid, actor) = dispatch_fixture(&f);
    for i in 0..40 {
        f.req(&[
            "new-stopped",
            &wire::hex(format!("fixture-{i}").as_bytes()),
            &wire::hex(f.root.to_str().unwrap().as_bytes()),
        ]);
        let id = f.snapshot().active;
        let meta = flere::workspace::CardMeta {
            notes: "synthetic-note ".repeat(900),
            ..Default::default()
        };
        f.req(&[
            "metadata",
            &id.to_string(),
            &wire::hex(&serde_json::to_vec(&meta).unwrap()),
        ]);
    }
    let raw: Value = serde_json::from_slice(&f.req(&["list"])).unwrap();
    let mut ids = Vec::new();
    let mut args = json!({});
    let mut bytes = 0;
    loop {
        let page = dispatch_call(&f, &actor, "list_workspaces", args.clone()).unwrap();
        let size = serde_json::to_vec(&page).unwrap().len();
        assert!(size < 34 * 1024);
        bytes += size;
        for card in page["workspaces"].as_array().unwrap() {
            assert_eq!(card["detail"], false);
            assert!(card["meta"].get("notes").is_none());
            let full = raw["workspaces"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["id"] == card["id"])
                .unwrap();
            assert_eq!(card["tabs"], full["tabs"]);
            ids.push(card["id"].as_u64().unwrap());
        }
        if page["next_after"].is_null() {
            assert_eq!(page["remaining"], 0);
            break;
        }
        args = json!({"after":page["next_after"]});
    }
    let expected: Vec<_> = raw["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, expected);
    assert!(bytes * 10 < serde_json::to_vec(&raw).unwrap().len());
    let compact = dispatch_call(&f, &actor, "list_workspaces", json!({"workspace":wid})).unwrap();
    let card = &compact["workspaces"][0];
    assert!(
        dispatch_call(
            &f,
            &actor,
            "update_workspace",
            json!({"workspace":wid,
        "expected_epoch":compact["epoch"],"expected":{"name":card["name"],"meta":card["meta"]},
        "notes":"must reject incomplete expected metadata"})
        )
        .is_err()
    );
    let detailed = dispatch_call(
        &f,
        &actor,
        "list_workspaces",
        json!({"workspace":wid,"detail":true}),
    )
    .unwrap();
    assert_eq!(
        detailed["workspaces"][0]["meta"],
        raw["workspaces"][0]["meta"]
    );
    let update = card_update(&f, &actor, wid, json!({"notes":"guarded update"}));
    dispatch_call(&f, &actor, "update_workspace", update.clone()).unwrap();
    assert!(dispatch_call(&f, &actor, "update_workspace", update).is_err());
    assert!(dispatch_call(&f, &actor, "list_workspaces", json!({"detail":true})).is_err());
    assert!(dispatch_call(&f, &actor, "list_workspaces", json!({"limit":0})).is_err());
    let (context, size) = mcp_context(&f, &actor);
    assert_eq!(context["workspace"]["id"], wid);
    assert!(context.get("workspaces").is_none());
    assert!(context["workspace"]["meta"].get("notes").is_none());
    assert!(size < 2500, "compact MCP context: {size} bytes");
    eprintln!(
        "synthetic inventory: raw={} compact_pages={bytes}; MCP context={size}",
        serde_json::to_vec(&raw).unwrap().len()
    );
}

#[test]
fn context_budget_summaries_pages_ack_receipts_and_read_only_retries() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let (_, sender) = dispatch_fixture(&f);
    let (wid, recipient) = dispatch_fixture(&f);
    let mut ids = Vec::new();
    // An oversized serialized message followed by a small one must neither truncate
    // the first message nor skip it while trying to fill a page.
    let bodies = [
        "a".repeat(15000),
        "b".repeat(15000),
        "\u{1}".repeat(8000),
        "tail".into(),
    ];
    for body in &bodies {
        let sent =
            dispatch_call(&f, &sender, "send_message", json!({"to":wid,"body":body})).unwrap();
        assert!(sent["message"].get("body").is_none());
        assert_eq!(sent["message"]["body_bytes"], body.len());
        ids.push(sent["message"]["id"].clone());
    }
    let (context, size) = mcp_context(&f, &recipient);
    assert_eq!(context["pending_messages"], 4);
    assert!(size < 8000);
    for message in context["messages"].as_array().unwrap() {
        assert!(message.get("body").is_none());
        assert!(message["native_surfaced"].is_null());
    }
    let page = dispatch_call(&f, &recipient, "inbox", json!({})).unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 2);
    assert_eq!(page["remaining"], 2);
    assert_eq!(page["next_after"], ids[1]);
    assert_eq!(page["messages"][0]["body"], bodies[0]);
    assert_eq!(page["messages"][1]["body"], bodies[1]);
    let before = fs::metadata(f.state.join("workspaces.v2.json")).unwrap();
    let repeated = dispatch_call(&f, &recipient, "inbox", json!({})).unwrap();
    let after = fs::metadata(f.state.join("workspaces.v2.json")).unwrap();
    assert_eq!(page, repeated);
    assert_eq!(
        (before.ino(), before.mtime_nsec()),
        (after.ino(), after.mtime_nsec())
    );
    let ack = dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":[ids[0],ids[1]]})).unwrap();
    assert_eq!(ack["messages"], json!([]));
    assert_eq!(ack["pending"], 2);
    assert_eq!(ack["acknowledged"], json!([ids[0], ids[1]]));
    let unseen = dispatch_call(&f, &sender, "message_status", json!({"id":ids[2]})).unwrap();
    assert!(unseen["message"]["native_surfaced"].is_null());
    let next = dispatch_call(&f, &recipient, "inbox", json!({"after":page["next_after"]})).unwrap();
    assert_eq!(next["messages"].as_array().unwrap().len(), 1);
    assert_eq!(next["messages"][0]["body"], bodies[2]);
    assert_eq!(next["next_after"], ids[2]);
    let tail = dispatch_call(&f, &recipient, "inbox", json!({"after":next["next_after"]})).unwrap();
    assert_eq!(tail["messages"][0]["body"], bodies[3]);
    assert!(tail["next_after"].is_null());
    assert!(dispatch_call(&f, &sender, "inbox", json!({"after":ids[0]})).is_err());
    assert!(dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":"invalid"})).is_err());
    assert!(
        dispatch_call(
            &f,
            &recipient,
            "inbox",
            json!({"ack_ids":[ids[2],"unknown"]})
        )
        .is_err()
    );
    let status = dispatch_call(
        &f,
        &sender,
        "message_status",
        json!({"id":ids[2],"detail":true}),
    )
    .unwrap();
    assert!(status["message"]["acknowledged"].is_null());
    assert_eq!(status["message"]["body"], bodies[2]);
    // A persistence failure must not claim an ACK or retain an in-memory ACK.
    let store = f.state.join("workspaces.v2.json");
    let retained = f.state.join("retained.json");
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    assert!(dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":[ids[2]]})).is_err());
    assert!(dispatch_call(&f, &sender, "message_status", json!({"id":ids[2]})).unwrap()["message"]["acknowledged"].is_null());
    // An unchanged read still succeeds even when writes are unavailable.
    dispatch_call(&f, &recipient, "inbox", json!({"after":ids[2]})).unwrap();
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
    dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":[ids[2],ids[3]]})).unwrap();
    let mut later = Vec::new();
    for n in 0..12 {
        let sent = dispatch_call(
            &f,
            &sender,
            "send_message",
            json!({"to":wid,"body":format!("later-{n}")}),
        )
        .unwrap();
        later.push(sent["message"]["id"].clone());
    }
    let first = dispatch_call(&f, &recipient, "inbox", json!({})).unwrap();
    assert_eq!(first["messages"].as_array().unwrap().len(), 8);
    assert_eq!(first["pending"], 12);
    assert_eq!(first["remaining"], 4);
    dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":later[..8]})).unwrap();
    let appended = dispatch_call(
        &f,
        &sender,
        "send_message",
        json!({"to":wid,"body":"arrived between pages"}),
    )
    .unwrap();
    later.push(appended["message"]["id"].clone());
    let second = dispatch_call(
        &f,
        &recipient,
        "inbox",
        json!({"after":first["next_after"]}),
    )
    .unwrap();
    let returned: Vec<_> = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].clone())
        .collect();
    assert_eq!(returned, later[8..]);
    assert_eq!(second["pending"], 5);
    assert!(second["next_after"].is_null());
}

#[test]
fn context_budget_keeps_decisions_latest_checkpoint_and_explicit_history() {
    let f = Fixture::new();
    let (wid, actor) = dispatch_fixture(&f);
    for n in 0..12 {
        let response = dispatch_call(
            &f,
            &actor,
            "checkpoint",
            json!({"body":format!("checkpoint-{n} {}", "x".repeat(2000))}),
        )
        .unwrap();
        assert!(response["saved"].get("body").is_none());
    }
    let decision = dispatch_call(&f, &actor, "request_decision", json!({"question":"Which scope?","recommendation":"Local fixture","evidence":"fixture-only"})).unwrap();
    assert!(decision["decision"].get("question").is_none());
    f.req(&[
        "coordinate",
        &wid.to_string(),
        "answer_decision",
        &wire::hex(
            &serde_json::to_vec(
                &json!({"id":decision["decision"]["id"],"answer":"Keep changes in this fixture"}),
            )
            .unwrap(),
        ),
    ]);
    let (context, size) = mcp_context(&f, &actor);
    assert_eq!(context["checkpoints"].as_array().unwrap().len(), 1);
    assert!(
        context["checkpoints"][0]["body"]
            .as_str()
            .unwrap()
            .starts_with("checkpoint-11")
    );
    assert_eq!(
        context["decisions"][0]["answer"],
        "Keep changes in this fixture"
    );
    let detailed = dispatch_call(&f, &actor, "context", json!({"detail":true})).unwrap();
    assert_eq!(detailed["checkpoints"].as_array().unwrap().len(), 8);
    assert_eq!(context["decisions"], detailed["decisions"]);
    assert!(size * 4 < serde_json::to_vec(&detailed).unwrap().len());
    let human: Value =
        serde_json::from_slice(&f.req(&["coordinate", &wid.to_string(), "context", "7b7d"]))
            .unwrap();
    assert_eq!(human["checkpoints"], detailed["checkpoints"]);
    let bytes = fs::read(f.state.join("workspaces.v2.json")).unwrap();
    let store: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        store["coordination"]["checkpoints"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert!(bytes.len() < serde_json::to_vec_pretty(&store).unwrap().len());
}

#[test]
fn context_deltas_survive_mcp_reconnects_and_reset_on_supervisor_refresh() {
    let f = Fixture::new();
    let (_, sender) = dispatch_fixture(&f);
    let (wid, recipient) = dispatch_fixture(&f);
    let (full, _) = mcp_context(&f, &recipient);
    assert_eq!(full["context_mode"], "full");
    let revision = full["context_revision"].clone();
    let (unchanged, size) = mcp_context_args(&f, &recipient, json!({"since":revision}));
    assert_eq!(unchanged["context_mode"], "delta");
    assert_eq!(unchanged["changes"], json!({}));
    assert_eq!(unchanged["context_revision"], revision);
    assert_eq!(unchanged["identity"]["session"], recipient.id);
    assert_eq!(unchanged["identity"]["run"], recipient.run);
    assert!(size < 600);
    let sent = dispatch_call(
        &f,
        &sender,
        "send_message",
        json!({"to":wid,"body":"Exact correction: keep all work local. 界"}),
    )
    .unwrap();
    let id = sent["message"]["id"].clone();
    let (delta, _) = mcp_context_args(&f, &recipient, json!({"since":revision}));
    assert_eq!(delta["base_revision"], revision);
    assert_eq!(delta["changes"]["pending_messages"], 1);
    assert_eq!(delta["changes"]["messages"][0]["id"], id);
    assert!(delta["changes"]["messages"][0].get("body").is_none());
    assert!(delta["changes"]["messages"][0]["native_surfaced"].is_null());
    let revision = delta["context_revision"].clone();
    let inbox = dispatch_call(&f, &recipient, "inbox", json!({})).unwrap();
    assert_eq!(
        inbox["messages"][0]["body"],
        "Exact correction: keep all work local. 界"
    );
    dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":[id]})).unwrap();
    let (acked, _) = mcp_context_args(&f, &recipient, json!({"since":revision}));
    assert_eq!(acked["changes"]["messages"], json!([]));
    assert_eq!(acked["changes"]["pending_messages"], 0);
    let current = acked["context_revision"].clone();
    // A revision is a cache hint, never access to another native run's context.
    let (foreign, _) = mcp_context_args(&f, &sender, json!({"since":current}));
    assert_eq!(foreign["context_mode"], "full");
    let (reset, _) = mcp_context(&f, &recipient);
    assert_eq!(reset["context_mode"], "full");
    assert_eq!(reset["pending_messages"], 0);
    let (detail, _) = mcp_context_args(&f, &recipient, json!({"detail":true,"since":current}));
    assert_eq!(detail["detail"], true);
    assert!(detail.get("context_mode").is_none());
    let before = f.snapshot();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = wire::request(
            &f.state,
            &[
                "agent-operation",
                &recipient.id.to_string(),
                &recipient.run,
                "context",
                &wire::hex(&serde_json::to_vec(&json!({"since":current})).unwrap()),
            ],
        ) {
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            if value["context_mode"] == "full" {
                assert_eq!(value["pending_messages"], 0);
                assert_eq!(value["messaging"]["run"], recipient.run);
                break;
            }
        }
        assert!(
            Instant::now() < end,
            "refresh did not discard the optional cache"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.snapshot().epoch, before.epoch);
    assert!(
        f.snapshot()
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .any(|t| t.run == recipient.run && t.pid == recipient.pid)
    );
}

#[test]
fn decision_history_pages_preserve_evidence_and_reads_never_require_writes() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let (wid, actor) = dispatch_fixture(&f);
    let (_, other) = dispatch_fixture(&f);
    let mut records = Vec::new();
    for n in 0..40 {
        let args = json!({"question":format!("Question {n}"),"recommendation":"Keep scope exact",
            "evidence":if n == 0 { "\u{1}".repeat(8000) } else { "evidence 界".repeat(20) }});
        let result: Value = serde_json::from_slice(&f.req(&[
            "coordinate",
            &wid.to_string(),
            "request_decision",
            &wire::hex(&serde_json::to_vec(&args).unwrap()),
        ]))
        .unwrap();
        records.push(result["decision"].clone());
    }
    let (context, _) = mcp_context(&f, &actor);
    assert_eq!(context["decisions_total"], 40);
    assert_eq!(context["decisions_remaining"], 8);
    assert_eq!(context["pending_decisions"], 40);
    assert_eq!(context["decisions"].as_array().unwrap().len(), 32);
    let store = f.state.join("workspaces.v2.json");
    let before = fs::metadata(&store).unwrap();
    let first = dispatch_call(&f, &actor, "decisions", json!({})).unwrap();
    assert_eq!(first["decisions"], json!([records[0]]));
    assert_eq!(first["remaining"], 39);
    assert_eq!(first["next_after"], records[0]["id"]);
    let mut retrieved = first["decisions"].as_array().unwrap().clone();
    let mut cursor = first["next_after"].clone();
    loop {
        let page = dispatch_call(&f, &actor, "decisions", json!({"after":cursor})).unwrap();
        assert!(page["decisions"].as_array().unwrap().len() <= 8);
        retrieved.extend(page["decisions"].as_array().unwrap().iter().cloned());
        cursor = page["next_after"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(retrieved, records);
    let after = fs::metadata(&store).unwrap();
    assert_eq!(
        (before.ino(), before.mtime_nsec()),
        (after.ino(), after.mtime_nsec())
    );
    assert!(dispatch_call(&f, &other, "decisions", json!({"after":records[0]["id"]})).is_err());
    assert!(dispatch_call(&f, &other, "decisions", json!({"id":records[0]["id"]})).is_err());
    for args in [
        json!({"limit":0}),
        json!({"limit":33}),
        json!({"after":3}),
        json!({"id":records[0]["id"],"limit":1}),
        json!({"id":false}),
    ] {
        assert!(dispatch_call(&f, &actor, "decisions", args).is_err());
    }
    let human: Value =
        serde_json::from_slice(&f.req(&["coordinate", &wid.to_string(), "decisions", "7b7d"]))
            .unwrap();
    assert_eq!(human["decisions"], json!(records));
    // An answer changes a record without invalidating its chronological cursor.
    let answer = json!({"id":records[0]["id"],"answer":"New correction: retain the exact original constraint."});
    f.req(&[
        "coordinate",
        &wid.to_string(),
        "answer_decision",
        &wire::hex(&serde_json::to_vec(&answer).unwrap()),
    ]);
    let targeted = dispatch_call(&f, &actor, "decisions", json!({"id":records[0]["id"]})).unwrap();
    assert_eq!(targeted["decisions"][0]["answer"], answer["answer"]);
    assert_eq!(targeted["pending"], 39);
    let later = dispatch_call(&f, &actor, "decisions", json!({"after":records[0]["id"]})).unwrap();
    assert_eq!(later["decisions"][0], records[1]);
    let (context, _) = mcp_context(&f, &actor);
    assert_eq!(context["pending_decisions"], 39);
    assert!(
        context["decisions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["id"] != records[0]["id"])
    );
    // A read must still work if the next atomic store replacement is unavailable.
    let retained = f.state.join("retained-decisions.json");
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    let read = dispatch_call(&f, &actor, "decisions", json!({"id":records[0]["id"]})).unwrap();
    assert_eq!(read, targeted);
    let human: Value =
        serde_json::from_slice(&f.req(&["coordinate", &wid.to_string(), "decisions", "7b7d"]))
            .unwrap();
    assert_eq!(human["decisions"].as_array().unwrap().len(), 40);
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
}

#[test]
fn checkpoint_history_is_exact_scoped_and_stable_across_appends_and_refresh() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let (_, actor) = dispatch_fixture(&f);
    let (_, other) = dispatch_fixture(&f);
    let mut bodies = Vec::new();
    for n in 0..18 {
        let body = if n == 0 {
            "\u{1}".repeat(8000)
        } else {
            format!("exact checkpoint {n} 界")
        };
        let operation = if n % 3 == 0 {
            "submit_result"
        } else {
            "checkpoint"
        };
        dispatch_call(&f, &actor, operation, json!({"body":body})).unwrap();
        bodies.push((operation, body));
        dispatch_call(
            &f,
            &other,
            "checkpoint",
            json!({"body":"other workspace evidence"}),
        )
        .unwrap();
    }
    dispatch_call(&f, &actor, "set_status", json!({"status":"in-progress"})).unwrap();
    let (context, _) = mcp_context(&f, &actor);
    assert_eq!(context["checkpoints_total"], 18);
    assert_eq!(context["checkpoints"].as_array().unwrap().len(), 1);
    let latest = context["latest_checkpoint_id"].clone();
    let store = f.state.join("workspaces.v2.json");
    let before = fs::metadata(&store).unwrap();
    let first = dispatch_call(&f, &actor, "checkpoints", json!({})).unwrap();
    assert_eq!(first["checkpoints"].as_array().unwrap().len(), 1);
    assert_eq!(first["checkpoints"][0]["checkpoint"]["body"], bodies[0].1);
    let mut records = first["checkpoints"].as_array().unwrap().clone();
    let mut cursor = first["next_after"].clone();
    loop {
        let page = dispatch_call(&f, &actor, "checkpoints", json!({"after":cursor})).unwrap();
        assert!(page["checkpoints"].as_array().unwrap().len() <= 8);
        records.extend(page["checkpoints"].as_array().unwrap().iter().cloned());
        cursor = page["next_after"].clone();
        if cursor.is_null() {
            break;
        }
    }
    assert_eq!(records.len(), bodies.len());
    for (record, (kind, body)) in records.iter().zip(&bodies) {
        assert_eq!(record["checkpoint"]["kind"], *kind);
        assert_eq!(record["checkpoint"]["body"], *body);
    }
    assert_eq!(records.last().unwrap()["id"], latest);
    let after = fs::metadata(&store).unwrap();
    assert_eq!(
        (before.ino(), before.mtime_nsec()),
        (after.ino(), after.mtime_nsec())
    );
    assert_eq!(
        dispatch_call(&f, &actor, "context", json!({})).unwrap()["workspace"]["meta"]["status"],
        "in-progress"
    );
    for args in [json!({"id":latest}), json!({"after":latest})] {
        assert!(dispatch_call(&f, &other, "checkpoints", args).is_err());
    }
    for args in [
        json!({"id":"checkpoint:+0"}),
        json!({"id":"checkpoint:00"}),
        json!({"id":false}),
        json!({"limit":0}),
        json!({"id":latest,"after":latest}),
    ] {
        assert!(dispatch_call(&f, &actor, "checkpoints", args).is_err());
    }
    dispatch_call(
        &f,
        &actor,
        "checkpoint",
        json!({"body":"appended evidence"}),
    )
    .unwrap();
    let appended = dispatch_call(&f, &actor, "checkpoints", json!({"after":latest})).unwrap();
    assert_eq!(appended["checkpoints"].as_array().unwrap().len(), 1);
    assert_eq!(
        appended["checkpoints"][0]["checkpoint"]["body"],
        "appended evidence"
    );
    let retained = f.state.join("retained-checkpoints.json");
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    let exact = dispatch_call(&f, &actor, "checkpoints", json!({"id":latest})).unwrap();
    assert_eq!(exact["checkpoints"][0], *records.last().unwrap());
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
    let revision =
        dispatch_call(&f, &actor, "context", json!({})).unwrap()["context_revision"].clone();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(c) = dispatch_call(&f, &actor, "context", json!({"since":revision}))
            && c["context_mode"] == "full"
        {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    let refreshed = dispatch_call(&f, &actor, "checkpoints", json!({"id":latest})).unwrap();
    assert_eq!(refreshed, exact);
}

#[test]
fn verified_compaction_resets_only_the_exact_context_cache_without_rewriting_history() {
    let f = Fixture::new();
    let actor = mailbox_native(&f, "compacting", "11111111-aaaa-bbbb-eeee-111111111111");
    let other = mailbox_native(&f, "unrelated", "22222222-aaaa-bbbb-eeee-222222222222");
    for native in [&actor, &other] {
        assert_eq!(
            mailbox_event(
                native,
                1,
                json!({"event":"SessionStart","turn":"",
            "process_queue":false,"handle":false})
            )["code"],
            0
        );
        assert_eq!(
            mailbox_event(
                native,
                2,
                json!({"event":"UserPromptSubmit",
            "turn":"main","handle":false})
            )["code"],
            0
        );
    }
    dispatch_call(
        &f,
        &actor.tab,
        "set_focus",
        json!({"seconds":30,"reason":"synthetic context reset fixture"}),
    )
    .unwrap();
    let body = "Exact retained evidence: keep work local; preserve provenance. 界";
    let sent = dispatch_call(
        &f,
        &other.tab,
        "send_message",
        json!({"to":actor.wid,"body":body}),
    )
    .unwrap();
    let id = sent["message"]["id"].clone();
    let inbox = dispatch_call(&f, &actor.tab, "inbox", json!({})).unwrap();
    assert_eq!(inbox["messages"][0]["body"], body);
    dispatch_call(&f, &actor.tab, "inbox", json!({"ack_ids":[id]})).unwrap();
    dispatch_call(
        &f,
        &actor.tab,
        "checkpoint",
        json!({"body":"Original checkpoint: preserve every exact record."}),
    )
    .unwrap();
    let decision = dispatch_call(&f, &actor.tab, "request_decision", json!({
        "question":"Which scope?","recommendation":"Keep work local.","evidence":"Synthetic context-loss fixture."})).unwrap();
    f.req(&[
        "coordinate",
        &actor.wid.to_string(),
        "answer_decision",
        &wire::hex(
            &serde_json::to_vec(&json!({"id":decision["decision"]["id"],
        "answer":"Keep work local and retain all evidence."}))
            .unwrap(),
        ),
    ]);
    let unread_body = "Pending correction: read this after compaction before acting.";
    let unread = dispatch_call(
        &f,
        &other.tab,
        "send_message",
        json!({"to":actor.wid,"body":unread_body}),
    )
    .unwrap();
    let unread_id = unread["message"]["id"].clone();
    // Let native discovery and saved tab layouts settle before checking writes.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let inventory: Value = serde_json::from_slice(&f.req(&["list"])).unwrap();
        let linked = [&actor, &other].iter().all(|native| {
            inventory["workspaces"].as_array().unwrap().iter().any(|w| {
                w["id"] == native.wid
                    && w["meta"]["conversations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|c| c["uuid"] == native.uuid)
            })
        });
        if linked {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture conversations were not discovered"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(1100));
    let store = f.state.join("workspaces.v2.json");
    let stored_before = fs::read(&store).unwrap();
    let (full, _) = mcp_context(&f, &actor.tab);
    assert_eq!(full["pending_messages"], 1);
    assert_eq!(full["messages"][0]["id"], unread_id);
    assert!(full["messages"][0]["native_surfaced"].is_null());
    let mut expected = full.clone();
    expected.as_object_mut().unwrap().remove("context_mode");
    expected.as_object_mut().unwrap().remove("context_revision");
    let mut revision = full["context_revision"].clone();
    let (other_full, _) = mcp_context(&f, &other.tab);
    let other_revision = other_full["context_revision"].clone();
    assert_eq!(
        mailbox_event(
            &actor,
            3,
            json!({"event":"PreToolUse","turn":"main","handle":false})
        )["code"],
        0
    );
    let (routine, _) = mcp_context_args(&f, &actor.tab, json!({"since":revision}));
    assert_eq!(routine["context_mode"], "delta");
    assert_eq!(routine["changes"], json!({}));
    assert_eq!(routine["context_revision"], revision);
    // Rejected native identities cannot evict even an optional context base.
    let event = json!({"hook_event_name":"PreCompact","session_id":actor.uuid,
        "transcript_path":actor.root.join("rollout-fixture.jsonl")});
    assert!(
        wire::request(
            &f.state,
            &[
                "inbox-hook",
                &actor.tab.id.to_string(),
                "ffffffffffffffffffffffffffffffff",
                &wire::hex(&serde_json::to_vec(&event).unwrap())
            ]
        )
        .is_err()
    );
    assert_eq!(
        mailbox_event(
            &actor,
            4,
            json!({"event":"PreCompact","turn":"","handle":false,
        "extra":{"session_id":other.uuid,"transcript_path":other.root.join("rollout-fixture.jsonl")}})
        )["code"],
        1
    );
    let (unaffected, _) = mcp_context_args(&f, &actor.tab, json!({"since":revision}));
    assert_eq!(unaffected["context_mode"], "delta");
    assert_eq!(unaffected["changes"], json!({}));
    for (sequence, name) in [(5, "PreCompact"), (6, "PostCompact")] {
        assert_eq!(
            mailbox_event(
                &actor,
                sequence,
                json!({"event":name,"turn":"","handle":false})
            )["code"],
            0
        );
        let (reset, full_bytes) = mcp_context_args(&f, &actor.tab, json!({"since":revision}));
        assert_eq!(reset["context_mode"], "full", "{name} reused a lost base");
        assert_ne!(reset["context_revision"], revision);
        revision = reset["context_revision"].clone();
        let mut reconstructed = reset;
        reconstructed
            .as_object_mut()
            .unwrap()
            .remove("context_mode");
        reconstructed
            .as_object_mut()
            .unwrap()
            .remove("context_revision");
        assert_eq!(
            reconstructed, expected,
            "compaction changed exact source evidence"
        );
        let (unrelated, _) = mcp_context_args(&f, &other.tab, json!({"since":other_revision}));
        assert_eq!(unrelated["context_mode"], "delta");
        assert_eq!(unrelated["changes"], json!({}));
        // A fresh base is usable, including one obtained between Pre/PostCompact.
        let (fresh, delta_bytes) = mcp_context_args(&f, &actor.tab, json!({"since":revision}));
        assert_eq!(fresh["context_mode"], "delta");
        assert_eq!(fresh["changes"], json!({}));
        eprintln!(
            "{name}: full compact view={full_bytes} body bytes, unchanged delta={delta_bytes} body bytes"
        );
    }
    assert_eq!(fs::read(&store).unwrap(), stored_before);
    let status = dispatch_call(
        &f,
        &actor.tab,
        "message_status",
        json!({"id":id,"detail":true}),
    )
    .unwrap();
    assert_eq!(status["message"]["body"], body);
    assert!(!status["message"]["acknowledged"].is_null());
    let resumed_inbox = dispatch_call(&f, &actor.tab, "inbox", json!({})).unwrap();
    assert_eq!(resumed_inbox["messages"].as_array().unwrap().len(), 1);
    assert_eq!(resumed_inbox["messages"][0]["id"], unread_id);
    assert_eq!(resumed_inbox["messages"][0]["body"], unread_body);
    assert!(resumed_inbox["messages"][0]["acknowledged"].is_null());
    dispatch_call(&f, &actor.tab, "inbox", json!({"ack_ids":[unread_id]})).unwrap();
    let (handled, _) = mcp_context_args(&f, &actor.tab, json!({"since":revision}));
    assert_eq!(handled["context_mode"], "delta");
    assert_eq!(handled["changes"]["pending_messages"], 0);
    assert_eq!(handled["changes"]["messages"], json!([]));
}

#[test]
fn acknowledged_inbox_history_is_opt_in_exact_paged_and_does_not_repeat_handling() {
    use std::os::unix::fs::MetadataExt;
    let f = Fixture::new();
    let (_, sender) = dispatch_fixture(&f);
    let (wid, recipient) = dispatch_fixture(&f);
    let (_, unrelated) = dispatch_fixture(&f);
    let mut ids = Vec::new();
    let mut bodies = Vec::new();
    for n in 0..12 {
        let body = if n == 0 {
            "\u{1}".repeat(8000)
        } else {
            format!("Retained evidence {n}: preserve exact constraints. 界")
        };
        let sent =
            dispatch_call(&f, &sender, "send_message", json!({"to":wid,"body":body})).unwrap();
        ids.push(sent["message"]["id"].clone());
        bodies.push(body);
    }
    // Handle every original message, then model losing all IDs at compaction.
    loop {
        let page = dispatch_call(&f, &recipient, "inbox", json!({})).unwrap();
        let handled: Vec<_> = page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].clone())
            .collect();
        if handled.is_empty() {
            break;
        }
        dispatch_call(&f, &recipient, "inbox", json!({"ack_ids":handled})).unwrap();
    }
    let pending = dispatch_call(
        &f,
        &sender,
        "send_message",
        json!({"to":wid,"body":"New correction still needs handling."}),
    )
    .unwrap();
    let pending_id = pending["message"]["id"].clone();
    ids.push(pending_id.clone());
    bodies.push("New correction still needs handling.".into());
    let (context, _) = mcp_context(&f, &recipient);
    assert_eq!(context["messages_total"], ids.len());
    assert_eq!(context["pending_messages"], 1);
    assert_eq!(context["messages"].as_array().unwrap().len(), 1);
    let store = f.state.join("workspaces.v2.json");
    let before = fs::metadata(&store).unwrap();
    let first = dispatch_call(
        &f,
        &recipient,
        "inbox",
        json!({"include_acknowledged":true}),
    )
    .unwrap();
    assert_eq!(first["messages"].as_array().unwrap().len(), 1);
    assert_eq!(first["messages"][0]["id"], ids[0]);
    assert_eq!(first["messages"][0]["body"], bodies[0]);
    assert!(!first["messages"][0]["acknowledged"].is_null());
    assert_eq!(first["total"], ids.len());
    assert_eq!(first["remaining"], ids.len() - 1);
    assert_eq!(first["pending"], 1);
    let after = fs::metadata(&store).unwrap();
    assert_eq!(
        (before.ino(), before.mtime_nsec()),
        (after.ino(), after.mtime_nsec())
    );
    assert!(
        dispatch_call(&f, &sender, "message_status", json!({"id":pending_id})).unwrap()["message"]
            ["native_surfaced"]
            .is_null()
    );
    let mut recovered = first["messages"].as_array().unwrap().clone();
    let mut cursor = first["next_after"].clone();
    while !cursor.is_null() {
        let page = dispatch_call(
            &f,
            &recipient,
            "inbox",
            json!({"include_acknowledged":true,"after":cursor,"limit":3}),
        )
        .unwrap();
        assert!(page["messages"].as_array().unwrap().len() <= 3);
        assert_eq!(page["pending"], 1);
        recovered.extend(page["messages"].as_array().unwrap().iter().cloned());
        cursor = page["next_after"].clone();
    }
    assert_eq!(recovered.len(), ids.len());
    for ((message, id), body) in recovered.iter().zip(&ids).zip(&bodies) {
        assert_eq!(message["id"], *id);
        assert_eq!(message["body"], *body);
    }
    assert!(recovered.last().unwrap()["acknowledged"].is_null());
    let durable: Value = serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
    assert_eq!(json!(recovered), durable["coordination"]["messages"]);
    // Default inbox never reopens old handled work, even after history discovery.
    let default = dispatch_call(&f, &recipient, "inbox", json!({})).unwrap();
    assert_eq!(default["messages"].as_array().unwrap().len(), 1);
    assert_eq!(default["messages"][0]["id"], pending_id);
    assert_eq!(default["total"], 1);
    for args in [
        json!({"include_acknowledged":"true","ack_ids":[pending_id]}),
        json!({"include_acknowledged":true,"after":"unknown","ack_ids":[pending_id]}),
    ] {
        assert!(dispatch_call(&f, &recipient, "inbox", args).is_err());
    }
    assert!(
        dispatch_call(
            &f,
            &unrelated,
            "inbox",
            json!({"include_acknowledged":true,"after":ids[0]})
        )
        .is_err()
    );
    assert_eq!(
        dispatch_call(
            &f,
            &unrelated,
            "inbox",
            json!({"include_acknowledged":true})
        )
        .unwrap()["total"],
        0
    );
    // Re-reading surfaced history requires no durable write and does not ACK.
    let retained = f.state.join("retained-messages.json");
    fs::rename(&store, &retained).unwrap();
    fs::create_dir(&store).unwrap();
    let last = dispatch_call(
        &f,
        &recipient,
        "inbox",
        json!({"include_acknowledged":true,"after":ids[ids.len()-2]}),
    )
    .unwrap();
    assert_eq!(last["messages"], json!([recovered.last().unwrap()]));
    fs::remove_dir(&store).unwrap();
    fs::rename(&retained, &store).unwrap();
    // Explicit ACK+page reads return the post-ACK evidence consistently.
    let acknowledged = dispatch_call(
        &f,
        &recipient,
        "inbox",
        json!({"include_acknowledged":true,"ack_ids":[pending_id],"after":ids[ids.len()-2]}),
    )
    .unwrap();
    assert_eq!(acknowledged["pending"], 0);
    assert!(!acknowledged["messages"][0]["acknowledged"].is_null());
    let receipt = dispatch_call(
        &f,
        &recipient,
        "inbox",
        json!({"include_acknowledged":true,"ack_ids":[pending_id]}),
    )
    .unwrap();
    assert_eq!(receipt["messages"], json!([]));
    assert_eq!(receipt["pending"], 0);
    // Stable message cursors survive refresh and include later appended evidence.
    let previous = f.snapshot();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let deadline = Instant::now() + Duration::from_secs(5);
    let refreshed = loop {
        if let Ok(page) = dispatch_call(
            &f,
            &recipient,
            "inbox",
            json!({"include_acknowledged":true,"after":ids[ids.len()-2]}),
        ) {
            break page;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(refreshed["messages"], acknowledged["messages"]);
    assert_eq!(f.snapshot().epoch, previous.epoch);
    let appended = dispatch_call(
        &f,
        &sender,
        "send_message",
        json!({"to":wid,"body":"Appended after refresh."}),
    )
    .unwrap();
    let tail = dispatch_call(
        &f,
        &recipient,
        "inbox",
        json!({"include_acknowledged":true,"after":pending_id}),
    )
    .unwrap();
    assert_eq!(tail["messages"].as_array().unwrap().len(), 1);
    assert_eq!(tail["messages"][0]["id"], appended["message"]["id"]);
    assert_eq!(tail["pending"], 1);
}
