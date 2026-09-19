//! Save through the session-preserving supervisor when it supports queued writes.
use super::*;

pub(super) struct Persistence {
    epoch: String,
    queued: bool,
    accepted: Vec<u8>,
    pending: bool,
    checked: Instant,
}
impl Persistence {
    pub(super) fn load(state: &Path, epoch: &str) -> io::Result<(Preferences, Self, String)> {
        let (prefs, queued, pending, error) =
            match wire::request(state, &["ui-preferences", "get", epoch]) {
                Ok(bytes) => {
                    let mut value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                    let prefs = serde_json::from_value(value["preferences"].take())
                        .map_err(io::Error::other)?;
                    let pending = value["pending"].as_bool().unwrap_or(false);
                    let error = value["error"].as_str().unwrap_or_default().to_string();
                    (prefs, true, pending, error)
                }
                // Mixed-version attach keeps the existing file format and the
                // old synchronous behavior until the supervisor is refreshed.
                Err(e) if e.to_string() == "unknown command" => (
                    crate::workspace::load_preferences(state),
                    false,
                    false,
                    String::new(),
                ),
                Err(e) => return Err(e),
            };
        let accepted = if error.is_empty() {
            serde_json::to_vec(&prefs).map_err(io::Error::other)?
        } else {
            Vec::new()
        };
        Ok((
            prefs,
            Self {
                epoch: epoch.into(),
                queued,
                accepted,
                pending,
                checked: Instant::now(),
            },
            error,
        ))
    }
    pub(super) fn save(&mut self, state: &Path, prefs: &Preferences) -> io::Result<()> {
        let bytes = serde_json::to_vec(prefs).map_err(io::Error::other)?;
        if bytes == self.accepted {
            return Ok(());
        }
        if self.queued {
            let reply = wire::request(
                state,
                &[
                    "ui-preferences",
                    "put",
                    &self.epoch,
                    std::str::from_utf8(&bytes).map_err(io::Error::other)?,
                ],
            )?;
            self.status(&reply)?;
        } else {
            crate::workspace::atomic_write(
                &state.join("ui.json"),
                &serde_json::to_vec_pretty(prefs).map_err(io::Error::other)?,
            )?;
        }
        self.accepted = bytes;
        Ok(())
    }
    fn status(&mut self, reply: &[u8]) -> io::Result<()> {
        let value: serde_json::Value = serde_json::from_slice(reply).map_err(io::Error::other)?;
        self.pending = value["pending"].as_bool().unwrap_or(false);
        if let Some(error) = value["error"].as_str() {
            self.accepted.clear();
            return Err(wire::invalid(error));
        }
        Ok(())
    }
    pub(super) fn poll(&mut self, state: &Path) -> io::Result<()> {
        if !self.pending || self.checked.elapsed() < Duration::from_secs(1) {
            return Ok(());
        }
        self.checked = Instant::now();
        let reply = wire::request(state, &["ui-preferences", "status", &self.epoch])?;
        self.status(&reply)
    }
}
