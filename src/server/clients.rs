//! Keep one bounded snapshot per watcher, allowing a temporarily stalled UI.
use super::*;

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

    pub(super) fn flush_output(&mut self, now: Instant) {
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
