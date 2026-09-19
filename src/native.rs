//! Explicit native launches and exact conversation metadata. Manual resumes have no prompt;
//! fresh agent dispatch adds only a bounded assignment reference. No permission overrides.
pub mod delivery;
pub mod launcher;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, BufRead, Read},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conversation {
    pub harness: String,
    pub uuid: String,
    pub cwd: PathBuf,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct HostSpec {
    pub id: u64,
    pub run: String,
    pub harness: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub shell: String,
    pub conversation: String,
    #[serde(default)]
    pub dispatch: String,
    #[serde(default)]
    pub launcher: Option<(u32, String)>,
}
#[derive(Serialize, Deserialize)]
struct Profile {
    name: String,
    command: Vec<String>,
}
pub fn valid_uuid(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
pub fn arguments(state: &Path, harness: &str, uuid: &str) -> io::Result<Vec<String>> {
    if !uuid.is_empty() && !valid_uuid(uuid) {
        return Err(crate::wire::invalid(
            "resume requires an exact conversation UUID",
        ));
    }
    let mut argv = match harness {
        "codex" => vec!["codex".into()],
        "claude" => vec!["claude".into()],
        "copilot" => vec!["copilot".into()],
        _ => return Err(crate::wire::invalid("choose codex, claude or copilot")),
    };
    let path = state.join("harnesses.json");
    if path.exists() {
        let mut b = Vec::new();
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?
            .take(65537)
            .read_to_end(&mut b)?;
        if b.len() > 65536 {
            return Err(crate::wire::invalid("harness config exceeds bound"));
        }
        let profiles: Vec<Profile> = serde_json::from_slice(&b).map_err(io::Error::other)?;
        if let Some(p) = profiles.into_iter().find(|p| p.name == harness) {
            argv = p.command;
        }
    }
    if argv.is_empty() || argv[0].is_empty() || argv.iter().any(|s| s.contains('\0')) {
        return Err(crate::wire::invalid("invalid native argv"));
    }
    if harness == "codex"
        && Path::new(&argv[0])
            .file_name()
            .is_some_and(|s| s == "codex")
    {
        argv.extend(codex_flags(state)?);
    }
    if !uuid.is_empty() {
        match harness {
            "codex" => argv.extend(["resume".into(), uuid.into()]),
            "claude" => argv.extend(["--resume".into(), uuid.into()]),
            "copilot" => argv.push(format!("--resume={uuid}")),
            _ => unreachable!(),
        };
    }
    // No appended prompt, --last, shell interpolation, auto-approval, or trust override.
    Ok(argv)
}
pub(crate) fn codex_flags(state: &Path) -> io::Result<Vec<String>> {
    let binary = crate::os::executable_path()?;
    let settings = [
        format!(
            "mcp_servers.flere.command={}",
            serde_json::to_string(&binary.to_string_lossy()).map_err(io::Error::other)?
        ),
        format!(
            "mcp_servers.flere.args={}",
            serde_json::to_string(&[
                "--state",
                state
                    .to_str()
                    .ok_or_else(|| crate::wire::invalid("state must be UTF-8"))?,
                "mcp"
            ])
            .map_err(io::Error::other)?
        ),
        "mcp_servers.flere.env_vars=[\"FLERE_SESSION\",\"FLERE_RUN\"]".into(),
    ];
    let mut flags = Vec::new();
    for setting in settings {
        flags.extend(["-c".into(), setting]);
    }
    flags.extend(crate::inbox_hook::flags(&binary, state));
    Ok(flags)
}
/// With no discoverable ID, let the harness present its own resume picker.
pub fn resume_arguments(state: &Path, harness: &str, uuid: &str) -> io::Result<Vec<String>> {
    let mut argv = arguments(state, harness, uuid)?;
    if uuid.is_empty() {
        argv.push(
            if harness == "codex" {
                "resume"
            } else {
                "--resume"
            }
            .into(),
        );
    }
    Ok(argv)
}
pub fn write_spec(state: &Path, spec: &HostSpec) -> io::Result<()> {
    let _timing = crate::diagnostics::measure("native-spec-write");
    let dir = state.join("runs");
    if !dir.exists() {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
    }
    crate::workspace::atomic_write(
        &dir.join(format!("{}.json", spec.run)),
        &serde_json::to_vec(spec).map_err(io::Error::other)?,
    )
}
pub fn host(state: &Path, run: &str) -> io::Result<()> {
    if run.len() != 32 || !run.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(crate::wire::invalid("invalid host run identity"));
    }
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state.join("runs").join(format!("{run}.json")))?;
    let m = f.metadata()?;
    if !m.is_file() || m.uid() != crate::os::uid() || m.mode() & 0o077 != 0 {
        return Err(crate::wire::invalid(
            "host specification must be owned/private",
        ));
    }
    let mut b = Vec::new();
    f.take(65537).read_to_end(&mut b)?;
    if b.len() > 65536 {
        return Err(crate::wire::invalid("host specification too large"));
    }
    let spec: HostSpec = serde_json::from_slice(&b).map_err(io::Error::other)?;
    if spec.run != run || spec.argv.is_empty() {
        return Err(crate::wire::invalid("host run mismatch"));
    }
    crate::os::host_signals();
    let mut command = Command::new(&spec.argv[0]);
    command.args(&spec.argv[1..]).current_dir(&spec.cwd);
    crate::os::clean_environment(&mut command);
    command
        .env("FLERE_STATE", state)
        .env("FLERE_SESSION", spec.id.to_string())
        .env("FLERE_RUN", run);
    let notify = |event: &str, detail: &str| {
        if !spec.dispatch.is_empty() {
            let _ = crate::wire::request(
                state,
                &[
                    "worker-event",
                    &spec.id.to_string(),
                    run,
                    event,
                    &crate::wire::hex(detail.as_bytes()),
                ],
            );
        }
    };
    let result = match command.spawn() {
        Ok(mut child) => {
            notify("started", &format!("native PID {}", child.id()));
            child.wait()
        }
        Err(e) => Err(e),
    };
    match &result {
        Ok(status) => notify("exited", &status.to_string()),
        Err(e) => notify("failed", &e.to_string()),
    }
    match result {
        Ok(status) => eprintln!(
            "\r\n{} exited ({status}). Returning to {}.\r",
            spec.harness, spec.shell
        ),
        Err(e) => eprintln!(
            "\r\n{} could not start: {}\r",
            spec.harness,
            crate::wire::passive(&e.to_string())
        ),
    };
    let _ = crate::wire::request(state, &["native-ended", &spec.id.to_string(), run]);
    let mut shell = crate::close::shell_command(state, run, &spec.shell)?;
    shell.current_dir(&spec.cwd);
    crate::os::clean_environment(&mut shell);
    Err(shell.exec())
}
fn process_start(pid: u32) -> io::Result<String> {
    crate::os::child_identity(pid).map(|(_, start)| start)
}
/// Inspect only descendants of the owned host and metadata on their open rollout FDs.
/// No transcript search, timestamps-as-identity, latest-session heuristic, or global process scan.
pub fn discover_codex(root: u32, cwd: &Path) -> io::Result<Option<String>> {
    let before = process_start(root)?;
    let mut pids = vec![root];
    let mut ids = std::collections::BTreeSet::new();
    let mut index = 0;
    while index < pids.len() && index < 128 {
        let pid = pids[index];
        index += 1;
        if let Ok(children) = crate::os::process_children(pid, 128usize.saturating_sub(pids.len()))
        {
            pids.extend(children);
        }
        let Ok(exe) = crate::os::process_executable(pid) else {
            continue;
        };
        if exe.file_name().is_none_or(|s| s != "codex") {
            continue;
        }
        let Ok(start) = process_start(pid) else {
            continue;
        };
        let Ok(files) = crate::os::process_files(pid, 256) else {
            continue;
        };
        for entry in files {
            let path = &entry.path;
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let Ok(file) = entry.open() else {
                continue;
            };
            if !file.metadata().is_ok_and(|m| m.is_file()) {
                continue;
            }
            let mut line = String::new();
            let mut reader = io::BufReader::new(file.take(65536));
            if reader.read_line(&mut line).is_err() || line.len() >= 65536 {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let p = &v["payload"];
            let Some(id) = p["id"].as_str() else {
                continue;
            };
            if v["type"] == "session_meta"
                && p["source"] == "cli"
                && p["cwd"].as_str() == cwd.to_str()
                && valid_uuid(id)
                && process_start(pid).ok().as_ref() == Some(&start)
            {
                ids.insert(id.to_owned());
            }
        }
    }
    if process_start(root)? != before {
        return Err(crate::wire::invalid(
            "native host changed during inspection",
        ));
    }
    if ids.len() > 1 {
        return Err(crate::wire::invalid(
            "ambiguous native conversation; no identity recorded",
        ));
    }
    Ok(ids.into_iter().next())
}
/// Display-only observation of the live status immediately above the native composer.
/// It never authorizes input, acknowledgements, cancellation or workflow transitions.
pub fn working_screen(term: &crate::terminal::Terminal) -> bool {
    let end = (0..term.grid.rows)
        .rev()
        .find(|y| !term.grid.line(*y).trim().is_empty())
        .map_or(0, |y| y + 1);
    let lines: Vec<_> = (end.saturating_sub(12)..end)
        .map(|y| term.grid.line(y))
        .collect();
    for (i, line) in lines.iter().enumerate() {
        let line = line.trim();
        if !line.ends_with("esc to interrupt)")
            || !line.starts_with(['•', '·', '✶', '✳', '✻', '✽', '✢', '*'])
        {
            continue;
        }
        let Some((_, tail)) = line.rsplit_once('(') else {
            continue;
        };
        let unit = tail.trim_start_matches(|c: char| c.is_ascii_digit());
        if !tail.starts_with(|c: char| c.is_ascii_digit())
            || !(unit.starts_with("h ") || unit.starts_with("m ") || unit.starts_with('s'))
        {
            continue;
        }
        let mut below = lines
            .iter()
            .skip(i + 1)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let mut next = below.next();
        // The automatic approval reviewer shows one tool caption between its
        // running status and the composer. This is activity, not a human choice.
        if line.contains("Reviewing approval request (")
            && next.is_some_and(|s| s.starts_with("└ "))
        {
            next = below.next();
        }
        if let Some(next) = next {
            let Some(rest) = next.strip_prefix('›').or_else(|| next.strip_prefix('❯')) else {
                return false;
            };
            return !rest.trim_start().starts_with(|c: char| c.is_ascii_digit());
        }
    }
    false
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_uuid_only() {
        assert!(valid_uuid("12345678-1234-1234-1234-123456789012"));
        assert!(!valid_uuid("--last"));
        assert!(!valid_uuid("latest"));
        assert!(!valid_uuid("12345678-1234-1234-1234-12345678901g"));
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    #[test]
    fn automatic_approval_review_with_a_tool_caption_is_working() {
        let mut t = crate::terminal::Terminal::new(100, 20);
        t.feed(
            "• Reviewing approval request (6s • esc to interrupt)\r\n  └ MCP get_context on flere\r\n\r\n› Ask Codex to do anything".as_bytes(),
        );
        assert!(working_screen(&t));
        t.feed(b"\x1b[2J\x1b[H");
        t.feed(
            "• Reviewing approval request (6s • esc to interrupt)\r\n  └ MCP get_context on flere\r\n\r\n› 1. Allow\r\n  2. Cancel".as_bytes(),
        );
        assert!(
            !working_screen(&t),
            "human approval choices are not activity"
        );
        t.feed(b"\x1b[2J\x1b[H");
        t.feed(
            "• Reviewing approval request (6s • esc to interrupt)\r\n  └ MCP get_context on flere\r\nAn unrelated response\r\n› ".as_bytes(),
        );
        assert!(!working_screen(&t), "old status text is not live activity");
    }

    #[test]
    fn activity_requires_live_status_and_composer_not_approval_or_a_quote() {
        let mut t = crate::terminal::Terminal::new(100, 20);
        t.feed("• Working (4s • esc to interrupt)\r\n\r\n› ".as_bytes());
        assert!(working_screen(&t));
        t.feed(b"\x1b[2J\x1b[H");
        t.feed("• Working (4s • esc to interrupt)\r\n\r\n› 1. Allow\r\n  2. Cancel".as_bytes());
        assert!(!working_screen(&t));
        t.feed(b"\x1b[2J\x1b[H");
        t.feed("Documentation quotes: Working (4s • esc to interrupt)\r\n› ".as_bytes());
        assert!(!working_screen(&t));
        t.feed(b"\x1b[2J\x1b[HReady\r\n");
        assert!(!working_screen(&t));
    }
}
