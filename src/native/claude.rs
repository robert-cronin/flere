//! Claude's private launch settings and live hook caller proof. Transcript bodies
//! are never read, and no guessed session or idle keystroke delivery is used.
use super::*;
use crate::{inbox_hook::Input, native::delivery::Target};
use serde_json::json;

// Supported by Claude Code 2.1.63; do not register Codex-only Interrupt/PostCompact.
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "PreCompact",
    "SessionEnd",
];

/// Preserve explicit CLI settings/MCP sources while adding one scoped overlay.
/// Claude's --mcp-config is variadic; repeated --settings takes the last value.
pub(crate) fn configure(state: &Path, cwd: &Path, argv: &mut Vec<String>) -> io::Result<()> {
    let mut kept = vec![argv[0].clone()];
    let mut settings_source = None;
    let mut sources = Vec::new();
    let mut i = 1;
    while i < argv.len() && argv[i] != "--" {
        let arg = &argv[i];
        let (name, attached) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(n, v)| (n, Some(v)));
        if matches!(name, "--settings" | "--mcp-config") {
            let value = if let Some(value) = attached {
                value.to_string()
            } else {
                i += 1;
                argv.get(i)
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .ok_or_else(|| crate::wire::invalid("missing Claude configuration value"))?
            };
            if name == "--settings" {
                settings_source = Some(value);
            } else {
                sources.push(value);
                while argv.get(i + 1).is_some_and(|a| !a.starts_with('-')) {
                    i += 1;
                    sources.push(argv[i].clone());
                }
            }
        } else {
            kept.push(arg.clone());
            // Never interpret another option's value as a configuration flag.
            if attached.is_none() && super::launcher::claude_value_option(name) {
                i += 1;
                kept.push(
                    argv.get(i)
                        .cloned()
                        .ok_or_else(|| crate::wire::invalid("missing Claude option value"))?,
                );
            }
        }
        i += 1;
    }
    let mut settings = if let Some(source) = settings_source {
        let bytes = if source.trim_start().starts_with('{') {
            source.into_bytes()
        } else {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(cwd.join(source))?;
            if !file.metadata()?.is_file() {
                return Err(crate::wire::invalid(
                    "Claude settings must be a regular file",
                ));
            }
            let mut bytes = Vec::new();
            file.take(65537).read_to_end(&mut bytes)?;
            bytes
        };
        if bytes.len() > 65536 {
            return Err(crate::wire::invalid("Claude settings exceeds bound"));
        }
        serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)?
    } else {
        json!({})
    };
    let object = settings
        .as_object_mut()
        .ok_or_else(|| crate::wire::invalid("Claude settings must be an object"))?;
    let binary = crate::os::executable_path()?;
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\"'\"'"));
    let command = format!(
        "{} --state {} _claude-inbox-hook",
        quote(&binary),
        quote(state)
    );
    let hooks = object
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| crate::wire::invalid("Claude hooks must be an object"))?;
    for event in EVENTS {
        let entries = hooks
            .entry(*event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| crate::wire::invalid("Claude hook entries must be an array"))?;
        entries.insert(
            0,
            json!({"hooks":[{"type":"command","command":command,
            "timeout":if *event == "SessionEnd" {3} else {5}}]}),
        );
    }
    let mut env = profiles(state)?
        .into_iter()
        .find(|p| p.name == "claude")
        .map(|p| p.env)
        .unwrap_or_default();
    if let Some(extra) = object.get("env") {
        env.extend(
            serde_json::from_value::<std::collections::BTreeMap<String, String>>(extra.clone())
                .map_err(io::Error::other)?,
        );
    }
    if env.iter().any(|(k, v)| {
        k.is_empty()
            || k.contains(['=', '\0'])
            || v.contains('\0')
            || k.starts_with("FLERE")
            || matches!(k.as_str(), "HOME" | "PATH" | "CLAUDE_CONFIG_DIR")
    }) {
        return Err(crate::wire::invalid("invalid Claude profile environment"));
    }
    object.insert("env".into(), json!(env));
    let bytes = serde_json::to_vec(&settings).map_err(io::Error::other)?;
    if bytes.len() > 65536 {
        return Err(crate::wire::invalid("Claude merged settings exceeds bound"));
    }
    let dir = state.join("cache").join("claude");
    crate::close::private_directory(&state.join("cache"))?;
    crate::close::private_directory(&dir)?;
    let path = dir.join(format!("{}.json", crate::os::nonce()?));
    crate::workspace::atomic_write(&path, &bytes)?;
    let mcp = json!({"mcpServers":{"flere":{"type":"stdio","command":binary,
        "args":["--state",state,"mcp"]}}});
    kept.push("--mcp-config".into());
    kept.extend(sources);
    kept.push(mcp.to_string());
    // End the variadic MCP option before the original positional/delimited args.
    kept.extend(["--settings".into(), path.to_string_lossy().into_owned()]);
    kept.extend_from_slice(&argv[i..]);
    *argv = kept;
    Ok(())
}

pub(crate) fn settings_path(spec: &HostSpec) -> Option<&Path> {
    spec.argv
        .windows(2)
        .find(|p| {
            p[0] == "--settings"
                && Path::new(&p[1])
                    .parent()
                    .is_some_and(|d| d.ends_with("cache/claude"))
        })
        .map(|p| Path::new(&p[1]))
}
pub(crate) fn configured(spec: &HostSpec) -> bool {
    spec.harness == "claude"
        && settings_path(spec).is_some()
        && spec.argv.iter().any(|a| a == "--mcp-config")
        && spec.argv.iter().any(|a| {
            serde_json::from_str::<serde_json::Value>(a)
                .is_ok_and(|v| v["mcpServers"]["flere"].is_object())
        })
}
pub(crate) fn cleanup(state: &Path, spec: &HostSpec) {
    if let Some(path) = settings_path(spec)
        && path.parent() == Some(state.join("cache/claude").as_path())
        && path
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        let _ = fs::remove_file(path);
    }
}

fn belongs(pid: u32, host: u32) -> io::Result<()> {
    let mut next = pid;
    for _ in 0..32 {
        if next == host {
            return Ok(());
        }
        let parent = crate::os::child_identity(next)?.0;
        if parent == 0 || parent == next {
            break;
        }
        next = parent;
    }
    Err(crate::wire::invalid(
        "Claude process is outside the owned native host",
    ))
}
fn process_matches(pid: u32, spec: &HostSpec) -> io::Result<bool> {
    let Some(settings) = settings_path(spec) else {
        return Ok(false);
    };
    let args = crate::os::process_arguments(pid)?;
    let args: Vec<_> = args
        .strip_suffix(&[0])
        .unwrap_or(&args)
        .split(|b| *b == 0)
        .collect();
    if !args
        .windows(2)
        .any(|p| p[0] == b"--settings" && p[1] == settings.as_os_str().as_encoded_bytes())
    {
        return Ok(false);
    }
    let env = crate::os::process_environment(pid)?;
    let session = format!("FLERE_SESSION={}", spec.id);
    let run = format!("FLERE_RUN={}", spec.run);
    Ok(env.split(|b| *b == 0).any(|e| e == session.as_bytes())
        && env.split(|b| *b == 0).any(|e| e == run.as_bytes())
        && crate::os::process_cwd(pid)? == spec.cwd)
}
pub(crate) fn verify(host: u32, spec: &HostSpec, target: &Target) -> io::Result<()> {
    if host != target.host
        || !configured(spec)
        || crate::os::child_identity(host)?.1 != target.host_start
        || crate::os::child_identity(target.pid)?.1 != target.start
        || match (
            crate::os::process_executable_id(target.pid)?,
            target.executable_id,
        ) {
            (Some(actual), Some(expected)) => actual != expected,
            _ => crate::os::process_executable(target.pid)? != target.exe,
        }
        || target.cwd != spec.cwd
        || target.uuid != spec.conversation
        || !process_matches(target.pid, spec)?
    {
        return Err(crate::wire::invalid(
            "Claude native process identity changed",
        ));
    }
    belongs(target.pid, host)
}
pub(crate) fn inspect(host: u32, spec: &HostSpec, event: &Input) -> io::Result<Target> {
    let (caller, start) = event
        .caller
        .as_ref()
        .ok_or_else(|| crate::wire::invalid("missing Claude hook caller"))?;
    if !configured(spec)
        || !valid_uuid(&event.session_id)
        || !EVENTS.contains(&event.hook_event_name.as_str())
        || event.transcript_path.len() > 4096
        || event.tool_name.len() > 1024
        || !Path::new(&event.transcript_path).is_absolute()
        || Path::new(&event.transcript_path)
            .extension()
            .is_none_or(|ext| ext != "jsonl")
        || Path::new(&event.transcript_path)
            .file_stem()
            .and_then(|s| s.to_str())
            != Some(&event.session_id)
        || crate::os::child_identity(*caller)?.1 != *start
    {
        return Err(crate::wire::invalid("invalid Claude hook identity"));
    }
    // The command adapter is still alive while its synchronous socket request runs.
    // Never trust a PID supplied by native JSON or a sibling session's environment.
    let settings = settings_path(spec).unwrap();
    let mut bytes = Vec::new();
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(settings)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != crate::os::uid() || meta.mode() & 0o077 != 0 {
        return Err(crate::wire::invalid(
            "Claude hook settings must be owned/private",
        ));
    }
    file.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(crate::wire::invalid("Claude hook settings exceeds bound"));
    }
    let config: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let command = config["hooks"][&event.hook_event_name][0]["hooks"][0]["command"]
        .as_str()
        .ok_or_else(|| crate::wire::invalid("Claude hook is not configured"))?;
    let expected = shell_words::split(command).map_err(io::Error::other)?;
    let args = crate::os::process_arguments(*caller)?;
    let args: Vec<_> = args
        .strip_suffix(&[0])
        .unwrap_or(&args)
        .split(|b| *b == 0)
        .collect();
    if args.len() != expected.len()
        || !args.iter().zip(&expected).all(|(a, e)| *a == e.as_bytes())
        || expected.last().map(String::as_str) != Some("_claude-inbox-hook")
        || match crate::os::process_executable_id(*caller)? {
            Some(id) => fs::metadata(&expected[0]).map(|m| (m.dev(), m.ino())).ok() != Some(id),
            None => crate::os::process_executable(*caller)? != Path::new(&expected[0]),
        }
    {
        return Err(crate::wire::invalid("Claude hook adapter is not live"));
    }
    belongs(*caller, host)?;
    let host_start = crate::os::child_identity(host)?.1;
    let mut next = crate::os::child_identity(*caller)?.0;
    for _ in 0..32 {
        if process_matches(next, spec)? {
            let target = Target {
                host,
                host_start: host_start.clone(),
                pid: next,
                start: crate::os::child_identity(next)?.1,
                exe: crate::os::process_executable(next)?,
                executable_id: crate::os::process_executable_id(next)?,
                cwd: spec.cwd.clone(),
                uuid: event.session_id.clone(),
                transcript: PathBuf::from(&event.transcript_path),
            };
            if crate::os::child_identity(*caller)?.1 != *start
                || crate::os::child_identity(host)?.1 != host_start
            {
                return Err(crate::wire::invalid("Claude hook process changed"));
            }
            return Ok(target);
        }
        if next == host {
            break;
        }
        next = crate::os::child_identity(next)?.0;
    }
    Err(crate::wire::invalid(
        "Claude hook is outside its configured native process",
    ))
}
