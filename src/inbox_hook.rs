//! Codex command-hook adapter. Bodies, tool output and transcript content never enter a notice.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    path::Path,
};
#[derive(Default, Serialize, Deserialize)]
pub struct Input {
    pub hook_event_name: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turn_id: String,
    #[serde(default)]
    pub transcript_path: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub tool_name: String,
    #[serde(default)]
    pub stop_hook_active: bool,
}
pub const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
    "Interrupt",
    "PreCompact",
    "PostCompact",
    "SessionEnd",
];
pub fn flags(binary: &Path, state: &Path) -> Vec<String> {
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\"'\"'"));
    let command = serde_json::to_string(&format!(
        "{} --state {} _inbox-hook",
        quote(binary),
        quote(state)
    ))
    .unwrap();
    EVENTS
        .iter()
        .flat_map(|event| {
            // A notice uses two bounded socket round trips; Interrupt emits no notice.
            let timeout = if *event == "Interrupt" { 3 } else { 5 };
            [
                "-c".into(),
                format!(
                    "hooks.{event}=[{{hooks=[{{type=\"command\",command={command},timeout={timeout}}}]}}]"
                ),
            ]
        })
        .collect()
}
pub fn run(state: &Path, input: impl Read, mut output: impl Write) -> io::Result<()> {
    let mut data = Vec::new();
    input.take(1024 * 1024 + 1).read_to_end(&mut data)?;
    if data.len() > 1024 * 1024 {
        return Err(crate::wire::invalid("hook input exceeds bound"));
    }
    let event: Input = serde_json::from_slice(&data).map_err(io::Error::other)?;
    if !EVENTS.contains(&event.hook_event_name.as_str()) {
        return Err(crate::wire::invalid("unsupported hook event"));
    }
    if !event.agent_id.is_empty() {
        return writeln!(output, "{{}}");
    }
    let session = std::env::var("FLERE_SESSION").map_err(io::Error::other)?;
    let run = std::env::var("FLERE_RUN").map_err(io::Error::other)?;
    let bytes = crate::wire::request(
        state,
        &[
            "inbox-hook",
            &session,
            &run,
            &crate::wire::hex(&serde_json::to_vec(&event).map_err(io::Error::other)?),
        ],
    )?;
    let result: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let response = result.get("output").cloned().unwrap_or(json!({}));
    writeln!(output, "{response}")?;
    output.flush()?;
    if let Some(lease) = result["lease"].as_str() {
        crate::wire::request(state, &["confirm-notice", &session, &run, lease])?;
    }
    Ok(())
}
