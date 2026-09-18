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
            if let Some(id) = transcript_id(file, cwd)? {
                transcripts.insert(path.clone(), id);
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
fn transcript_id(file: impl Read, cwd: &Path) -> io::Result<Option<String>> {
    let mut line = String::new();
    io::BufReader::new(file.take(65536)).read_line(&mut line)?;
    if line.len() >= 65536 {
        return Ok(None);
    }
    let Ok(v) = serde_json::from_str::<Value>(&line) else {
        return Ok(None);
    };
    let p = &v["payload"];
    Ok(p["id"]
        .as_str()
        .filter(|id| {
            v["type"] == "session_meta"
                && p["source"] == "cli"
                && p["cwd"].as_str() == cwd.to_str()
                && valid_uuid(id)
        })
        .map(str::to_owned))
}

/// Goal continuations and native agent messages can begin a main turn without
/// UserPromptSubmit. Prove a changed turn from this process's open transcript;
/// neither hook input nor a latest-session filename is sufficient evidence.
pub fn recorded_turn_matches(target: &Target, turn: &str) -> io::Result<bool> {
    if process_start(target.pid)? != target.start {
        return Err(crate::wire::invalid("native process changed"));
    }
    let entry = crate::os::process_files(target.pid, 256)?
        .into_iter()
        .find(|entry| entry.path == target.transcript)
        .ok_or_else(|| crate::wire::invalid("native transcript is no longer open"))?;
    let mut file = entry.open()?;
    if !file.metadata()?.is_file()
        || transcript_id(&mut file, &target.cwd)?.as_deref() != Some(&target.uuid)
    {
        return Err(crate::wire::invalid("native transcript identity changed"));
    }
    let recorded = recorded_turn(file)?;
    if process_start(target.pid)? != target.start {
        return Err(crate::wire::invalid("native process changed"));
    }
    Ok(recorded.as_deref() == Some(turn))
}

const TURN_SCAN_LIMIT: u64 = 1024 * 1024;
fn recorded_turn(mut file: impl Read + io::Seek) -> io::Result<Option<String>> {
    use io::SeekFrom;
    let end = file.seek(SeekFrom::End(0))?;
    let start = end.saturating_sub(TURN_SCAN_LIMIT);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    (&mut file).take(end - start).read_to_end(&mut bytes)?;
    if file.seek(SeekFrom::End(0))? != end {
        return Ok(None);
    }
    // A concurrent partial write cannot prove which turn is newest.
    if bytes.last() != Some(&b'\n') {
        return Ok(None);
    }
    let bytes = if start == 0 {
        bytes.as_slice()
    } else {
        let Some(first) = bytes.iter().position(|b| *b == b'\n') else {
            return Ok(None);
        };
        &bytes[first + 1..]
    };
    #[derive(Deserialize)]
    struct Record<'a> {
        #[serde(rename = "type", borrow)]
        kind: &'a str,
        #[serde(borrow)]
        payload: Payload<'a>,
    }
    #[derive(Deserialize)]
    struct Payload<'a> {
        #[serde(rename = "type", default, borrow)]
        kind: Option<&'a str>,
        #[serde(default, borrow)]
        turn_id: Option<&'a str>,
    }
    for line in bytes.split(|b| *b == b'\n').rev().filter(|s| !s.is_empty()) {
        let Ok(record) = serde_json::from_slice::<Record<'_>>(line) else {
            return Ok(None);
        };
        if record.kind == "turn_context"
            || (record.kind == "event_msg" && record.payload.kind == Some("task_started"))
        {
            return Ok(record
                .payload
                .turn_id
                .filter(|turn| !turn.is_empty() && turn.len() <= 1024)
                .map(str::to_owned));
        }
    }
    Ok(None)
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
/// An extra idle guard, including before the first hook. No keys are synthesized.
pub fn empty_composer(term: &crate::terminal::Terminal) -> bool {
    composer_block(term).is_none()
}
/// Return the failed predicate so a waiting message never claims that a
/// completed answer is necessarily a draft or an unanswered approval.
pub fn composer_block(term: &crate::terminal::Terminal) -> Option<&'static str> {
    if term.is_alternate() {
        return Some("Native alternate screen is open; waiting for the main composer.");
    }
    let y = term.grid.y;
    if !term.cursor
        || term.grid.x != 2
        || y < term.grid.rows.saturating_sub(7)
        || y >= term.grid.rows.saturating_sub(2)
    {
        return Some("Native cursor is outside the recognized idle composer.");
    }
    let row_cells = &term.grid.cells[y * term.grid.cols..(y + 1) * term.grid.cols];
    if !matches!(row_cells[0].text.as_str(), "›" | "❯") {
        return Some("Native prompt shape is unrecognized; no input sent.");
    }
    let cells = &row_cells[2..];
    let placeholder_end = cells
        .iter()
        .rposition(|c| !c.text.trim().is_empty() && c.style.dim);
    // A decorative particle can replace the separator or abut the placeholder.
    // Never accept literal Braille in an otherwise unstyled/empty composer.
    if !row_cells[1].text.trim().is_empty()
        && !(placeholder_end.is_some() && single_braille_dot(&row_cells[1].text))
    {
        return Some("Native prompt separator is unrecognized; no input sent.");
    }
    for row in term.grid.rows.saturating_sub(14)..term.grid.rows {
        let line = term.grid.line(row).to_lowercase();
        let line = line.trim();
        if line == "esc to interrupt"
            || (line.contains("esc to interrupt") && line.starts_with(['•', '◦']))
        {
            return Some("Native activity indicator is visible; waiting for idle.");
        }
        // Match dialog headings/controls, not words anywhere in finished prose.
        // PermissionRequest lifecycle observations independently block delivery.
        if line == "permission"
            || [
                "approve command?",
                "native permission:",
                "trust this repository",
                "do you trust ",
                "would you like to run ",
                "would you like to make ",
                "review hooks",
                "review hook:",
                "allow this ",
            ]
            .iter()
            .any(|heading| line.starts_with(heading))
        {
            return Some("Native permission or trust dialog is visible; human attention required.");
        }
        if row != y && (line.starts_with('›') || line.starts_with('❯')) {
            return Some(
                "Another native prompt or selection is visible; waiting for the composer.",
            );
        }
    }
    if !cells.iter().enumerate().all(|(i, c)| {
        c.text.trim().is_empty()
            || c.style.dim
            || (placeholder_end.is_some_and(|end| i > end) && single_braille_dot(&c.text))
    }) {
        return Some("Native composer contains a draft or unrecognized text; draft preserved.");
    }
    // A multiline draft can have an empty first line with the cursor at column two.
    for row in y + 1..term.grid.rows.saturating_sub(1) {
        if term.grid.cells[row * term.grid.cols..(row + 1) * term.grid.cols]
            .iter()
            .any(|c| {
                !c.text.trim().is_empty()
                    && !(placeholder_end.is_some() && single_braille_dot(&c.text))
            })
        {
            return Some("Native composer has a nonempty continuation; draft or dialog preserved.");
        }
    }
    None
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
    fn recorded_turn_uses_only_latest_native_boundary() {
        let read = |text: &str| recorded_turn(io::Cursor::new(text.as_bytes())).unwrap();
        let first = "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"first\"}}\n";
        let next = "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"next\"}}\n";
        assert_eq!(read(first).as_deref(), Some("first"));
        assert_eq!(read(&format!("{first}{next}")).as_deref(), Some("next"));
        let output = serde_json::json!({"type":"response_item","payload":{
            "type":"function_call_output","output":next,"turn_id":"forged"}});
        assert_eq!(
            read(&format!("{first}{output}\n")).as_deref(),
            Some("first")
        );
        assert_eq!(read(&format!("{output}\n")), None);
        assert_eq!(read(&format!("{first}{{\"type\":\"event_msg\"")), None);
        assert_eq!(read(&format!("{first}invalid\n")), None);
        assert_eq!(
            read(&format!(
                "{first}{{\"type\":\"turn_context\",\"payload\":{{}}}}\n"
            )),
            None
        );
        assert_eq!(
            read(&format!(
                "{first}{{\"type\":\"turn_context\",\"payload\":{{\"turn_id\":\"\"}}}}\n"
            )),
            None
        );
        let long =
            serde_json::json!({"type":"turn_context","payload":{"turn_id":"x".repeat(1025)}});
        assert_eq!(read(&format!("{first}{long}\n")), None);
        let padding = "x".repeat(TURN_SCAN_LIMIT as usize);
        let big = serde_json::json!({"type":"response_item","payload":{"output":padding}});
        // Missing evidence is not guessed, and a truncated leading record is skipped.
        assert_eq!(read(&format!("{first}{big}\n")), None);
        assert_eq!(read(&format!("{big}\n{next}")).as_deref(), Some("next"));
    }

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
    fn completed_prose_and_placeholder_edge_particles_do_not_hide_idle() {
        let mut t = Terminal::new(80, 24);
        t.feed("\x1b[21;1H› \x1b[2mAsk Codex to do anything\x1b[0m\x1b[21;3H".as_bytes());
        for prose in [
            "The operation requires permission.",
            "This is a trusted source.",
            "I reviewed the approval and allow rules.",
            "The docs mention esc to interrupt.",
        ] {
            let mut completed = t.clone();
            completed.feed(format!("\x1b[16;1H{prose}\x1b[21;3H").as_bytes());
            assert!(
                empty_composer(&completed),
                "{prose}: {:?}",
                composer_block(&completed)
            );
        }
        for column in [2, 26, 30] {
            let mut animated = t.clone();
            animated.feed(format!("\x1b[21;{column}H⠁\x1b[21;3H").as_bytes());
            assert!(empty_composer(&animated), "column {column}");
        }
        let mut inside = t.clone();
        inside.feed("\x1b[21;8H⠁\x1b[21;3H".as_bytes());
        assert!(!empty_composer(&inside));
        let mut bare = t.clone();
        bare.feed("\x1b[21;3H\x1b[K\x1b[21;2H⠁\x1b[21;3H".as_bytes());
        assert!(!empty_composer(&bare));
        for dialog in [
            "Do you trust the contents of this directory?",
            "Would you like to run the following command?",
            "• Working (4s • esc to interrupt)",
        ] {
            let mut blocked = t.clone();
            blocked.feed(format!("\x1b[19;1H{dialog}\x1b[21;3H").as_bytes());
            assert!(!empty_composer(&blocked), "{dialog}");
        }
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
