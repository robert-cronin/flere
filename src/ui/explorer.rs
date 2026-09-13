//! Directory reads can wait on mounts or macOS folder access. Keep one bounded
//! worker per attachment; the UI only schedules and applies generation-matched results.
use super::*;
use std::sync::mpsc::{self, Receiver, SyncSender};
type Entry = (String, PathBuf, bool);
struct Listing {
    entries: Vec<Entry>,
    error: Option<String>,
}
pub(super) struct Explorer {
    pub root: PathBuf,
    pub entries: Vec<Entry>,
    pub selected: usize,
    viewport: viewport::Viewport,
    pub loading: bool,
    pub error: Option<String>,
    generation: u64,
}
impl Explorer {
    pub fn new(root: PathBuf) -> Self {
        let entries = parent_entry(&root);
        Self {
            root,
            entries,
            selected: 0,
            viewport: viewport::Viewport::default(),
            loading: true,
            error: None,
            generation: 0,
        }
    }
    pub fn load(&mut self, root: PathBuf) {
        let generation = self.generation.wrapping_add(1);
        *self = Self::new(root);
        self.generation = generation;
    }
    pub(super) fn start(&self, rows: usize) -> usize {
        self.viewport
            .start(self.selected, self.entries.len(), rows, 0)
    }
    fn apply(&mut self, target: &Target, listing: Listing) -> bool {
        if self.root != target.root || self.generation != target.generation {
            return false;
        }
        self.entries = listing.entries;
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.error = listing.error;
        self.loading = false;
        true
    }
}
fn parent_entry(root: &Path) -> Vec<Entry> {
    root.parent()
        .map(|p| vec![("..".into(), p.into(), true)])
        .unwrap_or_default()
}
// Signals can interrupt opendir/readdir/stat on macOS. Restart the whole
// bounded listing so a partial iteration cannot skip or duplicate an entry.
// Persistent interruptions get an explicit retry action, never an infinite loop.
fn retry_interrupted<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    for attempt in 0..3 {
        match operation() {
            Err(e) if e.kind() == io::ErrorKind::Interrupted && attempt < 2 && !os::stopping() => {
                crate::diagnostics::record("explorer-retry", &format!("attempt={}", attempt + 1));
            }
            result => return result,
        }
    }
    unreachable!()
}
fn read(root: &Path) -> Listing {
    let started = Instant::now();
    let mut entries = parent_entry(root);
    let result = retry_interrupted(|| {
        fs::read_dir(root).and_then(|read| {
            read.take(512)
                .map(|entry| {
                    let e = entry?;
                    Ok((
                        e.file_name().to_string_lossy().into_owned(),
                        e.path(),
                        e.file_type()?.is_dir(),
                    ))
                })
                .collect::<io::Result<Vec<Entry>>>()
        })
    });
    let error = match result {
        Ok(mut rest) => {
            rest.sort_by(|a, b| {
                b.2.cmp(&a.2)
                    .then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase()))
            });
            entries.extend(rest);
            None
        }
        Err(e) => {
            crate::diagnostics::record(
                "explorer-error",
                &format!("kind={:?} os={:?}", e.kind(), e.raw_os_error()),
            );
            Some(match e.kind() {
                io::ErrorKind::PermissionDenied => "Folder access denied".into(),
                io::ErrorKind::NotFound => "Directory no longer exists".into(),
                io::ErrorKind::Interrupted => "Folder read interrupted. Press r to retry".into(),
                _ => format!("Directory unavailable: {}", wire::passive(&e.to_string())),
            })
        }
    };
    crate::diagnostics::slow("explorer-read", started);
    Listing { entries, error }
}
#[derive(Clone)]
struct Target {
    epoch: String,
    wid: u64,
    root: PathBuf,
    generation: u64,
}
pub(super) struct Loader {
    send: SyncSender<PathBuf>,
    receive: Receiver<Listing>,
    target: Option<Target>,
}
impl Loader {
    pub fn new() -> Self {
        Self::with_reader(read)
    }
    fn with_reader(reader: impl Fn(&Path) -> Listing + Send + 'static) -> Self {
        let (send, requests) = mpsc::sync_channel::<PathBuf>(1);
        let (results, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(root) = requests.recv() {
                if results.send(reader(&root)).is_err() {
                    break;
                }
            }
        });
        Self {
            send,
            receive,
            target: None,
        }
    }
    fn request(&mut self, epoch: &str, wid: u64, e: &Explorer) {
        if e.loading && self.target.is_none() && self.send.try_send(e.root.clone()).is_ok() {
            self.target = Some(Target {
                epoch: epoch.into(),
                wid,
                root: e.root.clone(),
                generation: e.generation,
            });
        }
    }
    fn poll(&mut self) -> Option<(Target, Listing)> {
        match self.receive.try_recv() {
            Ok(listing) => Some((self.target.take()?, listing)),
            Err(mpsc::TryRecvError::Disconnected) => Some((
                self.target.take()?,
                Listing {
                    entries: Vec::new(),
                    error: Some("Directory reader unavailable".into()),
                },
            )),
            Err(mpsc::TryRecvError::Empty) => None,
        }
    }
}
impl Ui {
    pub(super) fn tick_explorer(&mut self) -> bool {
        let mut dirty = false;
        if let Some((target, listing)) = self.explorer_loader.poll()
            && target.epoch == self.snapshot.epoch
            && let Some(e) = self.explorers.get_mut(&target.wid)
        {
            dirty = e.apply(&target, listing);
        }
        // Only schedule the current folder. Rapid navigation coalesces to the latest
        // request, with no thread per click and no queue of abandoned directory reads.
        if let Some(e) = self.explorers.get(&self.snapshot.active) {
            self.explorer_loader
                .request(&self.snapshot.epoch, self.snapshot.active, e);
        }
        dirty
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_directory_does_not_block_navigation_or_accept_stale_results() {
        let (started, start) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        let mut loader = Loader::with_reader(move |root| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            Listing {
                entries: parent_entry(root),
                error: None,
            }
        });
        let mut e = Explorer::new("/blocked".into());
        loader.request("epoch", 7, &e);
        start.recv_timeout(Duration::from_secs(1)).unwrap();
        // The read stays blocked until explicitly released. Poll, refresh, and
        // parent navigation must all finish while that syscall is still waiting.
        for _ in 0..100 {
            assert!(loader.poll().is_none());
            e.load("/blocked".into());
            loader.request("epoch", 7, &e);
        }
        assert!(start.try_recv().is_err()); // exactly one worker request
        assert_eq!(e.entries[0].0, "..");
        release.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(1);
        let (old, listing) = loop {
            if let Some(result) = loader.poll() {
                break result;
            }
            assert!(Instant::now() < end);
            std::thread::yield_now();
        };
        assert!(!e.apply(&old, listing));
        assert!(e.loading);
        e.load("/available".into());
        loader.request("epoch", 7, &e);
        start.recv_timeout(Duration::from_secs(1)).unwrap();
        release.send(()).unwrap();
        let end = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some((target, listing)) = loader.poll() {
                assert_eq!(target.root, Path::new("/available"));
                assert!(e.apply(&target, listing));
                break;
            }
            assert!(Instant::now() < end);
            std::thread::yield_now();
        }
        assert!(!e.loading);
    }
    #[test]
    fn interrupted_listing_retries_but_other_errors_and_repeated_interruptions_stop() {
        let mut calls = 0;
        let entries = retry_interrupted(|| {
            calls += 1;
            if calls < 3 {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            } else {
                Ok(vec!["complete listing"])
            }
        })
        .unwrap();
        assert_eq!(entries, vec!["complete listing"]);
        assert_eq!(calls, 3);
        calls = 0;
        let failure = retry_interrupted::<()>(|| {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::Interrupted))
        })
        .unwrap_err();
        assert_eq!(failure.kind(), io::ErrorKind::Interrupted);
        assert_eq!(calls, 3);
        calls = 0;
        let denied = retry_interrupted::<()>(|| {
            calls += 1;
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls, 1);
    }
    #[test]
    fn missing_directory_reports_failure_instead_of_an_empty_listing() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(format!("missing-{}", os::nonce().unwrap()));
        let result = read(&root);
        assert_eq!(result.error.as_deref(), Some("Directory no longer exists"));
        assert_eq!(result.entries.len(), 1); // parent remains usable
    }
}
