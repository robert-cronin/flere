//! Same-PID exec handoff: PTY masters, native children and connected UI sockets survive.
use super::*;
use serde::{Deserialize, Serialize};
use std::os::unix::process::CommandExt;
// Version 10 prevents older supervisors from replaying queue-submitted receipts.
const VERSION: u32 = 10;
#[derive(Serialize, Deserialize)]
struct SavedClient {
    fd: i32,
    input: Vec<u8>,
    output: Vec<u8>,
    offset: usize,
    watch: bool,
    #[serde(default)]
    panes: bool,
    #[serde(default)]
    links: bool,
    #[serde(default)]
    build: Option<serde_json::Value>,
}
#[derive(Serialize, Deserialize)]
struct SavedSession {
    id: u64,
    run: String,
    #[serde(default)]
    shell_run: String,
    fd: i32,
    pid: u32,
    start: String,
    status: Option<i32>,
    term: Terminal,
    input: VecDeque<u8>,
    alive: bool,
    ended: bool,
    title: String,
    kind: String,
    path: String,
    native: Option<crate::native::HostSpec>,
    #[serde(default)]
    working: bool,
}
#[derive(Serialize, Deserialize)]
struct SavedWorkspace {
    id: u64,
    name: String,
    cwd: PathBuf,
    selected: u64,
    meta: CardMeta,
    tabs: Vec<SavedSession>,
    #[serde(default)]
    split: Option<crate::panes::PaneLayout>,
}
#[derive(Serialize, Deserialize)]
struct Handoff {
    version: u32,
    #[serde(default)]
    restoration: restore::State,
    #[serde(default)]
    tasks: tasks::Tasks,
    pid: u32,
    state: PathBuf,
    epoch: String,
    generation: u64,
    active: u64,
    next: u64,
    cols: u16,
    rows: u16,
    lock: i32,
    listener: i32,
    workspaces: Vec<SavedWorkspace>,
    clients: Vec<SavedClient>,
}
fn path(state: &Path, token: &str) -> io::Result<PathBuf> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid refresh token"));
    }
    Ok(state.join(format!("refresh-{token}.json")))
}
fn read(state: &Path, token: &str, preflight: bool) -> io::Result<Handoff> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path(state, token)?)?;
    let m = f.metadata()?;
    if !m.is_file() || m.uid() != os::uid() || m.mode() & 0o077 != 0 || m.len() > 256 * 1024 * 1024
    {
        return Err(invalid("invalid refresh image permissions/size"));
    }
    let h: Handoff = serde_json::from_reader(io::BufReader::new(f)).map_err(io::Error::other)?;
    if !(1..=VERSION).contains(&h.version)
        || h.state != state
        || h.pid
            != if preflight {
                os::parent_pid()
            } else {
                std::process::id()
            }
        || !(2..=240).contains(&h.cols)
        || !(2..=100).contains(&h.rows)
    {
        return Err(invalid("incompatible refresh identity/version"));
    }
    let mut fds = std::collections::HashSet::new();
    for fd in [h.lock, h.listener]
        .into_iter()
        .chain(h.clients.iter().map(|c| c.fd))
        .chain(
            h.workspaces
                .iter()
                .flat_map(|w| w.tabs.iter().map(|t| t.fd)),
        )
    {
        if fd < 3 || !fds.insert(fd) {
            return Err(invalid("duplicate or invalid refresh descriptor"));
        }
    }
    for w in &h.workspaces {
        w.meta.validate()?;
        if let Some(split) = &w.split {
            split.validate(&w.tabs.iter().map(|t| t.id).collect::<Vec<_>>())?;
            if split.groups[usize::from(split.focused)].selected != w.selected {
                return Err(invalid("invalid focused pane refresh identity"));
            }
            let rectangles = crate::panes::geometry(
                h.cols as usize,
                h.rows as usize,
                split.axis,
                split.ratio,
                split.zoomed,
                split.focused,
            );
            for (group, rect) in split.groups.iter().zip(rectangles) {
                if let Some(rect) = rect
                    && w.tabs
                        .iter()
                        .filter(|t| group.tabs.contains(&t.id))
                        .any(|t| (t.term.grid.cols, t.term.grid.rows) != (rect.cols, rect.rows))
                {
                    return Err(invalid("invalid pane terminal refresh dimensions"));
                }
            }
        }
        for t in &w.tabs {
            if !t.term.valid_state()
                || t.input.len() > 131072
                || t.run.len() != 32
                || (!t.shell_run.is_empty()
                    && (t.shell_run.len() != 32
                        || !t.shell_run.bytes().all(|b| b.is_ascii_hexdigit())))
            {
                return Err(invalid("invalid terminal refresh image"));
            }
        }
    }
    let mut saved_orders = std::collections::BTreeSet::new();
    h.tasks.validate()?;
    for (wid, tid, run) in h.tasks.identities() {
        if !h
            .workspaces
            .iter()
            .any(|w| w.id == wid && w.tabs.iter().any(|t| t.id == tid && t.run == run))
        {
            return Err(invalid("task refresh identity has no owned terminal"));
        }
    }
    for p in &h.restoration.pending {
        p.tab.validate()?;
        if !h.workspaces.iter().any(|w| w.id == p.workspace)
            || !saved_orders.insert(p.tab.order)
            || p.tab.order >= h.next
        {
            return Err(invalid("invalid pending tab refresh identity"));
        }
    }
    for w in &h.workspaces {
        if h.restoration
            .pending
            .iter()
            .filter(|p| p.workspace == w.id)
            .count()
            > 512
        {
            return Err(invalid("too many pending tabs in refresh"));
        }
        for t in &w.tabs {
            let order = h.restoration.order.get(&t.run).copied().unwrap_or(t.id);
            if order == 0 || order >= h.next || !saved_orders.insert(order) {
                return Err(invalid("invalid live tab refresh identity"));
            }
        }
        if let Some(split) = h.restoration.panes.get(&w.id) {
            let orders = h
                .restoration
                .pending
                .iter()
                .filter(|p| p.workspace == w.id)
                .map(|p| p.tab.order)
                .chain(
                    w.tabs
                        .iter()
                        .map(|t| h.restoration.order.get(&t.run).copied().unwrap_or(t.id)),
                )
                .collect::<Vec<_>>();
            split.validate(&orders)?;
        }
    }
    for c in &h.clients {
        if c.input.len() > wire::MAX_REQUEST + 4
            || c.output.len() > wire::MAX + 4
            || c.offset > c.output.len()
            || c.links && (!c.watch || !c.panes)
        {
            return Err(invalid("invalid client refresh image"));
        }
    }
    Ok(h)
}
pub(super) fn validate(state: &Path, token: &str) -> io::Result<()> {
    read(state, token, true).map(|_| ())
}
pub(super) fn replace(
    s: &mut Server,
    lock: &File,
    listener: &UnixListener,
    clients: &[Client],
    binary: &Path,
) -> io::Result<()> {
    // Routine hooks are kept in memory. Persist them before exec because the
    // restored supervisor reuses this epoch and loads coordination from disk.
    // This also preserves permission/activity guards with older refresh readers.
    // A failed flush must leave the current supervisor and its sessions intact.
    s.preferences.flush(&s.state)?;
    s.persist()?;
    let token = os::nonce()?;
    let mut workspaces = Vec::new();
    for w in &s.workspaces {
        let mut tabs = Vec::new();
        for t in &w.tabs {
            let status = t.child.raw_status();
            let start = if status.is_some() {
                String::new()
            } else {
                os::child_identity(t.child.id())?.1
            };
            tabs.push(SavedSession {
                shell_run: t.shell_run.clone(),
                id: t.id,
                run: t.run.clone(),
                fd: t.master.as_raw_fd(),
                pid: t.child.id(),
                start,
                status,
                term: t.term.clone(),
                input: t.input.clone(),
                alive: t.alive,
                ended: t.ended,
                title: t.title.clone(),
                kind: t.kind.clone(),
                path: t.path.clone(),
                native: t.native.clone(),
                working: t.working,
            });
        }
        workspaces.push(SavedWorkspace {
            id: w.id,
            name: w.name.clone(),
            cwd: w.cwd.clone(),
            selected: w.selected,
            meta: w.meta.clone(),
            tabs,
            split: w.split.clone(),
        });
    }
    let h = Handoff {
        tasks: s.tasks.clone(),
        version: VERSION,
        restoration: s.restoration.clone(),
        pid: std::process::id(),
        state: s.state.clone(),
        epoch: s.epoch.clone(),
        generation: s.generation + 1,
        active: s.active,
        next: s.next,
        cols: s.cols,
        rows: s.rows,
        lock: lock.as_raw_fd(),
        listener: listener.as_raw_fd(),
        workspaces,
        clients: clients
            .iter()
            .map(|c| SavedClient {
                fd: c.stream.as_raw_fd(),
                input: c.input.clone(),
                output: c.output.clone(),
                offset: c.offset,
                watch: c.watch,
                panes: c.panes,
                links: c.links,
                build: c.build.clone(),
            })
            .collect(),
    };
    let file = path(&s.state, &token)?;
    let bytes = serde_json::to_vec(&h).map_err(io::Error::other)?;
    if bytes.len() > 256 * 1024 * 1024 {
        return Err(invalid(
            "runtime exceeds refresh image bound; sessions retained",
        ));
    }
    atomic_write(&file, &bytes)?;
    let result = (|| {
        let mut check = std::process::Command::new(binary)
            .arg("--state")
            .arg(&s.state)
            .arg("_validate-refresh")
            .arg(&token)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()?;
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = check.try_wait()? {
                if !status.success() {
                    return Err(invalid(
                        "new binary rejected refresh image; existing sessions retained",
                    ));
                }
                break;
            }
            if Instant::now() > until {
                let _ = check.kill();
                let _ = check.wait();
                return Err(invalid(
                    "new binary preflight timed out; existing sessions retained",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        s.audit("refresh-exec", 0, &binary.to_string_lossy())?;
        s.file_transfers.cancel_for_refresh()?;
        let fds: Vec<_> = [h.lock, h.listener]
            .into_iter()
            .chain(h.clients.iter().map(|c| c.fd))
            .chain(
                h.workspaces
                    .iter()
                    .flat_map(|w| w.tabs.iter().map(|t| t.fd)),
            )
            .collect();
        for fd in &fds {
            if let Err(e) = os::inherit_fd(*fd, true) {
                for previous in &fds {
                    let _ = os::inherit_fd(*previous, false);
                }
                return Err(e);
            }
        }
        let error = std::process::Command::new(binary)
            .arg("--state")
            .arg(&s.state)
            .arg("_restore")
            .arg(&token)
            .exec();
        for fd in fds {
            let _ = os::inherit_fd(fd, false);
        }
        Err(error)
    })();
    let _ = fs::remove_file(file);
    result
}
pub(super) fn restore(state: &Path, token: &str) -> io::Result<()> {
    private_state(state)?;
    let h = read(state, token, false)?;
    let lock = os::adopt_file(h.lock)?;
    os::lock(lock.as_raw_fd())?;
    let listener = os::adopt_listener(h.listener)?;
    if listener.local_addr()?.as_pathname() != Some(state.join("control.sock").as_path()) {
        return Err(invalid("restored listener belongs to another state"));
    }
    let mut s = Server::load(state)?;
    s.restoration = h.restoration;
    s.tasks = h.tasks;
    s.epoch = h.epoch;
    s.generation = h.generation;
    s.active = h.active;
    s.next = h.next;
    s.cols = h.cols;
    s.rows = h.rows;
    s.workspaces.clear();
    for w in h.workspaces {
        let mut tabs = Vec::new();
        for t in w.tabs {
            let master = os::adopt_file(t.fd)?;
            let child = os::Process::restore(t.pid, t.status, &t.start)?;
            tabs.push(Session {
                shell_run: t.shell_run,
                id: t.id,
                run: t.run,
                master,
                child,
                term: t.term,
                input: t.input,
                alive: t.alive,
                ended: t.ended,
                title: t.title,
                kind: t.kind,
                path: t.path,
                native: t.native,
                working: t.working,
            });
        }
        s.workspaces.push(Workspace {
            id: w.id,
            name: w.name,
            cwd: w.cwd,
            selected: w.selected,
            meta: w.meta,
            tabs,
            split: w.split,
        });
    }
    let mut clients = Vec::new();
    for c in h.clients {
        clients.push(Client {
            stream: os::adopt_stream(c.fd)?,
            input: c.input,
            output: c.output,
            offset: c.offset,
            watch: c.watch,
            panes: c.panes,
            links: c.links,
            build: c.build,
            done: false,
            deadline: Instant::now() + Client::output_timeout(c.watch),
        });
    }
    fs::remove_file(path(state, token)?)?;
    s.last_refresh = "Refreshed Flere; all terminal sessions preserved".into();
    s.audit(
        "refresh-restored",
        0,
        "same PID, socket, PTYs and native processes",
    )?;
    run_loop(state, lock, listener, s, clients)
}

#[cfg(test)]
mod hyperlink_refresh_tests {
    use super::*;

    #[test]
    fn client_refresh_defaults_legacy_links_off_and_preserves_partial_frames() {
        let legacy = serde_json::json!({
            "fd": 4, "input": [], "output": [0, 0, 0, 3, 5, 0, 0],
            "offset": 5, "watch": true, "panes": true, "build": null,
        });
        let mut saved: SavedClient = serde_json::from_value(legacy).unwrap();
        assert!(!saved.links);
        saved.links = true;
        saved.output[4] = 6;
        let restored: SavedClient =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert!(restored.watch && restored.panes && restored.links);
        assert_eq!(restored.output, saved.output);
        assert_eq!(restored.offset, saved.offset);
        let build: serde_json::Value = serde_json::from_str(crate::build_info::json()).unwrap();
        let range = &build["compatibility"]["refresh_handoff"];
        assert_eq!(range["current"], VERSION);
        assert_eq!(range["read_min"], 1);
        assert_eq!(range["read_max"], VERSION);
        assert_eq!(build["compatibility"]["saved_state"]["current"], 11);
    }
}
