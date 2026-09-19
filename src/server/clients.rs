//! Keep one bounded snapshot per watcher, allowing a temporarily stalled UI.
use super::*;

/// Build only projections that a live watcher can accept. A partial frame cannot
/// be replaced; the run loop keeps dirty set until it can send the latest state.
pub(super) fn queue_watch_snapshots<S: std::borrow::Borrow<Snapshot>>(
    clients: &mut [Client],
    snapshot: impl FnOnce() -> S,
) {
    let ready = |c: &&Client| c.watch && !c.done && c.offset == 0;
    let projection = |c: &Client| if c.links { 2 } else { usize::from(c.panes) };
    let mut requested = [false; 3];
    for client in clients.iter().filter(ready) {
        requested[projection(client)] = true;
    }
    if requested == [false; 3] {
        return;
    }
    let frames = snapshot().borrow().watch_frames(requested);
    for client in clients
        .iter_mut()
        .filter(|c| c.watch && !c.done && c.offset == 0)
    {
        let frame = frames[projection(client)]
            .as_ref()
            .expect("requested watch projection");
        client.queue_snapshot(frame, Instant::now());
    }
}

impl Client {
    pub(super) fn output_timeout(watch: bool) -> Duration {
        // A UI can block on SSH/terminal output or local filesystem work. Its
        // stream is long-lived; the two-second request budget is inappropriate.
        Duration::from_secs(if watch { 30 } else { 2 })
    }

    pub(super) fn queue_snapshot(&mut self, frame: &[u8], now: Instant) {
        if self.output.is_empty() {
            self.output = frame.to_vec();
            self.offset = 0;
            self.deadline = now + Self::output_timeout(true);
        } else if self.offset == 0 {
            // Replacing an unsent frame neither grows the queue nor counts as
            // transport progress. A reader that never resumes still expires.
            self.output = frame.to_vec();
        }
        // A partially sent frame must finish before a fresh snapshot is queued.
    }

    pub(super) fn expire_waiting(&mut self, now: Instant) {
        if self.done
            || self.watch
            || !self.output.is_empty()
            || now <= self.deadline
            || self.input.len() < 4
        {
            return;
        }
        let size = u32::from_be_bytes(self.input[..4].try_into().unwrap()) as usize;
        if size > wire::MAX_REQUEST || self.input.len() < size + 4 {
            return;
        }
        // Only undispatched requests are in this list with empty output. The
        // active mutation owns its removed socket until its final result.
        // Clear the exact queued frame before replying; it cannot run later.
        self.input.clear();
        self.output = wire::frame(b"!Request not started: supervisor is busy; no command was executed. Try again after the pending save finishes.");
        self.offset = 0;
        self.deadline = now + Self::output_timeout(false);
        crate::diagnostics::record("request-not-started", "queue deadline expired");
    }

    pub(super) fn flush_output(&mut self, now: Instant) {
        self.expire_waiting(now);
        if self.offset < self.output.len() {
            match self.stream.write(&self.output[self.offset..]) {
                Ok(0) => self.done = true,
                Ok(n) => {
                    self.offset += n;
                    if self.watch {
                        self.deadline = now + Self::output_timeout(true);
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => {
                    crate::diagnostics::error("client-write-error", &e);
                    self.done = true;
                }
            }
            if self.offset == self.output.len() {
                self.output.clear();
                self.offset = 0;
                if !self.watch {
                    self.done = true;
                }
            }
        }
        if !self.done && (!self.watch || !self.output.is_empty()) && now > self.deadline {
            crate::diagnostics::record(
                "client-timeout",
                &format!(
                    "watch={} pending_bytes={} sent_bytes={}",
                    self.watch,
                    self.output.len().saturating_sub(self.offset),
                    self.offset
                ),
            );
            self.done = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(watch: bool, now: Instant) -> (Client, UnixStream) {
        let (stream, reader) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        reader.set_nonblocking(true).unwrap();
        (
            Client {
                stream,
                input: Vec::new(),
                output: Vec::new(),
                offset: 0,
                watch,
                panes: false,
                links: false,
                build: None,
                done: false,
                deadline: now + Client::output_timeout(watch),
            },
            reader,
        )
    }

    fn drain(reader: &mut UnixStream) {
        let mut buffer = [0; 65536];
        loop {
            match reader.read(&mut buffer) {
                Ok(n) => assert!(n > 0),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => panic!("reader failed: {e}"),
            }
        }
    }

    fn snapshot(generation: u64) -> Snapshot {
        Snapshot {
            epoch: "a".repeat(32),
            generation,
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
            cells: vec![crate::terminal::Cell::default(); 4],
            split: None,
        }
    }

    #[test]
    fn snapshot_work_requires_a_reader_that_can_accept_a_frame() {
        queue_watch_snapshots(&mut [], || -> Snapshot { panic!("detached snapshot") });
        let now = Instant::now();
        let (request, _r1) = client(false, now);
        let (mut closed, _r2) = client(true, now);
        closed.done = true;
        let (mut partial, _r3) = client(true, now);
        partial.output = wire::frame(&snapshot(1).encode());
        partial.offset = 7;
        let retained = partial.output.clone();
        let mut clients = [request, closed, partial];
        queue_watch_snapshots(&mut clients, || -> Snapshot {
            panic!("no ready snapshot reader")
        });
        assert_eq!(clients[2].output, retained);
        assert_eq!(clients[2].offset, 7);
        assert_eq!(clients[2].deadline, now + Client::output_timeout(true));
        // Once its partial frame drains, a slow reader must receive current state.
        clients[2].output.clear();
        clients[2].offset = 0;
        queue_watch_snapshots(&mut clients, || snapshot(9));
        let bytes = wire::read_frame(&mut clients[2].output.as_slice()).unwrap();
        assert_eq!(Snapshot::decode(&bytes).unwrap().generation, 9);
    }

    #[test]
    fn mixed_watchers_get_their_projection_and_unsent_frames_coalesce() {
        let now = Instant::now();
        let mut readers = Vec::new();
        let mut clients = Vec::new();
        for projection in [0, 1, 2, 2] {
            let (mut c, reader) = client(true, now);
            c.panes = projection > 0;
            c.links = projection == 2;
            c.output = b"superseded unsent frame".to_vec();
            clients.push(c);
            readers.push(reader);
        }
        let mut state = snapshot(3);
        state.cells[0].link = crate::terminal::hyperlinks::Hyperlink::new("https://example.com");
        queue_watch_snapshots(&mut clients, || state);
        for (c, version) in clients.iter().zip([4, 5, 6, 6]) {
            let bytes = wire::read_frame(&mut c.output.as_slice()).unwrap();
            assert_eq!(bytes[0], version);
            let decoded = Snapshot::decode(&bytes).unwrap();
            assert_eq!(decoded.generation, 3);
            assert_eq!(decoded.cells[0].link.is_some(), version == 6);
            assert_eq!(c.deadline, now + Client::output_timeout(true));
        }
        assert_eq!(clients[2].output, clients[3].output);
    }

    #[test]
    fn watch_progress_renews_the_stall_budget_without_corrupting_partial_frames() {
        let now = Instant::now();
        let (mut c, mut reader) = client(true, now);
        let frame = vec![b'a'; 1024 * 1024];
        c.queue_snapshot(&frame, now);
        c.flush_output(now);
        assert!(c.offset > 0 && c.offset < frame.len());
        c.queue_snapshot(b"newer frame", now + Duration::from_secs(3));
        assert_eq!(c.output, frame);
        c.flush_output(now + Duration::from_secs(3));
        assert!(!c.done, "a temporary UI stall must not detach its watch");

        // Progress can resume after the old frame deadline. The watchdog is
        // about an unresponsive reader, not the total duration of a large frame.
        drain(&mut reader);
        c.flush_output(now + Duration::from_secs(31));
        assert!(!c.done);
        assert_eq!(c.deadline, now + Duration::from_secs(61));
        c.flush_output(now + Duration::from_secs(62));
        assert!(c.done, "a reader that stops making progress must expire");
    }

    #[test]
    fn coalescing_unsent_frames_does_not_renew_the_stall_budget() {
        let now = Instant::now();
        let (mut c, _reader) = client(true, now);
        // Fill only the kernel buffer, leaving the next queued frame unsent.
        c.output = vec![b'x'; 1024 * 1024];
        c.flush_output(now);
        assert!(c.offset > 0 && c.offset < c.output.len());
        c.output.clear();
        c.offset = 0;
        c.queue_snapshot(b"old snapshot", now);
        let deadline = c.deadline;
        c.queue_snapshot(b"latest snapshot", now + Duration::from_secs(29));
        assert_eq!(c.output, b"latest snapshot");
        assert_eq!(c.deadline, deadline);
        c.flush_output(now + Duration::from_secs(31));
        assert!(c.done);
    }

    #[test]
    fn expired_complete_request_gets_a_definite_not_started_reply_and_cannot_run_later() {
        let now = Instant::now();
        let (mut request, mut reader) = client(false, now);
        request.input = wire::frame(b"rename\t1\t6e6577");
        request.expire_waiting(now + Duration::from_secs(3));
        assert!(request.input.is_empty());
        assert!(!request.done);
        request.flush_output(now + Duration::from_secs(3));
        let reply = wire::read_frame(&mut reader).unwrap();
        assert!(reply.starts_with(b"!Request not started:"));
        assert!(request.done);
    }

    #[test]
    fn partial_final_response_is_never_rewritten_as_a_not_started_receipt() {
        let now = Instant::now();
        let (mut request, _reader) = client(false, now);
        request.output = wire::frame(&vec![b'x'; 1024 * 1024]);
        request.flush_output(now);
        assert!(request.offset > 0 && request.offset < request.output.len());
        let original = request.output.clone();
        request.flush_output(now + Duration::from_secs(3));
        assert!(request.done);
        assert_eq!(request.output, original);
    }

    #[test]
    fn incomplete_requests_still_expire_but_idle_watches_do_not() {
        let now = Instant::now();
        let (mut request, _reader) = client(false, now);
        request.input.push(0);
        request.flush_output(now + Duration::from_secs(3));
        assert!(request.done);
        let (mut watch, _reader) = client(true, now);
        watch.flush_output(now + Duration::from_secs(60));
        assert!(!watch.done);
    }
}
