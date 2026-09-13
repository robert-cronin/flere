//! Exact-target close dialog. Press and release must land on the same button.
use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: usize,
    y: usize,
    w: usize,
    h: usize,
}
impl Rect {
    fn contains(self, x: usize, y: usize) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}
struct Geometry {
    panel: Rect,
    cancel: Rect,
    close: Rect,
}
impl Geometry {
    fn new(l: Layout, directory: bool) -> Self {
        let w = 64.min(l.width.saturating_sub(2));
        let h = (if directory { 17 } else { 13 }).min(l.height);
        let panel = Rect {
            x: (l.width - w) / 2,
            y: (l.height - h) / 2,
            w,
            h,
        };
        let tall = h >= 11 && w >= 38;
        let bh = if tall { 3 } else { 1 };
        let bw = (w.saturating_sub(6) / 2).min(22);
        let buttons_y = panel.y + h - if tall { 5 } else { 3 };
        let cancel = Rect {
            x: panel.x + 2,
            y: buttons_y,
            w: bw,
            h: bh,
        };
        let close = Rect {
            x: panel.x + w - 2 - bw,
            y: buttons_y,
            w: bw,
            h: bh,
        };
        Self {
            panel,
            cancel,
            close,
        }
    }
    fn hit(&self, x: usize, y: usize) -> Option<bool> {
        if self.cancel.contains(x, y) {
            Some(false)
        } else if self.close.contains(x, y) {
            Some(true)
        } else {
            None
        }
    }
}
pub(super) struct Confirmation {
    id: u64,
    run: String,
    title: String,
    reason: String,
    epoch: String,
    workspace: u64,
    directory: Option<PathBuf>,
    selected: bool,
    pressed: Option<(bool, Layout)>,
}
impl Confirmation {
    pub fn new(t: &crate::model::TabView, epoch: &str, workspace: u64) -> Self {
        Self {
            id: t.id,
            run: t.run.clone(),
            title: t.title.clone(),
            reason: "Unsaved edits and running work may be lost.".into(),
            epoch: epoch.into(),
            workspace,
            directory: None,
            selected: false,
            pressed: None,
        }
    }
    pub fn directory(t: &crate::model::TabView, s: &Snapshot, path: PathBuf) -> Self {
        let mut confirm = Self::new(t, &s.epoch, s.active);
        confirm.title = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned();
        confirm.directory = Some(path);
        confirm
    }
    fn matches(&self, s: &Snapshot) -> bool {
        (self.directory.is_none() || s.tab == self.id)
            && s.epoch == self.epoch
            && s.active == self.workspace
            && s.workspace()
                .is_some_and(|w| w.tabs.iter().any(|t| t.id == self.id && t.run == self.run))
    }
    pub fn paint(&self, c: &mut Canvas, l: Layout) {
        let g = Geometry::new(l, self.directory.is_some());
        let r = g.panel;
        c.fill(r.x + 1, r.y + 1, r.w, r.h, style(BG, BG, false));
        c.fill(r.x, r.y, r.w, r.h, style(TEXT, PANEL, false));
        c.border(r.x, r.y, r.w, r.h, GOLD);
        let width = r.w.saturating_sub(4);
        c.text(
            r.x + 2,
            r.y + 1,
            width,
            if self.directory.is_some() {
                "Add project?"
            } else {
                "Close terminal?"
            },
            style(GOLD, PANEL, true),
        );
        c.text(
            r.x + 2,
            r.y + 2,
            width,
            &self.title,
            style(TEXT, PANEL, true),
        );
        if let Some(path) = &self.directory {
            let rows = g.cancel.y.saturating_sub(r.y + 5);
            for (i, line) in chrome::wrap(&path.to_string_lossy(), width, rows.max(1))
                .iter()
                .enumerate()
            {
                c.text(r.x + 2, r.y + 3 + i, width, line, style(CYAN, PANEL, false));
            }
            if rows >= 3 {
                c.text(
                    r.x + 2,
                    g.cancel.y - 2,
                    width,
                    "Adds a project card. Start a terminal when ready.",
                    style(TEXT, PANEL, false),
                );
            }
        } else if r.h >= 11 {
            c.text(
                r.x + 2,
                r.y + 3,
                width,
                &format!(
                    "Tab {} · run {}",
                    self.id,
                    &self.run[..self.run.len().min(8)]
                ),
                style(MUTED, PANEL, false),
            );
            c.text(
                r.x + 2,
                r.y + 5,
                width,
                &self.reason,
                style(TEXT, PANEL, false),
            );
            c.text(
                r.x + 2,
                r.y + 6,
                width,
                "Closing stops this terminal's process.",
                style(GOLD, PANEL, false),
            );
        } else if r.h >= 8 {
            c.text(
                r.x + 2,
                r.y + 3,
                width,
                "Unsaved work may be lost.",
                style(GOLD, PANEL, false),
            );
        }
        for (close, button) in [(false, g.cancel), (true, g.close)] {
            let selected = self.selected == close;
            let accent = if close && self.directory.is_none() {
                Color::Rgb(255, 119, 137)
            } else {
                CYAN
            };
            let bg = if selected { ACTIVE_BG } else { PANEL };
            let fg = accent;
            c.fill(
                button.x,
                button.y,
                button.w,
                button.h,
                style(fg, bg, selected),
            );
            if selected {
                for row in 0..button.h {
                    c.text(button.x, button.y + row, 1, "▌", style(accent, bg, true));
                }
            }
            let label = if close && self.directory.is_some() {
                if button.w >= 11 { "Add project" } else { "Add" }
            } else if close {
                if button.w >= 16 {
                    "Close terminal"
                } else {
                    "Close"
                }
            } else {
                "Cancel"
            };
            let label = if selected && button.w >= label.len() + 4 {
                format!("› {label} ‹")
            } else {
                label.into()
            };
            c.text(
                button.x + (button.w.saturating_sub(label.chars().count())) / 2,
                button.y + button.h / 2,
                button.w,
                &label,
                style(fg, bg, selected),
            );
        }
        c.text(
            r.x + 2,
            r.y + r.h - 2,
            width,
            "Tab / ← → choose · Enter select · Esc cancel",
            style(MUTED, PANEL, false),
        );
    }
}
pub(super) struct PendingClose {
    target: Confirmation,
    token: String,
    checked: Instant,
}
impl Ui {
    pub(super) fn request_close(&mut self, id: u64) {
        self.cancel_close();
        let Some(tab) = self
            .snapshot
            .workspace()
            .and_then(|w| w.tabs.iter().find(|t| t.id == id))
            .cloned()
        else {
            return;
        };
        self.flush_input();
        let target = Confirmation::new(&tab, &self.snapshot.epoch, self.snapshot.active);
        let result = wire::request(
            &self.state,
            &[
                "close-check",
                &target.epoch,
                &target.workspace.to_string(),
                &target.id.to_string(),
                &target.run,
            ],
        )
        .and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)
        });
        match result {
            Ok(value) if value["status"] == "pending" => {
                if let Some(token) = value["token"].as_str() {
                    self.pending_close = Some(PendingClose {
                        target,
                        token: token.into(),
                        checked: Instant::now(),
                    });
                    self.notice = "Checking whether this tab can close safely…".into();
                } else {
                    self.confirm = Some(target);
                }
            }
            Ok(value) if value["status"] == "closed" => {}
            Ok(value) => {
                let mut target = target;
                if let Some(reason) = value["reason"].as_str().filter(|r| !r.is_empty()) {
                    target.reason = wire::passive(reason);
                }
                self.confirm = Some(target);
            }
            Err(_) => self.confirm = Some(target), // Older supervisors keep the existing confirmation.
        }
    }
    pub(super) fn cancel_close(&mut self) {
        if let Some(p) = self.pending_close.take() {
            let t = &p.target;
            let _ = wire::request(
                &self.state,
                &[
                    "close-cancel",
                    &t.epoch,
                    &t.workspace.to_string(),
                    &t.id.to_string(),
                    &t.run,
                    &p.token,
                ],
            );
            self.notice.clear();
        }
    }
    pub(super) fn tick_close(&mut self) -> bool {
        let Some(p) = &mut self.pending_close else {
            return false;
        };
        if p.checked.elapsed() < Duration::from_millis(60) {
            return false;
        }
        p.checked = Instant::now();
        let t = &p.target;
        let result = wire::request(
            &self.state,
            &[
                "close-poll",
                &t.epoch,
                &t.workspace.to_string(),
                &t.id.to_string(),
                &t.run,
                &p.token,
            ],
        )
        .and_then(|bytes| {
            serde_json::from_slice::<serde_json::Value>(&bytes).map_err(io::Error::other)
        });
        if matches!(&result, Ok(v) if v["status"] == "pending") {
            if !t.matches(&self.snapshot) {
                self.cancel_close();
            }
            return false;
        }
        let p = self.pending_close.take().unwrap();
        let mut target = p.target;
        let _ = wire::request(
            &self.state,
            &[
                "close-cancel",
                &target.epoch,
                &target.workspace.to_string(),
                &target.id.to_string(),
                &target.run,
                &p.token,
            ],
        );
        self.notice.clear();
        if target.matches(&self.snapshot) && !matches!(&result, Ok(v) if v["status"] == "closed") {
            target.reason = match result {
                Ok(v) => wire::passive(
                    v["reason"]
                        .as_str()
                        .unwrap_or("The program could not verify a safe exit"),
                ),
                Err(_) => "The program could not verify a safe exit".into(),
            };
            self.confirm = Some(target);
        }
        true
    }
    pub(super) fn confirm_key(&mut self, key: &Key) -> bool {
        let Some(mut confirm) = self.confirm.take() else {
            return false;
        };
        if !confirm.matches(&self.snapshot) {
            self.notice = "Close cancelled: terminal changed".into();
            return true;
        }
        let g = Geometry::new(self.layout, confirm.directory.is_some());
        let mut choice = None;
        match key {
            Key::Bytes(b) => match b.as_slice() {
                b"\t" | b"\x1b[Z" => {
                    confirm.selected = !confirm.selected;
                    confirm.pressed = None;
                }
                b"\x1b[D" | b"h" => {
                    confirm.selected = false;
                    confirm.pressed = None;
                }
                b"\x1b[C" | b"l" => {
                    confirm.selected = true;
                    confirm.pressed = None;
                }
                b"\r" | b" " => choice = Some(confirm.selected),
                b"y" => choice = Some(true),
                b"\x1b" | b"n" | b"q" => choice = Some(false),
                _ => {}
            },
            Key::Mouse { x, y } => {
                confirm.pressed = g.hit(*x, *y).map(|b| (b, self.layout));
                if let Some((b, _)) = confirm.pressed {
                    confirm.selected = b;
                } else if !g.panel.contains(*x, *y) {
                    choice = Some(false);
                }
            }
            Key::Release { x, y } => {
                if let Some((pressed, layout)) = confirm.pressed.take()
                    && layout == self.layout
                    && g.hit(*x, *y) == Some(pressed)
                {
                    choice = Some(pressed);
                }
            }
            Key::Drag { x, y }
                if confirm
                    .pressed
                    .is_some_and(|(b, l)| l != self.layout || g.hit(*x, *y) != Some(b)) =>
            {
                confirm.pressed = None;
            }
            _ => {} // The dialog owns wheel, hover and pasted text too.
        }
        if let Some(close) = choice {
            self.paste_target = None;
            if close {
                if let Some(path) = &confirm.directory {
                    self.add_directory_project(&confirm, path);
                } else {
                    self.command(&["close", &confirm.id.to_string(), &confirm.run]);
                }
            }
        } else {
            self.confirm = Some(confirm);
        }
        true
    }
    fn add_directory_project(&mut self, confirm: &Confirmation, path: &Path) {
        let result = (|| -> io::Result<()> {
            self.flush_input();
            let fresh = self.read_snapshot()?;
            if !confirm.matches(&fresh) || !path.is_dir() || path.canonicalize()? != path {
                return Err(io::Error::other(
                    "Project click cancelled: directory or active terminal changed",
                ));
            }
            self.snapshot(fresh);
            if let Some(w) = self
                .snapshot
                .workspaces
                .iter()
                .find(|w| Path::new(&w.cwd) == path)
            {
                let (id, tab) = (w.id, w.tabs.first().map_or(0, |t| t.id));
                self.command(&["focus", &id.to_string(), &tab.to_string()]);
                self.notice = "Project directory is already in Flere".into();
                return Ok(());
            }
            let response = wire::request(
                &self.state,
                &[
                    "new-stopped",
                    &wire::hex(confirm.title.as_bytes()),
                    &wire::hex(path.to_string_lossy().as_bytes()),
                ],
            )?;
            let response: serde_json::Value =
                serde_json::from_slice(&response).map_err(io::Error::other)?;
            let id = response["workspace"]
                .as_u64()
                .ok_or_else(|| io::Error::other("Missing created workspace"))?;
            let meta = crate::workspace::CardMeta {
                project: confirm.title.clone(),
                ..Default::default()
            };
            self.set_meta(id, meta);
            self.focus = Focus::Cards;
            self.nav = true;
            self.side_scroll = None;
            if self.notice.is_empty() {
                self.notice = "Project added · t opens a terminal".into();
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.notice = wire::passive(&e.to_string());
        }
    }
}
