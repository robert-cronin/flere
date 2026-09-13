//! Read-only proof for the exact owned Codex process and native queue environment.
use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub host: u32,
    pub host_start: String,
    pub pid: u32,
    pub start: String,
    pub exe: PathBuf,
    pub cwd: PathBuf,
    pub uuid: String,
    pub transcript: PathBuf,
}
pub fn inspect(host: u32, cwd: &Path, session: u64, run: &str) -> io::Result<Target> {
    let host_start = process_start(host)?;
    let mut pids = vec![host];
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    let mut index = 0;
    while index < pids.len() && index < 128 {
        let pid = pids[index];
        index += 1;
        if !seen.insert(pid) {
            continue;
        }
        let children = match crate::os::process_children(pid, 128usize.saturating_sub(pids.len())) {
            Ok(children) => children,
            // A reaped descendant cannot be a live target. Other unreadable or
            // oversized branches make the uniqueness proof incomplete.
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        pids.extend(children);
        let Ok(exe) = crate::os::process_executable(pid) else {
            continue;
        };
        if exe.file_name().is_none_or(|n| n != "codex") {
            continue;
        }
        let start = process_start(pid)?;
        if crate::os::process_cwd(pid)? != cwd {
            continue;
        }
        let env = environment(pid)?;
        if env.get("FLERE_RUN").map(String::as_str) != Some(run)
            || env.get("FLERE_SESSION") != Some(&session.to_string())
        {
            continue;
        }
        let mut transcripts = BTreeMap::new();
        for entry in crate::os::process_files(pid, 256)? {
            let path = &entry.path;
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            let file = entry.open()?;
            if !file.metadata()?.is_file() {
                continue;
            }
            let mut line = String::new();
            io::BufReader::new(file.take(65536)).read_line(&mut line)?;
            if line.len() >= 65536 {
                continue;
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else {
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
            {
                transcripts.insert(path.clone(), id.to_owned());
            }
        }
        if process_start(pid)? != start {
            return Err(crate::wire::invalid("native process changed"));
        }
        for (transcript, uuid) in transcripts {
            targets.push(Target {
                host,
                host_start: host_start.clone(),
                pid,
                start: start.clone(),
                exe: exe.clone(),
                cwd: cwd.into(),
                uuid,
                transcript,
            });
        }
    }
    if process_start(host)? != host_start || targets.len() != 1 {
        return Err(crate::wire::invalid(
            "native target is missing, changed or ambiguous",
        ));
    }
    Ok(targets.remove(0))
}
fn environment(pid: u32) -> io::Result<BTreeMap<String, String>> {
    let data = crate::os::process_environment(pid)?;
    Ok(data
        .split(|b| *b == 0)
        .filter_map(|p| {
            let (k, v) = std::str::from_utf8(p).ok()?.split_once('=')?;
            // Only identity/config-location fields are read; credentials are not retained.
            matches!(
                k,
                "HOME" | "CODEX_HOME" | "XDG_CONFIG_HOME" | "FLERE_SESSION" | "FLERE_RUN"
            )
            .then(|| (k.to_owned(), v.to_owned()))
        })
        .collect())
}
pub fn queue_command(
    target: &Target,
    state: &Path,
    session: u64,
    run: &str,
    notice: &str,
) -> io::Result<Command> {
    if inspect(target.host, &target.cwd, session, run)? != *target {
        return Err(crate::wire::invalid("queue target changed"));
    }
    let env = environment(target.pid)?;
    let tmp = private_cache(state, run)?;
    let mut command = Command::new(&target.exe);
    command
        .args(["queue", "--thread", &target.uuid, "--message", notice])
        .current_dir(&target.cwd);
    crate::os::clean_environment(&mut command);
    for key in ["HOME", "CODEX_HOME", "XDG_CONFIG_HOME"] {
        command.env_remove(key);
        if let Some(value) = env.get(key) {
            command.env(key, value);
        }
    }
    command
        .env("TMPDIR", &tmp)
        .env("TMP", &tmp)
        .env("TEMP", &tmp);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    Ok(command)
}
// Validate each child before descending; never follow a cache/run/tmp symlink or
// silently repair permissions on an existing path. Socket name limits do not apply.
fn private_cache(state: &Path, run: &str) -> io::Result<PathBuf> {
    use std::os::unix::fs::DirBuilderExt;
    if run.len() != 32 || !run.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(crate::wire::invalid("invalid cache run identity"));
    }
    let mut path = state.to_path_buf();
    for name in ["cache", run, "tmp"] {
        path.push(name);
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let m = fs::symlink_metadata(&path)?;
        if !m.is_dir() || m.uid() != crate::os::uid() || m.mode() & 0o077 != 0 {
            return Err(crate::wire::invalid(
                "queue cache must be an owned private directory, not a symlink",
            ));
        }
    }
    Ok(path)
}
/// An extra guard after a trusted idle lifecycle event. No keys are synthesized.
pub fn empty_composer(term: &crate::terminal::Terminal) -> bool {
    if term.is_alternate()
        || !term.cursor
        || term.grid.x != 2
        || term.grid.y < term.grid.rows.saturating_sub(7)
    {
        return false;
    }
    let y = term.grid.y;
    let line = term.grid.line(y);
    if !line.starts_with("› ") && !line.starts_with("❯ ") {
        return false;
    }
    for row in term.grid.rows.saturating_sub(14)..term.grid.rows {
        let line = term.grid.line(row).to_lowercase();
        if [
            "esc to interrupt",
            "trust",
            "approve",
            "permission",
            "allow",
            "review hook",
        ]
        .iter()
        .any(|s| line.contains(s))
        {
            return false;
        }
        if row != y && (line.trim_start().starts_with('›') || line.trim_start().starts_with('❯'))
        {
            return false;
        }
    }
    let cells = &term.grid.cells[y * term.grid.cols + 2..(y + 1) * term.grid.cols];
    let placeholder_end = cells
        .iter()
        .rposition(|c| !c.text.trim().is_empty() && c.style.dim);
    // A multiline draft can have an empty first line with its cursor at column
    // two. Its continuation still occupies the composer above the final footer.
    if y >= term.grid.rows.saturating_sub(2) {
        return false;
    }
    for row in y + 1..term.grid.rows.saturating_sub(1) {
        if term.grid.cells[row * term.grid.cols..(row + 1) * term.grid.cols]
            .iter()
            .any(|c| {
                !c.text.trim().is_empty()
                    && !(placeholder_end.is_some() && single_braille_dot(&c.text))
            })
        {
            return false;
        }
    }
    // Codex 0.154 animates single Braille dots after the dim placeholder. Accept
    // these only beyond a present, entirely dim placeholder; a literal draft
    // (including placeholder words or Braille) has no such styled prefix.
    cells.iter().enumerate().all(|(i, c)| {
        c.text.trim().is_empty()
            || c.style.dim
            || (placeholder_end.is_some_and(|end| i > end + 1) && single_braille_dot(&c.text))
    })
}
fn single_braille_dot(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(c) = chars.next() else {
        return false;
    };
    chars.next().is_none()
        && (0x2801..=0x2880).contains(&(c as u32))
        && (c as u32 - 0x2800).is_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;
    #[test]
    fn composer_requires_native_idle_shape_and_excludes_drafts_and_dialogs() {
        let mut t = Terminal::new(80, 24);
        t.feed(b"\x1b[21;1H");
        t.feed("› ".as_bytes());
        t.feed(b"\x1b[2mAsk Codex to do anything\x1b[0m\x1b[21;3H");
        assert!(empty_composer(&t));
        let mut animated = t.clone();
        animated.feed("\x1b[21;30H⠈  ⠂  ⠁    ⠄\x1b[21;3H".as_bytes());
        assert!(empty_composer(&animated));
        animated.feed("\x1b[21;30Htyped draft\x1b[21;3H".as_bytes());
        assert!(!empty_composer(&animated));
        let mut multiline = t.clone();
        multiline.feed(b"\x1b[21;3H\x1b[K\x1b[22;3Hsecond draft line\x1b[21;3H");
        assert!(!empty_composer(&multiline));
        let mut braille_draft = t.clone();
        braille_draft.feed("\x1b[21;3H\x1b[K⠈  ⠂  ⠁\x1b[21;3H".as_bytes());
        assert!(!empty_composer(&braille_draft));
        let mut draft = t.clone();
        draft.feed(b"Ask Codex to do anything\x1b[21;3H");
        assert!(!empty_composer(&draft));
        for banner in [
            "esc to interrupt",
            "approve command?",
            "trust this repository",
            "Review hooks",
            "permission",
        ] {
            let mut dialog = t.clone();
            dialog.feed(format!("\x1b[19;1H{banner}\x1b[21;3H").as_bytes());
            assert!(!empty_composer(&dialog), "{banner}");
        }
        t.feed(b"\x1b[?1049h");
        assert!(!empty_composer(&t));
    }
    #[test]
    fn queue_cache_checks_each_component_without_socket_path_limit() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(crate::os::nonce().unwrap());
        fs::create_dir_all(&root).unwrap();
        let long = root.join("long-private-cache-root-not-a-socket-state");
        fs::create_dir(&long).unwrap();
        let run = "a".repeat(32);
        let tmp = private_cache(&long, &run).unwrap();
        assert!(tmp.is_dir());
        fs::remove_dir(&tmp).unwrap();
        symlink(&root, &tmp).unwrap();
        assert!(private_cache(&long, &run).is_err());
        fs::remove_file(&tmp).unwrap();
        fs::set_permissions(long.join("cache"), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_cache(&long, &run).is_err());
        assert!(private_cache(&long, "../../bad").is_err());
        fs::remove_dir_all(&root).unwrap();
    }
}
