use super::*;

fn setup_claude(f: &Fixture) {
    let bin = f.root.join("bin");
    fs::create_dir(&bin).unwrap();
    let script = f.root.join("claude_probe.py");
    fs::write(&script, include_str!("../fixtures/claude_probe.py")).unwrap();
    fs::write(
        bin.join("claude"),
        format!(
            "#!/bin/sh\nexec /usr/bin/python3 '{}' \"$@\"\n",
            script.display()
        ),
    )
    .unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        f.root.join(".bashrc"),
        format!("export PATH='{}:/usr/bin:/bin'\n", bin.display()),
    )
    .unwrap();
    fs::write(f.state.join("harnesses.json"),serde_json::to_vec(&json!([{
        "name":"claude","command":["/usr/bin/python3",script],
        "env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:12345","ANTHROPIC_AUTH_TOKEN":"fixture-only","ANTHROPIC_API_KEY":"","ANTHROPIC_MODEL":"fixture-model"}
    }])).unwrap()).unwrap();
}
fn request(f: &Fixture, t: &TabView, value: Value) -> Value {
    f.send(t, format!("{value}\n").as_bytes());
    serde_json::from_str(&wait_file(
        &f.root.join(format!("reply-{}.json", value["id"])),
        10,
    ))
    .unwrap()
}
fn send(f: &Fixture, _from: &TabView, to: u64, intent: &str) -> String {
    let result: Value = serde_json::from_slice(
        &f.req(&[
            "coordinate",
            &to.to_string(),
            "send_message",
            &wire::hex(
                json!({"to":to,"intent":intent,"body":"fixture body is never injected"})
                    .to_string()
                    .as_bytes(),
            ),
        ]),
    )
    .unwrap();
    result["message"]["id"].as_str().unwrap().to_string()
}
fn status(f: &Fixture, _from: &TabView, id: &str) -> Value {
    let result: Value = serde_json::from_slice(&f.req(&[
        "coordinate",
        &f.snapshot().active.to_string(),
        "message_status",
        &wire::hex(json!({"id":id}).to_string().as_bytes()),
    ]))
    .unwrap();
    result["message"].clone()
}

#[test]
fn claude_shell_mcp_hooks_gateway_profile_and_exact_resume() {
    for records in [false, true] {
        let f = Fixture::with_shell_editor_history(
            "/bin/bash",
            "/usr/bin/vim -Nu NONE -n --noplugin",
            records,
        );
        setup_claude(&f);
        let extra_settings = json!({"permissions":{"defaultMode":"default"},"env":{"EXTRA_CLAUDE":"kept"},
            "hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"true"}]}]},"theme":"dark"});
        fs::write(
            f.root.join("extra-settings.json"),
            extra_settings.to_string(),
        )
        .unwrap();
        fs::write(
            f.root.join("extra-mcp.json"),
            json!({"mcpServers":{"fixture":{"command":"/fixture/command"}}}).to_string(),
        )
        .unwrap();
        let shell = f.new_workspace("Claude registration");
        let wid = f.snapshot().active;
        f.send(
            &shell,
            b"kept=KEPT_CLAUDE; claude --settings extra-settings.json --mcp-config extra-mcp.json --resume 11111111-2222-3333-4444-555555555555\n",
        );
        let t = wait_kind(&f, "agent");
        f.wait_text(&t, "CLAUDE_READY");
        let receipt = record(&f, &t);
        assert_eq!(receipt["start"]["code"], 0, "{receipt}");
        assert_eq!(receipt["context"]["code"], 0);
        let context: Value = serde_json::from_str(
            receipt["context"]["response"]["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(context["workspace"]["id"], wid);
        assert_eq!(context["messaging"]["session"], t.id);
        assert_eq!(context["messaging"]["run"], t.run);
        assert_eq!(
            context["messaging"]["conversation"],
            "11111111-2222-3333-4444-555555555555"
        );
        assert_ne!(t.run, shell.run);
        assert_eq!((t.id, t.pid), (shell.id, shell.pid));
        let overlay = PathBuf::from(receipt["settings_path"].as_str().unwrap());
        assert_eq!(
            fs::metadata(&overlay).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            receipt["settings"]["env"]["ANTHROPIC_MODEL"],
            "fixture-model"
        );
        assert_eq!(
            receipt["settings"]["permissions"],
            extra_settings["permissions"]
        );
        assert_eq!(receipt["settings"]["theme"], "dark");
        assert_eq!(receipt["settings"]["env"]["EXTRA_CLAUDE"], "kept");
        assert_eq!(
            receipt["settings"]["hooks"]["UserPromptSubmit"][1],
            extra_settings["hooks"]["UserPromptSubmit"][0]
        );
        assert_eq!(
            receipt["mcp_servers"]["fixture"]["command"],
            "/fixture/command"
        );
        assert!(receipt["settings"]["hooks"]["Interrupt"].is_null());
        assert!(!f.snapshot().session().unwrap().working);

        let id = send(&f, &t, wid, "quiet");
        let notice = request(
            &f,
            &t,
            json!({"id":1,"event":"UserPromptSubmit","extra":{"prompt":"private user text","caller":[1,"forged"]}}),
        );
        assert_eq!(notice["code"], 0, "{notice}");
        assert!(f.snapshot().session().unwrap().working);
        std::thread::sleep(Duration::from_millis(1100));
        assert!(
            f.snapshot().session().unwrap().working,
            "background observation must retain Claude activity"
        );
        let text = notice["out"]["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(text.contains(&id));
        assert!(!text.contains("private user text") && !text.contains("fixture body"));
        let delivered = status(&f, &t, &id);
        assert_eq!(delivered["delivery"]["outcome"], "hook-emitted");
        assert!(delivered["acknowledged"].is_null());
        request(
            &f,
            &t,
            json!({"id":2,"mcp":"inbox","arguments":{"ack_ids":[id]}}),
        );
        assert!(!status(&f, &t, &id)["acknowledged"].is_null());

        let id = send(&f, &t, wid, "interrupt");
        let permission = request(&f, &t, json!({"id":3,"event":"PermissionRequest"}));
        assert_eq!(permission["out"], json!({}));
        assert!(!f.snapshot().session().unwrap().working);
        let repeated_stop = request(
            &f,
            &t,
            json!({"id":4,"event":"Stop","extra":{"stop_hook_active":true}}),
        );
        assert_eq!(repeated_stop["out"], json!({}));
        let stop = request(&f, &t, json!({"id":5,"event":"Stop"}));
        assert_eq!(stop["out"]["decision"], "block");
        assert!(!f.snapshot().session().unwrap().working);
        assert!(stop["out"]["reason"].as_str().unwrap().contains(&id));
        assert!(status(&f, &t, &id)["acknowledged"].is_null());

        let pending = send(&f, &t, wid, "quiet");
        dispatch_call(
            &f,
            &t,
            "set_focus",
            json!({"seconds":60,"reason":"fixture focus"}),
        )
        .unwrap();
        assert_eq!(
            request(
                &f,
                &t,
                json!({"id":6,"event":"PostToolUse","extra":{"tool_name":"Bash"}})
            )["out"],
            json!({})
        );
        dispatch_call(&f, &t, "set_focus", json!({"seconds":0})).unwrap();
        assert_eq!(
            request(
                &f,
                &t,
                json!({"id":7,"event":"PostToolUse","extra":{"agent_id":"subagent","tool_name":"Bash"}})
            )["out"],
            json!({})
        );
        assert_eq!(
            request(
                &f,
                &t,
                json!({"id":8,"event":"PostToolUse","extra":{"tool_name":"mcp__flere__get_context"}})
            )["out"],
            json!({})
        );
        let wrong = request(
            &f,
            &t,
            json!({"id":9,"event":"PostToolUse","extra":{"transcript_path":"/fixture/wrong.jsonl"}}),
        );
        assert_ne!(wrong["code"], 0);
        let notice = request(
            &f,
            &t,
            json!({"id":10,"event":"PostToolUse","extra":{"tool_name":"Bash"}}),
        );
        assert!(notice["out"].to_string().contains(&pending));
        assert!(wire::request(&f.state,&["inbox-hook",&t.id.to_string(),&t.run,&wire::hex(serde_json::to_string(&json!({"hook_event_name":"SessionStart","session_id":"11111111-2222-3333-4444-555555555555","transcript_path":"/fixture/11111111-2222-3333-4444-555555555555.jsonl","caller":[std::process::id(),os::child_identity(std::process::id()).unwrap().1]})).unwrap().as_bytes())]).is_err());

        let switched = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        assert_eq!(
            request(
                &f,
                &t,
                json!({"id":11,"uuid":switched,"event":"SessionStart"})
            )["code"],
            0
        );
        let activation = dispatch_call(&f, &t, "messaging_activation", json!({})).unwrap();
        assert_eq!(activation["conversation"], switched);
        assert!(
            activation["instructions"]
                .as_str()
                .unwrap()
                .contains("no idle queue")
        );
        if !records {
            let saved: Value =
                serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap())
                    .unwrap();
            assert_eq!(
                saved["workspaces"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|w| w["id"] == wid)
                    .unwrap()["meta"]["last_conversation"]["uuid"],
                switched
            );
        }
        f.send(&t, b"{\"exit\":true}\n");
        let back = wait_kind(&f, "shell");
        f.send(&back, b"echo $kept\n");
        f.wait_text(&back, "KEPT_CLAUDE");
        assert!(!overlay.exists());
        assert!(dispatch_call(&f, &t, "get_context", json!({})).is_err());
    }
}

#[test]
fn claude_card_start_and_management_passthrough() {
    let f = Fixture::with_shell_editor("/bin/bash", "/usr/bin/vim -Nu NONE -n --noplugin");
    setup_claude(&f);
    let mut profiles: Value =
        serde_json::from_slice(&fs::read(f.state.join("harnesses.json")).unwrap()).unwrap();
    profiles[0]["command"].as_array_mut().unwrap().extend([
        json!("--settings={\"theme\":\"dark\",\"env\":{\"ANTHROPIC_MODEL\":\"explicit-model\"}}"),
        json!("--mcp-config={\"mcpServers\":{\"fixture\":{\"command\":\"/fixture/command\"}}}"),
        json!("--append-system-prompt"),
        json!("--settings"),
    ]);
    fs::write(f.state.join("harnesses.json"), profiles.to_string()).unwrap();
    let shell = f.new_workspace("Claude passthrough");
    let wid = f.snapshot().active;
    for command in [
        "claude --version",
        "claude mcp list",
        "claude --print fixture",
    ] {
        f.send(
            &shell,
            format!("{command}; echo CLAUDE_STATUS:$?\n").as_bytes(),
        );
        f.wait_text(&shell, "CLAUDE_STATUS:7");
        assert_eq!(f.snapshot().session().unwrap().kind, "shell");
        let passed: Value =
            serde_json::from_str(&wait_file(&f.root.join("passthrough.json"), 5)).unwrap();
        assert!(
            !passed["args"]
                .as_array()
                .unwrap()
                .contains(&json!("--settings"))
        );
    }
    f.req(&["native", &wid.to_string(), "claude", ""]);
    let t = wait_kind(&f, "agent");
    f.wait_text(&t, "CLAUDE_READY");
    let receipt = record(&f, &t);
    assert_eq!(receipt["start"]["code"], 0, "{receipt}");
    assert_eq!(receipt["settings"]["theme"], "dark");
    assert_eq!(
        receipt["settings"]["env"]["ANTHROPIC_MODEL"],
        "explicit-model"
    );
    assert_eq!(
        receipt["mcp_servers"]["fixture"]["command"],
        "/fixture/command"
    );
    assert!(receipt["settings"]["permissions"].is_null());
    assert!(
        !receipt["args"]
            .as_array()
            .unwrap()
            .contains(&json!("--resume"))
    );
    let id = send(&f, &t, wid, "quiet");
    let delivered: Value = serde_json::from_slice(
        &f.req(&[
            "coordinate",
            &wid.to_string(),
            "deliver_message",
            &wire::hex(
                json!({"id":id,"session":t.id,"run":t.run})
                    .to_string()
                    .as_bytes(),
            ),
        ]),
    )
    .unwrap();
    assert_eq!(
        delivered["message"]["delivery"]["outcome"],
        "waiting-for-hook"
    );
    assert!(status(&f, &t, &id)["acknowledged"].is_null());
}

#[test]
fn claude_rejects_identity_overrides_and_oversized_overlay_before_launch() {
    let f = Fixture::with_shell_editor("/bin/bash", "/usr/bin/vim -Nu NONE -n --noplugin");
    setup_claude(&f);
    f.new_workspace("Claude invalid config");
    let wid = f.snapshot().active;
    let path = f.state.join("harnesses.json");
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for env in [
        json!({"FLERE_RUN":"forged"}),
        json!({"HOME":"/fixture"}),
        json!({"EXTRA":"x".repeat(64000)}),
    ] {
        let mut config = original.clone();
        config[0]["env"] = env;
        fs::write(&path, config.to_string()).unwrap();
        assert!(wire::request(&f.state, &["native", &wid.to_string(), "claude", ""]).is_err());
        assert_eq!(f.snapshot().session().unwrap().kind, "shell");
        let dir = f.state.join("cache/claude");
        assert!(!dir.exists() || fs::read_dir(dir).unwrap().next().is_none());
    }
}
