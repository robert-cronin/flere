//! Close transactions never turn a clean observation into a forceful signal.
use super::*;
use crate::close::{self as integration, Reply};

pub(super) struct Pending {
    workspace: u64,
    id: u64,
    run: String,
    shell_run: String,
    token: String,
    root: u32,
    pid: u32,
    ownership: Vec<(u32, u32, String, PathBuf)>,
    phase: &'static str,
    deadline: Instant,
    reason: Option<String>,
    closed: bool,
}

impl Pending {
    fn response(&self) -> serde_json::Value {
        serde_json::json!({
            "token": self.token,
            "status": if self.closed { "closed" } else if self.reason.is_some() { "confirm" } else { "pending" },
            "reason": self.reason.as_deref().unwrap_or("")
        })
    }
    fn finish(&mut self, state: &Path, reason: String) {
        let _ = fs::remove_file(integration::directory(state, &self.shell_run).join("request"));
        self.reason = Some(reason);
        self.deadline = Instant::now() + Duration::from_secs(10);
    }
}

fn jobs_are_helpers(pid: u32, reply: &Reply) -> io::Result<bool> {
    let mut children = os::process_children(pid, 128)?;
    children.sort_unstable();
    let mut reported = reply.processes.clone();
    reported.sort_unstable();
    if children != reported || reply.services.len() > 64 {
        return Ok(false);
    }
    let mut services = reply.services.clone();
    for child in children {
        let identity = os::child_identity(child)?;
        if identity.0 != pid {
            return Ok(false);
        }
        let args = os::process_arguments(child)?;
        let args: Vec<_> = args
            .strip_suffix(&[0])
            .unwrap_or(&args)
            .split(|&b| b == 0)
            .collect();
        let matches = services.iter().position(|cmd| {
            cmd.len() == args.len()
                && !cmd.is_empty()
                && cmd.len() <= 64
                && cmd
                    .iter()
                    .zip(&args)
                    .all(|(expected, actual)| expected.as_bytes() == *actual)
        });
        let Some(index) = matches else {
            return Ok(false);
        };
        services.remove(index);
        if os::child_identity(child)? != identity {
            return Ok(false);
        }
    }
    Ok(true)
}

fn ownership(root: u32, pid: u32) -> io::Result<Vec<(u32, u32, String, PathBuf)>> {
    let mut chain = Vec::new();
    let mut next = pid;
    // Neovim's TUI may own a separate editor child. Require the exact ancestry
    // and reject siblings rather than trusting a PID read from a status file.
    for _ in 0..8 {
        let (parent, start) = os::child_identity(next)?;
        chain.push((next, parent, start, os::process_executable(next)?));
        if next == root {
            if parent != std::process::id() {
                break;
            }
            return Ok(chain);
        }
        if os::process_children(parent, 128)? != [next] {
            break;
        }
        next = parent;
    }
    Err(invalid("Close cancelled: program ownership is unknown"))
}

impl Server {
    pub(super) fn close_input(&mut self, id: u64, run: &str) {
        for p in &mut self.close_pending {
            if p.id == id && p.run == run && p.reason.is_none() && !p.closed {
                p.finish(
                    &self.state,
                    "Terminal activity changed while closing".into(),
                );
            }
        }
    }
    pub(super) fn close_busy(&self) -> bool {
        self.close_pending
            .iter()
            .any(|p| p.reason.is_none() && !p.closed)
    }
    pub(super) fn close_check(
        &mut self,
        epoch: &str,
        wid: u64,
        id: u64,
        run: &str,
    ) -> io::Result<Vec<u8>> {
        if epoch != self.epoch {
            return Err(invalid("Close cancelled: workbench changed"));
        }
        if let Some(p) = self
            .close_pending
            .iter()
            .find(|p| p.workspace == wid && p.id == id && p.run == run)
        {
            return serde_json::to_vec(&p.response()).map_err(io::Error::other);
        }
        let tab = self
            .workspaces
            .iter()
            .find(|w| w.id == wid)
            .and_then(|w| w.tabs.iter().find(|t| t.id == id && t.run == run))
            .ok_or_else(|| invalid("Close cancelled: terminal changed"))?;
        if tab.ended && !tab.alive && self.tasks.contains(run) {
            self.tasks.remove(run);
            self.restoration.closed.insert(run.into());
            self.changed();
            return serde_json::to_vec(&serde_json::json!({"status":"closed","reason":""}))
                .map_err(io::Error::other);
        }
        let warning = if tab.native.is_some() || tab.kind == "agent" {
            Some("A chat session is open")
        } else if !tab.input.is_empty() {
            Some("Terminal input is still pending")
        } else if !matches!(tab.kind.as_str(), "shell" | "editor") {
            Some("This application's state cannot be verified")
        } else {
            None
        };
        if let Some(reason) = warning {
            return serde_json::to_vec(&serde_json::json!({"status":"confirm", "reason":reason}))
                .map_err(io::Error::other);
        }
        let root = tab.child.id();
        let shell_run = tab.shell_identity().to_string();
        let dir = integration::directory(&self.state, &shell_run);
        let ready = integration::read(&dir.join("ready"))
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<u32>().ok());
        let Some(pid) = ready else {
            return serde_json::to_vec(&serde_json::json!({"status":"confirm", "reason":"This tab cannot verify that it is safe to close"})).map_err(io::Error::other);
        };
        if self.close_pending.len() >= 32 {
            return Err(invalid("Another close check is still pending"));
        }
        let ownership = ownership(root, pid)?;
        let token = os::nonce()?;
        integration::request(&dir, &token, "probe", &[])?;
        let pending = Pending {
            workspace: wid,
            id,
            run: run.into(),
            shell_run,
            token,
            root,
            pid,
            ownership,
            phase: "probe",
            deadline: Instant::now() + Duration::from_millis(900),
            reason: None,
            closed: false,
        };
        let result = serde_json::to_vec(&pending.response()).map_err(io::Error::other)?;
        self.audit("close-check", id, run)?;
        self.close_pending.push(pending);
        Ok(result)
    }

    pub(super) fn close_poll(
        &mut self,
        epoch: &str,
        wid: u64,
        id: u64,
        run: &str,
        token: &str,
    ) -> io::Result<Vec<u8>> {
        if epoch != self.epoch {
            return Err(invalid("Close cancelled: workbench changed"));
        }
        self.close_tick();
        let p = self
            .close_pending
            .iter()
            .find(|p| p.workspace == wid && p.id == id && p.run == run && p.token == token)
            .ok_or_else(|| invalid("Close check expired; retry closing the tab"))?;
        serde_json::to_vec(&p.response()).map_err(io::Error::other)
    }

    pub(super) fn close_cancel(
        &mut self,
        epoch: &str,
        wid: u64,
        id: u64,
        run: &str,
        token: &str,
    ) -> io::Result<Vec<u8>> {
        if epoch != self.epoch {
            return Err(invalid("Close cancelled: workbench changed"));
        }
        if let Some(index) = self
            .close_pending
            .iter()
            .position(|p| p.workspace == wid && p.id == id && p.run == run && p.token == token)
        {
            let p = self.close_pending.remove(index);
            let _ =
                fs::remove_file(integration::directory(&self.state, &p.shell_run).join("request"));
        }
        Ok(b"cancelled".to_vec())
    }

    pub(super) fn close_tick(&mut self) {
        for p in &mut self.close_pending {
            if p.reason.is_some() || p.closed {
                continue;
            }
            let tab = self
                .workspaces
                .iter()
                .find(|w| w.id == p.workspace)
                .and_then(|w| w.tabs.iter().find(|t| t.id == p.id && t.run == p.run));
            if tab.is_none() || tab.is_some_and(|t| t.ended && !t.alive) {
                p.closed = true;
                p.finish(&self.state, String::new());
                continue;
            }
            let tab = tab.unwrap();
            if Instant::now() >= p.deadline {
                p.finish(
                    &self.state,
                    "The program did not confirm a safe exit".into(),
                );
                continue;
            }
            if !tab.input.is_empty() || tab.child.id() != p.root || tab.native.is_some() {
                p.finish(
                    &self.state,
                    "Terminal activity changed while closing".into(),
                );
                continue;
            }
            let dir = integration::directory(&self.state, &p.shell_run);
            let Some(reply) = integration::read(&dir.join("reply"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Reply>(&bytes).ok())
                .filter(|r| r.token == p.token && r.phase == p.phase && r.pid == p.pid)
            else {
                continue;
            };
            if !reply.reason.is_empty() {
                p.finish(
                    &self.state,
                    wire::passive(&reply.reason).chars().take(160).collect(),
                );
                continue;
            }
            if p.phase == "commit" {
                continue;
            } // Only process exit completes a clean close.
            let verified = (|| -> io::Result<bool> {
                Ok(ownership(p.root, p.pid)? == p.ownership && jobs_are_helpers(p.pid, &reply)?)
            })()
            .unwrap_or(false);
            if !verified {
                p.finish(
                    &self.state,
                    "A command or background job is still running".into(),
                );
                continue;
            }
            if integration::request(&dir, &p.token, "commit", &reply.processes).is_err() {
                p.finish(
                    &self.state,
                    "The program could not receive its close request".into(),
                );
                continue;
            }
            p.phase = "commit";
            p.deadline = Instant::now() + Duration::from_millis(900);
        }
        self.close_pending
            .retain(|p| p.reason.is_none() || Instant::now() < p.deadline);
    }
}
