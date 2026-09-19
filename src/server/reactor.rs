//! Socket/terminal service shared by the ordinary loop and durable save barriers.
//! A barrier admits only volatile input and the last published display. It never
//! dispatches a second durable mutation or exposes a speculative coordination read.
use super::*;
use std::sync::Arc;

pub(super) struct Reactor {
    pub listener: UnixListener,
    pub clients: Vec<Client>,
    last_frame: Instant,
    published: Option<Arc<Snapshot>>,
    servicing: usize,
    save_input: std::collections::BTreeSet<(u64, String)>,
    saving: bool,
    pub failure: Option<io::Error>,
}

impl Reactor {
    pub fn new(listener: UnixListener, clients: Vec<Client>) -> Self {
        Self {
            listener,
            clients,
            last_frame: Instant::now() - Duration::from_secs(1),
            published: None,
            servicing: 0,
            save_input: Default::default(),
            saving: false,
            failure: None,
        }
    }

    fn poll(&mut self, server: &Server, timeout: i32) -> io::Result<()> {
        let mut fds = vec![os::PollFd {
            fd: self.listener.as_raw_fd(),
            events: os::READ,
            revents: 0,
        }];
        for c in &self.clients {
            fds.push(os::PollFd {
                fd: c.stream.as_raw_fd(),
                events: os::READ
                    | if c.output.len() > c.offset {
                        os::WRITE
                    } else {
                        0
                    },
                revents: 0,
            });
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
            });
        }
        os::wait(&mut fds, timeout)?;
        // Bound acceptance as well as retained clients, including under a flood
        // of connections that exceed the limit or fail the same-user check.
        for _ in 0..32 {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.clients.len() + self.servicing >= 32
                        || !os::same_user(stream.as_raw_fd())?
                    {
                        continue;
                    }
                    stream.set_nonblocking(true)?;
                    self.clients.push(Client {
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
        Ok(())
    }
}

impl Client {
    fn read_input(&mut self) {
        let mut buf = [0; 8192];
        for _ in 0..16 {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    self.done = true;
                    break;
                }
                Ok(n) => {
                    self.input.extend_from_slice(&buf[..n]);
                    if self.input.len() > wire::MAX_REQUEST + 4 {
                        self.done = true;
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.done = true;
                    break;
                }
            }
        }
        if self.watch && !self.input.is_empty() {
            self.done = true;
        }
    }

    fn start_watch(&mut self, watch: &WatchRequest<'_>) -> io::Result<()> {
        if let Some(encoded) = watch.build {
            let data = wire::unhex(encoded)?;
            if data.len() > 8192 {
                return Err(invalid("frontend metadata exceeds bound"));
            }
            let build: serde_json::Value =
                serde_json::from_slice(&data).map_err(io::Error::other)?;
            if build["schema_version"] != 1
                || build["build"]["component"] != "flere"
                || build["pid"]
                    .as_u64()
                    .is_none_or(|n| n == 0 || n > u32::MAX as u64)
                || build["attachment"]
                    .as_str()
                    .is_none_or(|s| s.len() != 32 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(invalid("invalid frontend metadata"));
            }
            self.build = Some(build);
        }
        self.watch = true;
        self.panes = watch.panes;
        self.links = watch.links;
        Ok(())
    }
}

// Leave deferred requests in their original framed buffer. This preserves both
// refresh serialization and EOF cancellation; disconnected requests are never
// replayed after the barrier. Invalid/oversized frames still fail promptly.
fn serviceable(bytes: &[u8], display: bool) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let n = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    if n > wire::MAX_REQUEST {
        return true;
    }
    let Some(data) = bytes.get(4..4 + n) else {
        return false;
    };
    let Ok(request) = std::str::from_utf8(data) else {
        return true;
    };
    let op = request.split('\t').next().unwrap_or_default();
    matches!(op, "ping" | "input" | "text") || display && WatchRequest::parse(request).is_some()
}

impl Server {
    pub(super) fn poll_io(&mut self, timeout: i32) -> io::Result<()> {
        let mut reactor = self.reactor.take().expect("active reactor");
        let result = reactor
            .failure
            .take()
            .map_or_else(|| reactor.poll(self, timeout), Err);
        self.reactor = Some(reactor);
        result
    }

    fn display(&mut self, barrier: bool) -> Arc<Snapshot> {
        if barrier {
            let snapshot = self.reactor.as_ref().unwrap().published.as_ref().unwrap();
            Arc::new(self.barrier_snapshot(snapshot))
        } else {
            let snapshot = Arc::new(self.snapshot());
            self.reactor.as_mut().unwrap().published = Some(snapshot.clone());
            snapshot
        }
    }

    pub(super) fn service_clients(&mut self, barrier: bool) {
        let sockets: Vec<_> = self
            .reactor
            .as_ref()
            .unwrap()
            .clients
            .iter()
            .map(|c| c.stream.as_raw_fd())
            .collect();
        for fd in sockets {
            // The current request owns its socket across all save waits. Other
            // clients remain available; no index can retarget this response.
            let reactor = self.reactor.as_mut().unwrap();
            let Some(index) = reactor
                .clients
                .iter()
                .position(|c| c.stream.as_raw_fd() == fd)
            else {
                continue;
            };
            let mut c = reactor.clients.remove(index);
            reactor.servicing += 1;
            c.read_input();
            c.expire_waiting(Instant::now());
            let display = self.reactor.as_ref().unwrap().published.is_some();
            if !c.done
                && !c.watch
                && c.output.is_empty()
                && (!barrier || serviceable(&c.input, display))
            {
                match wire::take_request_frame(&mut c.input) {
                    Ok(Some(bytes)) => {
                        let result = std::str::from_utf8(&bytes)
                            .map_err(io::Error::other)
                            .and_then(|request| {
                                if let Some(watch) = WatchRequest::parse(request) {
                                    c.start_watch(&watch)?;
                                    Ok(watch.snapshot(&self.display(barrier)))
                                } else {
                                    match request {
                                        "snapshot" => Ok(self.display(barrier).encode()),
                                        "snapshot-panes" => {
                                            Ok(self.display(barrier).encode_panes())
                                        }
                                        "snapshot-links" => {
                                            Ok(self.display(barrier).encode_links())
                                        }
                                        // No wrapper: it can checkpoint tab state.
                                        _ if barrier => self.command_inner(request),
                                        _ => self.command(request),
                                    }
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
            // A nested save may have accepted more sockets while this request
            // was removed. Admission counts servicing requests against the cap.
            let reactor = self.reactor.as_mut().unwrap();
            reactor.servicing -= 1;
            reactor.clients.push(c);
        }
    }

    pub(super) fn flush_clients(&mut self, barrier: bool) {
        let mut reactor = self.reactor.take().unwrap();
        if self.dirty
            && reactor.last_frame.elapsed() >= Duration::from_millis(16)
            && (!barrier || reactor.published.is_some())
        {
            clients::queue_watch_snapshots(&mut reactor.clients, || {
                if barrier {
                    Arc::new(self.barrier_snapshot(reactor.published.as_ref().unwrap()))
                } else {
                    let snapshot = Arc::new(self.snapshot());
                    reactor.published = Some(snapshot.clone());
                    snapshot
                }
            });
            self.dirty = reactor.clients.iter().any(|c| c.watch && c.offset > 0);
            reactor.last_frame = Instant::now();
        }
        for c in &mut reactor.clients {
            if !c.done {
                c.flush_output(Instant::now());
            }
        }
        reactor.clients.retain(|c| !c.done);
        if !reactor.clients.iter().any(|c| c.watch) {
            // Do not retain an extra metadata/grid image after the UI detaches.
            reactor.published = None;
        }
        self.reactor = Some(reactor);
    }

    fn barrier_snapshot(&self, published: &Snapshot) -> Snapshot {
        let mut snapshot = published.clone();
        // Keep the published generation. A direct snapshot from a completed
        // command must outrank an older layout queued during a later save.
        // Equal-generation terminal frames remain useful to existing watchers.
        let terminal = |id| {
            let prior = published
                .workspaces
                .iter()
                .find(|w| w.id == published.active)?
                .tabs
                .iter()
                .find(|t| t.id == id)?;
            self.workspaces
                .iter()
                .find(|w| w.id == published.active)?
                .tabs
                .iter()
                .find(|s| s.id == id && s.run == prior.run)
        };
        if let Some(s) = terminal(snapshot.tab)
            && s.term.grid.cols == snapshot.cols
            && s.term.grid.rows == snapshot.rows
        {
            snapshot.cols = s.term.grid.cols;
            snapshot.rows = s.term.grid.rows;
            snapshot.x = s.term.grid.x;
            snapshot.y = s.term.grid.y;
            snapshot.cursor = s.alive && s.term.cursor;
            snapshot.bracketed_paste = s.term.bracketed_paste;
            snapshot.app_cursor = s.term.app_cursor;
            snapshot.cells.clone_from(&s.term.grid.cells);
        } else {
            snapshot.cursor = false;
        }
        if let Some(split) = &mut snapshot.split {
            let pane = &mut split.other;
            if let Some(s) = terminal(pane.tab)
                && s.term.grid.cols == pane.cols
                && s.term.grid.rows == pane.rows
            {
                pane.cols = s.term.grid.cols;
                pane.rows = s.term.grid.rows;
                pane.x = s.term.grid.x;
                pane.y = s.term.grid.y;
                pane.cursor = s.alive && s.term.cursor;
                pane.bracketed_paste = s.term.bracketed_paste;
                pane.app_cursor = s.term.app_cursor;
                pane.cells.clone_from(&s.term.grid.cells);
            } else {
                pane.cursor = false;
            }
        }
        snapshot
    }

    // A delivery reservation must revalidate after any user input admitted by
    // its save barrier, even when those bytes have already drained into the PTY.
    pub(super) fn record_save_input(&mut self, session: u64, run: &str) {
        if let Some(reactor) = &mut self.reactor
            && reactor.saving
        {
            reactor.save_input.insert((session, run.into()));
        }
    }

    pub(super) fn input_during_save(&self, session: u64, run: &str) -> bool {
        self.reactor
            .as_ref()
            .is_some_and(|r| r.save_input.contains(&(session, run.into())))
    }

    pub(super) fn write_durable(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.with_durable_writer(|| atomic_write(path, bytes))
    }

    fn with_durable_writer(
        &mut self,
        write: impl FnOnce() -> io::Result<()> + Send,
    ) -> io::Result<()> {
        self.with_durable_job(write)
    }

    pub(super) fn with_durable_job<T: Send>(
        &mut self,
        write: impl FnOnce() -> io::Result<T> + Send,
    ) -> io::Result<T> {
        if self.reactor.is_none() {
            // Startup, shutdown and exec handoff have no active reactor.
            return write();
        }
        self.reactor.as_mut().unwrap().save_input.clear();
        self.reactor.as_mut().unwrap().saving = true;
        let result = std::thread::scope(|scope| {
            // Exactly one immutable bounded image and one writer; no enqueue
            // acknowledgement, coalescing, recursive save, or detached job.
            let writer = std::thread::Builder::new()
                .name("flere-durable".into())
                .spawn_scoped(scope, write)?;
            while !writer.is_finished() {
                if let Err(error) = self.poll_io(4) {
                    // A socket failure must not turn a successful disk commit
                    // into a mutation rollback. Surface it to the outer loop.
                    self.reactor.as_mut().unwrap().failure = Some(error);
                    break;
                }
                let (changed, retired) = self.drain_terminals();
                self.io_retired |= retired;
                if changed {
                    self.generation += 1;
                    self.dirty = true;
                }
                self.service_clients(true);
                self.flush_clients(true);
            }
            let result = writer
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("durable writer panicked")));
            // A barrier may have cleared dirty while publishing the old view.
            self.dirty = true;
            result
        });
        self.reactor.as_mut().unwrap().saving = false;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    struct Fixture {
        root: PathBuf,
        server: Server,
    }
    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tests")
                .join(&os::nonce().unwrap()[..12]);
            let state = root.join("s");
            private_state(&state).unwrap();
            let mut server = Server::load(&state).unwrap();
            let mut command = std::process::Command::new("/bin/sh");
            command.args(["-c", "stty -echo; printf 'READY\\n'; while IFS= read -r x; do printf 'SEEN:%s\\n' \"$x\"; done"])
                .env("ENV", "").env("HOME", &root);
            let (master, child) = os::spawn_command_pty(&root, &mut command, 80, 12).unwrap();
            server.active = 1;
            server.next = 3;
            server.workspaces.push(Workspace {
                id: 1,
                name: "fixture".into(),
                cwd: root.clone(),
                selected: 2,
                meta: CardMeta::default(),
                split: None,
                tabs: vec![Session {
                    id: 2,
                    run: os::nonce().unwrap(),
                    shell_run: String::new(),
                    master,
                    child,
                    term: Terminal::new(80, 12),
                    input: VecDeque::new(),
                    alive: true,
                    ended: false,
                    title: "owned shell".into(),
                    kind: "shell".into(),
                    path: String::new(),
                    native: None,
                    working: false,
                }],
            });
            server.persist().unwrap();
            let listener = UnixListener::bind(state.join("control.sock")).unwrap();
            listener.set_nonblocking(true).unwrap();
            server.reactor = Some(Reactor::new(listener, Vec::new()));
            let mut fixture = Self { root, server };
            let end = Instant::now() + Duration::from_secs(3);
            loop {
                fixture.server.drain_terminals();
                let snapshot = fixture.server.snapshot();
                if text(&snapshot).contains("READY") {
                    break;
                }
                assert!(Instant::now() < end, "owned shell startup");
                std::thread::sleep(Duration::from_millis(2));
            }
            fixture
        }
        fn watch(&mut self) -> UnixStream {
            let mut stream = wire::connect(&self.server.state).unwrap();
            stream.write_all(&wire::frame(b"watch-links")).unwrap();
            self.server.poll_io(0).unwrap();
            self.server.service_clients(false);
            self.server.flush_clients(false);
            let initial = Snapshot::decode(&wire::read_frame(&mut stream).unwrap()).unwrap();
            assert_eq!(initial.workspaces[0].meta.notes, "");
            stream
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for session in self.server.workspaces.iter_mut().flat_map(|w| &mut w.tabs) {
                let _ = session.child.kill();
                let _ = session.child.wait();
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn text(snapshot: &Snapshot) -> String {
        snapshot.cells.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn owned_storage_connection_returns_through_barrier_while_shell_progresses() {
        use crate::record_store::{CommitOutcome, InboxRequest, MailboxScope, Store};
        let mut fixture = Fixture::new();
        let database = fixture.root.join("database");
        fs::DirBuilder::new().mode(0o700).create(&database).unwrap();
        let mut store = Store::open(&database).unwrap();
        let original = serde_json::json!({"id":format!("{:032x}",1),"from":1,"to":1,
            "body":"exact durable evidence","intent":"quiet","saved":1,"surfaced":null,
            "native_surfaced":null,"acknowledged":null,"delivery":null});
        store.insert(std::slice::from_ref(&original)).unwrap();
        let mut watch = fixture.watch();
        let state = fixture.server.state.clone();
        let run = fixture.server.workspaces[0].tabs[0].run.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let (store, outcome) = std::thread::scope(|scope| {
            let client = scope.spawn(move || {
                ready.recv_timeout(Duration::from_secs(3)).unwrap();
                wire::request(
                    &state,
                    &["input", "2", &run, &wire::hex(b"storage-token\n")],
                )
                .unwrap();
                let end = Instant::now() + Duration::from_secs(3);
                loop {
                    let frame = Snapshot::decode(&wire::read_frame(&mut watch).unwrap()).unwrap();
                    assert_eq!(frame.workspaces[0].tabs[0].run, run);
                    if text(&frame).contains("SEEN:storage-token") {
                        break;
                    }
                    assert!(Instant::now() < end, "PTY stopped behind storage job");
                }
                release.send(()).unwrap();
            });
            let result = fixture
                .server
                .with_durable_job(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(4)).unwrap();
                    let id = format!("{:032x}", 1);
                    let outcome = store.inbox_witnessed(
                        &format!("{:032x}", 99),
                        InboxRequest {
                            scope: MailboxScope::Human { workspace: 1 },
                            include_acknowledged: true,
                            after: None,
                            limit: 1,
                            acknowledge: &[&id],
                            return_messages: true,
                            timestamp: 42,
                        },
                    );
                    Ok((store, outcome))
                })
                .unwrap();
            client.join().unwrap();
            result
        });
        let CommitOutcome::Committed(receipt) = outcome else {
            panic!("storage did not commit")
        };
        assert_eq!(receipt.messages.len(), 1);
        assert_eq!(receipt.messages[0]["body"], original["body"]);
        assert_eq!(receipt.messages[0]["acknowledged"], 42);
        drop(store);
        let reopened = Store::open(&database).unwrap();
        let record = reopened
            .get(
                original["id"].as_str().unwrap(),
                crate::record_store::Scope {
                    workspace: 1,
                    conversation: None,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(record, receipt.messages[0]);
    }

    #[test]
    fn barrier_keeps_exact_input_and_watch_output_live_without_publishing_staged_metadata() {
        let mut f = Fixture::new();
        let mut watch = f.watch();
        let state = f.server.state.clone();
        let run = f.server.workspaces[0].tabs[0].run.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let path = state.join("barrier-result");
        f.server.workspaces[0].meta.notes = "uncommitted metadata".into();
        let mut direct = std::thread::scope(|scope| {
            let path = &path;
            let state = &state;
            let run = &run;
            let client = scope.spawn(move || {
                ready.recv_timeout(Duration::from_secs(3)).unwrap();
                assert!(
                    wire::request(state, &["ping"])
                        .unwrap()
                        .starts_with(b"flere-v4")
                );
                assert!(wire::request(state, &["input", "2", "stale", "78"]).is_err());
                wire::request(state, &["input", "2", run, &wire::hex(b"barrier-token\n")]).unwrap();
                wire::request(state, &["text", "2", run, &wire::hex(b"literal-token")]).unwrap();
                wire::request(state, &["input", "2", run, "0a"]).unwrap();
                let end = Instant::now() + Duration::from_secs(2);
                loop {
                    let frame = Snapshot::decode(&wire::read_frame(&mut watch).unwrap()).unwrap();
                    assert_eq!(
                        frame.workspaces[0].meta.notes, "",
                        "speculative metadata escaped"
                    );
                    assert_eq!(frame.workspaces[0].tabs[0].run, *run);
                    if text(&frame).contains("SEEN:literal-token") {
                        assert!(text(&frame).contains("SEEN:barrier-token"));
                        break;
                    }
                    assert!(Instant::now() < end, "PTY output starved behind save");
                }
                let mut direct = wire::connect(state).unwrap();
                direct.write_all(&wire::frame(b"snapshot-links")).unwrap();
                wire::request(state, &["ping"]).unwrap();
                direct.set_nonblocking(true).unwrap();
                assert_eq!(
                    direct.read(&mut [0]).unwrap_err().kind(),
                    io::ErrorKind::WouldBlock,
                    "a direct read must wait for consistent committed state"
                );
                direct.set_nonblocking(false).unwrap();
                assert!(!path.exists(), "writer completed before its release");
                release.send(()).unwrap();
                direct
            });
            f.server
                .with_durable_writer(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(4)).unwrap();
                    atomic_write(path, b"committed")
                })
                .unwrap();
            client.join().unwrap()
        });
        assert!(
            f.server.dirty,
            "publish the committed state after a barrier"
        );
        f.server.service_clients(false);
        f.server.flush_clients(false);
        let committed = Snapshot::decode(&wire::read_frame(&mut direct).unwrap()).unwrap();
        assert_eq!(committed.workspaces[0].meta.notes, "uncommitted metadata");
        assert_eq!(fs::read(&path).unwrap(), b"committed");
        assert!(f.server.input_during_save(2, &run));
        assert!(!f.server.input_during_save(2, "stale"));
    }

    #[test]
    fn deferred_mutations_retain_their_socket_and_disconnect_cancels_without_replay() {
        let mut f = Fixture::new();
        let state = f.server.state.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let mut retained = std::thread::scope(|scope| {
            let state = &state;
            let client = scope.spawn(move || {
                ready.recv_timeout(Duration::from_secs(3)).unwrap();
                let mut cancelled = wire::connect(state).unwrap();
                cancelled
                    .write_all(&wire::frame(b"rename\t1\t63616e63656c6c6564"))
                    .unwrap();
                drop(cancelled);
                let mut retained = wire::connect(state).unwrap();
                retained
                    .write_all(&wire::frame(b"rename\t1\t72657461696e6564"))
                    .unwrap();
                // A following ping makes the supervisor service the earlier sockets.
                wire::request(state, &["ping"]).unwrap();
                retained.set_nonblocking(true).unwrap();
                let mut byte = [0];
                assert_eq!(
                    retained.read(&mut byte).unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
                retained.set_nonblocking(false).unwrap();
                release.send(()).unwrap();
                retained
            });
            let error = f
                .server
                .with_durable_writer(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(4)).unwrap();
                    Err(io::Error::other("controlled write failure"))
                })
                .unwrap_err();
            assert_eq!(error.to_string(), "controlled write failure");
            assert_eq!(f.server.workspaces[0].name, "fixture");
            client.join().unwrap()
        });
        f.server.service_clients(false);
        f.server.flush_clients(false);
        assert_eq!(wire::read_frame(&mut retained).unwrap(), b"ok");
        assert_eq!(f.server.workspaces[0].name, "retained");
        let saved: Saved =
            serde_json::from_slice(&fs::read(state.join("workspaces.v2.json")).unwrap()).unwrap();
        assert_eq!(saved.workspaces[0].name, "retained");
    }

    #[test]
    fn expired_queued_mutation_is_rejected_while_active_save_and_input_keep_their_owners() {
        let mut f = Fixture::new();
        let state = f.server.state.clone();
        let run = f.server.workspaces[0].tabs[0].run.clone();
        let (started, ready) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        std::thread::scope(|scope| {
            let client = scope.spawn(move || {
                ready.recv_timeout(Duration::from_secs(3)).unwrap();
                let error = wire::request(&state, &["rename", "1", "65787069726564"]).unwrap_err();
                assert!(
                    error.to_string().starts_with("Request not started:"),
                    "{error}"
                );
                assert_eq!(
                    wire::request(&state, &["text", "2", &run, "78"]).unwrap(),
                    b"ok"
                );
                assert!(wire::request(&state, &["ping"]).is_ok());
                release.send(()).unwrap();
            });
            f.server
                .with_durable_writer(move || {
                    started.send(()).unwrap();
                    wait.recv_timeout(Duration::from_secs(6)).unwrap();
                    Ok(())
                })
                .unwrap();
            client.join().unwrap();
        });
        f.server.service_clients(false);
        f.server.flush_clients(false);
        assert_eq!(f.server.workspaces[0].name, "fixture");
        let session = &f.server.workspaces[0].tabs[0];
        assert!(
            session.input.is_empty(),
            "literal bytes have drained to the PTY"
        );
        assert!(f.server.input_during_save(session.id, &session.run));
        let saved: Saved =
            serde_json::from_slice(&fs::read(f.server.state.join("workspaces.v2.json")).unwrap())
                .unwrap();
        assert_eq!(saved.workspaces[0].name, "fixture");
    }

    #[test]
    fn ordinary_input_remains_usable_when_an_unrelated_layout_checkpoint_cannot_save() {
        let mut f = Fixture::new();
        f.server.checkpoint_tabs().unwrap();
        let run = f.server.workspaces[0].tabs[0].run.clone();
        let pid = f.server.workspaces[0].tabs[0].child.id();
        f.server.workspaces[0].tabs[0].title = "pending title".into();
        let path = f.server.state.join("workspaces.v2.json");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(f.server.command("save-tabs").is_err());
        assert!(f.server.command("input\t2\tstale\t78").is_err());
        assert!(f.server.command(&format!("text\t2\t{run}\t0a")).is_err());
        for (op, bytes) in [
            ("input", b"raw-token\n".as_slice()),
            ("text", b"literal-token"),
            ("input", b"\n"),
        ] {
            assert_eq!(
                f.server
                    .command(&format!("{op}\t2\t{run}\t{}", wire::hex(bytes)))
                    .unwrap(),
                b"ok"
            );
        }
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            f.server.drain_terminals();
            let output = text(&f.server.snapshot());
            if output.contains("SEEN:literal-token") {
                assert_eq!(output.matches("SEEN:raw-token").count(), 1);
                assert_eq!(output.matches("SEEN:literal-token").count(), 1);
                break;
            }
            assert!(
                Instant::now() < until,
                "input did not reach the owned shell"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(path.is_dir());
        assert_eq!(f.server.workspaces[0].tabs[0].child.id(), pid);
        assert_eq!(f.server.workspaces[0].tabs[0].run, run);
        fs::remove_dir(&path).unwrap();
        f.server.command("save-tabs").unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.workspaces[0].tabs[0].title, "pending title");
        assert_eq!(saved.workspaces[0].selected, 2);
    }

    #[test]
    fn audit_failure_rejects_input_before_queueing_and_recovery_keeps_metadata_only() {
        let mut f = Fixture::new();
        let path = f.server.state.join("actions.log");
        let before = fs::read(&path).unwrap();
        let run = f.server.workspaces[0].tabs[0].run.clone();
        let audit = std::mem::replace(&mut f.server.audit, File::open(&path).unwrap());
        for op in ["input", "text"] {
            let result = f
                .server
                .command(&format!("{op}\t2\t{run}\t{}", wire::hex(b"rejected-token")));
            assert!(result.is_err());
            assert!(f.server.workspaces[0].tabs[0].input.is_empty());
            assert_eq!(fs::read(&path).unwrap(), before);
        }
        f.server.audit = audit;
        let detail = "line\nfield\t雪\r\u{1b}";
        f.server.audit("fixture", 2, detail).unwrap();
        for (op, bytes) in [("text", b"accepted-token".as_slice()), ("input", b"\n")] {
            assert_eq!(
                f.server
                    .command(&format!("{op}\t2\t{run}\t{}", wire::hex(bytes)))
                    .unwrap(),
                b"ok"
            );
        }
        let log = fs::read(&path).unwrap();
        assert!(log.starts_with(&before));
        let appended = std::str::from_utf8(&log[before.len()..]).unwrap();
        let lines: Vec<_> = appended.lines().collect();
        assert_eq!(lines.len(), 3);
        for (line, (op, expected)) in lines.iter().zip([
            ("fixture", detail.to_owned()),
            ("input", format!("run={run};bytes=14")),
            ("input", format!("run={run};bytes=1")),
        ]) {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 5);
            assert!(fields[0].parse::<u128>().unwrap() > 0);
            assert_eq!(fields[1], f.server.epoch);
            assert_eq!(fields[2], op);
            assert_eq!(fields[3], "2");
            assert_eq!(wire::text(fields[4]).unwrap(), expected);
        }
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            f.server.drain_terminals();
            let output = text(&f.server.snapshot());
            if output.contains("SEEN:accepted-token") {
                assert_eq!(output.matches("SEEN:accepted-token").count(), 1);
                assert!(!output.contains("rejected-token"));
                break;
            }
            assert!(
                Instant::now() < until,
                "accepted input did not reach owned child"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn activity_records_the_actual_input_bytes_including_empty_bracketed_paste() {
        let mut f = Fixture::new();
        let run = f.server.workspaces[0].tabs[0].run.clone();
        f.server.workspaces[0].tabs[0].term.bracketed_paste = true;
        f.server.reactor.as_mut().unwrap().saving = true;
        f.server
            .command_inner(&format!("text\t2\t{run}\t"))
            .unwrap();
        assert_eq!(
            f.server.workspaces[0].tabs[0]
                .input
                .iter()
                .copied()
                .collect::<Vec<_>>(),
            b"\x1b[200~\x1b[201~"
        );
        assert!(f.server.input_during_save(2, &run));
        let reactor = f.server.reactor.as_mut().unwrap();
        reactor.saving = false;
        reactor.save_input.clear();
        f.server
            .command_inner(&format!("input\t2\t{run}\t78"))
            .unwrap();
        assert!(!f.server.input_during_save(2, &run));
    }

    #[test]
    fn metadata_save_failure_rolls_back_and_display_does_not_adopt_failed_value() {
        let mut f = Fixture::new();
        let _watch = f.watch();
        let path = f.server.state.join("workspaces.v2.json");
        // Fail replacement after the temporary file sync, using only owned state.
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        let result = f.server.command(&format!(
            "metadata\t1\t{}",
            wire::hex(br#"{"notes":"failed"}"#)
        ));
        assert!(result.is_err());
        assert_eq!(f.server.workspaces[0].meta.notes, "");
        assert_eq!(f.server.display(false).workspaces[0].meta.notes, "");
    }

    #[test]
    fn frozen_display_never_reads_a_replacement_run_or_incompatible_viewport() {
        let mut f = Fixture::new();
        let published = f.server.snapshot();
        f.server.generation += 4;
        let original_text = text(&published);
        let tab = &mut f.server.workspaces[0].tabs[0];
        let run = tab.run.clone();
        tab.run = os::nonce().unwrap();
        tab.term.feed(b"REPLACEMENT SECRET");
        let frozen = f.server.barrier_snapshot(&published);
        assert_eq!(frozen.generation, published.generation);
        assert_eq!(text(&frozen), original_text);
        assert!(!frozen.cursor);
        let tab = &mut f.server.workspaces[0].tabs[0];
        tab.run = run;
        tab.term = Terminal::new(40, 6);
        tab.term.feed(b"DIFFERENT GEOMETRY");
        let frozen = f.server.barrier_snapshot(&published);
        assert_eq!(frozen.generation, published.generation);
        assert_eq!(text(&frozen), original_text);
        assert_eq!((frozen.cols, frozen.rows), (80, 12));
        assert!(!frozen.cursor);
    }

    #[test]
    fn admission_counts_the_request_owning_the_save_barrier() {
        let mut f = Fixture::new();
        f.server.reactor.as_mut().unwrap().servicing = 1;
        let readers: Vec<_> = (0..32)
            .map(|_| wire::connect(&f.server.state).unwrap())
            .collect();
        f.server.poll_io(0).unwrap();
        let reactor = f.server.reactor.as_ref().unwrap();
        assert_eq!(reactor.clients.len(), 31);
        assert_eq!(reactor.clients.len() + reactor.servicing, 32);
        drop(readers);
    }

    #[test]
    fn barrier_allowlist_excludes_every_durable_and_coordination_command() {
        for request in [
            "metadata\t1\t00",
            "rename\t1\t00",
            "list",
            "coord\tget_context",
            "stop",
            "refresh",
            "input-extra",
            "snapshot",
            "snapshot-panes",
            "snapshot-links",
        ] {
            assert!(
                !serviceable(&wire::frame(request.as_bytes()), true),
                "{request}"
            );
        }
        for request in ["ping", "input\t1\trun\t78", "text\t1\trun\t78"] {
            assert!(serviceable(&wire::frame(request.as_bytes()), false));
        }
        for request in ["watch", "watch-panes", "watch-links"] {
            assert!(!serviceable(&wire::frame(request.as_bytes()), false));
            assert!(serviceable(&wire::frame(request.as_bytes()), true));
        }
        assert!(!serviceable(&wire::frame(b"ping")[..6], true));
        assert!(serviceable(
            &((wire::MAX_REQUEST + 1) as u32).to_be_bytes(),
            true
        ));
    }
}
