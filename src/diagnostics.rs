//! Bounded lifecycle diagnostics, never terminal/input/image payloads.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
const LIMIT: u64 = 256 * 1024;
const RETAIN: usize = 8;
struct Log {
    file: File,
    path: PathBuf,
    bytes: u64,
}
static LOG: OnceLock<Mutex<Log>> = OnceLock::new();
fn timestamp() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
impl Log {
    fn open(dir: &Path, component: &str) -> io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(dir)?;
        let meta = fs::symlink_metadata(dir)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(io::Error::other(
                "diagnostics directory must not be a symlink",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if meta.permissions().mode() & 0o077 != 0 {
                return Err(io::Error::other(
                    "diagnostics directory must be private (0700)",
                ));
            }
        }
        let prefix = format!("{component}-");
        let mut old: Vec<_> = fs::read_dir(dir)?
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_type().is_ok_and(|t| t.is_file())
                    && e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".log"))
            })
            .collect();
        old.sort_by_key(|e| e.file_name());
        let remove = old.len().saturating_sub(RETAIN - 1);
        for entry in old.into_iter().take(remove) {
            // Only this component's private diagnostic files, never application state.
            let _ = fs::remove_file(entry.path());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = dir.join(format!("{component}-{nonce}-{}.log", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        Ok(Self {
            file,
            path,
            bytes: 0,
        })
    }
    fn write(&mut self, event: &str, detail: &str) -> io::Result<()> {
        let clean: String = detail
            .chars()
            .take(1024)
            .map(|c| {
                if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let line = format!(
            "{} pid={} {event} {clean}\n",
            timestamp(),
            std::process::id()
        );
        if self.bytes + line.len() as u64 > LIMIT {
            use std::io::{Seek, SeekFrom};
            self.file.set_len(0)?;
            self.file.seek(SeekFrom::Start(0))?;
            self.bytes = 0;
            let marker = b"diagnostics: older entries discarded at 256 KiB limit\n";
            self.file.write_all(marker)?;
            self.bytes += marker.len() as u64;
        }
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        self.bytes += line.len() as u64;
        Ok(())
    }
}
pub fn init(dir: &Path, component: &'static str) -> io::Result<()> {
    if LOG.get().is_some() {
        return Ok(());
    }
    let log = Log::open(dir, component)?;
    let _ = LOG.set(Mutex::new(log));
    record(
        "start",
        &format!(
            "component={component} version={} build={} target={}",
            env!("CARGO_PKG_VERSION"),
            crate::build_info::BUILD_ID,
            crate::build_info::TARGET,
        ),
    );
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Panic payloads can contain user data. Retain the source location only.
        record(
            "panic",
            &info.location().map(|l| l.to_string()).unwrap_or_default(),
        );
        previous(info);
    }));
    Ok(())
}
pub fn record(event: &'static str, detail: &str) {
    if let Some(log) = LOG.get()
        && let Ok(mut log) = log.lock()
    {
        let _ = log.write(event, detail); // Diagnostics never take down the frontend.
    }
}
/// Record stalls without retaining input, terminal contents, paths, or metadata.
pub fn slow(event: &'static str, started: std::time::Instant) {
    let elapsed = started.elapsed();
    if elapsed >= std::time::Duration::from_millis(250) {
        record(event, &format!("elapsed_ms={}", elapsed.as_millis()));
    }
}
/// Measure a supervisor stage even when it exits through an error path.
/// Only the static event name and elapsed time can reach the diagnostic log.
pub(crate) struct Timing {
    event: &'static str,
    started: std::time::Instant,
}
pub(crate) fn measure(event: &'static str) -> Timing {
    Timing {
        event,
        started: std::time::Instant::now(),
    }
}
impl Drop for Timing {
    fn drop(&mut self) {
        slow(self.event, self.started);
    }
}

/// Fixed-size stage accounting. Fast loops allocate nothing; slow-loop records
/// contain static stage names and durations only, never input or terminal data.
pub(crate) struct Stages<const N: usize> {
    event: &'static str,
    names: [&'static str; N],
    times: [std::time::Duration; N],
    started: std::time::Instant,
    previous: std::time::Instant,
    next: usize,
}
impl<const N: usize> Stages<N> {
    pub(crate) fn new(event: &'static str, names: [&'static str; N]) -> Self {
        let started = std::time::Instant::now();
        Self {
            event,
            names,
            times: [std::time::Duration::ZERO; N],
            started,
            previous: started,
            next: 0,
        }
    }
    pub(crate) fn mark(&mut self) {
        let now = std::time::Instant::now();
        self.times[self.next] = now.saturating_duration_since(self.previous);
        self.previous = now;
        self.next += 1;
    }
    pub(crate) fn finish(self) {
        let elapsed = self.started.elapsed();
        if elapsed >= std::time::Duration::from_millis(250) {
            use std::fmt::Write;
            let mut detail = format!("elapsed_ms={}", elapsed.as_millis());
            for (name, time) in self.names.iter().zip(&self.times).take(self.next) {
                let _ = write!(detail, " {name}_ms={}", time.as_millis());
            }
            record(self.event, &detail);
        }
    }
}

pub fn path() -> Option<PathBuf> {
    LOG.get()?.lock().ok().map(|l| l.path.clone())
}
pub fn error(stage: &'static str, error: &io::Error) {
    record(
        stage,
        &format!(
            "kind={:?} os={:?} {error}",
            error.kind(),
            error.raw_os_error()
        ),
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostic_files_are_private_bounded_and_pruned_without_following_links() {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .unwrap();
        let root = PathBuf::from(home).join(".cache/flere/tmp").join(format!(
            "diagnostics-test-{}-{}",
            std::process::id(),
            timestamp()
        ));
        let mut log = Log::open(&root, "test").unwrap();
        for _ in 0..400 {
            log.write("event", &"x".repeat(1024)).unwrap();
        }
        log.write("last", "no\nescape\x1b").unwrap();
        assert!(log.file.metadata().unwrap().len() <= LIMIT);
        let text = fs::read_to_string(&log.path).unwrap();
        assert!(text.contains("last no escape "));
        assert!(!text.contains('\x1b'));
        for _ in 0..12 {
            Log::open(&root, "test").unwrap();
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), RETAIN);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            let link = root.join("link");
            symlink(&root, &link).unwrap();
            assert!(Log::open(&link, "test").is_err());
        }
        drop(log);
        fs::remove_dir_all(root).unwrap();
    }
}
