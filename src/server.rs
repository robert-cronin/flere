//! One supervisor owns PTYs and multiplexes clients. Slow readers cannot block PTY draining.
use crate::{
    model::{Snapshot, TabView, WorkspaceView},
    os,
    terminal::Terminal,
    wire::{self, invalid},
    workspace::{CardMeta, atomic_write},
};
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    version: u32,
    #[serde(default)]
    active: u64,
    workspaces: Vec<SavedWorkspace>,
    #[serde(default)]
    coordination: Option<coordination::Coordination>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct SavedWorkspace {
    id: u64,
    name: String,
    cwd: PathBuf,
    #[serde(default)]
    meta: CardMeta,
    #[serde(default)]
    tabs: Vec<restore::Tab>,
    #[serde(default)]
    selected: u64,
    #[serde(default)]
    split: Option<crate::panes::PaneLayout>,
}
mod attachments;
mod chat_messages;
mod clients;
mod close;
mod context_view;
mod coordination;
mod delivery;
mod dispatch;
mod launcher;
mod panes;
mod projects;
mod refresh;
mod restore;
mod tasks;
mod terminal_input;
mod transfers;

struct Session {
    id: u64,
    run: String,
    shell_run: String,
    master: File,
    child: os::Process,
    term: Terminal,
    input: VecDeque<u8>,
    alive: bool,
    ended: bool,
    title: String,
    kind: String,
    path: String,
    native: Option<crate::native::HostSpec>,
    working: bool,
}
struct Workspace {
    id: u64,
    name: String,
    cwd: PathBuf,
    tabs: Vec<Session>,
    selected: u64,
    meta: CardMeta,
    split: Option<crate::panes::PaneLayout>,
}
struct Client {
    stream: UnixStream,
    input: Vec<u8>,
    output: Vec<u8>,
    offset: usize,
    watch: bool,
    panes: bool,
    links: bool,
    build: Option<serde_json::Value>,
    done: bool,
    deadline: Instant,
}
struct WatchRequest<'a> {
    panes: bool,
    links: bool,
    build: Option<&'a str>,
}
impl<'a> WatchRequest<'a> {
    fn parse(request: &'a str) -> Option<Self> {
        let (panes, links, build) = match request {
            "watch" => (false, false, None),
            "watch-panes" => (true, false, None),
            "watch-links" => (true, true, None),
            _ => {
                if let Some(build) = request.strip_prefix("watch-panes-build\t") {
                    (true, false, Some(build))
                } else {
                    let build = request.strip_prefix("watch-links-build\t")?;
                    (true, true, Some(build))
                }
            }
        };
        Some(Self {
            panes,
            links,
            build,
        })
    }
    fn snapshot(&self, snapshot: &Snapshot) -> Vec<u8> {
        if self.links {
            snapshot.encode_links()
        } else if self.panes {
            snapshot.encode_panes()
        } else {
            snapshot.encode()
        }
    }
}
struct WorktreeJob {
    workspace: u64,
    start_shell: bool,
    result: std::sync::mpsc::Receiver<io::Result<String>>,
}
struct Server {
    state: PathBuf,
    epoch: String,
    workspaces: Vec<Workspace>,
    active: u64,
    next: u64,
    cols: u16,
    rows: u16,
    generation: u64,
    dirty: bool,
    stop: bool,
    refresh: Option<PathBuf>,
    last_refresh: String,
    coordination: coordination::Coordination,
    audit: File,
    native_checked: Instant,
    restoration: restore::State,
    delivery_checked: Instant,
    delivery_cursor: usize,
    delivery_jobs: Vec<delivery::Job>,
    worktree_jobs: Vec<WorktreeJob>,
    attachment_tickets: Vec<attachments::Ticket>,
    terminal_tickets: Vec<terminal_input::Ticket>,
    close_pending: Vec<close::Pending>,
    file_transfers: transfers::Transfers,
    command_depth: u32,
    frontend_inventory: serde_json::Value,
    tasks: tasks::Tasks,
    cache_reservations: std::cell::RefCell<Vec<crate::editor::cache::Reservation>>,
    cache_cleaner: crate::editor::cache::Cleaner,
}
pub fn private_state(state: &Path) -> io::Result<()> {
    if state.as_os_str().len() > 85 {
        return Err(invalid(
            "state path too long for Unix socket; choose a shorter --state path",
        ));
    }
    if !state.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(state)?;
        fs::set_permissions(state, fs::Permissions::from_mode(0o700))?;
    }
    let m = fs::symlink_metadata(state)?;
    if !m.is_dir() || m.file_type().is_symlink() || m.uid() != os::uid() || m.mode() & 0o077 != 0 {
        return Err(invalid(
            "state must be an owned private directory (mode 0700), not a symlink",
        ));
    }
    Ok(())
}
fn secure_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}
impl Server {
    fn load(state: &Path) -> io::Result<Self> {
        let audit = OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(state.join("actions.log"))?;
        let mut s = Self {
            state: state.into(),
            epoch: os::nonce()?,
            workspaces: Vec::new(),
            active: 0,
            next: 1,
            cols: 80,
            rows: 24,
            generation: 0,
            dirty: true,
            stop: false,
            refresh: None,
            last_refresh: String::new(),
            file_transfers: Default::default(),
            command_depth: 0,
            frontend_inventory: serde_json::json!({"schema_version":1,"status":"known","tracked":[],"untracked":0}),
            coordination: coordination::Coordination::default(),
            audit,
            native_checked: Instant::now(),
            restoration: restore::State::default(),
            delivery_checked: Instant::now(),
            delivery_cursor: 0,
            delivery_jobs: Vec::new(),
            worktree_jobs: Vec::new(),
            attachment_tickets: Vec::new(),
            terminal_tickets: Vec::new(),
            close_pending: Vec::new(),
            tasks: Default::default(),
            cache_reservations: Default::default(),
            cache_cleaner: crate::editor::cache::Cleaner::new(state),
        };
        let load_store = || -> io::Result<String> {
            let f = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(state.join("workspaces.v1"))?;
            if !f.metadata()?.is_file() {
                return Err(invalid("workspace store must be a regular file"));
            }
            let mut data = String::new();
            f.take(1024 * 1024 + 1).read_to_string(&mut data)?;
            Ok(data)
        };
        let v2 = state.join("workspaces.v2.json");
        if v2.exists() {
            let f = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&v2)?;
            if !f.metadata()?.is_file() {
                return Err(invalid("workspace store must be a regular file"));
            }
            let mut bytes = Vec::new();
            f.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 8 * 1024 * 1024 {
                return Err(invalid("workspace metadata exceeds 8 MiB"));
            }
            let saved: Saved = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if !matches!(saved.version, 2..=10) {
                return Err(invalid("unsupported workspace store version"));
            }
            s.coordination = match saved.coordination {
                Some(c) => c,
                None => coordination::Coordination::load(state)?,
            };
            s.coordination.validate_dispatches()?;
            s.coordination.delivery.validate()?;
            s.coordination.validate_chat_messages()?;
            let saved_active = saved.active;
            let mut orders = std::collections::BTreeSet::new();
            for mut w in saved.workspaces {
                if w.tabs.len() > 512 {
                    return Err(invalid("too many saved tabs in workspace"));
                }
                s.restoration
                    .sequence
                    .insert(w.id, w.tabs.iter().map(|t| t.order).collect());
                if let Some(split) = w.split {
                    split.validate(&w.tabs.iter().map(|t| t.order).collect::<Vec<_>>())?;
                    s.restoration.panes.insert(w.id, split);
                }
                for tab in w.tabs {
                    tab.validate()?;
                    if !orders.insert(tab.order) {
                        return Err(invalid("duplicate saved tab identity"));
                    }
                    s.next = s.next.max(tab.order + 1);
                    s.restoration.pending.push(restore::Pending {
                        workspace: w.id,
                        tab,
                    });
                }
                s.restoration.selected.insert(w.id, w.selected);
                if w.meta.operation.starts_with("Preparing worktree") {
                    w.meta.operation="Interrupted worktree preparation; inspect this checkout before starting anything".into();
                }
                w.meta.validate()?;
                if w.id == 0 || w.id == u64::MAX || s.workspaces.iter().any(|v| v.id == w.id) {
                    return Err(invalid("invalid workspace identity"));
                }
                s.next = s.next.max(w.id + 1);
                s.workspaces.push(Workspace {
                    id: w.id,
                    name: w.name,
                    cwd: w.cwd,
                    meta: w.meta,
                    tabs: Vec::new(),
                    selected: 0,
                    split: None,
                });
            }
            s.active = s
                .workspaces
                .iter()
                .find(|w| !w.meta.archived)
                .map_or(0, |w| w.id);
            if s.workspaces
                .iter()
                .any(|w| w.id == saved_active && !w.meta.archived)
            {
                s.active = saved_active;
            }
            return Ok(s);
        }
        match load_store() {
            Ok(data) => {
                if data.len() > 1024 * 1024 {
                    return Err(invalid("workspace store exceeds bound"));
                }
                let mut lines = data.lines();
                if lines.next() != Some("flere-workspaces-v1") {
                    return Err(invalid("unknown workspace store version"));
                }
                for line in lines {
                    let p: Vec<_> = line.split('\t').collect();
                    if p.len() != 3 {
                        return Err(invalid("malformed workspace store"));
                    }
                    let id = p[0].parse::<u64>().map_err(io::Error::other)?;
                    if id == 0 || id == u64::MAX || s.workspaces.iter().any(|w| w.id == id) {
                        return Err(invalid("invalid workspace identity"));
                    }
                    s.next = s.next.max(id + 1);
                    s.workspaces.push(Workspace {
                        id,
                        name: wire::text(p[1])?,
                        cwd: PathBuf::from(wire::text(p[2])?),
                        tabs: Vec::new(),
                        selected: 0,
                        meta: CardMeta::default(),
                        split: None,
                    });
                }
                s.active = s.workspaces.first().map_or(0, |w| w.id);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        s.coordination = coordination::Coordination::load(state)?;
        Ok(s)
    }
    fn persist(&self) -> io::Result<()> {
        let saved = Saved {
            version: 10,
            active: self.active,
            coordination: Some(self.coordination.clone()),
            workspaces: self
                .workspaces
                .iter()
                .map(|w| SavedWorkspace {
                    id: w.id,
                    name: w.name.clone(),
                    cwd: w.cwd.clone(),
                    meta: w.meta.clone(),
                    tabs: self.saved_tabs(w),
                    selected: self.saved_selection(w),
                    split: self.saved_split(w),
                })
                .collect(),
        };
        // Exited/closing editors may still own a live descendant PTY while
        // intentionally disappearing from the cold-reopen layout. Protect such
        // retained Sessions before ANY writer can drop their last saved path.
        // Interior mutation is restricted to this single-threaded cache ledger.
        let saved_paths: std::collections::BTreeSet<_> = saved
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.kind == "editor")
            .map(|t| t.path.as_str())
            .collect();
        for path in self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.kind == "editor" && !saved_paths.contains(t.path.as_str()))
            .map(|t| Path::new(&t.path))
        {
            if !self
                .cache_reservations
                .borrow()
                .iter()
                .any(|pin| pin.protects(path))
                && let Some(pin) = crate::editor::cache::reserve_open(&self.state, path)?
            {
                self.cache_reservations.borrow_mut().push(pin);
            }
        }
        let data = serde_json::to_vec(&saved).map_err(io::Error::other)?;
        if data.len() > 8 * 1024 * 1024 {
            return Err(invalid("workspace metadata exceeds 8 MiB"));
        }
        atomic_write(&self.state.join("workspaces.v2.json"), &data)
    }

    fn changed(&mut self) {
        self.sync_panes();
        if let Err(e) = self.resize_sessions() {
            self.last_refresh = format!("Pane resize failed: {e}");
        }
        self.retain_attachment_tickets();
        self.generation += 1;
        self.dirty = true
    }
    fn audit(&mut self, op: &str, target: u64, detail: &str) -> io::Result<()> {
        writeln!(
            self.audit,
            "{}\t{}\t{}\t{}\t{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            self.epoch,
            op,
            target,
            wire::hex(detail.as_bytes())
        )?;
        self.audit.flush()
    }
    fn create_workspace(
        &mut self,
        name: String,
        cwd: PathBuf,
        meta: CardMeta,
        start_shell: bool,
    ) -> io::Result<Vec<u8>> {
        if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(invalid("name must be 1–256 bytes without controls"));
        }
        let cwd = fs::canonicalize(cwd)?;
        if !cwd.is_dir() || cwd.to_str().is_none() {
            return Err(invalid("cwd must be a UTF-8 directory"));
        }
        meta.validate()?;
        let id = self.next;
        self.next += 1;
        self.audit("new-request", id, &name)?;
        self.workspaces.push(Workspace {
            id,
            name,
            cwd,
            tabs: Vec::new(),
            split: None,
            selected: 0,
            meta,
        });
        if let Err(e) = self.persist() {
            self.workspaces.pop();
            return Err(e);
        }
        self.active = id;
        self.changed();
        let tab = if start_shell { self.spawn(id)? } else { 0 };
        Ok(format!("{{\"workspace\":{id},\"session\":{tab}}}").into_bytes())
    }
    fn spawn(&mut self, wid: u64) -> io::Result<u64> {
        self.pane_tab_capacity(wid)?;
        self.record_workspace_pane_selection(wid);
        let (cols, rows) = self.pane_size(wid);
        let w = self
            .workspaces
            .iter_mut()
            .find(|w| w.id == wid)
            .ok_or_else(|| invalid("unknown workspace"))?;
        if !w.meta.operation.is_empty() {
            return Err(invalid(&w.meta.operation));
        }
        let run = os::nonce()?;
        let shell = os::shell();
        let mut command = crate::close::shell_command(&self.state, &run, &shell)?;
        let (master, child) = os::spawn_command_pty(&w.cwd, &mut command, cols, rows)?;
        let id = self.next;
        self.next += 1;
        w.tabs.push(Session {
            shell_run: String::new(),
            id,
            run,
            master,
            child,
            term: Terminal::new(cols as usize, rows as usize),
            input: VecDeque::new(),
            alive: true,
            ended: false,
            kind: "shell".into(),
            native: None,
            working: false,
            path: String::new(),
            title: Path::new(&shell)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
        });
        w.selected = id;
        self.active = wid;
        self.changed();
        Ok(id)
    }
    fn session(&mut self, id: u64, run: &str) -> io::Result<&mut Session> {
        self.workspaces
            .iter_mut()
            .flat_map(|w| &mut w.tabs)
            .find(|s| s.id == id && s.run == run)
            .ok_or_else(|| invalid("stale or wrong session/run target"))
    }
    fn snapshot(&self) -> Snapshot {
        let w = self.workspaces.iter().find(|w| w.id == self.active);
        let s = w.and_then(|w| w.tabs.iter().find(|s| s.id == w.selected));
        Snapshot {
            epoch: self.epoch.clone(),
            generation: self.generation,
            active: self.active,
            tab: s.map_or(0, |s| s.id),
            workspaces: self
                .workspaces
                .iter()
                .map(|w| WorkspaceView {
                    id: w.id,
                    name: w.name.clone(),
                    cwd: w.cwd.to_string_lossy().into(),
                    meta: w.meta.clone(),
                    tabs: w
                        .tabs
                        .iter()
                        .map(|t| TabView {
                            id: t.id,
                            run: t.run.clone(),
                            pid: t.child.id(),
                            alive: t.alive,
                            title: t.title.clone(),
                            kind: t.kind.clone(),
                            path: t.path.clone(),
                            working: t.working,
                        })
                        .collect(),
                })
                .collect(),
            cols: s.map_or(self.cols as usize, |s| s.term.grid.cols),
            rows: s.map_or(self.rows as usize, |s| s.term.grid.rows),
            x: s.map_or(0, |s| s.term.grid.x),
            y: s.map_or(0, |s| s.term.grid.y),
            cursor: s.is_some_and(|s| s.alive && s.term.cursor),
            bracketed_paste: s.is_some_and(|s| s.term.bracketed_paste),
            app_cursor: s.is_some_and(|s| s.term.app_cursor),
            notice: self.last_refresh.clone(),
            cells: s.map_or_else(
                || vec![Default::default(); self.cols as usize * self.rows as usize],
                |s| s.term.grid.cells.clone(),
            ),
            split: w.and_then(|w| self.split_view(w)),
        }
    }
    fn command(&mut self, line: &str) -> io::Result<Vec<u8>> {
        let gate = self.file_transfers.command_gate();
        let outer = self.command_depth == 0;
        let _guard = if outer {
            Some(
                gate.lock()
                    .map_err(|_| invalid("file command gate poisoned"))?,
            )
        } else {
            None
        };
        self.command_depth += 1;
        let result = self.command_scoped(line);
        self.command_depth -= 1;
        if outer {
            self.expire_file_transfers();
        }
        result
    }
    fn command_scoped(&mut self, line: &str) -> io::Result<Vec<u8>> {
        let op = line.split('\t').next().unwrap_or_default();
        if matches!(op, "stop" | "save-tabs") {
            self.sample_tab_directories();
        }
        let result = self.command_inner(line);
        if result.is_ok()
            && (matches!(
                op,
                "focus"
                    | "focus-exact"
                    | "tab"
                    | "open"
                    | "open-epoch"
                    | "open-diff-epoch"
                    | "open-pane-epoch"
                    | "open-diff-pane-epoch"
                    | "open-at-pane-epoch"
                    | "open-at"
                    | "task-start"
                    | "native"
                    | "native-resume"
            ) || (op == "pane"
                && line
                    .split('\t')
                    .nth(6)
                    .is_some_and(|op| matches!(op, "focus" | "move" | "new-tab"))))
        {
            self.record_pane_selection();
            self.remember_native_selection()?;
        }
        if result.is_ok()
            && !matches!(
                op,
                "ping"
                    | "snapshot"
                    | "snapshot-panes"
                    | "snapshot-links"
                    | "list"
                    | "capture"
                    | "scrollback"
                    | "scrollback-links"
                    | "search-output"
                    | "command-jump"
                    | "command-jump-links"
                    | "command-output"
                    | "frontends"
                    | "task-list"
                    | "task-info"
                    | "file-begin"
                    | "file-read"
                    | "file-write"
                    | "file-finish"
                    | "file-cancel"
                    | "file-poll"
                    | "attachment-check"
                    | "attachment-text"
                    | "attachment-cancel"
                    | "literal-ticket"
                    | "literal-paste"
                    | "refresh-status"
                    | "build-info"
                    | "restore-next"
                    | "restore-workspace-next"
                    | "close-check"
                    | "close-poll"
                    | "close-cancel"
            )
        {
            self.checkpoint_tabs()?;
        }
        result
    }
    fn command_inner(&mut self, request: &str) -> io::Result<Vec<u8>> {
        let p: Vec<_> = request.split('\t').collect();
        let arg = |i: usize| p.get(i).copied().ok_or_else(|| invalid("missing argument"));
        let num = |i: usize| -> io::Result<u64> { arg(i)?.parse().map_err(io::Error::other) };
        match arg(0)? {
            "task-start" | "task-list" | "task-info" => self.task_operation(&p),
            "file-begin" | "file-read" | "file-write" | "file-finish" | "file-cancel"
            | "file-poll" => self.file_command(&p),
            "attachment-check" | "attachment-text" | "attachment-cancel" | "literal-ticket"
            | "literal-paste" => self.attachment_command(&p),
            "frontends" => serde_json::to_vec(&self.frontend_inventory).map_err(io::Error::other),
            "ping" => Ok(format!("flere-v4\t{}", self.epoch).into_bytes()),
            "build-info" if p.len() == 1 => {
                crate::build_status::supervisor(&self.epoch, self.generation)
            }
            "snapshot" => Ok(self.snapshot().encode()),
            "snapshot-panes" if p.len() == 1 => Ok(self.snapshot().encode_panes()),
            "snapshot-links" if p.len() == 1 => Ok(self.snapshot().encode_links()),
            "pane" => self.pane_command(&p),
            "resize-panes" if p.len() == 5 => {
                if arg(1)? != self.epoch || num(2)? != self.active {
                    return Err(invalid("stale pane viewport identity"));
                }
                let cols = num(3)?;
                let rows = num(4)?;
                if !(2..=240).contains(&cols) || !(2..=100).contains(&rows) {
                    return Err(invalid("invalid pane viewport dimensions"));
                }
                self.cols = cols as u16;
                self.rows = rows as u16;
                self.resize_sessions()?;
                self.changed();
                Ok(b"ok".to_vec())
            }
            "list" => {
                let workspaces: Vec<_> = self.workspaces.iter().map(|w| {
                    let tabs: Vec<_> = w.tabs.iter().map(|t| serde_json::json!({"id":t.id,"run":t.run,"pid":t.child.id(),"alive":t.alive,"title":t.title,"kind":t.kind,"path":t.path})).collect();
                    serde_json::json!({"id":w.id,"name":w.name,"cwd":w.cwd,"selected_tab":w.selected,"meta":w.meta,"tabs":tabs})
                }).collect();
                Ok(serde_json::to_vec(&serde_json::json!({"protocol":4,"epoch":self.epoch,"active_workspace":self.active,"workspaces":workspaces})).map_err(io::Error::other)?)
            }
            "lead" => Err(invalid(
                "Lead roles were removed; create a workspace and pin it instead",
            )),
            "add-project" | "add-project-stopped" => self.add_project(
                Path::new(&wire::text(arg(1)?)?),
                &wire::text(p.get(2).copied().unwrap_or(""))?,
                arg(0)? == "add-project",
            ),
            "new" | "new-stopped" => self.create_workspace(
                wire::text(arg(1)?)?,
                PathBuf::from(wire::text(arg(2)?)?),
                CardMeta::default(),
                arg(0)? == "new",
            ),
            "worktree" | "worktree-stopped" => {
                let name = wire::text(arg(1)?)?;
                let repo = fs::canonicalize(wire::text(arg(2)?)?)?;
                let branch = wire::text(arg(3)?)?;
                let base = wire::text(arg(4)?)?;
                let project = wire::text(arg(5)?)?;
                if name.is_empty()
                    || name.len() > 256
                    || name.chars().any(char::is_control)
                    || branch.is_empty()
                    || base.is_empty()
                {
                    return Err(invalid("name, local branch and base are required"));
                }
                let id = self.next;
                self.next += 1;
                let parent = self.state.join("worktrees");
                if !parent.exists() {
                    fs::DirBuilder::new().mode(0o700).create(&parent)?;
                }
                let cwd = parent.join(id.to_string());
                let meta = CardMeta {
                    project,
                    branch: branch.clone(),
                    operation: "Preparing worktree; do not start until ready".into(),
                    ..Default::default()
                };
                meta.validate()?;
                self.audit(
                    "worktree-request",
                    id,
                    &format!("repo={};branch={branch};base={base}", repo.display()),
                )?;
                self.workspaces.push(Workspace {
                    id,
                    name,
                    cwd: cwd.clone(),
                    tabs: Vec::new(),
                    split: None,
                    selected: 0,
                    meta,
                });
                if let Err(e) = self.persist() {
                    self.workspaces.pop();
                    return Err(e);
                }
                self.active = id;
                self.changed();
                let (tx, rx) = std::sync::mpsc::channel();
                self.worktree_jobs.push(WorktreeJob {
                    workspace: id,
                    start_shell: arg(0)? == "worktree",
                    result: rx,
                });
                std::thread::spawn(move || {
                    let _ = tx.send(crate::git::create_worktree(&repo, &cwd, &branch, &base));
                });
                Ok(format!("{{\"workspace\":{id},\"state\":\"preparing\"}}").into_bytes())
            }
            "tab" => {
                let id = num(1)?;
                self.audit("start-shell-request", id, "")?;
                let tab = self.spawn(id)?;
                Ok(format!("{{\"session\":{tab}}}").into_bytes())
            }
            "save-tabs" => Ok(b"saved".to_vec()),
            // Older frontends also restore only the visible card. New frontends
            // use the scoped command so a stale request cannot open another card.
            "restore-next" => self.restore_next(arg(1)?, self.active),
            "restore-workspace-next" => self.restore_next(arg(1)?, num(2)?),
            "restore-retry" => {
                if arg(1)? != self.epoch {
                    return Err(invalid("workspace epoch changed"));
                }
                let wid = num(2)?;
                self.restoration
                    .attempted
                    .retain(|(workspace, _)| *workspace != wid);
                Ok(b"ready".to_vec())
            }
            "resume-recent" => {
                let wid = num(2)?;
                let w = self
                    .workspaces
                    .iter()
                    .find(|w| w.id == wid)
                    .ok_or_else(|| invalid("workspace unavailable"))?;
                let recent = w.meta.recent_conversation();
                let Some(saved) = recent.or_else(|| w.meta.conversations.first()) else {
                    return Ok(b"none".to_vec());
                };
                let harness = saved.harness.clone();
                let uuid = recent.map_or("", |c| c.uuid.as_str()).to_string();
                self.command(&format!(
                    "resume-saved\t{}\t{}\t{}\t{}",
                    arg(1)?,
                    num(2)?,
                    harness,
                    uuid
                ))
            }
            "resume-saved" => {
                if arg(1)? != self.epoch {
                    return Err(invalid("workspace epoch changed; open the card again"));
                }
                let wid = num(2)?;
                if self.active != wid {
                    return Err(invalid("active workspace changed; open the card again"));
                }
                let harness = arg(3)?;
                let uuid = arg(4)?;
                let w = self
                    .workspaces
                    .iter()
                    .find(|w| w.id == wid && !w.meta.archived)
                    .ok_or_else(|| invalid("workspace unavailable"))?;
                if !w.meta.operation.is_empty() {
                    return Err(invalid(&w.meta.operation));
                }
                // Existing and failed saved tabs take precedence over the history fallback.
                if !w.tabs.is_empty() {
                    return Ok(b"already open".to_vec());
                }
                if self.restoration.pending.iter().any(|p| p.workspace == wid) {
                    return Err(invalid(
                        "saved tabs must be restored before reopening a conversation",
                    ));
                }
                let saved = w.meta.recent_conversation();
                let cwd = if let Some(saved) = saved {
                    if saved.harness != harness || saved.uuid != uuid {
                        return Err(invalid(
                            "saved conversation identity changed; open the card again",
                        ));
                    }
                    saved.cwd.clone()
                } else {
                    // Old multi-chat cards have no reliable recency. Only the user can pick.
                    if !uuid.is_empty()
                        || !w.meta.conversations.iter().any(|c| c.harness == harness)
                    {
                        return Err(invalid("choose a saved conversation in the native picker"));
                    }
                    w.cwd.clone()
                };
                if self.workspaces.iter().flat_map(|w| &w.tabs).any(|t| {
                    t.native.as_ref().is_some_and(|n| {
                        !uuid.is_empty() && n.harness == harness && n.conversation == uuid
                    })
                }) {
                    return Err(invalid(
                        "this saved chat is already open in another workspace",
                    ));
                }
                let spec = crate::native::HostSpec {
                    id: self.next,
                    run: os::nonce()?,
                    harness: harness.into(),
                    argv: crate::native::resume_arguments(&self.state, harness, uuid)?,
                    cwd,
                    shell: os::shell(),
                    conversation: uuid.into(),
                    dispatch: String::new(),
                    launcher: None,
                };
                crate::native::write_spec(&self.state, &spec)?;
                let id = spec.id;
                self.next += 1;
                self.spawn_native(wid, spec, true)?;
                self.remember_native_selection()?;
                Ok(format!("{{\"session\":{id}}}").into_bytes())
            }
            "native" => {
                let wid = num(1)?;
                if let Some(w) = self.workspaces.iter().find(|w| w.id == wid)
                    && !w.meta.operation.is_empty()
                {
                    return Err(invalid(&w.meta.operation));
                }
                let harness = arg(2)?;
                let uuid = arg(3)?;
                let cwd = self
                    .workspaces
                    .iter()
                    .find(|w| w.id == wid && !w.meta.archived)
                    .ok_or_else(|| invalid("unknown or archived workspace"))?
                    .cwd
                    .clone();
                if self.workspaces.iter().flat_map(|w| &w.tabs).any(|t| {
                    t.native
                        .as_ref()
                        .is_some_and(|n| !uuid.is_empty() && n.conversation == uuid)
                }) {
                    return Err(invalid(
                        "this native conversation/profile already has a hosted terminal; focus it instead",
                    ));
                }
                let argv = crate::native::arguments(&self.state, harness, uuid)?;
                let run = os::nonce()?;
                let id = self.next;
                self.next += 1;
                let spec = crate::native::HostSpec {
                    launcher: None,
                    id,
                    run: run.clone(),
                    harness: harness.into(),
                    argv,
                    cwd: cwd.clone(),
                    shell: os::shell(),
                    conversation: uuid.into(),
                    dispatch: String::new(),
                };
                self.audit(
                    "native-start-request",
                    wid,
                    &format!("run={run};harness={harness};uuid={uuid}"),
                )?;
                crate::native::write_spec(&self.state, &spec)?;
                // A human-supplied exact UUID is durable even when a harness has no
                // discoverable rollout metadata. Persist before starting a process.
                if !uuid.is_empty() {
                    let index = self.workspaces.iter().position(|w| w.id == wid).unwrap();
                    if !self.workspaces[index]
                        .meta
                        .conversations
                        .iter()
                        .any(|c| c.harness == harness && c.uuid == uuid)
                    {
                        self.workspaces[index].meta.conversations.push(
                            crate::native::Conversation {
                                harness: harness.into(),
                                uuid: uuid.into(),
                                cwd: cwd.clone(),
                            },
                        );
                        if let Err(e) = self.persist() {
                            self.workspaces[index].meta.conversations.pop();
                            return Err(e);
                        }
                    }
                }
                self.spawn_native(wid, spec, true)?;
                Ok(format!("{{\"session\":{id}}}").into_bytes())
            }
            "inbox-hook" => {
                let event =
                    serde_json::from_str(&wire::text(arg(3)?)?).map_err(io::Error::other)?;
                serde_json::to_vec(&self.observe_hook(num(1)?, arg(2)?, event)?)
                    .map_err(io::Error::other)
            }
            "confirm-notice" => {
                self.confirm_notice(num(1)?, arg(2)?, arg(3)?)?;
                Ok(b"confirmed".to_vec())
            }
            "worker-event" => {
                self.worker_event(num(1)?, arg(2)?, arg(3)?, &wire::text(arg(4)?)?)?;
                Ok(b"recorded".to_vec())
            }
            "shell-context" => self.shell_context(
                arg(1)?,
                num(2)?.try_into().map_err(io::Error::other)?,
                arg(3)?,
            ),
            "shell-native" => self.shell_native(&p),
            "native-ended" => {
                let id = num(1)?;
                let run = arg(2)?;
                self.session(id, run)?;
                self.audit("native-returned-to-shell", id, run)?;
                let s = self.session(id, run)?;
                if let Some(spec) = s.native.take() {
                    s.title = Path::new(&spec.shell)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into();
                    s.kind = "shell".into();
                    self.changed();
                }
                Ok(b"ok".to_vec())
            }
            "open-epoch" => {
                if arg(1)? != self.epoch {
                    return Err(invalid("stale editor workspace epoch"));
                }
                self.command(&format!("open\t{}\t{}", arg(2)?, arg(3)?))
            }
            "open-pane-epoch" | "open-diff-pane-epoch" | "open-at-pane-epoch" => {
                let comparison = p[0] == "open-diff-pane-epoch";
                let jump = p[0] == "open-at-pane-epoch";
                if p.len()
                    != if jump {
                        9
                    } else if comparison {
                        8
                    } else {
                        7
                    }
                {
                    return Err(invalid("invalid exact pane editor arguments"));
                }
                self.validate_pane_origin(arg(1)?, num(2)?, num(3)?, arg(4)?, num(5)?, true)?;
                // Guard and open run in one supervisor turn; another client
                // cannot redirect the editor into a different focused group.
                if jump {
                    self.command_inner(&format!(
                        "open-at\t{}\t{}\t{}\t{}",
                        arg(2)?,
                        arg(6)?,
                        arg(7)?,
                        arg(8)?
                    ))
                } else if comparison {
                    self.command_inner(&format!(
                        "open-diff-epoch\t{}\t{}\t{}\t{}",
                        arg(1)?,
                        arg(2)?,
                        arg(6)?,
                        arg(7)?
                    ))
                } else {
                    self.command_inner(&format!("open\t{}\t{}", arg(2)?, arg(6)?))
                }
            }
            "open" | "open-diff-epoch" | "open-at" => {
                let comparison = arg(0)? == "open-diff-epoch";
                let jump = if arg(0)? == "open-at" {
                    let (line, column) = (num(3)?, num(4)?);
                    if p.len() != 5
                        || !(1..=100_000_000).contains(&line)
                        || !(1..=1_000_000).contains(&column)
                    {
                        return Err(invalid("invalid editor line/column"));
                    }
                    Some((line as usize, column as usize))
                } else {
                    None
                };
                if comparison && arg(1)? != self.epoch {
                    return Err(invalid("stale editor workspace epoch"));
                }
                let offset = usize::from(comparison);
                let wid = num(1 + offset)?;
                let cwd = self
                    .workspaces
                    .iter()
                    .find(|w| w.id == wid)
                    .ok_or_else(|| invalid("unknown workspace"))?
                    .cwd
                    .clone();
                let before = if comparison {
                    Some(crate::editor::file(&cwd, &wire::text(arg(3)?)?)?)
                } else {
                    None
                };
                let path =
                    crate::editor::file(&cwd, &wire::text(arg(if comparison { 4 } else { 2 })?)?)?;
                // Keep generated snapshots protected until the durable tab
                // checkpoint succeeds, even if spawning succeeds but saving fails.
                for path in before.iter().chain(std::iter::once(&path)) {
                    if let Some(pin) = crate::editor::cache::reserve_open(&self.state, path)? {
                        self.cache_reservations.borrow_mut().push(pin);
                    }
                }
                let canonical = path.to_str().unwrap().to_owned();
                if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == wid)
                    && let Some(t) = w
                        .tabs
                        .iter()
                        .find(|t| t.kind == "editor" && t.path == canonical && t.alive && !t.ended)
                {
                    if let Some((line, column)) = jump {
                        crate::close::jump(&self.state, &t.run, t.child.id(), &path, line, column)?;
                    }
                    w.selected = t.id;
                    self.active = wid;
                    self.changed();
                    return Ok(b"existing-file-tab".to_vec());
                }
                let mut command = if let Some(before) = &before {
                    crate::editor::comparison_command(before, &path)?
                } else if let Some((line, column)) = jump {
                    crate::editor::command_at(&path, line, column)?
                } else {
                    crate::editor::command(&path)?
                };
                let run = os::nonce()?;
                command = crate::close::editor_command(&self.state, &run, command)?;
                let id = self.next;
                self.next += 1;
                self.audit("open-editor-request", wid, &canonical)?;
                self.pane_tab_capacity(wid)?;
                self.record_workspace_pane_selection(wid);
                let (cols, rows) = self.pane_size(wid);
                let (master, child) = os::spawn_command_pty(&cwd, &mut command, cols, rows)?;
                let w = self.workspaces.iter_mut().find(|w| w.id == wid).unwrap();
                w.tabs.push(Session {
                    shell_run: String::new(),
                    id,
                    run,
                    master,
                    child,
                    term: Terminal::new(cols as usize, rows as usize),
                    input: VecDeque::new(),
                    alive: true,
                    ended: false,
                    title: format!(
                        "{}{}",
                        if comparison { "diff · " } else { "" },
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                    kind: "editor".into(),
                    native: None,
                    working: false,
                    path: canonical,
                });
                w.selected = id;
                self.active = wid;
                self.changed();
                Ok(format!("{{\"session\":{id}}}").into_bytes())
            }
            "metadata" => {
                let wid = num(1)?;
                let mut meta: CardMeta =
                    serde_json::from_str(&wire::text(arg(2)?)?).map_err(io::Error::other)?;
                meta.validate()?;
                let index = self
                    .workspaces
                    .iter()
                    .position(|w| w.id == wid)
                    .ok_or_else(|| invalid("unknown workspace"))?;
                if meta.archived
                    && self.workspaces[index]
                        .tabs
                        .iter()
                        .any(|t| t.alive || !t.ended)
                {
                    return Err(invalid(
                        "close live terminals before archiving this workspace",
                    ));
                }
                meta.last_conversation = self.workspaces[index].meta.last_conversation.clone();
                meta.conversations = self.workspaces[index].meta.conversations.clone();
                meta.operation = self.workspaces[index].meta.operation.clone();
                meta.base_sha = self.workspaces[index].meta.base_sha.clone();
                self.audit("metadata", wid, "workflow change")?;
                let old = std::mem::replace(&mut self.workspaces[index].meta, meta);
                if let Err(e) = self.persist() {
                    self.workspaces[index].meta = old;
                    return Err(e);
                }
                self.changed();
                Ok(b"ok".to_vec())
            }
            "rename" => {
                let wid = num(1)?;
                let name = wire::text(arg(2)?)?;
                if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                    return Err(invalid("invalid workspace name"));
                }
                let index = self
                    .workspaces
                    .iter()
                    .position(|w| w.id == wid)
                    .ok_or_else(|| invalid("unknown workspace"))?;
                self.audit("rename", wid, &name)?;
                let old = std::mem::replace(&mut self.workspaces[index].name, name);
                if let Err(e) = self.persist() {
                    self.workspaces[index].name = old;
                    return Err(e);
                }
                self.changed();
                Ok(b"ok".to_vec())
            }
            "notes" => {
                if arg(1)? != self.epoch {
                    return Err(invalid("workspace epoch changed; reopen notes"));
                }
                let wid = num(2)?;
                let expected = wire::text(arg(3)?)?;
                let notes = wire::text(arg(4)?)?;
                if notes.len() > 65536 {
                    return Err(invalid("notes exceed 64 KiB"));
                }
                let index = self
                    .workspaces
                    .iter()
                    .position(|w| w.id == wid && !w.meta.archived)
                    .ok_or_else(|| invalid("workspace unavailable"))?;
                if self.workspaces[index].meta.notes != expected {
                    return Err(invalid(
                        "Notes changed elsewhere; Ctrl+Y copies your edit. Cancel and reopen to reconcile",
                    ));
                }
                self.audit("notes", wid, "notes update")?;
                let old = std::mem::replace(&mut self.workspaces[index].meta.notes, notes);
                if let Err(e) = self.persist() {
                    self.workspaces[index].meta.notes = old;
                    return Err(e);
                }
                self.changed();
                Ok(b"ok".to_vec())
            }
            "focus-exact" => {
                if arg(1)? != self.epoch {
                    return Err(invalid("workspace epoch changed; select again"));
                }
                let wid = num(2)?;
                let tid = num(3)?;
                let run = arg(4)?;
                let w = self
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == wid && !w.meta.archived)
                    .ok_or_else(|| invalid("workspace unavailable"))?;
                if tid == 0 {
                    if !run.is_empty() {
                        return Err(invalid("invalid workspace focus identity"));
                    }
                } else {
                    if !w.tabs.iter().any(|t| t.id == tid && t.run == run) {
                        return Err(invalid("terminal identity changed; select again"));
                    }
                    w.selected = tid;
                }
                self.active = wid;
                self.changed();
                Ok(b"ok".to_vec())
            }
            "focus" => {
                let wid = num(1)?;
                let tid = num(2)?;
                let w = self
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == wid)
                    .ok_or_else(|| invalid("unknown workspace"))?;
                if tid != 0 {
                    if !w.tabs.iter().any(|t| t.id == tid) {
                        return Err(invalid("tab does not belong to workspace"));
                    }
                    w.selected = tid;
                }
                self.active = wid;
                self.changed();
                Ok(b"ok".to_vec())
            }
            "image-ticket" => self.image_ticket(arg(1)?, num(2)?, num(3)?, arg(4)?),
            "image-paste" => self.image_paste(arg(1)?, Path::new(&wire::text(arg(2)?)?)),
            "image-cancel" => {
                let token = arg(1)?;
                self.attachment_tickets.retain(|t| !t.matches(token));
                Ok(b"cancelled".to_vec())
            }
            "text" => {
                let id = num(1)?;
                let run = arg(2)?;
                let value = wire::text(arg(3)?)?;
                let bracketed = self.session(id, run)?.term.bracketed_paste;
                if value
                    .chars()
                    .any(|c| c.is_control() && !(bracketed && matches!(c, '\n' | '\r' | '\t')))
                {
                    return Err(invalid(
                        "literal text cannot contain keys/control bytes; multiline text requires native bracketed paste",
                    ));
                }
                let bytes = if bracketed {
                    [
                        b"\x1b[200~".as_slice(),
                        value.as_bytes(),
                        b"\x1b[201~".as_slice(),
                    ]
                    .concat()
                } else {
                    value.into_bytes()
                };
                self.command(&format!("input\t{id}\t{run}\t{}", wire::hex(&bytes)))
            }
            "input" => {
                let id = num(1)?;
                let run = arg(2)?;
                let bytes = wire::unhex(arg(3)?)?;
                {
                    let s = self.session(id, run)?;
                    if !s.alive || s.ended {
                        return Err(invalid("session has exited"));
                    }
                    if s.input.len() + bytes.len() > 131072 {
                        return Err(invalid("input queue full; retry after draining"));
                    }
                }
                self.audit("input", id, &format!("run={run};bytes={}", bytes.len()))?;
                if !bytes.is_empty() {
                    self.close_input(id, run);
                }
                self.session(id, run)?.input.extend(bytes);
                Ok(b"ok".to_vec())
            }
            "close-check" => self.close_check(arg(1)?, num(2)?, num(3)?, arg(4)?),
            "close-poll" => self.close_poll(arg(1)?, num(2)?, num(3)?, arg(4)?, arg(5)?),
            "close-cancel" => self.close_cancel(arg(1)?, num(2)?, num(3)?, arg(4)?, arg(5)?),
            "close" => {
                let id = num(1)?;
                let run = arg(2)?;
                self.session(id, run)?;
                self.audit("close-request", id, run)?;
                self.restoration.closed.insert(run.into());
                self.tasks.remove(run);
                let s = self.session(id, run)?;
                // A retained, fully exited task may outlive its old process group.
                if !s.ended || s.alive {
                    os::hangup(s.master.as_raw_fd(), &mut s.child);
                }
                Ok(b"hangup-sent".to_vec())
            }
            "wheel" | "wheel-links" => {
                let id = num(1)?;
                let run = arg(2)?;
                let cols = num(3)?;
                let rows = num(4)?;
                let delta = arg(5)?
                    .parse::<i64>()
                    .map_err(|_| invalid("invalid wheel delta"))?
                    .clamp(-500, 500);
                let actual = self.session(id, run)?;
                if cols != actual.term.grid.cols as u64
                    || rows != actual.term.grid.rows as u64
                    || !self
                        .workspaces
                        .iter()
                        .any(|w| w.id == self.active && w.selected == id)
                {
                    return Err(invalid("stale wheel viewport or selected terminal"));
                }
                let s = self.session(id, run)?;
                let bytes = s.term.alternate_scroll_input(delta);
                if !bytes.is_empty() {
                    if !s.alive || s.ended || s.input.len() + bytes.len() > 131072 {
                        return Err(invalid("wheel target exited or input queue full"));
                    }
                    self.audit("alternate-wheel", id, &format!("run={run};delta={delta}"))?;
                    self.session(id, run)?.input.extend(bytes);
                }
                let page = self.session(id, run)?.term.scrollback(None, delta);
                Ok(if p[0] == "wheel-links" {
                    page.encode_links()
                } else {
                    page.encode()
                })
            }
            "search-output" => {
                if p.len() != 5 {
                    return Err(invalid("invalid output search arguments"));
                }
                let query = wire::text(arg(4)?)?;
                if query.len() > 1024 {
                    return Err(invalid("output search query exceeds bound"));
                }
                serde_json::to_vec(
                    &self
                        .session(num(1)?, arg(2)?)?
                        .term
                        .search_output(num(3)?, &query),
                )
                .map_err(io::Error::other)
            }
            "command-jump" | "command-jump-links" | "command-output" => {
                let anchor = if arg(3)? == "live" {
                    None
                } else {
                    Some(num(3)?)
                };
                let terminal = &self.session(num(1)?, arg(2)?)?.term;
                if p[0] == "command-output" {
                    if p.len() != 4 {
                        return Err(invalid("invalid command output arguments"));
                    }
                    Ok(terminal.command_output(anchor)?.into_bytes())
                } else {
                    if p.len() != 5 || !matches!(arg(4)?, "previous" | "next") {
                        return Err(invalid("invalid command navigation arguments"));
                    }
                    let page = terminal.command_jump(anchor, arg(4)? == "previous")?;
                    Ok(if p[0] == "command-jump-links" {
                        page.encode_links()
                    } else {
                        page.encode()
                    })
                }
            }
            "scrollback" | "scrollback-links" => {
                let id = num(1)?;
                let run = arg(2)?;
                let anchor = if arg(3)? == "live" {
                    None
                } else {
                    Some(num(3)?)
                };
                let delta = arg(4)?
                    .parse::<i64>()
                    .map_err(|_| invalid("invalid scroll delta"))?
                    .clamp(-500, 500);
                let page = self.session(id, run)?.term.scrollback(anchor, delta);
                Ok(if p[0] == "scrollback-links" {
                    page.encode_links()
                } else {
                    page.encode()
                })
            }
            "capture" => {
                let id = num(1)?;
                let run = arg(2)?;
                let lines = num(3)?.clamp(1, 200) as usize;
                let s = self.session(id, run)?;
                Ok(format!(
                    "{{\"session\":{},\"run\":{},\"pid\":{},\"alive\":{},\"text\":{}}}",
                    id,
                    wire::json(run),
                    s.child.id(),
                    s.alive,
                    wire::json(&s.term.capture(lines))
                )
                .into_bytes())
            }
            "resize" => {
                if self.workspaces.iter().any(|w| w.split.is_some()) {
                    return Err(invalid(
                        "split panes require resize-panes; legacy resize ignored",
                    ));
                }
                let cols = num(1)?.clamp(2, 240) as u16;
                let rows = num(2)?.clamp(2, 100) as u16;
                if (cols, rows) != (self.cols, self.rows) {
                    for w in &mut self.workspaces {
                        for s in &mut w.tabs {
                            if s.alive {
                                os::resize(s.master.as_raw_fd(), cols, rows)?;
                            }
                            s.term.resize(cols as usize, rows as usize);
                        }
                    }
                    self.cols = cols;
                    self.rows = rows;
                    self.changed();
                }
                Ok(b"ok".to_vec())
            }
            "coordinate" => {
                let wid = num(1)?;
                let args = serde_json::from_str(&wire::text(arg(3)?)?).map_err(io::Error::other)?;
                serde_json::to_vec(&self.coordinate(wid, None, arg(2)?, &args)?)
                    .map_err(io::Error::other)
            }
            "agent-operation" => {
                let id = num(1)?;
                let run = arg(2)?;
                let w = self
                    .workspaces
                    .iter()
                    .find(|w| {
                        w.tabs.iter().any(|t| {
                            t.id == id && t.run == run && t.alive && !t.ended && t.native.is_some()
                        })
                    })
                    .ok_or_else(|| invalid("stale or non-native agent run"))?;
                let wid = w.id;
                let op = arg(3)?;
                let args: serde_json::Value =
                    serde_json::from_str(&wire::text(arg(4)?)?).map_err(io::Error::other)?;
                let mut value = if matches!(op, "inspect_terminal" | "send_terminal_input") {
                    self.terminal_operation((id, run), op, &args)?
                } else if op == "read_terminal" {
                    let lines = args["lines"].as_u64().unwrap_or(80).clamp(1, 200);
                    serde_json::from_slice(
                        &self.command(&format!("capture\t{id}\t{run}\t{lines}"))?,
                    )
                    .map_err(io::Error::other)?
                } else {
                    self.coordinate(wid, Some((id, run)), op, &args)?
                };
                if !matches!(
                    op,
                    "inbox" | "context" | "message_status" | "messaging_activation" | "set_focus"
                ) {
                    self.attach_notice(wid, id, run, &mut value)?;
                }
                serde_json::to_vec(&value).map_err(io::Error::other)
            }
            "refresh-status" => Ok(self.last_refresh.as_bytes().to_vec()),
            "refresh-epoch" => {
                if p.len() != 3 || arg(1)? != self.epoch {
                    return Err(invalid("stale supervisor update epoch"));
                }
                self.command_inner(&format!("refresh\t{}", arg(2)?))
            }
            "refresh" => {
                if self.close_busy() {
                    return Err(invalid(
                        "wait for the current tab close check before refreshing",
                    ));
                }
                if !self.delivery_jobs.is_empty() {
                    return Err(invalid(
                        "native message handoff is in progress; retry refresh after its receipt",
                    ));
                }
                if !self.worktree_jobs.is_empty() {
                    return Err(invalid(
                        "wait for worktree preparation before refreshing the supervisor",
                    ));
                }
                let binary = PathBuf::from(wire::text(arg(1)?)?);
                if !binary.is_absolute() || !binary.is_file() {
                    return Err(invalid("refresh needs an absolute executable path"));
                }
                self.audit("refresh-request", 0, &binary.to_string_lossy())?;
                self.last_refresh = "Refresh requested; keeping all sessions".into();
                self.refresh = Some(binary);
                Ok(b"refresh-requested; status is recorded in actions.log".to_vec())
            }
            "stop" => {
                self.audit("stop-request", 0, "explicit terminate")?;
                self.stop = true;
                Ok(b"stopping".to_vec())
            }
            _ => Err(invalid("unknown command")),
        }
    }
    fn finish_worktrees(&mut self) -> bool {
        let mut complete = Vec::new();
        self.worktree_jobs
            .retain(|job| match job.result.try_recv() {
                Ok(result) => {
                    complete.push((job.workspace, job.start_shell, result));
                    false
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    complete.push((
                        job.workspace,
                        job.start_shell,
                        Err(io::Error::other(
                            "worktree worker disconnected; inspect the path before retrying",
                        )),
                    ));
                    false
                }
                Err(_) => true,
            });
        let changed = !complete.is_empty();
        for (id, start_shell, result) in complete {
            let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id) else {
                continue;
            };
            let ready = result.is_ok();
            match result {
                Ok(sha) => {
                    w.meta.base_sha = sha;
                    w.meta.operation.clear();
                }
                Err(e) => {
                    w.meta.operation = format!(
                        "Worktree incomplete: {}. Files retained for inspection.",
                        wire::passive(&e.to_string())
                    )
                }
            };
            let archived = w.meta.archived;
            if let Err(e) = self.persist() {
                let _ = self.audit("worktree-save-failed", id, &e.to_string());
                continue;
            }
            if ready && !archived && start_shell {
                let previous = self.active;
                if let Err(e) = self.spawn(id) {
                    let _ = self.audit("worktree-shell-failed", id, &e.to_string());
                }
                self.active = previous;
            }
        }
        changed
    }
    fn observe_native(&mut self) -> bool {
        if self.stop || os::stopping() || self.native_checked.elapsed() < Duration::from_secs(1) {
            return false;
        }
        self.native_checked = Instant::now();
        self.sample_tab_directories();
        let mut changed = self.checkpoint_tabs().is_err();
        let mut recorded = false;
        for w in &mut self.workspaces {
            for s in &mut w.tabs {
                if !s.alive || s.ended {
                    if s.working {
                        s.working = false;
                        changed = true;
                    }
                    continue;
                }
                let working = crate::native::working_screen(&s.term)
                    && crate::native::discover_codex(
                        s.child.id(),
                        s.native.as_ref().map_or(&w.cwd, |n| &n.cwd),
                    )
                    .is_ok_and(|id| id.is_some());
                if s.working != working {
                    s.working = working;
                    changed = true;
                }
                let Some(spec) = &mut s.native else {
                    continue;
                };
                if spec.harness != "codex" {
                    continue;
                }
                if spec.launcher.as_ref().is_some_and(|(pid, start)| {
                    !os::child_identity(*pid).is_ok_and(|p| p.1 == *start)
                }) {
                    continue; // Lost launcher ownership requires explicit repair, not adoption of another child.
                }
                if let Ok(Some(uuid)) = crate::native::discover_codex(s.child.id(), &spec.cwd) {
                    // A harness may switch conversations itself. Follow its current,
                    // unambiguous owned rollout rather than freezing the launch UUID.
                    if spec.conversation != uuid {
                        let saved = crate::native::Conversation {
                            harness: "codex".into(),
                            uuid: uuid.clone(),
                            cwd: spec.cwd.clone(),
                        };
                        if !w.meta.conversations.contains(&saved) {
                            w.meta.conversations.push(saved.clone());
                        }
                        w.meta.last_conversation = Some(saved);
                        recorded = true;
                        changed = true;
                    }
                    spec.conversation = uuid;
                }
            }
        }
        if recorded {
            if let Err(e) = self.persist() {
                let _ = self.audit("conversation-save-failed", 0, &e.to_string());
            } else {
                let _ = self.audit("conversation-linked", 0, "exact owned process metadata");
            }
        }
        changed
    }
    fn drain(&mut self) -> bool {
        let gate = self.file_transfers.command_gate();
        let outer = self.command_depth == 0;
        // The gate protects only commit ordering; workers never hold it over I/O.
        let _guard = outer.then(|| gate.lock().unwrap_or_else(|e| e.into_inner()));
        self.command_depth += 1;
        let changed = self.drain_scoped();
        self.command_depth -= 1;
        if outer {
            self.expire_file_transfers();
        }
        changed
    }
    fn drain_scoped(&mut self) -> bool {
        self.cache_cleaner.tick();
        // Reply to terminal probes before process and filesystem observation,
        // which can exceed a native program's startup response deadline.
        let mut changed = false;
        for w in &mut self.workspaces {
            for s in &mut w.tabs {
                if !s.ended {
                    match s.child.try_wait() {
                        Ok(Some(status)) => {
                            s.ended = true;
                            self.tasks.exited(&s.run, status.code());
                            s.title = self.tasks.title(&s.run).unwrap_or_else(|| {
                                format!(
                                    "exited {}",
                                    status
                                        .code()
                                        .map_or_else(|| "signal".into(), |c| c.to_string())
                                )
                            });
                            changed = true
                        }
                        Ok(None) => {}
                        Err(_) => {}
                    }
                }
                if s.alive {
                    let mut buf = [0; 8192];
                    for _ in 0..8 {
                        match s.master.read(&mut buf) {
                            Ok(0) => {
                                s.alive = false;
                                changed = true;
                                break;
                            }
                            Ok(n) => {
                                s.term.feed(&buf[..n]);
                                self.tasks.output(&s.run);
                                changed = true
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                            Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                                s.alive = false;
                                changed = true;
                                break;
                            }
                            Err(_) => {
                                s.alive = false;
                                changed = true;
                                break;
                            }
                        }
                    }
                    if s.input.len() + s.term.replies.len() <= 131072 {
                        s.input.extend(std::mem::take(&mut s.term.replies))
                    } else {
                        s.term.replies.clear()
                    }
                    if !s.input.is_empty() {
                        let bytes = s.input.make_contiguous();
                        match s.master.write(bytes) {
                            Ok(n) => {
                                s.input.drain(..n);
                            }
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(_) => {
                                s.input.clear();
                            }
                        }
                    }
                }
            }
        }
        changed |= self.observe_native() | self.finish_worktrees();
        self.close_tick();
        changed |= self.tasks_tick();
        for w in &mut self.workspaces {
            let before = w.tabs.len();
            for t in w.tabs.iter().filter(|t| t.ended && !t.alive) {
                let _ =
                    fs::remove_dir_all(crate::close::directory(&self.state, t.shell_identity()));
            }
            w.tabs
                .retain(|s| !s.ended || s.alive || self.tasks.contains(&s.run));
            if w.tabs.len() != before {
                if !w.tabs.iter().any(|s| s.id == w.selected) {
                    w.selected = w.tabs.last().map_or(0, |s| s.id);
                }
                changed = true;
            }
        }

        self.delivery_tick();
        if !self.stop
            && !os::stopping()
            && changed
            && let Err(e) = self.checkpoint_tabs()
        {
            let notice = e.to_string();
            if self.last_refresh != notice {
                self.last_refresh = notice;
                self.changed();
            }
        }
        changed
    }
}
struct SocketCleanup(PathBuf);
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub fn serve(state: &Path) -> io::Result<()> {
    private_state(state)?;
    let lock = secure_file(&state.join("supervisor.lock"))?;
    os::lock(lock.as_raw_fd())
        .map_err(|_| io::Error::other("another supervisor owns this state"))?;
    let socket = state.join("control.sock");
    if let Ok(m) = fs::symlink_metadata(&socket) {
        if !m.file_type().is_socket() || m.uid() != os::uid() {
            return Err(invalid("refusing to replace foreign/non-socket path"));
        }
        fs::remove_file(&socket)?;
    }
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let server = Server::load(state)?;
    run_loop(state, lock, listener, server, Vec::new())
}
fn run_loop(
    state: &Path,
    lock: File,
    listener: UnixListener,
    mut server: Server,
    mut clients: Vec<Client>,
) -> io::Result<()> {
    let _cleanup = SocketCleanup(state.join("control.sock"));
    os::signals();
    let mut last_frame = Instant::now() - Duration::from_secs(1);
    while !os::stopping() {
        let mut fds = vec![os::PollFd {
            fd: listener.as_raw_fd(),
            events: os::READ,
            revents: 0,
        }];
        for c in &clients {
            fds.push(os::PollFd {
                fd: c.stream.as_raw_fd(),
                events: os::READ
                    | if c.output.len() > c.offset {
                        os::WRITE
                    } else {
                        0
                    },
                revents: 0,
            })
        }
        for s in server
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|s| s.alive)
        {
            fds.push(os::PollFd {
                fd: s.master.as_raw_fd(),
                events: os::READ | if !s.input.is_empty() { os::WRITE } else { 0 },
                revents: 0,
            })
        }
        os::wait(&mut fds, if server.dirty { 16 } else { 250 })?;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if clients.len() >= 32 || !os::same_user(stream.as_raw_fd())? {
                        continue;
                    }
                    stream.set_nonblocking(true)?;
                    clients.push(Client {
                        stream,
                        input: Vec::new(),
                        output: Vec::new(),
                        offset: 0,
                        watch: false,
                        panes: false,
                        links: false,
                        build: None,
                        done: false,
                        deadline: Instant::now() + Client::output_timeout(false),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if server.drain() {
            server.changed()
        }
        server.expire_file_transfers();
        server.frontend_inventory = serde_json::json!({
            "schema_version":1,"status":"known",
            "tracked":clients.iter().filter(|c| c.watch && !c.done).filter_map(|c| c.build.clone()).collect::<Vec<_>>(),
            "untracked":clients.iter().filter(|c| c.watch && !c.done && c.build.is_none()).count(),
        });
        for c in &mut clients {
            let mut buf = [0; 8192];
            for _ in 0..16 {
                match c.stream.read(&mut buf) {
                    Ok(0) => {
                        c.done = true;
                        break;
                    }
                    Ok(n) => {
                        c.input.extend_from_slice(&buf[..n]);
                        if c.input.len() > wire::MAX_REQUEST + 4 {
                            c.done = true;
                            break;
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(_) => {
                        c.done = true;
                        break;
                    }
                }
            }
            if c.done {
                continue;
            }
            if c.watch {
                if !c.input.is_empty() {
                    c.done = true
                }
                continue;
            }
            if c.output.is_empty() {
                match wire::take_request_frame(&mut c.input) {
                    Ok(Some(b)) => {
                        let result =
                            std::str::from_utf8(&b)
                                .map_err(io::Error::other)
                                .and_then(|request| {
                                    if let Some(watch) = WatchRequest::parse(request) {
                                        if let Some(encoded) = watch.build {
                                            let data = wire::unhex(encoded)?;
                                            if data.len() > 8192 {
                                                return Err(invalid(
                                                    "frontend metadata exceeds bound",
                                                ));
                                            }
                                            let build: serde_json::Value =
                                                serde_json::from_slice(&data)
                                                    .map_err(io::Error::other)?;
                                            if build["schema_version"] != 1
                                                || build["build"]["component"] != "flere"
                                                || build["pid"]
                                                    .as_u64()
                                                    .is_none_or(|n| n == 0 || n > u32::MAX as u64)
                                                || build["attachment"].as_str().is_none_or(|s| {
                                                    s.len() != 32
                                                        || !s.bytes().all(|b| b.is_ascii_hexdigit())
                                                })
                                            {
                                                return Err(invalid("invalid frontend metadata"));
                                            }
                                            c.build = Some(build);
                                        }
                                        c.watch = true;
                                        c.panes = watch.panes;
                                        c.links = watch.links;
                                        Ok(watch.snapshot(&server.snapshot()))
                                    } else {
                                        server.command(request)
                                    }
                                });
                        c.output = wire::frame(&result.unwrap_or_else(|e| {
                            format!("!{}", wire::passive(&e.to_string())).into_bytes()
                        }));
                        c.deadline = Instant::now() + Client::output_timeout(c.watch);
                    }
                    Err(_) => c.done = true,
                    Ok(None) => {}
                }
            }
        }
        if server.dirty && last_frame.elapsed() >= Duration::from_millis(16) {
            let snapshot = server.snapshot();
            let frame = wire::frame(&snapshot.encode());
            let pane_frame = wire::frame(&snapshot.encode_panes());
            let link_frame = clients
                .iter()
                .any(|c| c.watch && c.links && !c.done)
                .then(|| wire::frame(&snapshot.encode_links()));
            for c in clients.iter_mut().filter(|c| c.watch && !c.done) {
                let frame = if c.links {
                    link_frame.as_ref().expect("link watcher frame")
                } else if c.panes {
                    &pane_frame
                } else {
                    &frame
                };
                c.queue_snapshot(frame, Instant::now());
            }
            server.dirty = clients.iter().any(|c| c.watch && c.offset > 0);
            last_frame = Instant::now();
        }
        for c in &mut clients {
            if c.done {
                continue;
            }
            c.flush_output(Instant::now());
        }
        clients.retain(|c| !c.done);
        if let Some(binary) = server.refresh.take()
            && let Err(e) = refresh::replace(&mut server, &lock, &listener, &clients, &binary)
        {
            server.last_refresh = format!("Refresh failed: {e}");
            server.changed();
            let _ = server.audit("refresh-failed", 0, &e.to_string());
        }
        if server.stop && clients.iter().all(|c| c.watch || c.output.is_empty()) {
            break;
        }
    }
    // Checkpoint before hangup; shutdown observation must not erase the layout.
    server.sample_tab_directories();
    if let Err(e) = server.checkpoint_tabs() {
        let _ = server.audit("shutdown-tabs-save-failed", 0, &e.to_string());
    }
    server.stop_delivery_jobs();
    for s in server.workspaces.iter_mut().flat_map(|w| &mut w.tabs) {
        if !s.ended || s.alive {
            os::hangup(s.master.as_raw_fd(), &mut s.child)
        }
    }
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end
        && server
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .any(|s| !s.ended)
    {
        server.drain();
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

pub fn restore(state: &Path, token: &str) -> io::Result<()> {
    refresh::restore(state, token)
}
pub fn validate_refresh(state: &Path, token: &str) -> io::Result<()> {
    refresh::validate(state, token)
}

#[cfg(test)]
mod hyperlink_watch_tests {
    use super::*;
    use crate::terminal::{Cell, hyperlinks::Hyperlink};

    #[test]
    fn link_watch_is_explicit_and_legacy_watch_keeps_its_projection() {
        let mut snapshot = Snapshot {
            epoch: "a".repeat(32),
            generation: 1,
            active: 0,
            tab: 0,
            workspaces: Vec::new(),
            cols: 2,
            rows: 2,
            x: 0,
            y: 0,
            cursor: false,
            bracketed_paste: false,
            app_cursor: false,
            notice: String::new(),
            cells: vec![Cell::default(); 4],
            split: None,
        };
        snapshot.cells[0].link = Hyperlink::new("https://github.com/example/watch");
        for (request, version, links, build) in [
            ("watch", 4, false, None),
            ("watch-panes", 5, false, None),
            ("watch-panes-build\tmetadata", 5, false, Some("metadata")),
            ("watch-links", 6, true, None),
            ("watch-links-build\tmetadata", 6, true, Some("metadata")),
        ] {
            let watch = WatchRequest::parse(request).unwrap();
            assert_eq!(watch.links, links);
            assert_eq!(watch.build, build);
            let bytes = watch.snapshot(&snapshot);
            assert_eq!(bytes[0], version);
            assert_eq!(
                Snapshot::decode(&bytes).unwrap().cells[0].link.is_some(),
                links
            );
        }
        for unsupported in [
            "watch-links-build",
            "watch-links\textra",
            "watch-panes\textra",
            "watch-future",
        ] {
            assert!(WatchRequest::parse(unsupported).is_none());
        }
    }
}
