//! Four bounded file workers keep filesystem stalls off the supervisor loop.
//! A short in-memory gate orders commit-start against exact pane mutations;
//! workers never perform filesystem operations while holding that gate.
use super::*;
use crate::{
    remote_files::{Sink, Source},
    remote_services as service,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender, TryRecvError},
};

#[derive(Clone)]
struct Origin {
    epoch: String,
    workspace: u64,
    tab: u64,
    run: String,
    revision: u64,
    attachment: bool,
}
enum Body {
    Read(Source),
    Write(Sink),
    Attach {
        sink: Sink,
        reservation: crate::attachments::FileDrop,
    },
}
enum Job {
    Read(u64),
    Write(u64, Vec<u8>),
    Prepare,
    Commit,
}
enum Reply {
    Data(Vec<u8>),
    Prepared,
    Finished(Vec<u8>),
}
struct Worker {
    jobs: SyncSender<Job>,
    results: Receiver<io::Result<Reply>>,
    cancelled: Arc<AtomicBool>,
    committing: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
struct Transfer {
    token: String,
    origin: Origin,
    worker: Worker,
    busy: bool,
    touched: Instant,
}
#[derive(Default)]
pub(super) struct Transfers {
    pending: Vec<Transfer>,
    retired: Vec<std::thread::JoinHandle<()>>,
    gate: Arc<Mutex<()>>,
}
impl Transfers {
    pub(super) fn command_gate(&self) -> Arc<Mutex<()>> {
        Arc::clone(&self.gate)
    }
    fn retire(&mut self, mut transfer: Transfer) {
        transfer.worker.cancelled.store(true, Ordering::Release);
        if let Some(thread) = transfer.worker.thread.take() {
            self.retired.push(thread);
        }
    }
    pub(super) fn cancel_for_refresh(&mut self) -> io::Result<()> {
        let pending = std::mem::take(&mut self.pending);
        for transfer in pending {
            self.retire(transfer);
        }
        self.retired.retain(|t| !t.is_finished());
        if self.retired.is_empty() {
            Ok(())
        } else {
            Err(invalid(
                "File workers are cleaning up; retry refresh after their filesystem responds",
            ))
        }
    }
}
fn pending(token: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"token":token,"pending":true})).unwrap()
}
fn complete(bytes: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"pending":false,"data":wire::hex(bytes)})).unwrap()
}
fn start_worker(
    upload: bool,
    path: PathBuf,
    size: u64,
    token: String,
    gate: Arc<Mutex<()>>,
) -> io::Result<Worker> {
    let chosen = path.clone();
    start_worker_with(path, size, gate, move || {
        if upload {
            Ok(Body::Write(Sink::new(&chosen, size, &token)?))
        } else {
            Ok(Body::Read(Source::open(&chosen)?))
        }
    })
}
fn start_worker_with(
    path: PathBuf,
    size: u64,
    gate: Arc<Mutex<()>>,
    open: impl FnOnce() -> io::Result<Body> + Send + 'static,
) -> io::Result<Worker> {
    let (jobs, commands) = mpsc::sync_channel(1);
    let (output, results) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancelled);
    let committing = Arc::new(AtomicBool::new(false));
    let commit = Arc::clone(&committing);
    let thread = std::thread::Builder::new()
        .name("flere-file".into())
        .spawn(move || {
            let mut body = match open() {
                Ok(body) => body,
                Err(error) => {
                    let _ = output.send(Err(error));
                    return;
                }
            };
            if stop.load(Ordering::Acquire) {
                return;
            }
            let (name, size) = match &body {
                Body::Read(source) => (source.name.clone(), source.size),
                Body::Write(_) | Body::Attach { .. } => (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    size,
                ),
            };
            if output
                .send(Ok(Reply::Data(service::metadata(&name, size).unwrap())))
                .is_err()
            {
                return;
            }
            while let Ok(job) = commands.recv() {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let result = match job {
                    Job::Read(offset) => match &mut body {
                        Body::Read(source) => source.read(offset).map(Reply::Data),
                        _ => Err(invalid("invalid file direction")),
                    },
                    Job::Write(offset, bytes) => match &mut body {
                        Body::Write(sink) | Body::Attach { sink, .. } => sink
                            .write(offset, &bytes)
                            .map(|()| Reply::Data(b"ok".to_vec())),
                        _ => Err(invalid("invalid file direction")),
                    },
                    Job::Prepare => match &mut body {
                        Body::Read(source) => source
                            .finish()
                            .map(|()| Reply::Finished(b"download complete".to_vec())),
                        Body::Write(sink) | Body::Attach { sink, .. } => {
                            sink.prepare().map(|()| Reply::Prepared)
                        }
                    },
                    Job::Commit => {
                        // The supervisor holds this gate while changing the exact origin
                        // and cancelling stale workers. Claiming commit is its linearization
                        // point; the potentially blocking publication runs after unlock.
                        {
                            let Ok(_guard) = gate.lock() else {
                                break;
                            };
                            if stop.load(Ordering::Acquire) {
                                break;
                            }
                            commit.store(true, Ordering::Release);
                        }
                        let result = match body {
                            Body::Write(sink) => sink
                                .publish()
                                .map(|p| Reply::Finished(p.to_string_lossy().as_bytes().to_vec())),
                            Body::Attach { sink, reservation } => {
                                let result = sink.publish().map_err(|error| {
                                    io::Error::new(error.kind(), format!(
                                        "Attachment publication failed; any completed file is retained at {} and was not pasted: {error}",
                                        reservation.destination.display()
                                    ))
                                });
                                drop(reservation);
                                result.map(|p| {
                                    Reply::Finished(p.to_string_lossy().as_bytes().to_vec())
                                })
                            }
                            _ => Err(invalid("invalid file commit")),
                        };
                        let _ = output.send(result);
                        return;
                    }
                };
                let finished = matches!(result, Ok(Reply::Finished(_))) || result.is_err();
                if stop.load(Ordering::Acquire) || output.send(result).is_err() || finished {
                    break;
                }
            }
        })?;
    Ok(Worker {
        jobs,
        results,
        cancelled,
        committing,
        thread: Some(thread),
    })
}
impl Server {
    pub(super) fn expire_file_transfers(&mut self) {
        self.file_transfers.retired.retain(|t| !t.is_finished());
        let pending = std::mem::take(&mut self.file_transfers.pending);
        for transfer in pending {
            if transfer.touched.elapsed() < Duration::from_secs(service::IDLE_SECONDS)
                && self.validate_file_origin(&transfer.origin).is_ok()
            {
                self.file_transfers.pending.push(transfer);
            } else {
                self.file_transfers.retire(transfer);
            }
        }
    }
    fn validate_file_origin(&self, origin: &Origin) -> io::Result<()> {
        if origin.attachment {
            return self.attachment_target(
                &origin.epoch,
                origin.workspace,
                origin.tab,
                &origin.run,
                origin.revision,
            );
        }
        self.validate_pane_origin(
            &origin.epoch,
            origin.workspace,
            origin.tab,
            &origin.run,
            origin.revision,
            true,
        )
    }
    pub(super) fn file_command(&mut self, p: &[&str]) -> io::Result<Vec<u8>> {
        self.expire_file_transfers();
        if p.first() == Some(&"file-begin") {
            if !(8..=10).contains(&p.len()) {
                return Err(invalid("invalid file transfer arguments"));
            }
            let (upload, attachment) = match (p[1], p.len()) {
                ("upload", 9) => (true, false),
                ("download", 8) => (false, false),
                ("attach", 10) => (true, true),
                _ => return Err(invalid("invalid file direction")),
            };
            let number = |i: usize| {
                p[i].parse::<u64>()
                    .map_err(|_| invalid("invalid file identity"))
            };
            let origin = Origin {
                epoch: p[2].into(),
                workspace: number(3)?,
                tab: number(4)?,
                run: p[5].into(),
                revision: number(6)?,
                attachment,
            };
            if attachment {
                self.consume_attachment_ticket(
                    p[9],
                    &origin.epoch,
                    origin.workspace,
                    origin.tab,
                    &origin.run,
                    origin.revision,
                )?;
            }
            self.validate_file_origin(&origin)?;
            if self.file_transfers.pending.len() + self.file_transfers.retired.len() >= 4 {
                return Err(invalid(
                    "four file workers are already pending or cleaning up",
                ));
            }
            let size = if upload { number(8)? } else { 0 };
            if size > service::FILE_LIMIT {
                return Err(invalid("file exceeds 128 MiB"));
            }
            let value = wire::text(p[7])?;
            if attachment {
                service::name(&value)?;
            }
            let path = PathBuf::from(&value);
            let token = os::nonce()?;
            let worker = if attachment {
                let nonce = token.clone();
                start_worker_with(path, size, self.file_transfers.command_gate(), move || {
                    let reservation = crate::attachments::FileDrop::new(&value, size, &nonce)?;
                    let sink = Sink::new(&reservation.destination, size, &nonce)?;
                    Ok(Body::Attach { sink, reservation })
                })?
            } else {
                start_worker(
                    upload,
                    path,
                    size,
                    token.clone(),
                    self.file_transfers.command_gate(),
                )?
            };
            self.file_transfers.pending.push(Transfer {
                token: token.clone(),
                origin,
                worker,
                busy: true,
                touched: Instant::now(),
            });
            return Ok(pending(&token));
        }
        let operation = *p.first().ok_or_else(|| invalid("missing file operation"))?;
        let token = *p.get(1).ok_or_else(|| invalid("missing file identity"))?;
        let index=self.file_transfers.pending.iter().position(|t|t.token==token).ok_or_else(||invalid("File transfer no longer active; an already-started publication may still complete"))?;
        let mut transfer = self.file_transfers.pending.remove(index);
        let result = (|| -> io::Result<(Vec<u8>, bool)> {
            self.validate_file_origin(&transfer.origin)?;
            if operation == "file-cancel" && p.len() == 2 {
                let message = if transfer.worker.committing.load(Ordering::Acquire) {
                    "Commit already started; destination publication may complete"
                } else {
                    "File transfer cancelled; private staging cleanup is pending"
                };
                return Ok((message.as_bytes().to_vec(), false));
            }
            if operation == "file-poll" && p.len() == 2 {
                if !transfer.busy {
                    return Err(invalid("no file operation is pending"));
                }
                match transfer.worker.results.try_recv() {
                    Ok(Ok(Reply::Data(bytes))) => {
                        transfer.touched = Instant::now();
                        transfer.busy = false;
                        return Ok((complete(&bytes), true));
                    }
                    Ok(Ok(Reply::Prepared)) => {
                        transfer.touched = Instant::now();
                        self.validate_file_origin(&transfer.origin)?;
                        transfer
                            .worker
                            .jobs
                            .try_send(Job::Commit)
                            .map_err(io::Error::other)?;
                        return Ok((pending(token), true));
                    }
                    Ok(Ok(Reply::Finished(bytes))) => {
                        if transfer.origin.attachment {
                            let path = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
                            let paste =
                                [b"\x1b[200~".as_slice(), &bytes, b"\x1b[201~".as_slice()].concat();
                            self.paste_attachment(
                                &transfer.origin.epoch,
                                transfer.origin.workspace,
                                transfer.origin.tab,
                                &transfer.origin.run,
                                transfer.origin.revision,
                                &paste,
                            )
                            .map_err(|error| {
                                invalid(&format!(
                                    "Attachment retained at {path}; path was not pasted: {error}"
                                ))
                            })?;
                            let message = format!(
                                "File path paste queued for the chat draft without Enter: {path}. Appearance depends on the chat and file type."
                            );
                            return Ok((complete(message.as_bytes()), false));
                        }
                        if transfer.worker.committing.load(Ordering::Acquire) {
                            self.audit(
                                "file-upload",
                                transfer.origin.workspace,
                                &String::from_utf8_lossy(&bytes),
                            )?;
                        }
                        return Ok((complete(&bytes), false));
                    }
                    Ok(Err(error)) => return Err(error),
                    Err(TryRecvError::Empty) => return Ok((pending(token), true)),
                    Err(TryRecvError::Disconnected) => {
                        return Err(invalid("file worker ended before completion"));
                    }
                }
            }
            if transfer.busy {
                return Err(invalid("wait for the pending file operation"));
            }
            let job = match operation {
                "file-read" if p.len() == 3 => {
                    Job::Read(p[2].parse().map_err(|_| invalid("invalid file offset"))?)
                }
                "file-write" if p.len() == 4 => {
                    let bytes = wire::unhex(p[3])?;
                    if bytes.is_empty() || bytes.len() > service::CHUNK {
                        return Err(invalid("file chunk exceeds bound"));
                    }
                    Job::Write(
                        p[2].parse().map_err(|_| invalid("invalid file offset"))?,
                        bytes,
                    )
                }
                "file-finish" if p.len() == 2 => Job::Prepare,
                _ => return Err(invalid("invalid file operation")),
            };
            transfer
                .worker
                .jobs
                .try_send(job)
                .map_err(io::Error::other)?;
            transfer.busy = true;
            transfer.touched = Instant::now();
            Ok((pending(token), true))
        })();
        match result {
            Ok((reply, true)) => {
                self.file_transfers.pending.push(transfer);
                Ok(reply)
            }
            Ok((reply, false)) => {
                self.file_transfers.retire(transfer);
                Ok(reply)
            }
            Err(error) => {
                self.file_transfers.retire(transfer);
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            command.args(["-c","printf 'READY\\n'; while IFS= read -r value; do printf 'REPLY:%s\\n' \"$value\"; done"])
                .env("HOME",&root).env("ENV","");
            let (master, child) = os::spawn_command_pty(&root, &mut command, 80, 24).unwrap();
            server.active = 1;
            server.next = 3;
            server.workspaces.push(Workspace {
                id: 1,
                name: "Worker fixture".into(),
                cwd: root.clone(),
                selected: 2,
                meta: CardMeta::default(),
                split: None,
                tabs: vec![Session {
                    id: 2,
                    run: "owned-run".into(),
                    master,
                    child,
                    term: Terminal::new(80, 24),
                    input: VecDeque::new(),
                    alive: true,
                    ended: false,
                    title: "probe".into(),
                    kind: "shell".into(),
                    path: String::new(),
                    native: None,
                    working: false,
                }],
            });
            Self { root, server }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for workspace in &mut self.server.workspaces {
                for tab in &mut workspace.tabs {
                    os::hangup(tab.master.as_raw_fd(), &mut tab.child);
                }
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn blocked_file_worker_keeps_supervisor_terminal_io_and_cancel_responsive() {
        let mut f = Fixture::new();
        let path = f.root.join("blocked.txt");
        let target = path.clone();
        let (release, blocked) = mpsc::sync_channel::<()>(1);
        let (entered, started) = mpsc::sync_channel(1);
        let worker = start_worker_with(
            path.clone(),
            4,
            f.server.file_transfers.command_gate(),
            move || {
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                Ok(Body::Write(Sink::new(&target, 4, "abcdef")?))
            },
        )
        .unwrap();
        let origin = Origin {
            epoch: f.server.epoch.clone(),
            workspace: 1,
            tab: 2,
            run: "owned-run".into(),
            revision: 0,
            attachment: false,
        };
        f.server.file_transfers.pending.push(Transfer {
            token: "blocked".into(),
            origin,
            worker,
            busy: true,
            touched: Instant::now(),
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        let start = Instant::now();
        let poll = f.server.command("file-poll\tblocked").unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&poll).unwrap()["pending"],
            true
        );
        f.server
            .command(&format!(
                "input\t2\towned-run\t{}",
                wire::hex(b"still draining\n")
            ))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            f.server.drain();
            if f.server.workspaces[0].tabs[0]
                .term
                .capture(24)
                .contains("REPLY:still draining")
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a stalled file worker blocked real PTY input/output"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let cancelled = f.server.command("file-cancel\tblocked").unwrap();
        assert!(String::from_utf8_lossy(&cancelled).contains("cancelled"));
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(!path.exists());
        // A blocked system call cannot be killed safely; its slot stays reserved
        // until it resumes, notices cancellation, and cleans its private staging.
        assert_eq!(f.server.file_transfers.retired.len(), 1);
        assert!(
            f.server.file_transfers.cancel_for_refresh().is_err(),
            "refresh must not abandon a blocked worker's staging cleanup"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !f
            .server
            .file_transfers
            .retired
            .iter()
            .all(|t| t.is_finished())
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!path.exists());
        assert!(!f.root.join(".flere-transfer-abcdef.part").exists());
        assert!(f.server.file_transfers.cancel_for_refresh().is_ok());
    }
    #[test]
    fn commit_claim_waits_only_for_origin_gate_and_cancellation_wins_before_claim() {
        let f = Fixture::new();
        let path = f.root.join("not-published.txt");
        let gate = Arc::new(Mutex::new(()));
        let mut worker =
            start_worker(true, path.clone(), 0, "123abc".into(), Arc::clone(&gate)).unwrap();
        assert!(matches!(
            worker
                .results
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap(),
            Reply::Data(_)
        ));
        worker.jobs.send(Job::Prepare).unwrap();
        assert!(matches!(
            worker
                .results
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap(),
            Reply::Prepared
        ));
        let guard = gate.lock().unwrap();
        worker.jobs.send(Job::Commit).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!worker.committing.load(Ordering::Acquire));
        worker.cancelled.store(true, Ordering::Release);
        drop(guard);
        worker.thread.take().unwrap().join().unwrap();
        assert!(!path.exists());
        assert!(!f.root.join(".flere-transfer-123abc.part").exists());
    }
    fn mark_native(f: &mut Fixture) {
        let tab = &mut f.server.workspaces[0].tabs[0];
        tab.term.bracketed_paste = true;
        tab.native = Some(crate::native::HostSpec {
            id: tab.id,
            run: tab.run.clone(),
            harness: "codex".into(),
            argv: Vec::new(),
            cwd: f.root.clone(),
            shell: "/bin/sh".into(),
            conversation: String::new(),
            dispatch: String::new(),
        });
    }
    fn attachment_transfer(f: &Fixture, worker: Worker) -> Transfer {
        Transfer {
            token: "attachment".into(),
            origin: Origin {
                epoch: f.server.epoch.clone(),
                workspace: 1,
                tab: 2,
                run: "owned-run".into(),
                revision: 0,
                attachment: true,
            },
            worker,
            busy: true,
            touched: Instant::now(),
        }
    }
    #[test]
    fn attachment_cancel_during_claimed_publication_retains_file_and_never_pastes() {
        let mut f = Fixture::new();
        mark_native(&mut f);
        let destination = f.root.join("late-publication.txt");
        let mut sink = Sink::new(&destination, 0, "abcdef").unwrap();
        sink.prepare().unwrap();
        let (jobs, _commands) = mpsc::sync_channel(1);
        let (output, results) = mpsc::sync_channel(1);
        let (release, stalled) = mpsc::sync_channel::<()>(1);
        // Model a filesystem stall after the worker has claimed commit. Cancel
        // cannot recall this syscall, but must discard its later paste result.
        let thread = std::thread::spawn(move || {
            stalled.recv().unwrap();
            let path = sink.publish().unwrap();
            let _ = output.send(Ok(Reply::Finished(
                path.to_str().unwrap().as_bytes().to_vec(),
            )));
        });
        let worker = Worker {
            jobs,
            results,
            cancelled: Arc::new(AtomicBool::new(false)),
            committing: Arc::new(AtomicBool::new(true)),
            thread: Some(thread),
        };
        let transfer = attachment_transfer(&f, worker);
        f.server.file_transfers.pending.push(transfer);
        let response = f.server.command("file-cancel\tattachment").unwrap();
        assert!(
            String::from_utf8(response)
                .unwrap()
                .contains("may complete")
        );
        assert!(!destination.exists());
        release.send(()).unwrap();
        let thread = f.server.file_transfers.retired.pop().unwrap();
        thread.join().unwrap();
        assert!(destination.exists());
        assert!(!f.root.join(".flere-transfer-abcdef.part").exists());
        assert!(f.server.command("file-poll\tattachment").is_err());
        assert!(f.server.workspaces[0].tabs[0].input.is_empty());
    }
    #[test]
    fn published_attachment_with_full_input_queue_is_retained_without_retry() {
        let mut f = Fixture::new();
        mark_native(&mut f);
        let path = f.root.join("retained.txt");
        let worker = start_worker(
            true,
            path.clone(),
            0,
            "123abc".into(),
            f.server.file_transfers.command_gate(),
        )
        .unwrap();
        worker
            .results
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        worker.jobs.send(Job::Prepare).unwrap();
        worker
            .results
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        worker.jobs.send(Job::Commit).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !worker.thread.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(path.exists());
        let transfer = attachment_transfer(&f, worker);
        f.server.file_transfers.pending.push(transfer);
        f.server.workspaces[0].tabs[0].input = vec![b'x'; 131072].into();
        let error = f
            .server
            .command("file-poll\tattachment")
            .unwrap_err()
            .to_string();
        assert!(error.contains("retained") && error.contains(path.to_str().unwrap()));
        assert_eq!(f.server.workspaces[0].tabs[0].input.len(), 131072);
        f.server.workspaces[0].tabs[0].input.clear();
        assert!(f.server.command("file-poll\tattachment").is_err());
        assert!(f.server.workspaces[0].tabs[0].input.is_empty());
        assert!(path.exists());
    }
    #[test]
    fn literal_tickets_preserve_shell_editor_paste_modes_and_reject_retargeting() {
        let mut f = Fixture::new();
        let value = b"'/a local/file.txt'";
        let bytes = [b"\x1b[200~".as_slice(), value, b"\x1b[201~".as_slice()].concat();
        let issue = format!("literal-ticket\t{}\t1\t2\towned-run\t0", f.server.epoch);
        for kind in ["shell", "file"] {
            for bracketed in [false, true] {
                f.server.workspaces[0].tabs[0].kind = kind.into();
                f.server.workspaces[0].tabs[0].term.bracketed_paste = bracketed;
                let token = String::from_utf8(f.server.command(&issue).unwrap()).unwrap();
                let paste = format!("literal-paste\t{token}\t{}", wire::hex(&bytes));
                f.server.command(&paste).unwrap();
                let actual = f.server.workspaces[0].tabs[0]
                    .input
                    .drain(..)
                    .collect::<Vec<_>>();
                assert_eq!(actual, if bracketed { bytes.as_slice() } else { value });
                assert!(f.server.command(&paste).is_err());
            }
        }
        let token = String::from_utf8(f.server.command(&issue).unwrap()).unwrap();
        f.server.active = 0;
        f.server.changed();
        f.server.active = 1;
        f.server.changed();
        assert!(
            f.server
                .command(&format!("literal-paste\t{token}\t{}", wire::hex(&bytes)))
                .is_err()
        );
        assert!(f.server.workspaces[0].tabs[0].input.is_empty());
        let token = String::from_utf8(f.server.command(&issue).unwrap()).unwrap();
        assert!(
            f.server
                .command(&format!("attachment-text\t{token}\t{}", wire::hex(&bytes)))
                .is_err()
        );
        assert!(
            f.server
                .command(&format!("literal-paste\t{token}\t{}", wire::hex(&bytes)))
                .is_err()
        );
        let token = String::from_utf8(f.server.command(&issue).unwrap()).unwrap();
        f.server.workspaces[0].tabs[0].ended = true;
        assert!(
            f.server
                .command(&format!("literal-paste\t{token}\t{}", wire::hex(&bytes)))
                .is_err()
        );
        assert!(f.server.workspaces[0].tabs[0].input.is_empty());
        assert!(!f.root.join(".cache/flere/chat-attachments").exists());
    }
}
