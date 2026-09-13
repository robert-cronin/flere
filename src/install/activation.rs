use super::{BuildMetadata, InstalledPackage, MAX_JSON};
use crate::{
    model::Snapshot,
    wire::{self, invalid},
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct SessionIdentity {
    pub workspace: u64,
    pub tab: u64,
    pub run: String,
    pub pid: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeIdentity {
    pub state: PathBuf,
    pub epoch: String,
    pub pid: u32,
    pub build: BuildMetadata,
    pub sessions: Vec<SessionIdentity>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub status: String,
    pub supervisor: String,
    pub initiating_frontend: String,
    pub other_frontends: String,
    pub detail: String,
    pub before: Option<RuntimeIdentity>,
    pub after: Option<RuntimeIdentity>,
    #[serde(default)]
    pub can_restore_installation: bool,
}
impl Activation {
    pub fn not_running() -> Self {
        Self {
            status: "installed".into(),
            supervisor: "not_running".into(),
            initiating_frontend: "not_requested".into(),
            other_frontends: "untracked".into(),
            detail: "Installed; no supervisor was started".into(),
            before: None,
            after: None,
            can_restore_installation: false,
        }
    }
    pub fn frontend_applied(mut self) -> Self {
        if self.supervisor == "applied" {
            self.initiating_frontend = "applied".into();
            self.status = "applied".into();
            self.detail = "Installed; supervisor and initiating frontend applied; other attachments are reported separately".into();
        }
        self
    }
}

pub fn runtime_identity(state: &Path) -> io::Result<RuntimeIdentity> {
    let bytes = wire::request(state, &["build-info"])?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(invalid("runtime build metadata exceeds bound"));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let build: BuildMetadata =
        serde_json::from_value(value["build"].clone()).map_err(io::Error::other)?;
    build.validate()?;
    let epoch = value["epoch"]
        .as_str()
        .filter(|e| e.len() == 32 && e.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("invalid runtime epoch"))?;
    let pid = value["pid"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= u32::MAX as u64)
        .ok_or_else(|| invalid("invalid runtime PID"))? as u32;
    let snapshot = match wire::request(state, &["snapshot-panes"]) {
        Ok(bytes) => Snapshot::decode(&bytes)?,
        Err(error) if error.to_string() == "unknown command" => {
            Snapshot::decode(&wire::request(state, &["snapshot"])?)?
        }
        Err(error) => return Err(error),
    };
    if snapshot.epoch != epoch {
        return Err(invalid("runtime changed during update discovery"));
    }
    let mut sessions = Vec::new();
    for workspace in snapshot.workspaces {
        for tab in workspace.tabs {
            if sessions.len() >= 8192
                || tab.run.len() != 32
                || !tab.run.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid(
                    "update identity receipt supports at most 8192 exact session runs; no installation was attempted",
                ));
            }
            sessions.push(SessionIdentity {
                workspace: workspace.id,
                tab: tab.id,
                run: tab.run,
                pid: tab.pid,
            });
        }
    }
    sessions.sort();
    Ok(RuntimeIdentity {
        state: state.into(),
        epoch: epoch.into(),
        pid,
        build,
        sessions,
    })
}

/// A malformed request is rejected before mutation/checkpointing on supporting
/// supervisors. Older runtimes cannot silently fall back to an unguarded refresh.
pub fn ensure_update_endpoint(state: &Path) -> io::Result<()> {
    match wire::request(state, &["refresh-epoch"]) {
        Err(error) if error.to_string() == "stale supervisor update epoch" => Ok(()),
        Err(error) if error.to_string() == "unknown command" => Err(invalid(
            "running supervisor predates exact update activation; use its explicit session-preserving refresh once before managed updates; no installation was attempted",
        )),
        _ => Err(invalid(
            "could not verify exact supervisor update capability; no installation was attempted",
        )),
    }
}

/// Apply only the exact observed supervisor. A reply accepting refresh is not a
/// success receipt. No failure path kills/recreates sessions or guesses a rollback.
pub fn activate(
    state: &Path,
    package: &InstalledPackage,
    expected: &RuntimeIdentity,
) -> Activation {
    let mut result = Activation {
        status: "partial".into(),
        supervisor: "not_applied".into(),
        initiating_frontend: "pending".into(),
        other_frontends: "untracked".into(),
        detail: String::new(),
        before: Some(expected.clone()),
        after: None,
        can_restore_installation: false,
    };
    let mut request_started = false;
    let mut definitely_rejected = false;
    let apply = (|| -> io::Result<RuntimeIdentity> {
        if expected.state != state || !package.manifest.build.accepts_runtime(&expected.build) {
            return Err(invalid(
                "candidate does not accept the selected runtime handoff/state",
            ));
        }
        let before = runtime_identity(state)?;
        if before.epoch != expected.epoch
            || before.pid != expected.pid
            || before.build != expected.build
            || before.sessions != expected.sessions
        {
            return Err(invalid(
                "runtime/session ownership changed after update discovery; explicit retry required",
            ));
        }
        package.manifest.verify(
            package
                .executable
                .parent()
                .ok_or_else(|| invalid("candidate has no package directory"))?,
        )?;
        if before.build == package.manifest.build {
            // Stamps may be shared by distinct payloads; the only honest no-op is
            // embedded-metadata equality, not proof of running payload equality.
            // Still perform explicit refresh so the requested payload is executed.
        }
        request_started = true;
        let request = wire::request(
            state,
            &[
                "refresh-epoch",
                &expected.epoch,
                &wire::hex(package.executable.as_os_str().as_encoded_bytes()),
            ],
        );
        if let Err(error) = &request {
            definitely_rejected = error.kind() == io::ErrorKind::Other;
        }
        request?;
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match runtime_identity(state) {
                Ok(after) => {
                    if after.epoch != expected.epoch || after.pid != expected.pid {
                        return Err(invalid("supervisor identity changed during update"));
                    }
                    if after.build == package.manifest.build {
                        if after.sessions != expected.sessions {
                            return Err(invalid(
                                "runtime session set changed during update; continuity could not be verified",
                            ));
                        }
                        // Generation changes on any output. Actual refresh notice
                        // distinguishes a same-stamp refresh from its predecessor.
                        let notice = wire::request(state, &["refresh-status"])?;
                        if String::from_utf8_lossy(&notice).starts_with("Refreshed Flere;") {
                            return Ok(after);
                        }
                    }
                    if let Ok(notice) = wire::request(state, &["refresh-status"])
                        && String::from_utf8_lossy(&notice).starts_with("Refresh failed:")
                    {
                        definitely_rejected = true;
                        return Err(io::Error::other(wire::passive(&String::from_utf8_lossy(
                            &notice,
                        ))));
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::InvalidData | io::ErrorKind::PermissionDenied
                    ) =>
                {
                    return Err(error);
                }
                Err(_) => {}
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "activation outcome is unknown; installed package retained and sessions were not restarted",
                ));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    })();
    match apply {
        Ok(after) => {
            result.supervisor = "applied".into();
            result.after = Some(after);
            result.detail = "Supervisor applied with exact session identities preserved; initiating frontend still needs the selected executable".into();
        }
        Err(error) => {
            result.detail = wire::passive(&error.to_string());
            if (!request_started || definitely_rejected)
                && let Ok(actual) = runtime_identity(state)
                && actual.epoch == expected.epoch
                && actual.pid == expected.pid
                && actual.build == expected.build
                && actual.sessions == expected.sessions
            {
                result.supervisor = "rejected".into();
                result.can_restore_installation = true;
                result.after = Some(actual);
            }
        }
    }
    result
}
