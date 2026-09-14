//! Per-attachment scroll position; never modifies the child's screen or input.
use super::*;
use crate::model::ScrollbackPage;

pub(super) struct Scrollback {
    location: (String, u64, u64, String),
    layout: Layout,
    pub page: ScrollbackPage,
}
pub(super) struct SmoothScroll {
    location: (String, u64, u64, String),
    layout: Layout,
    remaining: i64,
    frames: i64,
    checked: Instant,
}
impl Scrollback {
    pub(super) fn new(snapshot: &Snapshot, layout: Layout, page: ScrollbackPage) -> Self {
        Self {
            location: location(snapshot),
            layout,
            page,
        }
    }
    pub fn matches(&self, snapshot: &Snapshot, layout: Layout) -> bool {
        self.location == location(snapshot)
            && self.layout == layout
            && (self.page.cols, self.page.rows) == (snapshot.cols, snapshot.rows)
    }
    pub(super) fn matches_at(
        &self,
        epoch: &str,
        workspace: u64,
        tab: u64,
        run: &str,
        layout: Layout,
        size: (usize, usize),
    ) -> bool {
        self.location == (epoch.into(), workspace, tab, run.into())
            && self.layout == layout
            && (self.page.cols, self.page.rows) == size
    }
    pub fn paint(&self, canvas: &mut Canvas) {
        let l = self.layout;
        for y in 0..l.rows.min(self.page.rows) {
            for x in 0..l.cols.min(self.page.cols) {
                let mut cell = self.page.cells[y * self.page.cols + x].clone();
                if cell.style.fg == Color::Default {
                    cell.style.fg = crate::terminal::DEFAULT_FG;
                }
                if cell.style.bg == Color::Default {
                    cell.style.bg = crate::terminal::DEFAULT_BG;
                }
                canvas.cells[(l.terminal_y + y) * canvas.width + l.terminal_x + x] = cell;
            }
        }
    }
}
fn location(s: &Snapshot) -> (String, u64, u64, String) {
    (
        s.epoch.clone(),
        s.active,
        s.tab,
        s.session().map(|t| t.run.clone()).unwrap_or_default(),
    )
}
impl Ui {
    pub(super) fn command_history(&mut self, operation: &str) {
        let Some(tab) = self.snapshot.session() else {
            return;
        };
        let anchor = self
            .scrollback
            .as_ref()
            .map(|s| s.page.position.to_string())
            .unwrap_or_else(|| "live".into());
        let id = tab.id.to_string();
        let mut fields = vec![
            if operation == "copy" {
                "command-output"
            } else {
                self.page_command("command-jump")
            },
            &id,
            &tab.run,
            &anchor,
        ];
        if operation != "copy" {
            fields.push(operation);
        }
        match wire::request(&self.state, &fields) {
            Ok(bytes) if operation == "copy" => {
                self.clipboard = Some(String::from_utf8_lossy(&bytes).into_owned());
                self.notice = "Command output copied".into();
            }
            Ok(bytes) => match ScrollbackPage::decode(&bytes) {
                Ok(page) if (page.cols, page.rows) == (self.snapshot.cols, self.snapshot.rows) => {
                    self.selection = None;
                    self.smooth_scroll = None;
                    self.scrollback = Some(Scrollback::new(
                        &self.snapshot,
                        self.terminal_layout(),
                        page,
                    ));
                }
                Ok(_) => self.notice = "Terminal resized; retry command navigation".into(),
                Err(e) => self.notice = e.to_string(),
            },
            Err(e) => self.notice = wire::passive(&e.to_string()),
        }
    }
    // NAV scrolling is always local, including in native alternate-screen apps.
    // Keep pane/tab movement and the child's normal control keys independent.
    pub(super) fn nav_scroll_key(&mut self, key: &[u8]) -> bool {
        if self.focus != Focus::Terminal
            || self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
        {
            return false;
        }
        let page = self.terminal_layout().rows.saturating_sub(1).max(1) as i64;
        let full = self.terminal_layout().rows.max(1) as i64;
        let delta = match key {
            b"\x1b[1;5A" => -(self.terminal_layout().rows as i64), // Ctrl+Up, exact viewport.
            b"\x1b[1;5B" => self.terminal_layout().rows as i64,    // Ctrl+Down.
            b"u" => -3,
            b"d" => 3,
            b"\x19" => -1,    // Ctrl+Y
            b"\x05" => 1,     // Ctrl+E
            b"\x15" => -full, // Ctrl+U
            b"\x04" => full,  // Ctrl+D
            b"\x1b[5~" | b"\x1b[5;2~" => -page,
            b"\x1b[6~" | b"\x1b[6;2~" => page,
            b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" | b"\x1b[1;2H" | b"\x1b[1;2~"
            | b"\x1b[7;2~" => i64::MIN,
            b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\x1b[8~" => {
                self.smooth_scroll = None;
                self.scrollback = None;
                return true;
            }
            _ => return false,
        };
        if delta != i64::MIN && delta.abs() > 1 {
            self.smooth_terminal(delta);
        } else {
            self.scroll_terminal(delta);
        }
        true
    }
    fn smooth_terminal(&mut self, delta: i64) {
        if self.prefs.reduced_motion {
            self.scroll_terminal(delta);
            return;
        }
        // Repeated keys extend the current destination; reversal responds immediately.
        if let Some(motion) = &mut self.smooth_scroll
            && motion.remaining.signum() == delta.signum()
        {
            motion.remaining = (motion.remaining + delta).clamp(-100_000, 100_000);
            motion.frames = 8;
            return;
        }
        self.smooth_scroll = None;
        let before = self.scrollback.as_ref().map(|s| s.page.position);
        self.scroll_page(delta.signum(), false);
        if let Some(view) = &self.scrollback
            && Some(view.page.position) != before
        {
            self.smooth_scroll = Some(SmoothScroll {
                location: location(&self.snapshot),
                layout: self.terminal_layout(),
                remaining: delta - delta.signum(),
                frames: 8,
                checked: Instant::now(),
            });
        }
    }
    pub(super) fn tick_scroll(&mut self) -> bool {
        let Some(mut motion) = self.smooth_scroll.take() else {
            return false;
        };
        if !self.nav
            || self.focus != Focus::Terminal
            || self.menu
            || self.board
            || self.form.is_some()
            || self.confirm.is_some()
            || self.selection.is_some()
            || self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
            || self.scrollback.is_none()
            || motion.location != location(&self.snapshot)
            || motion.layout != self.terminal_layout()
        {
            return false;
        }
        if motion.checked.elapsed() < Duration::from_millis(20) && !self.prefs.reduced_motion {
            self.smooth_scroll = Some(motion);
            return false;
        }
        let step = if self.prefs.reduced_motion {
            motion.remaining
        } else {
            motion.remaining.signum()
                * ((motion.remaining.abs() + motion.frames - 1) / motion.frames)
        };
        let before = self.scrollback.as_ref().map(|s| s.page.position);
        self.scroll_page(step, false);
        motion.remaining -= step;
        motion.frames = (motion.frames - 1).max(1);
        motion.checked = Instant::now();
        if motion.remaining != 0
            && self
                .scrollback
                .as_ref()
                .is_some_and(|s| Some(s.page.position) != before)
        {
            self.smooth_scroll = Some(motion);
        }
        true
    }
    pub(super) fn scroll_terminal(&mut self, delta: i64) {
        self.smooth_scroll = None;
        self.scroll_page(delta, false);
    }
    fn scroll_page(&mut self, delta: i64, native_wheel: bool) {
        if self.selection.as_ref().is_some_and(|s| s.dragging) {
            return;
        }
        let Some(t) = self.snapshot.session() else {
            return;
        };
        // Down at the live bottom is inert; no request and no native key synthesis.
        if self.scrollback.is_none() && delta >= 0 && !native_wheel {
            return;
        }
        let oldest = delta == i64::MIN;
        let anchor = if oldest {
            "0".into()
        } else {
            self.scrollback
                .as_ref()
                .map(|s| s.page.position.to_string())
                .unwrap_or_else(|| "live".into())
        };
        let delta = if oldest { 0 } else { delta };
        let result = if native_wheel && self.scrollback.is_none() {
            self.flush_input();
            let t = self.snapshot.session().unwrap();
            wire::request(
                &self.state,
                &[
                    self.page_command("wheel"),
                    &t.id.to_string(),
                    &t.run,
                    &self.snapshot.cols.to_string(),
                    &self.snapshot.rows.to_string(),
                    &delta.to_string(),
                ],
            )
        } else {
            wire::request(
                &self.state,
                &[
                    self.page_command("scrollback"),
                    &t.id.to_string(),
                    &t.run,
                    &anchor,
                    &delta.to_string(),
                ],
            )
        }
        .and_then(|b| ScrollbackPage::decode(&b));
        self.selection = None;
        match result {
            Ok(page) if (page.cols, page.rows) == (self.snapshot.cols, self.snapshot.rows) => {
                self.notice.clear();
                self.scrollback = if page.available && page.position < page.end {
                    Some(Scrollback::new(
                        &self.snapshot,
                        self.terminal_layout(),
                        page,
                    ))
                } else {
                    None
                };
            }
            Ok(_) => {
                self.scrollback = None;
            }
            Err(e) => {
                self.scrollback = None;
                self.notice = format!("Scrollback: {}", wire::passive(&e.to_string()));
            }
        }
    }
    pub(super) fn wheel(&mut self, x: usize, y: usize, delta: i64, local: bool) {
        if !self.menu
            && self.confirm.is_none()
            && self.form.is_none()
            && !self.board
            && !self.selection.as_ref().is_some_and(|s| s.dragging)
            && (self.sidebar_wheel(x, y, delta)
                || self.pet_hit(x, y)
                || self.git_wheel(x, y, delta)
                || self.inspector_wheel(x, y, delta))
        {
            return;
        }
        if !self.menu
            && self.confirm.is_none()
            && self.form.is_none()
            && !self.board
            && let Some(group) = self.pane_at(x, y, false)
            && !self.pane_focus(group)
        {
            return;
        }
        let l = self.terminal_layout();
        if self.menu
            || self.confirm.is_some()
            || self.form.is_some()
            || self.board
            || self.selection.as_ref().is_some_and(|s| s.dragging)
            || x < l.terminal_x
            || x >= l.terminal_x + l.cols
            || y < l.terminal_y
            || y >= l.terminal_y + l.rows
        {
            return;
        }
        if let Some(p) = self
            .previews
            .get_mut(&(self.snapshot.active, self.snapshot.tab))
        {
            self.selection = None;
            let last = p.wrapped(l.cols).len().saturating_sub(l.rows - 1);
            p.offset = p.offset.saturating_add_signed(delta as isize).min(last);
        } else {
            self.smooth_scroll = None;
            self.scroll_page(delta, !local);
        }
    }
}
