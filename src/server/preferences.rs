//! UI settings survive frontend detach; at most one write and one latest value.
//! This queue never owns messages, approvals, terminal input, or workspace state.
use super::*;
use std::sync::{Arc, mpsc};

const LIMIT: usize = 64 * 1024;
struct Job {
    bytes: Arc<Vec<u8>>,
    result: mpsc::Receiver<io::Result<()>>,
}
#[derive(Default)]
pub(super) struct Writer {
    state: Option<PathBuf>,
    latest: Option<Arc<Vec<u8>>>,
    durable: Option<Arc<Vec<u8>>>,
    job: Option<Job>,
    error: Option<String>,
}
impl Writer {
    fn start(&mut self, state: &Path) -> io::Result<()> {
        self.state = Some(state.into());
        let path = state.join("ui.json");
        let result = self.start_with(move |bytes| atomic_write(&path, bytes));
        if let Err(e) = &result {
            self.error = Some(format!("Could not save UI settings: {e}"));
        }
        result
    }
    fn start_with(
        &mut self,
        write: impl FnOnce(&[u8]) -> io::Result<()> + Send + 'static,
    ) -> io::Result<()> {
        if self.job.is_some() || self.latest == self.durable {
            return Ok(());
        }
        let Some(bytes) = self.latest.clone() else {
            return Ok(());
        };
        let (sender, result) = mpsc::sync_channel(1);
        let data = bytes.clone();
        std::thread::Builder::new()
            .name("flere-preferences".into())
            .spawn(move || {
                let _ = sender.send(write(&data));
            })?;
        self.job = Some(Job { bytes, result });
        self.error = None;
        Ok(())
    }
    fn finish(&mut self, result: io::Result<()>) -> bool {
        let job = self.job.take().expect("preference completion owns its job");
        match result {
            Ok(()) => {
                self.durable = Some(job.bytes.clone());
                self.error = None;
            }
            Err(e) => {
                // Rename may have succeeded before directory sync failed. Do not
                // consider the previous bytes durable after any failed write.
                self.durable = None;
                self.error = Some(format!("Could not save UI settings: {e}"));
                crate::diagnostics::record("ui-preferences-save", "failed");
            }
        }
        self.latest.as_ref() != Some(&job.bytes)
    }
    pub(super) fn poll(&mut self, state: &Path) {
        let result = self
            .job
            .as_ref()
            .and_then(|job| match job.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err(io::Error::other("UI settings writer stopped")))
                }
            });
        if let Some(result) = result
            && self.finish(result)
            && let Err(e) = self.start(state)
        {
            self.error = Some(format!("Could not save UI settings: {e}"));
        }
    }
    // Only supervisor refresh/shutdown wait for durability. A frontend may
    // detach immediately after enqueue; a new frontend reads latest below.
    pub(super) fn flush(&mut self, state: &Path) -> io::Result<()> {
        loop {
            let Some(job) = &self.job else {
                return self.error.as_ref().map_or(Ok(()), |e| Err(invalid(e)));
            };
            let result = job
                .result
                .recv()
                .unwrap_or_else(|_| Err(io::Error::other("UI settings writer stopped")));
            if self.finish(result) {
                self.start(state)?;
            }
        }
    }
    fn status(&self) -> serde_json::Value {
        serde_json::json!({"pending":self.job.is_some(),"error":self.error})
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(state) = self.state.clone()
            && self.flush(&state).is_err()
        {
            crate::diagnostics::record("ui-preferences-save", "final-flush-failed");
        }
    }
}
impl Server {
    pub(super) fn preferences_command(&mut self, p: &[&str]) -> io::Result<Vec<u8>> {
        if p.get(2).copied() != Some(self.epoch.as_str()) {
            return Err(invalid("stale UI settings epoch"));
        }
        self.preferences.poll(&self.state);
        let result = match p.get(1).copied() {
            Some("get") if p.len() == 3 => {
                // Old frontends still write this file directly. Once our writer
                // is idle, re-read it on attachment so those changes remain visible.
                if self.preferences.job.is_none() && self.preferences.error.is_none() {
                    let prefs = crate::workspace::load_preferences(&self.state);
                    let bytes =
                        Arc::new(serde_json::to_vec_pretty(&prefs).map_err(io::Error::other)?);
                    self.preferences.durable = Some(bytes.clone());
                    self.preferences.latest = Some(bytes);
                }
                let mut value = self.preferences.status();
                value["preferences"] = serde_json::from_slice(
                    self.preferences
                        .latest
                        .as_deref()
                        .map_or(b"{}", |v| v.as_slice()),
                )
                .map_err(io::Error::other)?;
                value
            }
            Some("put") if p.len() == 4 => {
                if p[3].len() > LIMIT {
                    return Err(invalid("UI settings exceed 64 KiB"));
                }
                let prefs: crate::workspace::Preferences =
                    serde_json::from_str(p[3]).map_err(io::Error::other)?;
                let bytes = serde_json::to_vec_pretty(&prefs).map_err(io::Error::other)?;
                if bytes.len() > LIMIT {
                    return Err(invalid("UI settings exceed 64 KiB"));
                }
                self.preferences.latest = Some(Arc::new(bytes));
                // A later put, including identical data, retries a failed save.
                // Polling alone never creates an unbounded disk retry loop.
                self.preferences.start(&self.state)?;
                self.preferences.status()
            }
            Some("status") if p.len() == 3 => self.preferences.status(),
            _ => return Err(invalid("invalid UI settings operation")),
        };
        serde_json::to_vec(&result).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set(writer: &mut Writer, bytes: &[u8]) {
        writer.latest = Some(Arc::new(bytes.to_vec()));
    }
    #[test]
    fn slow_writer_coalesces_latest_and_never_blocks_enqueue_or_reads() {
        let mut writer = Writer::default();
        let (release, wait) = mpsc::sync_channel(1);
        let (started, ready) = mpsc::sync_channel(1);
        set(&mut writer, b"first");
        writer
            .start_with(move |bytes| {
                assert_eq!(bytes, b"first");
                started.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(())
            })
            .unwrap();
        ready.recv_timeout(Duration::from_secs(2)).unwrap();
        for _ in 0..1000 {
            set(&mut writer, b"intermediate");
            writer
                .start_with(|_| panic!("only one writer may run"))
                .unwrap();
        }
        set(&mut writer, b"last");
        assert_eq!(writer.latest.as_deref().unwrap().as_slice(), b"last");
        assert_eq!(writer.status()["pending"], true);
        release.send(()).unwrap();
        let result = writer
            .job
            .as_ref()
            .unwrap()
            .result
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(writer.finish(result));
        writer
            .start_with(|bytes| {
                assert_eq!(bytes, b"last");
                Ok(())
            })
            .unwrap();
        let result = writer
            .job
            .as_ref()
            .unwrap()
            .result
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(!writer.finish(result));
        assert_eq!(writer.latest, writer.durable);
        writer
            .start_with(|_| panic!("unchanged data must not be synced again"))
            .unwrap();
        assert_eq!(writer.status()["pending"], false);
    }
    #[test]
    fn failure_preserves_latest_and_requires_explicit_retry() {
        let mut writer = Writer::default();
        set(&mut writer, b"retained");
        writer
            .start_with(|_| Err(io::Error::other("injected failure")))
            .unwrap();
        let result = writer
            .job
            .as_ref()
            .unwrap()
            .result
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(!writer.finish(result));
        assert!(
            writer.status()["error"]
                .as_str()
                .unwrap()
                .contains("injected failure")
        );
        assert!(writer.job.is_none());
        assert!(writer.durable.is_none());
        writer
            .start_with(|bytes| {
                assert_eq!(bytes, b"retained");
                Ok(())
            })
            .unwrap();
        let result = writer
            .job
            .as_ref()
            .unwrap()
            .result
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(!writer.finish(result));
        assert_eq!(writer.latest, writer.durable);
        assert!(writer.error.is_none());
    }
    #[test]
    fn returning_to_previous_settings_during_write_still_restores_them() {
        let mut writer = Writer::default();
        set(&mut writer, b"original");
        writer.durable = writer.latest.clone();
        set(&mut writer, b"temporary");
        writer.start_with(|_| Ok(())).unwrap();
        set(&mut writer, b"original");
        let result = writer
            .job
            .as_ref()
            .unwrap()
            .result
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(writer.finish(result));
        assert_ne!(writer.latest, writer.durable);
        writer
            .start_with(|bytes| {
                assert_eq!(bytes, b"original");
                Ok(())
            })
            .unwrap();
        let result = writer
            .job
            .as_ref()
            .unwrap()
            .result
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        assert!(!writer.finish(result));
        assert_eq!(writer.latest, writer.durable);
    }
}
