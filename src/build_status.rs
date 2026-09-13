//! Read-only build diagnostics. Identity comes from each process's embedded
//! metadata, never the contents of its executable path after replacement.
use crate::{build_info, os, wire};
use serde_json::{Value, json};
use std::{io, path::Path};

fn embedded() -> Value {
    serde_json::from_str(build_info::json()).expect("build script emits valid build metadata")
}

pub(crate) fn supervisor(epoch: &str, generation: u64) -> io::Result<Vec<u8>> {
    serde_json::to_vec(&json!({
        "schema_version": 1,
        "build": embedded(),
        "pid": std::process::id(),
        "epoch": epoch,
        "generation": generation,
    }))
    .map_err(io::Error::other)
}

fn valid_response(value: &Value) -> bool {
    let build = &value["build"];
    value["schema_version"] == 1
        && value["pid"]
            .as_u64()
            .is_some_and(|pid| pid > 0 && pid <= u32::MAX as u64)
        && value["generation"].as_u64().is_some()
        && value["epoch"].as_str().is_some_and(|epoch| {
            epoch.len() == 32 && epoch.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        && build["schema_version"] == 1
        && build["component"] == "flere"
        && ["package_version", "build_id", "target", "profile", "rustc"]
            .iter()
            .all(|field| build[field].as_str().is_some_and(|text| !text.is_empty()))
        && build["compatibility"].is_object()
}

/// Observe only this selected state. This command never creates state, starts a
/// supervisor, refreshes it, or treats a matching supervisor as an updated UI.
pub fn report(state: &Path) -> io::Result<Vec<u8>> {
    let executable = embedded();
    let installation = match crate::install::Store::for_user("flere") {
        Ok(store) => match store.status() {
            Ok(Some(receipt)) => json!({"status": "managed", "receipt": receipt}),
            Ok(None) => json!({"status": "untracked"}),
            Err(error) => json!({"status": "unknown", "reason": wire::passive(&error.to_string())}),
        },
        Err(_) => json!({"status": "untracked"}),
    };
    let mut comparison = "unknown";
    let supervisor = match wire::request(state, &["build-info"]) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(mut value) if bytes.len() <= 65536 && valid_response(&value) => {
                comparison = if value["build"] == executable {
                    "same_build"
                } else {
                    "different_build"
                };
                value["status"] = json!("known");
                value
            }
            _ => json!({
                "status": "unknown_build",
                "reason": "Unsupported or invalid supervisor build metadata",
            }),
        },
        Err(error) if error.to_string() == "unknown command" => json!({
            "status": "unknown_build",
            "reason": "This supervisor does not expose build identity",
        }),
        Err(error) => json!({
            "status": "unreachable",
            "reason": wire::passive(&error.to_string()),
        }),
    };
    let frontends = if supervisor["status"] == "known" {
        match wire::request(state, &["frontends"]) {
            Ok(bytes) if bytes.len() <= 65536 => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value)
                    if value["schema_version"] == 1
                        && value["status"] == "known"
                        && value["tracked"].is_array()
                        && value["untracked"].as_u64().is_some() =>
                {
                    value
                }
                _ => json!({"status": "unknown", "reason": "Unsupported frontend inventory"}),
            },
            Ok(_) => json!({"status": "unknown", "reason": "Frontend inventory exceeds bound"}),
            Err(_) => json!({"status": "untracked"}),
        }
    } else {
        json!({"status": "untracked"})
    };
    serde_json::to_vec_pretty(&json!({
        "schema_version": 1,
        "state": state.to_string_lossy(),
        "executable": {
            "path": os::executable_path()?.to_string_lossy(),
            "build": executable,
        },
        "supervisor": supervisor,
        "comparison": comparison,
        "comparison_basis": "embedded_metadata",
        "installation": installation,
        "frontends": frontends,
    }))
    .map_err(io::Error::other)
}
