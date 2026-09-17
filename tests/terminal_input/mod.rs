use super::*;
use serde_json::{Value, json};

fn rpc(f: &Fixture, actor: &TabView, method: &str, params: Value) -> Value {
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
        json!({
            "jsonrpc":"2.0","id":1,"method":method,"params":params
        })
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
}
fn call(f: &Fixture, actor: &TabView, op: &str, args: Value) -> Result<Value, String> {
    let response = rpc(f, actor, "tools/call", json!({"name":op,"arguments":args}));
    let text = response["content"][0]["text"].as_str().unwrap();
    if response["isError"] == true {
        Err(text.into())
    } else {
        Ok(serde_json::from_str(text).unwrap())
    }
}
fn target(f: &Fixture, tab: &TabView) -> Value {
    let list: Value = serde_json::from_slice(&f.req(&["list"])).unwrap();
    let workspace = list["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| {
            w["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["id"] == tab.id)
        })
        .unwrap();
    json!({"expected_epoch":list["epoch"],"workspace":workspace["id"],"session":tab.id,"run":tab.run})
}
fn inspect(f: &Fixture, actor: &TabView, target: &Value) -> Value {
    call(f, actor, "inspect_terminal", json!({"target":target})).unwrap()
}
fn send(f: &Fixture, actor: &TabView, target: &Value, actions: Value) -> Value {
    let observation = inspect(f, actor, target);
    call(
        f,
        actor,
        "send_terminal_input",
        json!({"target":target,"ticket":observation["ticket"],"actions":actions}),
    )
    .unwrap()
}
fn probe(f: &Fixture, native: bool) -> (TabView, PathBuf) {
    let script = f.root.join(if native {
        "native-probe.py"
    } else {
        "shell-probe.py"
    });
    fs::write(
        &script,
        r#"import os, tty
from pathlib import Path
tty.setraw(0)
output = Path(__file__).with_suffix(".bytes")
output.write_bytes(b"")
os.write(1, b"\x1b[?2004h\x1b[?1hPROBE_READY\r\n")
with output.open("ab", buffering=0) as out:
    while True:
        data = os.read(0, 4096)
        if not data:
            break
        out.write(data)
"#,
    )
    .unwrap();
    let mut tab = f.new_workspace("input receiver");
    if native {
        let wid = f.snapshot().active;
        fs::write(
            f.state.join("harnesses.json"),
            serde_json::to_vec(&json!([
                {"name":"codex","command":["/usr/bin/python3",script]}
            ]))
            .unwrap(),
        )
        .unwrap();
        f.req(&["native", &wid.to_string(), "codex", ""]);
        tab = f.snapshot().session().unwrap().clone();
    } else {
        f.send(
            &tab,
            format!("python3 -u '{}'\r", script.display()).as_bytes(),
        );
    }
    f.wait_text(&tab, "PROBE_READY");
    (tab, script.with_extension("bytes"))
}
fn wait_bytes(path: &std::path::Path, expected: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let bytes = fs::read(path).unwrap_or_default();
        if bytes.len() >= expected.len() {
            assert_eq!(bytes, expected);
            return;
        }
        assert!(
            Instant::now() < deadline,
            "missing bytes: {bytes:?}, expected {expected:?}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn terminal_input_mcp_transmits_arbitrary_sequences_to_shell_and_native_pty() {
    let f = Fixture::new();
    let (actor_wid, actor) = dispatch_fixture(&f);
    let catalog = rpc(&f, &actor, "tools/list", json!({}));
    let tools = catalog["tools"].as_array().unwrap();
    let schema = &tools
        .iter()
        .find(|t| t["name"] == "send_terminal_input")
        .unwrap()["inputSchema"];
    assert_eq!(schema["required"], json!(["target", "ticket", "actions"]));
    let variants = schema["properties"]["actions"]["items"]["oneOf"]
        .as_array()
        .unwrap();
    for name in ["text", "paste", "key", "hex"] {
        assert!(
            variants
                .iter()
                .any(|s| s["required"] == json!([name]) && s["additionalProperties"] == false)
        );
    }
    for native in [false, true] {
        let (tab, output) = probe(&f, native);
        let target = target(&f, &tab);
        f.req(&["focus", &actor_wid.to_string(), &actor.id.to_string()]);
        let observed = inspect(&f, &actor, &target);
        assert!(observed["text"].as_str().unwrap().contains("PROBE_READY"));
        assert_eq!(observed["bracketed_paste"], true);
        assert_eq!(observed["app_cursor"], true);
        let actions = json!([{"text":"λA"},{"key":"Ctrl+C"},{"key":"Alt+A"},
            {"key":"Up"},{"key":"F12"},{"key":"Enter"},{"key":"Shift+Tab"},
            {"paste":"one\ntwo"},{"hex":"00ff1b5b313b3544"}]);
        let args = json!({"target":target,"ticket":observed["ticket"],"actions":actions});
        let queued = call(&f, &actor, "send_terminal_input", args.clone()).unwrap();
        let expected =
            b"\xce\xbbA\x03\x1bA\x1bOA\x1b[24~\r\x1b[Z\x1b[200~one\ntwo\x1b[201~\x00\xff\x1b[1;5D";
        assert_eq!(queued["queued_bytes"], expected.len());
        assert_eq!(queued["outcome"], "queued");
        wait_bytes(&output, expected);
        assert!(
            call(&f, &actor, "send_terminal_input", args)
                .unwrap_err()
                .contains("consumed")
        );
        send(&f, &actor, &target, json!([{"text":"TAIL"}]));
        wait_bytes(&output, &[expected.as_slice(), b"TAIL"].concat());
        assert_eq!(f.snapshot().active, actor_wid);
        assert_eq!(f.snapshot().session().unwrap().id, actor.id);
        // Audit provenance records counts and owners, never typed content.
        let audit = fs::read_to_string(f.state.join("actions.log")).unwrap();
        assert!(audit.contains("agent-terminal-input"));
        for line in audit.lines() {
            let detail = wire::text(line.split('\t').nth(4).unwrap()).unwrap();
            assert!(!detail.contains("one\ntwo"));
            assert!(!detail.contains(&wire::hex("λA".as_bytes())));
        }
        assert!(
            !fs::read_to_string(f.state.join("workspaces.v2.json"))
                .unwrap()
                .contains(observed["ticket"].as_str().unwrap())
        );
    }
}

#[test]
fn terminal_input_rejects_wrong_owners_partial_actions_and_refresh_replay() {
    let f = Fixture::new();
    let (_, actor) = dispatch_fixture(&f);
    let (_, other) = dispatch_fixture(&f);
    let (tab, output) = probe(&f, false);
    let target = target(&f, &tab);
    let observed = inspect(&f, &actor, &target);
    let args =
        json!({"target":target,"ticket":observed["ticket"],"actions":[{"text":"ONLY_ONCE"}]});
    assert!(call(&f, &other, "send_terminal_input", args.clone()).is_err());
    for (field, value) in [
        ("expected_epoch", json!("wrong")),
        ("workspace", json!(99999)),
        ("session", json!(actor.id)),
        ("run", json!("wrong")),
    ] {
        let mut wrong = args.clone();
        wrong["target"][field] = value;
        assert!(call(&f, &actor, "send_terminal_input", wrong).is_err());
    }
    assert!(
        call(&f, &tab, "inspect_terminal", json!({"target":target}))
            .unwrap_err()
            .contains("non-native")
    );
    assert!(
        wire::request(
            &f.state,
            &[
                "coordinate",
                "1",
                "send_terminal_input",
                &wire::hex(serde_json::to_string(&args).unwrap().as_bytes())
            ]
        )
        .is_err()
    );
    call(&f, &actor, "send_terminal_input", args.clone()).unwrap();
    wait_bytes(&output, b"ONLY_ONCE");
    let observed = inspect(&f, &actor, &target);
    let invalid = json!({"target":target,"ticket":observed["ticket"],
        "actions":[{"text":"MUST_NOT_LEAK"},{"key":"invalid-key"}]});
    assert!(call(&f, &actor, "send_terminal_input", invalid).is_err());
    let token = inspect(&f, &actor, &target)["ticket"].clone();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if wire::request(&f.state, &["refresh-status"])
            .is_ok_and(|v| String::from_utf8_lossy(&v).contains("all terminal sessions preserved"))
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        call(
            &f,
            &actor,
            "send_terminal_input",
            json!({"target":target,"ticket":token,"actions":[{"text":"REPLAY"}]})
        )
        .is_err()
    );
    send(&f, &actor, &target, json!([{"text":"TAIL"}]));
    wait_bytes(&output, b"ONLY_ONCETAIL");
    let token = inspect(&f, &actor, &target)["ticket"].clone();
    f.req(&["close", &tab.id.to_string(), &tab.run]);
    assert!(
        call(
            &f,
            &actor,
            "send_terminal_input",
            json!({"target":target,"ticket":token,"actions":[{"text":"STOPPED"}]})
        )
        .is_err()
    );
    assert!(call(&f, &actor, "send_terminal_input", args).is_err());
}

#[test]
fn terminal_input_operates_real_shell_draft_and_vim_through_agent_cli() {
    let f = Fixture::new();
    let (_, actor) = dispatch_fixture(&f);
    let shell = f.new_workspace("shell input");
    let target_shell = target(&f, &shell);
    f.send(&shell, b"UNSUBMITTED_FIXTURE_DRAFT");
    f.wait_text(&shell, "UNSUBMITTED_FIXTURE_DRAFT");
    let observed = inspect(&f, &actor, &target_shell);
    let args = json!({"target":target_shell,"ticket":observed["ticket"],"actions":[
        {"key":"Ctrl+U"},{"text":"printf shell-ok > terminal-result"},{"key":"Enter"}]});
    let result = Command::new(env!("CARGO_BIN_EXE_flere"))
        .args([
            "--state",
            f.state.to_str().unwrap(),
            "agent-call",
            "send_terminal_input",
            &args.to_string(),
        ])
        .env("FLERE_SESSION", actor.id.to_string())
        .env("FLERE_RUN", &actor.run)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(wait_file(&f.root.join("terminal-result"), 4), "shell-ok");

    let path = f.root.join("edited.txt");
    fs::write(&path, "").unwrap();
    let wid = f.snapshot().active;
    f.req(&[
        "open",
        &wid.to_string(),
        &wire::hex(path.to_str().unwrap().as_bytes()),
    ]);
    let editor = f.snapshot().session().unwrap().clone();
    let target_editor = target(&f, &editor);
    f.wait_text(&editor, "edited.txt");
    send(
        &f,
        &actor,
        &target_editor,
        json!([{"text":"i"},{"paste":"edited through agent"}]),
    );
    f.wait_text(&editor, "edited through agent");
    send(
        &f,
        &actor,
        &target_editor,
        json!([{"key":"Escape"},{"text":":w"},{"key":"Enter"}]),
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        if fs::read_to_string(&path).unwrap() == "edited through agent\n" {
            break;
        }
        assert!(Instant::now() < deadline, "{}", f.capture(&editor));
        std::thread::sleep(Duration::from_millis(20));
    }
}
