//! One ordered output owner per frontend. A blocked terminal must not block input.
//!
//! Render only with no pending output: skipped renders retain their dirty flag,
//! so the next diff is against the last admitted frame. Background transfers
//! yield as well. Control packets keep their order and have a hard byte bound.
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
};

// Includes the batch currently blocked in write_all, not just queued batches.
// A single frame can contain styled text, links, clipboard and inline graphics.
const LIMIT: usize = 16 * 1024 * 1024;
const BATCH_LIMIT: usize = 1024;
// Only attachment teardown may use this reserve. Normal frames, transfers and
// replies cannot consume it. The fixed destructor set needs at most six batches.
const CLEANUP_LIMIT: usize = 64 * 1024;
const CLEANUP_BATCH_LIMIT: usize = 16;
struct Batch {
    bytes: Box<[u8]>,
    cleanup: bool,
}
#[derive(Default)]
struct State {
    queue: VecDeque<Batch>,
    batches: usize,
    pending: usize,
    cleanup_batches: usize,
    cleanup_pending: usize,
    peak: usize,
    closing: bool,
    rejected: bool,
    error: Option<io::Error>,
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    limit: usize,
}
thread_local! {
    // Only the frontend thread submits bytes; the writer never calls UI code.
    static CURRENT: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) };
}
fn failure(state: &State) -> io::Result<()> {
    if let Some(error) = &state.error {
        Err(io::Error::new(error.kind(), error.to_string()))
    } else if state.rejected {
        Err(io::Error::other("frontend output exceeds bounded backlog"))
    } else {
        Ok(())
    }
}
impl Shared {
    fn submit(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.admit(bytes, false)
    }
    fn cleanup(&self, bytes: Vec<u8>) -> io::Result<()> {
        self.admit(bytes, true)
    }
    fn admit(&self, bytes: Vec<u8>, cleanup: bool) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let mut state = self.state.lock().unwrap();
        if state.closing || state.error.is_some() {
            failure(&state)?;
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "frontend output closed",
            ));
        }
        let (pending, batches, limit, batch_limit) = if cleanup {
            (
                state.cleanup_pending,
                state.cleanup_batches,
                CLEANUP_LIMIT,
                CLEANUP_BATCH_LIMIT,
            )
        } else {
            (
                state.pending - state.cleanup_pending,
                state.batches - state.cleanup_batches,
                self.limit,
                BATCH_LIMIT,
            )
        };
        if bytes.len() > limit.saturating_sub(pending) || batches == batch_limit {
            // Reject atomically. Keep draining admitted bytes; the original
            // error remains visible after ordered teardown uses its reserve.
            state.rejected = true;
            return failure(&state);
        }
        state.pending += bytes.len();
        state.batches += 1;
        if cleanup {
            state.cleanup_pending += bytes.len();
            state.cleanup_batches += 1;
        }
        state.peak = state.peak.max(state.pending);
        state.queue.push_back(Batch {
            bytes: bytes.into_boxed_slice(),
            cleanup,
        });
        self.changed.notify_one();
        Ok(())
    }
    fn ready(&self) -> bool {
        self.state.lock().unwrap().pending == 0
    }
    fn run(&self, mut writer: impl Write, mut notify: UnixStream) {
        loop {
            let Batch { bytes, cleanup } = {
                let mut state = self.state.lock().unwrap();
                while state.queue.is_empty() && !state.closing {
                    state = self.changed.wait(state).unwrap();
                }
                match state.queue.pop_front() {
                    Some(bytes) => bytes,
                    None => break,
                }
            };
            let started = std::time::Instant::now();
            let result = writer.write_all(&bytes).and_then(|()| writer.flush());
            crate::diagnostics::slow("terminal-write", started);
            let mut state = self.state.lock().unwrap();
            state.pending -= bytes.len();
            state.batches -= 1;
            if cleanup {
                state.cleanup_pending -= bytes.len();
                state.cleanup_batches -= 1;
            }
            if let Err(error) = result {
                state.error = Some(error);
                state.queue.clear();
                state.pending = 0;
                state.batches = 0;
                state.cleanup_pending = 0;
                state.cleanup_batches = 0;
                let _ = notify.write(&[1]);
                break;
            }
            if state.pending == 0 {
                // Nonblocking notification, never wait for the UI while holding
                // this lock. A full socket already makes the poll fd readable.
                let _ = notify.write(&[1]);
            }
        }
    }
}

pub(super) struct Output {
    shared: Arc<Shared>,
    wake: UnixStream,
    worker: Option<JoinHandle<()>>,
}
impl Output {
    pub fn start() -> io::Result<Self> {
        if CURRENT.with(|current| current.borrow().is_some()) {
            return Err(io::Error::other("frontend output already active"));
        }
        let output = Self::spawn(
            |shared, notify| shared.run(io::stdout().lock(), notify),
            LIMIT,
        )?;
        CURRENT.with(|current| *current.borrow_mut() = Some(output.shared.clone()));
        Ok(output)
    }
    fn spawn(
        run: impl FnOnce(Arc<Shared>, UnixStream) + Send + 'static,
        limit: usize,
    ) -> io::Result<Self> {
        let (wake, notify) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        notify.set_nonblocking(true)?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            limit,
        });
        let owner = shared.clone();
        let worker = thread::Builder::new()
            .name("flere-output".into())
            .spawn(move || run(owner, notify))?;
        Ok(Self {
            shared,
            wake,
            worker: Some(worker),
        })
    }
    pub fn fd(&self) -> i32 {
        self.wake.as_raw_fd()
    }
    pub fn poll(&mut self) -> io::Result<()> {
        let mut bytes = [0; 256];
        let mut closed = false;
        loop {
            match self.wake.read(&mut bytes) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(_) => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        failure(&self.shared.state.lock().unwrap())?;
        if closed {
            return Err(io::Error::other("frontend output writer stopped"));
        }
        Ok(())
    }
    pub fn finish(&mut self) -> io::Result<()> {
        if let Some(worker) = self.worker.take() {
            {
                let mut state = self.shared.state.lock().unwrap();
                state.closing = true;
                self.shared.changed.notify_one();
            }
            let result = worker.join();
            CURRENT.with(|current| {
                let mut current = current.borrow_mut();
                if current
                    .as_ref()
                    .is_some_and(|s| Arc::ptr_eq(s, &self.shared))
                {
                    *current = None;
                }
            });
            if result.is_err() {
                return Err(io::Error::other("frontend output writer stopped"));
            }
            let state = self.shared.state.lock().unwrap();
            crate::diagnostics::record(
                "terminal-output",
                &format!(
                    "peak_bytes={} limit_bytes={}",
                    state.peak,
                    self.shared.limit + CLEANUP_LIMIT
                ),
            );
        }
        failure(&self.shared.state.lock().unwrap())
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub(super) fn ready() -> bool {
    CURRENT.with(|current| current.borrow().as_ref().is_none_or(|s| s.ready()))
}
pub(super) fn submit(bytes: Vec<u8>) -> io::Result<()> {
    send(bytes, false)
}
pub(super) fn cleanup(bytes: Vec<u8>) -> io::Result<()> {
    send(bytes, true)
}
fn send(bytes: Vec<u8>, cleanup: bool) -> io::Result<()> {
    CURRENT.with(|current| match current.borrow().as_ref() {
        Some(shared) => {
            if cleanup {
                shared.cleanup(bytes)
            } else {
                shared.submit(bytes)
            }
        }
        // The standalone intro and isolated renderer tests have no output owner.
        None => {
            let mut stdout = io::stdout().lock();
            stdout.write_all(&bytes)?;
            stdout.flush()
        }
    })
}

/// Packet::write flushes once after serialization. Admit a whole packet at that
/// boundary so a rejected oversized packet cannot leave half a header queued.
#[derive(Default)]
pub(super) struct PacketWriter {
    bytes: Vec<u8>,
    cleanup: bool,
}
pub(super) fn writer() -> PacketWriter {
    PacketWriter::default()
}
pub(super) fn cleanup_writer() -> PacketWriter {
    PacketWriter {
        cleanup: true,
        ..PacketWriter::default()
    }
}
impl Write for PacketWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > (crate::remote_protocol::MAX + 4).saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("frontend packet exceeds bound"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        send(std::mem::take(&mut self.bytes), self.cleanup)
    }
}

#[cfg(test)]
pub(in crate::ui) mod tests {
    use super::*;
    use std::sync::mpsc;
    struct Gated {
        started: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
        bytes: Arc<Mutex<Vec<u8>>>,
    }
    impl Write for Gated {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.started.send(()).unwrap();
            self.release
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            // Real transports may split even a complete admitted packet.
            let n = bytes.len().min(3);
            self.bytes.lock().unwrap().extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    /// Run real cleanup producers with all ordinary byte capacity blocked.
    pub(in crate::ui) fn saturated(action: impl FnOnce()) -> Vec<u8> {
        let (started, observed) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let sink = Gated {
            started,
            release: resume,
            bytes: bytes.clone(),
        };
        let mut output = Output::spawn(move |s, n| s.run(sink, n), 4).unwrap();
        CURRENT.with(|c| *c.borrow_mut() = Some(output.shared.clone()));
        output.shared.submit(b"full".to_vec()).unwrap();
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(output.shared.submit(b"overflow".to_vec()).is_err());
        action();
        assert!(output.shared.state.lock().unwrap().peak <= 4 + CLEANUP_LIMIT);
        for _ in 0..1000 {
            release.send(()).unwrap();
        }
        assert!(output.finish().is_err(), "overflow must still be reported");
        let bytes = bytes.lock().unwrap();
        assert!(bytes.starts_with(b"full"));
        bytes[4..].to_vec()
    }
    #[test]
    fn remote_graphics_cleanup_packets_precede_terminal_restoration_when_full() {
        use crate::remote_protocol::{AVATAR_CLEAR, OUTPUT, PREVIEW_CLEAR, Packet};
        let bytes = saturated(|| {
            Packet::new(AVATAR_CLEAR, 0, [])
                .write(&mut cleanup_writer())
                .unwrap();
            Packet::new(PREVIEW_CLEAR, 7, [])
                .write(&mut cleanup_writer())
                .unwrap();
            drop(super::super::DisplayGuard { remote: true });
        });
        let mut bytes = &bytes[..];
        assert_eq!(
            Packet::read(&mut bytes).unwrap(),
            Packet::new(AVATAR_CLEAR, 0, [])
        );
        assert_eq!(
            Packet::read(&mut bytes).unwrap(),
            Packet::new(PREVIEW_CLEAR, 7, [])
        );
        let terminal = Packet::read(&mut bytes).unwrap();
        assert_eq!(terminal.tag, OUTPUT);
        assert!(terminal.data.ends_with(b"\x1b[?1049l"));
        assert!(bytes.is_empty());
    }
    #[test]
    fn cleanup_reserve_is_bounded_by_bytes_and_batch_count() {
        for byte_limit in [true, false] {
            let shared = Shared {
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
                limit: 4,
            };
            shared.submit(vec![1; 4]).unwrap();
            if byte_limit {
                shared.cleanup(vec![2; CLEANUP_LIMIT]).unwrap();
            } else {
                for _ in 0..CLEANUP_BATCH_LIMIT {
                    shared.cleanup(vec![2]).unwrap();
                }
            }
            assert!(shared.cleanup(vec![3]).is_err());
            assert!(shared.submit(vec![4]).is_err());
            let state = shared.state.lock().unwrap();
            assert!(state.pending <= 4 + CLEANUP_LIMIT);
            assert!(state.batches <= BATCH_LIMIT + CLEANUP_BATCH_LIMIT);
        }
    }
    #[test]
    fn completely_full_batch_budget_preserves_terminal_cleanup_order() {
        let (started, observed) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let sink = Gated {
            started,
            release: resume,
            bytes: bytes.clone(),
        };
        let mut output = Output::spawn(move |s, n| s.run(sink, n), LIMIT).unwrap();
        CURRENT.with(|c| *c.borrow_mut() = Some(output.shared.clone()));
        output.shared.submit(vec![b'x']).unwrap();
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        for _ in 1..BATCH_LIMIT {
            output.shared.submit(vec![b'x']).unwrap();
        }
        assert!(output.shared.submit(vec![b'y']).is_err());
        drop(super::super::DisplayGuard { remote: false });
        assert_eq!(output.shared.state.lock().unwrap().batches, BATCH_LIMIT + 1);
        for _ in 0..BATCH_LIMIT + 100 {
            release.send(()).unwrap();
        }
        assert!(output.finish().is_err());
        let bytes = bytes.lock().unwrap();
        assert_eq!(&bytes[..BATCH_LIMIT], vec![b'x'; BATCH_LIMIT]);
        assert!(bytes[BATCH_LIMIT..].ends_with(b"\x1b[?1049l"));
    }
    #[test]
    fn blocked_output_counts_inflight_bytes_and_preserves_order_on_drain() {
        let (started, observed) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let sink = Gated {
            started,
            release: resume,
            bytes: bytes.clone(),
        };
        let mut output = Output::spawn(move |s, n| s.run(sink, n), 16).unwrap();
        output.shared.submit(b"first".to_vec()).unwrap();
        observed
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(!output.shared.ready());
        output.shared.submit(b"second".to_vec()).unwrap();
        assert_eq!(output.shared.state.lock().unwrap().pending, 11);
        assert!(output.shared.submit(vec![b'x'; 6]).is_err());
        // Rejection does not discard admitted bytes or prevent small cleanup.
        output.shared.submit(b"end".to_vec()).unwrap();
        for _ in 0..5 {
            release.send(()).unwrap();
        }
        assert!(output.finish().is_err());
        assert_eq!(&*bytes.lock().unwrap(), b"firstsecondend");
        assert_eq!(output.shared.state.lock().unwrap().peak, 14);
    }
    #[test]
    fn completely_full_output_still_restores_local_and_remote_terminals() {
        use crate::remote_protocol::{OUTPUT, Packet};
        for remote in [false, true] {
            let first = if remote {
                let mut bytes = Vec::new();
                Packet::new(OUTPUT, 0, b"frame").write(&mut bytes).unwrap();
                bytes
            } else {
                b"frame".to_vec()
            };
            let (started, observed) = mpsc::channel();
            let (release, resume) = mpsc::channel();
            let bytes = Arc::new(Mutex::new(Vec::new()));
            let sink = Gated {
                started,
                release: resume,
                bytes: bytes.clone(),
            };
            let mut output = Output::spawn(move |s, n| s.run(sink, n), first.len()).unwrap();
            CURRENT.with(|c| *c.borrow_mut() = Some(output.shared.clone()));
            output.shared.submit(first.clone()).unwrap();
            observed
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert!(output.shared.submit(b"overflow".to_vec()).is_err());
            // Real guard teardown must be admitted even with the normal budget
            // completely occupied by the writer's blocked in-flight batch.
            drop(super::super::DisplayGuard { remote });
            for _ in 0..100 {
                release.send(()).unwrap();
            }
            assert!(
                output.finish().is_err(),
                "the original overflow remains visible"
            );
            let bytes = bytes.lock().unwrap();
            assert!(bytes.starts_with(&first));
            let tail = &bytes[first.len()..];
            let restored = if remote {
                let mut input = tail;
                let packet = Packet::read(&mut input).expect("remote restoration packet missing");
                assert_eq!(packet.tag, OUTPUT);
                assert!(input.is_empty());
                packet.data
            } else {
                tail.to_vec()
            };
            assert!(
                restored.ends_with(b"\x1b[?1049l"),
                "terminal restoration was lost"
            );
        }
    }
    #[test]
    fn many_small_packets_cannot_grow_unbounded_queue_metadata() {
        let shared = Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            limit: LIMIT,
        };
        for _ in 0..BATCH_LIMIT {
            shared.submit(vec![1]).unwrap();
        }
        assert!(shared.submit(vec![2]).is_err());
        let state = shared.state.lock().unwrap();
        assert_eq!(state.queue.len(), BATCH_LIMIT);
        assert_eq!(state.pending, BATCH_LIMIT);
        assert_eq!(state.batches, BATCH_LIMIT);
    }
    #[test]
    fn transport_failure_is_reported_even_if_producers_ignore_their_error() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "reader closed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut output = Output::spawn(|s, n| s.run(Broken, n), 64).unwrap();
        output.shared.submit(b"frame".to_vec()).unwrap();
        assert_eq!(
            output.finish().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(output.poll().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }
    #[test]
    fn queued_packets_remain_whole_and_ordered_under_partial_writes() {
        use crate::remote_protocol::{OUTPUT, PREVIEW_CLEAR, Packet};
        let (release, resume) = mpsc::channel();
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let (started, _observed) = mpsc::channel();
        let sink = Gated {
            started,
            release: resume,
            bytes: bytes.clone(),
        };
        let mut output = Output::spawn(move |s, n| s.run(sink, n), 128).unwrap();
        CURRENT.with(|c| *c.borrow_mut() = Some(output.shared.clone()));
        let packets = [
            Packet::new(OUTPUT, 0, b"frame"),
            Packet::new(PREVIEW_CLEAR, 3, []),
        ];
        for packet in &packets {
            packet.write(&mut writer()).unwrap();
        }
        for _ in 0..11 {
            release.send(()).unwrap();
        }
        output.finish().unwrap();
        let bytes = bytes.lock().unwrap();
        let mut input = &bytes[..];
        for packet in packets {
            assert_eq!(Packet::read(&mut input).unwrap(), packet);
        }
        assert!(input.is_empty());
    }
}
