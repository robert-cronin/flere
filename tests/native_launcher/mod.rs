use super::*;
use serde_json::{Value, json};

fn setup(f: &Fixture) {
    let bin = f.root.join("bin");
    let native = f.root.join("native");
    fs::create_dir(&bin).unwrap();
    fs::create_dir(&native).unwrap();
    copy_fixture_python(&native.join("codex"));
    let script = f.root.join("fake.py");
    fs::write(&script, r#"import json, os, sys, subprocess
from pathlib import Path
root = Path(os.environ['HOME'])
args = sys.argv[1:]
if '--version' in args or (args and args[0] == 'exec'):
    (root / 'passthrough.json').write_text(json.dumps({'args': args, 'run': os.environ.get('FLERE_RUN')}))
    print('FAKE_VERSION', flush=True)
    sys.exit(7)
for i, arg in enumerate(args):
    if arg in ('-C', '--cd'): os.chdir(args[i+1])
    elif arg.startswith('--cd='): os.chdir(arg.split('=',1)[1])
run = os.environ['FLERE_RUN']
record = {'args': args, 'run': run, 'session': os.environ['FLERE_SESSION'], 'cwd': os.getcwd()}
command = next(a.split('=',1)[1] for a in args if a.startswith('mcp_servers.flere.command='))
result = subprocess.run([json.loads(command), '--state', os.environ['FLERE_STATE'], 'agent-call', 'get_context'], capture_output=True, text=True)
record['context_status'] = result.returncode
record['context'] = json.loads(result.stdout) if result.returncode == 0 else result.stderr
(root / ('launch-' + run + '.json')).write_text(json.dumps(record))
uuid = '11111111-2222-3333-4444-555555555555'
rollout = root / ('rollout-' + run + '.jsonl')
f = rollout.open('w+')
f.write(json.dumps({'type':'session_meta','payload':{'id':uuid,'source':'cli','cwd':os.getcwd()}}) + '\n')
f.flush()
print('\033[?2004hFAKE_READY', flush=True)
for line in sys.stdin:
    if line.strip() == 'exit':
        break
print('\033[?2004lFAKE_EXIT', flush=True)
"#).unwrap();
    let command = format!(
        "#!/bin/sh\nexec '{}' '{}' \"$@\"\n",
        native.join("codex").display(),
        script.display()
    );
    fs::write(bin.join("codex"), command).unwrap();
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(
        f.root.join(".bashrc"),
        format!("export PATH='{}:/usr/bin:/bin'\n", bin.display()),
    )
    .unwrap();
}
fn wait_kind(f: &Fixture, kind: &str) -> TabView {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(t) = f.snapshot().session().filter(|t| t.kind == kind) {
            return t.clone();
        }
        assert!(Instant::now() < until, "tab never became {kind}");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn record(f: &Fixture, t: &TabView) -> Value {
    let path = f.root.join(format!("launch-{}.json", t.run));
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = fs::read(&path)
            && let Ok(value) = serde_json::from_slice(&bytes)
        {
            return value;
        }
        assert!(
            Instant::now() < until,
            "native launch receipt missing: {}",
            f.capture(t)
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn shell_codex_resume_registers_exact_context_preserves_shell_and_rotates_run() {
    let f = Fixture::with_shell_editor("/bin/bash", "/usr/bin/vim -Nu NONE -n --noplugin");
    setup(&f);
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new()
            .append(true)
            .open(f.root.join(".bashrc"))
            .unwrap(),
        "alias codex='codex --no-alt-screen'"
    )
    .unwrap();
    let shell = f.new_workspace("Automatic native chat");
    let wid = f.snapshot().active;
    f.send(&shell, b"kept=KEPT_SHELL; codex resume --last\n");
    let native = wait_kind(&f, "agent");
    let receipt = record(&f, &native);
    assert_eq!((native.id, native.pid), (shell.id, shell.pid));
    assert_ne!(native.run, shell.run);
    assert_eq!(receipt["session"], native.id.to_string());
    assert_eq!(receipt["context_status"], 0, "{receipt}");
    let args = receipt["args"].as_array().unwrap();
    assert!(args.contains(&json!("--no-alt-screen")));
    assert_eq!(&args[args.len() - 2..], &[json!("resume"), json!("--last")]);
    assert!(
        wire::request(
            &f.state,
            &[
                "agent-operation",
                &shell.id.to_string(),
                &shell.run,
                "context",
                "7b7d"
            ]
        )
        .is_err()
    );
    let context = receipt["context"].to_string();
    assert!(
        context.contains(&format!("\"workspace\":{wid}"))
            || context.contains(&format!("\"workspace_id\":{wid}")),
        "{context}"
    );
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let saved: Value =
            serde_json::from_slice(&fs::read(f.state.join("workspaces.v2.json")).unwrap()).unwrap();
        if saved["workspaces"][0]["meta"]["last_conversation"]["uuid"]
            == "11111111-2222-3333-4444-555555555555"
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "native conversation was not recorded"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    let until = Instant::now() + Duration::from_secs(5);
    while !String::from_utf8(f.req(&["refresh-status"]))
        .unwrap()
        .contains("all terminal sessions preserved")
    {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.snapshot().session().unwrap().run, native.run);
    f.send(&native, b"exit\n");
    let shell_again = wait_kind(&f, "shell");
    f.send(&shell_again, b"echo $kept\n");
    f.wait_text(&shell_again, "KEPT_SHELL");
    f.send(&shell_again, b"bash --norc\n");
    f.wait_text(&shell_again, "$ ");
    f.send(&shell_again, b"codex\n");
    let second = wait_kind(&f, "agent");
    assert_ne!(second.run, native.run);
    assert_eq!(record(&f, &second)["context_status"], 0);
    assert!(
        wire::request(
            &f.state,
            &["native-ended", &native.id.to_string(), &native.run]
        )
        .is_err()
    );
    assert_eq!(f.snapshot().session().unwrap().kind, "agent");
}

#[test]
fn shell_management_commands_and_spoofed_ownership_do_not_register_chats() {
    let f = Fixture::with_shell_editor("/bin/bash", "/usr/bin/vim -Nu NONE -n --noplugin");
    setup(&f);
    let shell = f.new_workspace("CLI passthrough");
    f.send(&shell, b"codex --version; echo STATUS:$?\n");
    f.wait_text(&shell, "STATUS:7");
    let receipt: Value =
        serde_json::from_slice(&fs::read(f.root.join("passthrough.json")).unwrap()).unwrap();
    assert_eq!(receipt["args"], json!(["--version"]));
    assert!(receipt["run"].is_null());
    assert_eq!(f.snapshot().session().unwrap().run, shell.run);
    assert_eq!(f.snapshot().session().unwrap().kind, "shell");
    let pid = std::process::id();
    let start = os::child_identity(pid).unwrap().1;
    assert!(
        wire::request(
            &f.state,
            &["shell-context", &shell.run, &pid.to_string(), &start]
        )
        .is_err()
    );
    f.send(&shell, b"codex exec 'literal prompt'\n");
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        let value: Value =
            serde_json::from_slice(&fs::read(f.root.join("passthrough.json")).unwrap()).unwrap();
        if value["args"] == json!(["exec", "literal prompt"]) {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(f.snapshot().session().unwrap().kind, "shell");
}

#[test]
fn explicit_native_directory_is_recorded_without_changing_the_shell_directory() {
    let f = Fixture::with_shell_editor("/bin/bash", "/usr/bin/vim -Nu NONE -n --noplugin");
    setup(&f);
    let folder = f.root.join("native folder");
    fs::create_dir(&folder).unwrap();
    let shell = f.new_workspace("Native directory");
    f.send(&shell, b"codex -C 'native folder'\n");
    let native = wait_kind(&f, "agent");
    let receipt = record(&f, &native);
    assert_eq!(receipt["cwd"], folder.to_str().unwrap());
    let spec: Value = serde_json::from_slice(
        &fs::read(f.state.join("runs").join(format!("{}.json", native.run))).unwrap(),
    )
    .unwrap();
    assert_eq!(spec["cwd"], folder.to_str().unwrap());
    f.send(&native, b"exit\n");
    let shell = wait_kind(&f, "shell");
    assert_eq!(os::process_cwd(shell.pid).unwrap(), f.root);
}
