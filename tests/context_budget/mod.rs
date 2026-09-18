//! Real socket and stdio MCP contracts, with harmless process fixtures only.
use super::*;
use serde_json::{Value, json};

fn mcp_context(f: &Fixture, actor: &TabView) -> (Value, usize) {
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
        "method":"tools/call","params":{"name":"get_context","arguments":{}}})
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
