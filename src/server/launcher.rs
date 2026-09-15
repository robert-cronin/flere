//! Adopt only an explicit foreground launcher inside an owned shell's process tree.
use super::*;

impl Session {
    pub(super) fn shell_identity(&self) -> &str {
        if self.shell_run.is_empty() {
            &self.run
        } else {
            &self.shell_run
        }
    }
    fn owns_launcher(&self, pid: u32, start: &str) -> io::Result<()> {
        if !self.alive
            || self.ended
            || self.native.is_some()
            || self.kind != "shell"
            || os::child_identity(pid)?.1 != start
            || !os::foreground_process(self.master.as_raw_fd(), pid)?
        {
            return Err(invalid("Codex must start in a live foreground Flere shell"));
        }
        let root = self.child.id();
        let mut next = pid;
        for _ in 0..32 {
            let (parent, _) = os::child_identity(next)?;
            if next == root && parent == std::process::id() {
                if os::child_identity(pid)?.1 == start {
                    return Ok(());
                }
                break;
            }
            if parent == 0 || parent == next {
                break;
            }
            next = parent;
        }
        Err(invalid("shell launcher process ownership changed"))
    }
}

impl Server {
    pub(super) fn shell_context(
        &self,
        shell_run: &str,
        pid: u32,
        start: &str,
    ) -> io::Result<Vec<u8>> {
        let tab = self
            .workspaces
            .iter()
            .filter(|w| !w.meta.archived && w.meta.operation.is_empty())
            .flat_map(|w| &w.tabs)
            .find(|t| t.shell_identity() == shell_run)
            .ok_or_else(|| invalid("shell integration expired; open a new Flere terminal"))?;
        tab.owns_launcher(pid, start)?;
        serde_json::to_vec(&serde_json::json!({"epoch":self.epoch,"session":tab.id,"run":tab.run}))
            .map_err(io::Error::other)
    }
    pub(super) fn shell_native(&mut self, fields: &[&str]) -> io::Result<Vec<u8>> {
        let [_, epoch, session, run, shell_run, pid, start, argv] = fields else {
            return Err(invalid("invalid shell launch request"));
        };
        if *epoch != self.epoch {
            return Err(invalid("shell workspace epoch changed"));
        }
        let id = session.parse::<u64>().map_err(io::Error::other)?;
        let pid = pid.parse::<u32>().map_err(io::Error::other)?;
        self.shell_context(shell_run, pid, start)?;
        let t = self.session(id, run)?;
        if t.shell_identity() != *shell_run {
            return Err(invalid("shell identity changed"));
        }
        t.owns_launcher(pid, start)?;
        if !t.input.is_empty() {
            return Err(invalid("shell input is still pending; retry Codex"));
        }
        let mut argv: Vec<String> =
            serde_json::from_str(&wire::text(argv)?).map_err(io::Error::other)?;
        if argv.is_empty()
            || argv.len() > 4096
            || !Path::new(&argv[0]).is_absolute()
            || argv.iter().any(|s| s.contains('\0'))
        {
            return Err(invalid("invalid shell native arguments"));
        }
        let cwd = crate::native::launcher::working_directory(&argv[1..], &os::process_cwd(pid)?)?;
        let flags = crate::native::codex_flags(&self.state)?;
        argv.splice(1..1, flags);
        let new_run = os::nonce()?;
        let spec = crate::native::HostSpec {
            id,
            run: new_run.clone(),
            harness: "codex".into(),
            argv,
            cwd,
            shell: os::shell(),
            conversation: String::new(),
            dispatch: String::new(),
            launcher: Some((pid, start.to_string())),
        };
        crate::native::write_spec(&self.state, &spec)?;
        self.audit("shell-native-start", id, &new_run)?;
        // Recheck immediately before replacing the native authority for this PTY.
        self.session(id, run)?.owns_launcher(pid, start)?;
        self.close_input(id, run);
        if let Some(value) = self.restoration.order.remove(*run) {
            self.restoration.order.insert(new_run.clone(), value);
        }
        if let Some(value) = self.restoration.cwd.remove(*run) {
            self.restoration.cwd.insert(new_run.clone(), value);
        }
        if let Some(value) = self.restoration.owners.remove(*run) {
            self.restoration.owners.insert(new_run.clone(), value);
        }
        let t = self.session(id, run)?;
        t.shell_run = shell_run.to_string();
        t.run = new_run;
        t.kind = "agent".into();
        t.title = "codex".into();
        t.native = Some(spec.clone());
        t.working = false;
        let w = self
            .workspaces
            .iter_mut()
            .find(|w| w.tabs.iter().any(|t| t.id == id))
            .unwrap();
        w.meta.last_conversation = Some(crate::native::Conversation {
            harness: spec.harness.clone(),
            uuid: String::new(),
            cwd: spec.cwd.clone(),
        });
        self.changed();
        serde_json::to_vec(&spec).map_err(io::Error::other)
    }

    pub(super) fn remember_native_selection(&mut self) -> io::Result<()> {
        let Some(w) = self.workspaces.iter_mut().find(|w| w.id == self.active) else {
            return Ok(());
        };
        let Some(spec) = w
            .tabs
            .iter()
            .find(|t| t.id == w.selected)
            .and_then(|t| t.native.as_ref())
        else {
            return Ok(());
        };
        if !spec.conversation.is_empty() && !crate::native::valid_uuid(&spec.conversation) {
            return Ok(());
        }
        let saved = crate::native::Conversation {
            harness: spec.harness.clone(),
            uuid: spec.conversation.clone(),
            cwd: spec.cwd.clone(),
        };
        if (saved.uuid.is_empty() || w.meta.conversations.contains(&saved))
            && w.meta.last_conversation.as_ref() != Some(&saved)
        {
            w.meta.last_conversation = Some(saved);
            self.persist()?;
            self.changed();
        }
        Ok(())
    }
}
