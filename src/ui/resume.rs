//! A tab owns its resume details. The UI never asks for a conversation identifier.
use super::*;
impl Ui {
    pub(super) fn remember_workspace(&mut self) {
        self.prefs.history_epoch = self.snapshot.epoch.clone();
        self.prefs.history = self.history.clone();
        self.prefs.history_index = self.history_index;
        self.save_preferences();
    }
    pub(super) fn reopen_workspace(&mut self) {
        self.restore_pending = Some(self.snapshot.active);
        self.tick_restore();
    }
    pub(super) fn tick_restore(&mut self) -> bool {
        let Some(wid) = self.restore_pending else {
            return false;
        };
        // Browsing another card cancels the pending restoration; merely previewing
        // that card must not transfer the permission to start its saved programs.
        if self.snapshot.active != wid {
            self.restore_pending = None;
            return false;
        }
        let result = wire::request(
            &self.state,
            &[
                "restore-workspace-next",
                &self.snapshot.epoch,
                &wid.to_string(),
            ],
        );
        match result.and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)
        }) {
            Ok(value) => {
                self.restore_pending =
                    (value["remaining"].as_u64().unwrap_or(0) > 0).then_some(wid);
                if let Ok(snapshot) = self.read_snapshot() {
                    self.snapshot(snapshot);
                }
                if let Some(error) = value["error"].as_str().filter(|s| !s.is_empty()) {
                    self.notice = error.into();
                }
            }
            Err(e) => {
                self.restore_pending = None;
                self.notice = if e.to_string() == "unknown command" {
                    "Saved tabs need a Flere refresh: Ctrl+Space, then R".into()
                } else {
                    e.to_string()
                };
            }
        }
        if self.restore_pending.is_none() {
            self.remember_workspace();
        }
        true
    }
    pub(super) fn resume_saved_chat(&mut self) {
        let Some(w) = self.snapshot.workspace() else {
            return;
        };
        if w.meta.archived || !w.meta.operation.is_empty() {
            return;
        }
        let id = w.id;
        match wire::request(
            &self.state,
            &["restore-retry", &self.snapshot.epoch, &id.to_string()],
        ) {
            Ok(_) => {
                self.restore_pending = Some(self.snapshot.active);
                self.tick_restore();
            }
            Err(e) => {
                self.notice = e.to_string();
            }
        }
        // Explicit activation may reopen the last chat after every tab was closed.
        // Passive card preview and UI attachment never call this history fallback.
        if self
            .snapshot
            .workspace()
            .is_some_and(|w| w.id == id && w.tabs.is_empty())
            && self.restore_pending.is_none()
        {
            match wire::request(
                &self.state,
                &["resume-recent", &self.snapshot.epoch, &id.to_string()],
            ) {
                Ok(_) => {
                    if let Ok(snapshot) = self.read_snapshot() {
                        self.snapshot(snapshot);
                    }
                }
                Err(e) if e.to_string() == "unknown command" => {} // Older supervisors keep their existing Start action.
                Err(e) => self.notice = e.to_string(),
            }
        }
        self.remember_workspace();
    }
    pub(super) fn stopped_key(&mut self, bytes: &[u8]) -> bool {
        if self.snapshot.session().is_some()
            || self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
            || self.scrollback.is_some()
        {
            return false;
        }
        if bytes == b"S" {
            self.open_form("Start agent");
            return true;
        }
        if self.focus != Focus::Terminal || self.nav {
            return false;
        }
        match bytes {
            b"W" => self.resume_saved_chat(),
            b"t" => self.new_tab(),
            b" " => {
                self.menu = true;
                self.menu_index = 0;
                self.menu_query.clear();
                self.menu_search = false;
            }
            b"\r" => {
                self.resume_saved_chat();
                if self.snapshot.session().is_none()
                    && self.restore_pending.is_none()
                    && self.notice.is_empty()
                {
                    self.open_form("Start agent");
                }
            }
            _ => return false,
        }
        true
    }
}
