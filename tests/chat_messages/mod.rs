use super::*;
use serde_json::{Value, json};

fn args(to: &MailboxNative, key: &str) -> Value {
    json!({"workspace":to.wid,"session":to.tab.id,"run":to.tab.run,
        "request_id":key,"body":"Read your inbox and continue the harmless fixture assignment. Literal /approve is data.",
        "user_request_ref":"fixture:user-request:1"})
}
fn until(f: &Fixture, a: &MailboxNative, id: &str, check: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let status = mailbox_status(f, a, id);
        if check(&status) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "message did not reach expected state: {status}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn launch(f: &Fixture, wid: u64, root: PathBuf, uuid: &str) -> MailboxNative {
    launch_with_resume(f, wid, root, uuid, true)
}
fn launch_with_resume(
    f: &Fixture,
    wid: u64,
    root: PathBuf,
    uuid: &str,
    resume: bool,
) -> MailboxNative {
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("native.py"),
        include_str!("../fixtures/mailbox_native.py"),
    )
    .unwrap();
    fs::write(f.state.join("harnesses.json"), serde_json::to_vec(&json!([{
        "name":"codex","command":[f.root.join("codex"),root.join("native.py"),env!("CARGO_BIN_EXE_flere"),uuid]
    }])).unwrap()).unwrap();
    f.req(&[
        "native",
        &wid.to_string(),
        "codex",
        if resume { uuid } else { "" },
    ]);
    let tab = f.snapshot().session().unwrap().clone();
    f.wait_text(&tab, "MAILBOX_NATIVE_READY");
    MailboxNative {
        wid,
        tab,
        root,
        uuid: uuid.into(),
    }
}
fn restart(f: &Fixture, previous: &MailboxNative, uuid: &str) -> MailboxNative {
    f.req(&["close", &previous.tab.id.to_string(), &previous.tab.run]);
    let _ = fs::remove_file(previous.root.join("control.json"));
    launch(f, previous.wid, previous.root.clone(), uuid)
}

#[test]
fn chat_message_queue_submission_unblocks_without_model_tools_and_survives_helper_timeout() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-bbbb-bbbb-bbbb-111111111111");
    let b = mailbox_native(&f, "recipient", "22222222-bbbb-bbbb-bbbb-222222222222");
    fs::write(b.root.join("queue-mode"), "hang").unwrap();
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","process_queue":false,"handle_queue":false}),
    );
    let sent = dispatch_call(
        &f,
        &a.tab,
        "send_chat_message",
        args(&b, "receipt-without-tools"),
    )
    .unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    wait_file(&b.root.join("queue.jsonl"), 4);
    mailbox_event(&b, 2, json!({"process_queue":true}));
    let submitted = until(&f, &a, id, |m| {
        m["delivery"]["outcome"] == "queue-submitted"
    });
    assert!(submitted["surfaced"].is_null());
    assert!(submitted["native_surfaced"].is_null());
    assert!(submitted["acknowledged"].is_null());
    assert!(!b.root.join("handled.jsonl").exists());
    fs::remove_file(b.root.join("queue-mode")).unwrap();
    let later = dispatch_call(
        &f,
        &a.tab,
        "send_chat_message",
        args(&b, "next-without-tools"),
    )
    .unwrap();
    let later_id = later["message"]["id"].as_str().unwrap();
    let next = until(&f, &a, later_id, |m| {
        m["delivery"]["outcome"] == "queue-submitted"
    });
    assert!(next["native_surfaced"].is_null());
    assert!(next["acknowledged"].is_null());
    // The first helper is still running when its native input is observed. Its
    // eventual timeout must not downgrade that receipt or permit a replay.
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        mailbox_status(&f, &a, id)["delivery"],
        submitted["delivery"]
    );
    let duplicate = dispatch_call(
        &f,
        &a.tab,
        "send_chat_message",
        args(&b, "receipt-without-tools"),
    )
    .unwrap();
    assert_eq!(duplicate["message"]["id"], id);
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    assert!(!b.root.join("handled.jsonl").exists());
}

#[test]
fn chat_message_queue_submission_requires_exact_prompt_and_conversation_and_durable_save() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "33333333-bbbb-bbbb-bbbb-333333333333");
    let b = mailbox_native(&f, "recipient", "44444444-bbbb-bbbb-bbbb-444444444444");
    let other = mailbox_native(&f, "other", "55555555-bbbb-bbbb-bbbb-555555555555");
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","process_queue":false}),
    );
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", args(&b, "exact-prompt")).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    until(&f, &a, id, |m| m["delivery"]["outcome"] == "queued");
    let queued: Value = serde_json::from_str(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let text = queued["message"].as_str().unwrap();
    for (i, extra) in [
        json!({}),
        json!({"prompt":"ordinary private fixture prompt"}),
        json!({"prompt":format!("{text} ")}),
        json!({"prompt":text.replace(id, "00000000000000000000000000000000")}),
        json!({"prompt":text,"agent_id":"child-agent"}),
        json!({"queue_notice":{"workspace":b.wid,"session":b.tab.id,"run":b.tab.run,"id":id}}),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            mailbox_event(
                &b,
                i as u64 + 2,
                json!({"event":"UserPromptSubmit","extra":extra})
            )["code"],
            0
        );
        assert_eq!(mailbox_status(&f, &a, id)["delivery"]["outcome"], "queued");
    }
    mailbox_event(
        &other,
        1,
        json!({"event":"UserPromptSubmit","extra":{"prompt":text}}),
    );
    mailbox_event(
        &b,
        8,
        json!({"event":"Stop","handle":false,"extra":{"prompt":text}}),
    );
    assert_eq!(mailbox_status(&f, &a, id)["delivery"]["outcome"], "queued");
    let previous_observation = dispatch_call(&f, &b.tab, "messaging_activation", json!({}))
        .unwrap()["observation"]
        .clone();
    let store = f.state.join("workspaces.v2.json");
    let backup = f.state.join("retained-store.json");
    fs::rename(&store, &backup).unwrap();
    fs::create_dir(&store).unwrap();
    let failed = mailbox_event(
        &b,
        9,
        json!({"event":"UserPromptSubmit","extra":{"prompt":text}}),
    );
    assert_ne!(failed["code"], 0);
    fs::remove_dir(&store).unwrap();
    fs::rename(&backup, &store).unwrap();
    assert_eq!(
        dispatch_call(&f, &b.tab, "messaging_activation", json!({})).unwrap()["observation"],
        previous_observation,
        "a failed durable receipt must also roll back its hook observation"
    );
    assert_eq!(mailbox_status(&f, &a, id)["delivery"]["outcome"], "queued");
    assert_eq!(
        mailbox_event(
            &b,
            10,
            json!({"event":"UserPromptSubmit","extra":{"prompt":text}})
        )["code"],
        0
    );
    let submitted = until(&f, &a, id, |m| {
        m["delivery"]["outcome"] == "queue-submitted"
    });
    assert!(submitted["surfaced"].is_null());
    assert!(submitted["native_surfaced"].is_null());
    assert!(submitted["acknowledged"].is_null());
    let persisted: Value = serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
    let durable = persisted["coordination"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == id)
        .unwrap();
    assert_eq!(durable["delivery"], submitted["delivery"]);
    assert!(
        persisted["coordination"]["delivery"]["hooks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|h| { h["run"] == b.tab.run && h["event"] == "UserPromptSubmit" })
    );
    mailbox_event(
        &b,
        11,
        json!({"event":"UserPromptSubmit","extra":{"prompt":text}}),
    );
    assert_eq!(
        mailbox_status(&f, &a, id)["delivery"],
        submitted["delivery"]
    );
    assert!(
        !fs::read_to_string(store)
            .unwrap()
            .contains("ordinary private fixture prompt")
    );
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn chat_message_idle_nudge_preserves_provenance_and_deduplicates() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "11111111-aaaa-aaaa-aaaa-111111111111");
    let b = mailbox_native(&f, "recipient", "22222222-aaaa-aaaa-aaaa-222222222222");
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","screen":"completed","process_queue":false}),
    );
    dispatch_call(
        &f,
        &b.tab,
        "set_focus",
        json!({"seconds":2,"reason":"fixture DND"}),
    )
    .unwrap();
    let input = args(&b, "one-nudge");
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    assert_eq!(sent["message"]["delivery"]["outcome"], "deferred");
    assert_eq!(sent["message"]["chat"]["sender"]["kind"], "agent");
    assert_eq!(sent["message"]["chat"]["sender"]["conversation"], a.uuid);
    let duplicate = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    assert_eq!(duplicate["message"]["id"], id);
    assert_eq!(duplicate["duplicate"], true);
    assert!(!b.root.join("queue.jsonl").exists());
    let queued = until(&f, &a, id, |m| m["delivery"]["outcome"] == "queued");
    assert!(queued["native_surfaced"].is_null());
    assert!(queued["acknowledged"].is_null());
    assert_eq!(queued["chat"]["conversation"], b.uuid);
    mailbox_event(&b, 2, json!({"process_queue":true}));
    let handled = until(&f, &a, id, |m| !m["acknowledged"].is_null());
    assert!(!handled["native_surfaced"].is_null());
    let record: Value = serde_json::from_str(
        wait_file(&b.root.join("handled.jsonl"), 3)
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(record["workspace"], b.wid);
    assert_eq!(record["messages"][0]["body"], input["body"]);
    assert_eq!(
        record["messages"][0]["chat"]["user_request_ref"],
        input["user_request_ref"]
    );
    let queue = fs::read_to_string(b.root.join("queue.jsonl")).unwrap();
    assert_eq!(queue.lines().count(), 1);
    assert!(!queue.contains("/approve"));
    let duplicate = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    assert_eq!(duplicate["message"]["id"], id);
    let mut conflict = input.clone();
    conflict["body"] = "different work".into();
    assert!(dispatch_call(&f, &a.tab, "send_chat_message", conflict).is_err());
    let mut spoof = input.clone();
    spoof["from"] = "user".into();
    assert!(dispatch_call(&f, &a.tab, "send_chat_message", spoof).is_err());
    let mut stale = args(&b, "stale");
    stale["run"] = a.tab.run.clone().into();
    assert!(dispatch_call(&f, &a.tab, "send_chat_message", stale).is_err());
    let saved: Value =
        serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap();
    assert_eq!(
        saved["coordination"]["messages"].as_array().unwrap().len(),
        1
    );
    assert_eq!(saved["version"], 10);
    assert!(
        !fs::read_to_string(f.state.join("actions.log"))
            .unwrap()
            .contains("\tinput\t")
    );
}

#[test]
fn chat_message_is_private_to_the_conversation_across_all_boundaries() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "33333333-aaaa-aaaa-aaaa-333333333333");
    let b = mailbox_native(&f, "recipient", "44444444-aaaa-aaaa-aaaa-444444444444");
    let other = launch(
        &f,
        b.wid,
        b.root.join("other"),
        "55555555-aaaa-aaaa-aaaa-555555555555",
    );
    mailbox_event(&b, 1, json!({"screen":"active"}));
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", args(&b, "scoped")).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    for op in ["context", "inbox"] {
        let read = dispatch_call(&f, &other.tab, op, json!({})).unwrap();
        assert!(read["messages"].as_array().unwrap().is_empty());
    }
    assert!(dispatch_call(&f, &other.tab, "message_status", json!({"id":id})).is_err());
    assert!(dispatch_call(&f, &other.tab, "inbox", json!({"ack_ids":[id]})).is_err());
    assert!(
        dispatch_call(
            &f,
            &other.tab,
            "deliver_message",
            json!({"id":id,"session":b.tab.id,"run":b.tab.run})
        )
        .is_err()
    );
    let tool = dispatch_call(
        &f,
        &other.tab,
        "checkpoint",
        json!({"body":"ordinary fixture work"}),
    )
    .unwrap();
    assert!(tool.get("mailbox_notice").is_none());
    mailbox_event(&other, 1, json!({"event":"SessionStart","turn":""}));
    mailbox_event(&other, 2, json!({"event":"UserPromptSubmit"}));
    let hook = mailbox_event(
        &other,
        3,
        json!({"event":"PostToolUse","extra":{"tool_name":"Bash"}}),
    );
    assert_eq!(hook["output"], "{}\n");
    assert!(mailbox_status(&f, &a, id)["native_surfaced"].is_null());
    // Cached metadata for another tab must not hide a native conversation switch.
    let transcript = other.root.join("rollout-fixture.jsonl");
    let original = fs::read(&transcript).unwrap();
    let mut switched: Value = serde_json::from_slice(&original).unwrap();
    switched["payload"]["id"] = b.uuid.clone().into();
    fs::write(&transcript, serde_json::to_vec(&switched).unwrap()).unwrap();
    assert!(
        dispatch_call(
            &f,
            &a.tab,
            "deliver_message",
            json!({"id":id,"session":b.tab.id,"run":b.tab.run})
        )
        .is_err()
    );
    fs::write(&transcript, original).unwrap();
    until(&f, &a, id, |m| {
        m["delivery"]["outcome"] == "waiting-for-idle"
    });
    // Even the correct UUID cannot be selected while two native instances own it.
    // Model two harnesses arriving at the same UUID internally; explicit resume
    // already rejects duplicate hosted UUIDs before starting a native program.
    let duplicate = launch_with_resume(&f, b.wid, b.root.join("duplicate"), &b.uuid, false);
    assert!(
        dispatch_call(&f, &b.tab, "inbox", json!({})).unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        dispatch_call(
            &f,
            &a.tab,
            "deliver_message",
            json!({"id":id,"session":b.tab.id,"run":b.tab.run})
        )
        .is_err()
    );
    f.req(&["close", &duplicate.tab.id.to_string(), &duplicate.tab.run]);
    let inbox = dispatch_call(&f, &b.tab, "inbox", json!({})).unwrap();
    assert_eq!(inbox["messages"][0]["id"], id);
    assert_eq!(
        dispatch_call(&f, &b.tab, "messaging_activation", json!({})).unwrap()["session"],
        b.tab.id
    );
    dispatch_call(&f, &b.tab, "inbox", json!({"ack_ids":[id]})).unwrap();
}

#[test]
fn chat_message_active_tool_boundary_surfaces_only_saved_agent_content() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "12121212-aaaa-aaaa-aaaa-121212121212");
    let b = mailbox_native(&f, "recipient", "34343434-aaaa-aaaa-aaaa-343434343434");
    mailbox_event(&b, 1, json!({"event":"SessionStart","turn":""}));
    mailbox_event(&b, 2, json!({"event":"UserPromptSubmit","screen":"active"}));
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", args(&b, "tool-boundary")).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    assert_eq!(sent["message"]["delivery"]["outcome"], "waiting-for-hook");
    let hook = mailbox_event(
        &b,
        3,
        json!({"event":"PostToolUse","tool":true,"extra":{"tool_name":"Bash","tool_response":"untrusted unrelated text"}}),
    );
    assert!(hook["output"].as_str().unwrap().contains(id));
    assert!(
        !hook["output"]
            .as_str()
            .unwrap()
            .contains("untrusted unrelated text")
    );
    let done = until(&f, &a, id, |m| !m["acknowledged"].is_null());
    assert_eq!(done["delivery"]["outcome"], "hook-emitted");
    assert!(!b.root.join("queue.jsonl").exists());
}

#[test]
fn chat_message_failed_save_does_not_queue_or_consume_retry_key() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "56565656-aaaa-aaaa-aaaa-565656565656");
    let b = mailbox_native(&f, "recipient", "78787878-aaaa-aaaa-aaaa-787878787878");
    let store = f.state.join("workspaces.v2.json");
    let backup = f.state.join("fixture-backup.json");
    fs::rename(&store, &backup).unwrap();
    fs::create_dir(&store).unwrap();
    let input = args(&b, "retry-after-save-failure");
    assert!(dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).is_err());
    assert!(!b.root.join("queue.jsonl").exists());
    fs::remove_dir(&store).unwrap();
    fs::rename(&backup, &store).unwrap();
    let retry = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    assert_eq!(retry["duplicate"], false);
    let duplicate = dispatch_call(&f, &a.tab, "send_chat_message", input).unwrap();
    assert_eq!(duplicate["message"]["id"], retry["message"]["id"]);
    let stored: Value = serde_json::from_slice(&fs::read(store).unwrap()).unwrap();
    assert_eq!(
        stored["coordination"]["messages"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn chat_message_waits_through_replacement_and_follows_same_conversation_resume() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "66666666-aaaa-aaaa-aaaa-666666666666");
    let b = mailbox_native(&f, "recipient", "77777777-aaaa-aaaa-aaaa-777777777777");
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","screen":"draft"}),
    );
    let input = args(&b, "survives-resume");
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    let draft = until(&f, &a, id, |m| {
        m["delivery"]["detail"]
            .as_str()
            .is_some_and(|s| s.contains("draft"))
    });
    assert!(draft["native_surfaced"].is_null());
    assert!(!b.root.join("queue.jsonl").exists());
    let other = restart(&f, &b, "88888888-aaaa-aaaa-aaaa-888888888888");
    until(&f, &a, id, |m| {
        m["delivery"]["outcome"] == "target-unavailable"
    });
    assert!(
        dispatch_call(&f, &other.tab, "context", json!({})).unwrap()["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(dispatch_call(&f, &other.tab, "inbox", json!({"ack_ids":[id]})).is_err());
    assert!(
        dispatch_call(
            &f,
            &a.tab,
            "deliver_message",
            json!({"id":id,"session":other.tab.id,"run":other.tab.run})
        )
        .is_err()
    );
    let retry = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    assert_eq!(retry["message"]["id"], id); // lost reply, original now-stale target args
    let resumed = restart(&f, &other, &b.uuid);
    assert_ne!(resumed.tab.run, b.tab.run);
    let retry = dispatch_call(
        &f,
        &a.tab,
        "send_chat_message",
        args(&resumed, "survives-resume"),
    )
    .unwrap();
    assert_eq!(retry["message"]["id"], id); // refreshed target args, same immutable conversation
    mailbox_event(
        &resumed,
        10,
        json!({"event":"SessionStart","turn":"","screen":"completed"}),
    );
    let done = until(&f, &a, id, |m| !m["acknowledged"].is_null());
    assert_eq!(done["delivery"]["run"], resumed.tab.run);
    assert_eq!(done["chat"]["initial_run"], b.tab.run);
    let sender_resumed = restart(&f, &a, &a.uuid);
    let retry = dispatch_call(&f, &sender_resumed.tab, "send_chat_message", input).unwrap();
    assert_eq!(retry["message"]["id"], id);
    assert_eq!(retry["message"]["chat"]["sender"]["run"], a.tab.run);

    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn chat_message_unknown_handoff_never_requeues_after_resume_or_retry() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "99999999-aaaa-aaaa-aaaa-999999999999");
    let b = mailbox_native(&f, "recipient", "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa");
    fs::write(b.root.join("queue-mode"), "fail").unwrap();
    fs::write(b.root.join("hold-queue"), "").unwrap();
    mailbox_event(
        &b,
        1,
        json!({"event":"SessionStart","turn":"","process_queue":false}),
    );
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", args(&b, "uncertain")).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    until(&f, &a, id, |m| m["delivery"]["outcome"] == "unknown");
    let resumed = restart(&f, &b, &b.uuid);
    mailbox_event(
        &resumed,
        10,
        json!({"event":"SessionStart","turn":"","process_queue":false}),
    );
    for _ in 0..3 {
        assert_eq!(
            dispatch_call(&f, &a.tab, "send_chat_message", args(&resumed, "uncertain")).unwrap()["message"]
                ["id"],
            id
        );
        dispatch_call(
            &f,
            &a.tab,
            "deliver_message",
            json!({"id":id,"session":resumed.tab.id,"run":resumed.tab.run}),
        )
        .unwrap();
    }
    assert_eq!(mailbox_status(&f, &a, id)["delivery"]["outcome"], "unknown");
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    let another = dispatch_call(
        &f,
        &a.tab,
        "send_chat_message",
        args(&resumed, "later-message"),
    )
    .unwrap();
    let later = another["message"]["id"].as_str().unwrap();
    until(&f, &a, later, |m| {
        m["delivery"]["outcome"] == "waiting-for-receipt"
    });
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    // The original exact notice can arrive after resuming this same native
    // conversation. The receipt retains its original run, without replaying it.
    let queued: Value = serde_json::from_str(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    mailbox_event(
        &resumed,
        11,
        json!({"event":"UserPromptSubmit","extra":{"prompt":queued["message"]}}),
    );
    let submitted = until(&f, &a, id, |m| {
        m["delivery"]["outcome"] == "queue-submitted"
    });
    assert_eq!(submitted["delivery"]["run"], b.tab.run);
    assert!(submitted["native_surfaced"].is_null());
    assert!(submitted["acknowledged"].is_null());
    mailbox_event(&resumed, 12, json!({"event":"Stop","handle":false}));
    until(&f, &a, later, |m| m["delivery"]["outcome"] == "unknown");
    assert_eq!(
        fs::read_to_string(b.root.join("queue.jsonl"))
            .unwrap()
            .lines()
            .count(),
        2
    );
    let inbox = dispatch_call(&f, &resumed.tab, "inbox", json!({})).unwrap();
    assert!(
        inbox["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"] == id)
    );
    dispatch_call(&f, &resumed.tab, "inbox", json!({"ack_ids":[id]})).unwrap();
}

#[test]
fn chat_message_refresh_preserves_binding_retry_key_and_permission_guard() {
    let f = Fixture::new();
    let a = mailbox_native(&f, "sender", "bbbbbbbb-aaaa-aaaa-aaaa-bbbbbbbbbbbb");
    let b = mailbox_native(&f, "recipient", "cccccccc-aaaa-aaaa-aaaa-cccccccccccc");
    mailbox_event(&b, 1, json!({"event":"SessionStart","turn":""}));
    mailbox_event(&b, 2, json!({"event":"UserPromptSubmit","screen":"active"}));
    mailbox_event(
        &b,
        3,
        json!({"event":"PermissionRequest","screen":"approval"}),
    );
    let input = args(&b, "refresh");
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", input.clone()).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    assert_eq!(sent["message"]["delivery"]["outcome"], "waiting-for-human");
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let deadline = Instant::now() + Duration::from_secs(8);
    while !String::from_utf8(f.req(&["refresh-status"]))
        .unwrap()
        .contains("all terminal sessions preserved")
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(25));
    }
    let retry = dispatch_call(&f, &a.tab, "send_chat_message", input).unwrap();
    assert_eq!(retry["message"]["id"], id);
    assert_eq!(retry["message"]["chat"], sent["message"]["chat"]);
    assert_eq!(retry["message"]["delivery"]["outcome"], "waiting-for-human");
    assert!(!b.root.join("queue.jsonl").exists());
    mailbox_event(&b, 4, json!({"event":"Stop","screen":"idle"}));
    until(&f, &a, id, |m| !m["acknowledged"].is_null());
}

#[test]
fn chat_message_cold_start_validates_provenance_and_preserves_stopped_mail() {
    let mut f = Fixture::new();
    let a = mailbox_native(&f, "sender", "dddddddd-aaaa-aaaa-aaaa-dddddddddddd");
    let b = mailbox_native(&f, "recipient", "eeeeeeee-aaaa-aaaa-aaaa-eeeeeeeeeeee");
    let sent = dispatch_call(&f, &a.tab, "send_chat_message", args(&b, "cold-start")).unwrap();
    let id = sent["message"]["id"].as_str().unwrap();
    f.stop();
    let store = f.state.join("workspaces.v2.json");
    let saved = fs::read(&store).unwrap();
    let mut corrupt: Value = serde_json::from_slice(&saved).unwrap();
    corrupt["coordination"]["messages"][0]["chat"]["sender"]["kind"] = "human".into();
    fs::write(&store, serde_json::to_vec(&corrupt).unwrap()).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("HOME", &f.root)
        .env("TMPDIR", &f.root)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("conversation message identity"));
    // A duplicate persisted retry key is equally invalid; it must not silently
    // select an arbitrary body after restart.
    let mut duplicate: Value = serde_json::from_slice(&saved).unwrap();
    let mut extra = duplicate["coordination"]["messages"][0].clone();
    extra["id"] = "0123456789abcdef0123456789abcdef".into();
    duplicate["coordination"]["messages"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    fs::write(&store, serde_json::to_vec(&duplicate).unwrap()).unwrap();
    let rejected = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("HOME", &f.root)
        .env("TMPDIR", &f.root)
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    let mut previous: Value = serde_json::from_slice(&saved).unwrap();
    previous["version"] = 9.into();
    fs::write(&store, serde_json::to_vec(&previous).unwrap()).unwrap();
    let log = fs::File::create(f.root.join("cold-start.log")).unwrap();
    f.child = Command::new(env!("CARGO_BIN_EXE_flere"))
        .arg("--state")
        .arg(&f.state)
        .arg("serve")
        .env("HOME", &f.root)
        .env("TMPDIR", &f.root)
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    f.ready();
    assert!(f.snapshot().workspaces.iter().all(|w| w.tabs.is_empty()));
    let response = f.req(&[
        "coordinate",
        &b.wid.to_string(),
        "context",
        &wire::hex(b"{}"),
    ]);
    let context: Value = serde_json::from_slice(&response).unwrap();
    assert_eq!(context["messages"][0]["id"], id);
    let mut address = context["messages"][0]["chat"].clone();
    assert_eq!(address["user_request_ref"], "fixture:user-request:1");
    address.as_object_mut().unwrap().remove("user_request_ref");
    assert_eq!(address, sent["message"]["chat"]);
    assert!(context["messages"][0]["native_surfaced"].is_null());
    assert!(context["messages"][0]["acknowledged"].is_null());
    assert!(!b.root.join("queue.jsonl").exists());
}
