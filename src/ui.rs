//! Shared UI: one canvas, workspace-local child screens, no per-workspace sidebar processes.
use crate::{
    model::Snapshot,
    os,
    terminal::{Cell, Color, Style, char_width},
    wire,
    workspace::{Inspector, Preferences},
};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read, Write},
    os::fd::{AsFd, AsRawFd},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
mod arcade;
mod avatars;
mod cards;
mod checkout;
mod chrome;
mod confirmation;
mod context_menu;
mod explorer;
mod git_history;
mod github;
mod hyperlinks;
mod image_preview;
mod intro;
mod local_graphics;
mod panes;
mod pet;
mod polish;
mod project_picker;
mod remote;
mod remote_tools;
mod resume;
mod screensaver;
mod screenshot;
mod scrollback;
mod search;
mod selection;
mod sidebar;
mod tasks;
mod update;
mod viewport;
mod workflows;
use explorer::Explorer;
use workflows::{Form, GitResult};
// Flere chrome uses a restrained neon palette; child terminal colors stay native.
const BG: Color = Color::Rgb(6, 14, 22);
const PANEL: Color = Color::Rgb(9, 18, 27);
const TEXT: Color = Color::Rgb(216, 226, 238);
const MUTED: Color = Color::Rgb(122, 140, 175);
const CYAN: Color = Color::Rgb(67, 227, 247);
const MAGENTA: Color = Color::Rgb(177, 151, 230);
const ACTIVE_BG: Color = Color::Rgb(14, 42, 55);
const BORDER: Color = Color::Rgb(35, 54, 68);
const WORKING: Color = Color::Rgb(104, 172, 255);
const GOLD: Color = Color::Rgb(255, 205, 110);
fn style(fg: Color, bg: Color, bold: bool) -> Style {
    Style {
        fg,
        bg,
        bold,
        ..Style::default()
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Focus {
    Terminal,
    Cards,
    Files,
    Tabs,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    pub left: usize,
    pub right: usize,
    pub terminal_x: usize,
    pub terminal_y: usize,
    pub cols: usize,
    pub rows: usize,
}
impl Layout {
    pub fn new(width: usize, height: usize) -> Self {
        Self::with_preferences(width, height, &Preferences::default())
    }
    pub fn with_preferences(width: usize, height: usize, prefs: &Preferences) -> Self {
        let width = width.clamp(10, 320);
        let height = height.clamp(8, 106);
        let left = if prefs.zoom {
            0
        } else if width >= 100 {
            prefs.left.clamp(16, 70).min((width - 42) / 2)
        } else if width >= 56 {
            prefs.left.clamp(16, 30).min(width - 34)
        } else {
            0
        };
        let right = if !prefs.zoom && width >= 100 {
            prefs.right.clamp(16, 70).min(width - left - 34)
        } else {
            0
        };
        let cols = (width - left - right - 2).clamp(2, 240);
        let rows = (height - 6).clamp(2, 100);
        Self {
            width,
            height,
            left,
            right,
            terminal_x: left + 1,
            terminal_y: 4,
            cols,
            rows,
        }
    }
}
struct Canvas {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
    badges: Vec<crate::avatar::Badge>,
}
impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Self {
            width: w,
            height: h,
            badges: Vec::new(),
            cells: vec![
                Cell {
                    style: style(TEXT, BG, false),
                    ..Cell::default()
                };
                w * h
            ],
        }
    }
    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, s: Style) {
        self.badges.retain(|b| {
            !(usize::from(b.x) < x + w
                && usize::from(b.x) + usize::from(b.columns) > x
                && usize::from(b.y) >= y
                && usize::from(b.y) < y + h)
        });
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.cells[yy * self.width + xx] = Cell {
                    style: s,
                    ..Cell::default()
                }
            }
        }
    }
    fn text(&mut self, x: usize, y: usize, max: usize, text: &str, s: Style) {
        if y >= self.height {
            return;
        }
        self.badges.retain(|b| {
            !(usize::from(b.y) == y
                && usize::from(b.x) < x + max
                && usize::from(b.x) + usize::from(b.columns) > x)
        });
        let mut col = x;
        for c in wire::passive(text).chars() {
            let w = char_width(c) as usize;
            if w == 0 {
                if col > x {
                    self.cells[y * self.width + col - 1].text.push(c)
                }
                continue;
            }
            if col + w > self.width || col + w > x + max {
                break;
            }
            self.cells[y * self.width + col] = Cell {
                text: c.to_string(),
                width: w as u8,
                style: s,
                link: None,
            };
            if w == 2 {
                self.cells[y * self.width + col + 1] = Cell {
                    text: String::new(),
                    width: 0,
                    style: s,
                    link: None,
                }
            }
            col += w;
        }
    }
    fn border(&mut self, x: usize, y: usize, w: usize, h: usize, color: Color) {
        if w < 2 || h < 2 {
            return;
        }
        let s = style(color, BG, false);
        for xx in x + 1..x + w - 1 {
            self.text(xx, y, 1, "─", s);
            self.text(xx, y + h - 1, 1, "─", s);
        }
        for yy in y + 1..y + h - 1 {
            self.text(x, yy, 1, "│", s);
            self.text(x + w - 1, yy, 1, "│", s);
        }
        for (xx, yy, c) in [
            (x, y, "┌"),
            (x + w - 1, y, "┐"),
            (x, y + h - 1, "└"),
            (x + w - 1, y + h - 1, "┘"),
        ] {
            self.text(xx, yy, 1, c, s)
        }
    }
}
#[derive(Clone)]
enum PreviewAction {
    Git,
    Message { wid: u64, id: String },
    Decision { wid: u64, value: serde_json::Value },
}
struct Preview {
    name: String,
    lines: Vec<String>,
    offset: usize,
    action: Option<PreviewAction>,
}
impl Preview {
    fn wrapped(&self, cols: usize) -> Vec<String> {
        let cols = cols.max(2);
        let mut out = Vec::new();
        for line in &self.lines {
            let mut row = String::new();
            let mut width = 0;
            for ch in wire::passive(line).chars() {
                let n = char_width(ch) as usize;
                if n > 0 && width + n > cols {
                    out.push(std::mem::take(&mut row));
                    width = 0;
                }
                row.push(ch);
                width += n;
            }
            out.push(row);
        }
        out
    }
}
struct Ui {
    state: PathBuf,
    pane_capable: bool,
    link_capable: bool,
    snapshot: Snapshot,
    layout: Layout,
    focus: Focus,
    side_scroll: Option<usize>,
    cards: cards::Cards,
    nav: bool,
    menu: bool,
    menu_index: usize,
    menu_query: Vec<u8>,
    menu_search: bool,
    nav_help_at: Option<Instant>,
    mouse_hint: Option<(String, Instant)>,
    prefs: Preferences,
    form: Option<Form>,
    search: Option<search::Search>,
    update: Option<update::Update>,
    tasks: Option<tasks::Panel>,
    board: bool,
    board_column: usize,
    git_views: HashMap<u64, crate::git::GitView>,
    git_job: Option<std::sync::mpsc::Receiver<GitResult>>,
    git_checked: Instant,
    git_workspace: u64,
    git_history: HashMap<u64, git_history::Pane>,
    history_job: Option<std::sync::mpsc::Receiver<git_history::Result>>,
    history_pending: Option<git_history::Request>,
    git_hover: Option<(String, usize, Instant, bool)>,
    confirm: Option<confirmation::Confirmation>,
    context_menu: Option<context_menu::Menu>,
    pending_close: Option<confirmation::PendingClose>,
    screenshot_job: Option<screenshot::Job>,
    explorers: HashMap<u64, Explorer>,
    explorer_loader: explorer::Loader,
    image_loader: image_preview::Loader,
    details_scroll: HashMap<u64, usize>,
    previews: HashMap<(u64, u64), Preview>,
    notice: String,
    history: Vec<(u64, u64, String)>,
    history_index: usize,
    restore_pending: bool,
    quit: bool,
    refresh: bool,
    refresh_to: Option<PathBuf>,
    refresh_attempt: Option<String>,
    update_ack: Option<(String, Instant)>,
    animation_step: usize,
    animation_checked: Instant,
    drag: Option<bool>,
    pane_drag: Option<panes::DividerDrag>,
    selection: Option<selection::Selection>,
    clipboard: Option<String>,
    scrollback: Option<scrollback::Scrollback>,
    parked_scrollback: HashMap<(u64, u64, String), scrollback::Scrollback>,
    smooth_scroll: Option<scrollback::SmoothScroll>,
    coordination: serde_json::Value,
    input: Input,
    outgoing: Vec<(u64, String, Vec<u8>)>,
    paste_target: Option<(u64, String, bool)>,
    remote: Option<remote::Connection>,
    remote_tools: remote_tools::Tools,
    local_graphics: Option<local_graphics::Graphics>,
    project_icons: avatars::Projects,
    checkouts: checkout::Checkouts,
    pet: pet::Pet,
    screensaver: screensaver::Saver,
    arcade: Option<arcade::Arcade>,
    arcade_gate: arcade::Gate,
    arcade_keyboard: arcade::Keyboard,
    arcade_checked: Instant,
    arcade_nav: bool,
}
impl Ui {
    fn flush_input(&mut self) {
        for (id, run, bytes) in std::mem::take(&mut self.outgoing) {
            if let Err(e) = wire::request(
                &self.state,
                &["input", &id.to_string(), &run, &wire::hex(&bytes)],
            ) {
                self.notice = e.to_string();
            }
        }
    }
    fn command(&mut self, fields: &[&str]) {
        self.flush_input();
        match wire::request(&self.state, fields) {
            Ok(_) => {
                if fields[0] == "focus" {
                    self.side_scroll = None;
                }
                self.notice.clear();
                if matches!(
                    fields[0],
                    "new"
                        | "add-project"
                        | "tab"
                        | "focus"
                        | "focus-exact"
                        | "notes"
                        | "open"
                        | "open-epoch"
                        | "open-diff-epoch"
                        | "open-pane-epoch"
                        | "open-diff-pane-epoch"
                        | "open-at-pane-epoch"
                        | "task-start"
                        | "metadata"
                        | "rename"
                        | "native"
                        | "resume-saved"
                        | "worktree"
                        | "pane"
                        | "resize-panes"
                ) && let Ok(s) = self.read_snapshot()
                {
                    self.snapshot(s);
                }
            }
            Err(e) => self.notice = wire::passive(&e.to_string()),
        }
    }
    fn snapshot(&mut self, s: Snapshot) {
        let started = Instant::now();
        if s.epoch == self.snapshot.epoch && s.generation < self.snapshot.generation {
            return;
        }
        if s.notice != self.snapshot.notice && !s.notice.is_empty() {
            self.notice = s.notice.clone();
        }
        let changed_target = (s.epoch.as_str(), s.active, s.tab)
            != (
                self.snapshot.epoch.as_str(),
                self.snapshot.active,
                self.snapshot.tab,
            );
        if changed_target {
            if let Some(view) = self.scrollback.take()
                && let Some(tab) = self.snapshot.session()
            {
                self.parked_scrollback
                    .insert((self.snapshot.active, tab.id, tab.run.clone()), view);
            }
            self.smooth_scroll = None;
            self.scrollback = s.session().and_then(|tab| {
                self.parked_scrollback
                    .remove(&(s.active, tab.id, tab.run.clone()))
            });
        }
        let terminal_layout = panes::terminal_layout(self.layout, &s);
        if self.selection.as_ref().is_some_and(|selection| {
            !selection.matches(&s, terminal_layout)
                || !selection.dragging
                    && self.scrollback.is_none()
                    && s.cells != self.snapshot.cells
        }) {
            self.selection = None;
        }
        if self
            .scrollback
            .as_ref()
            .is_some_and(|view| !view.matches(&s, terminal_layout))
        {
            self.scrollback = None;
            self.smooth_scroll = None;
            self.selection = None;
        }
        if s.epoch != self.snapshot.epoch || s.active != self.snapshot.active {
            self.cards.cursor = None;
            self.side_scroll = None;
        }
        if s.epoch != self.snapshot.epoch {
            self.parked_scrollback.clear();
            self.previews.clear();
            self.explorers.clear();
            self.details_scroll.clear();
            self.git_history.clear();
            self.git_views.clear();
            self.history_pending = None;
        }
        if (s.epoch.as_str(), s.active, s.tab)
            != (
                self.snapshot.epoch.as_str(),
                self.snapshot.active,
                self.snapshot.tab,
            )
        {
            self.git_hover = None;
        }
        let old = (self.snapshot.active, self.snapshot.tab);
        self.snapshot = s;
        self.parked_scrollback.retain(|(wid, id, run), _| {
            self.snapshot
                .workspaces
                .iter()
                .any(|w| w.id == *wid && w.tabs.iter().any(|t| t.id == *id && t.run == *run))
        });
        if self.parked_scrollback.len() > 128 {
            self.parked_scrollback.clear();
        }
        self.card_reconcile();
        let location = (
            self.snapshot.active,
            self.snapshot.tab,
            self.snapshot
                .session()
                .map(|t| t.run.clone())
                .unwrap_or_default(),
        );
        if old != (self.snapshot.active, self.snapshot.tab)
            && self.history.get(self.history_index) != Some(&location)
        {
            self.history.truncate(self.history_index + 1);
            self.history.push(location);
            if self.history.len() > 128 {
                self.history.remove(0);
            }
            self.history_index = self.history.len() - 1;
        }
        if let Some(w) = self.snapshot.workspace() {
            self.explorers
                .entry(w.id)
                .or_insert_with(|| Explorer::new(PathBuf::from(&w.cwd)));
        }
        crate::diagnostics::slow("snapshot-apply", started);
    }
    fn queue_input(&mut self, id: u64, run: &str, b: &[u8]) {
        if let Some((last_id, last_run, bytes)) = self.outgoing.last_mut()
            && *last_id == id
            && last_run == run
            && bytes.len() + b.len() <= 65536
        {
            bytes.extend_from_slice(b)
        } else {
            self.outgoing.push((id, run.into(), b.to_vec()))
        }
    }
    fn send(&mut self, b: &[u8]) {
        if let Some(t) = self.snapshot.session() {
            let (id, run) = (t.id, t.run.clone());
            if self.snapshot.app_cursor
                && b.len() == 3
                && b.starts_with(b"\x1b[")
                && matches!(b[2], b'A'..=b'D' | b'H' | b'F')
            {
                self.queue_input(id, &run, &[27, b'O', b[2]])
            } else {
                self.queue_input(id, &run, b)
            }
        }
    }
    fn workspace(&mut self, delta: isize) {
        let ws = self.visible_cards();
        if ws.is_empty() {
            return;
        }
        let current = ws
            .iter()
            .position(|id| *id == self.snapshot.active)
            .unwrap_or(0);
        let index = (current as isize + delta).rem_euclid(ws.len() as isize) as usize;
        self.command(&["focus", &ws[index].to_string(), "0"]);
    }

    fn history(&mut self, delta: isize) {
        let mut i = self.history_index as isize + delta;
        while i >= 0 && i < self.history.len() as isize {
            let (wid, tid, run) = self.history[i as usize].clone();
            let valid = self.snapshot.workspaces.iter().any(|w| {
                w.id == wid
                    && !w.meta.archived
                    && (tid == 0 || w.tabs.iter().any(|t| t.id == tid && t.run == run))
            });
            if valid {
                self.history_index = i as usize;
                self.command(&[
                    "focus-exact",
                    &self.snapshot.epoch.clone(),
                    &wid.to_string(),
                    &tid.to_string(),
                    &run,
                ]);
                break;
            }
            i += delta;
        }
    }

    fn tab(&mut self, delta: isize) {
        if let Some(w) = self.snapshot.workspace() {
            let tabs = self.pane_tabs(self.pane_group());
            if tabs.is_empty() {
                return;
            }
            let current = tabs
                .iter()
                .position(|t| t.id == self.snapshot.tab)
                .unwrap_or(0);
            let i = (current as isize + delta).rem_euclid(tabs.len() as isize) as usize;
            let (id, tab, run) = (
                w.id.to_string(),
                tabs[i].id.to_string(),
                tabs[i].run.clone(),
            );
            self.command(&["focus-exact", &self.snapshot.epoch.clone(), &id, &tab, &run]);
        }
    }
    fn new_workspace(&mut self) {
        self.open_form("New worktree");
    }

    fn new_tab(&mut self) {
        if self.pane_capable && self.snapshot.session().is_some() {
            self.pane_action("new-tab", None);
        } else {
            let id = self.snapshot.active.to_string();
            self.command(&["tab", &id]);
        }
        self.menu = false;
        self.nav = false;
        self.focus = Focus::Terminal;
    }
    fn file_enter(&mut self) {
        let path = self
            .explorers
            .get(&self.snapshot.active)
            .and_then(|e| e.entries.get(e.selected))
            .filter(|(_, p, dir)| !dir && image_preview::image_path(p))
            .map(|(_, p, _)| p.clone());
        if let Some(path) = path {
            self.image_open(&path);
        } else {
            self.file_editor();
        }
    }
    fn file_editor(&mut self) {
        if let Some(ex) = self.explorers.get_mut(&self.snapshot.active)
            && let Some((_, path, dir)) = ex.entries.get(ex.selected).cloned()
        {
            if dir {
                ex.load(path);
            } else {
                if !self.editor_open(&self.editor_origin(), &path, None) {
                    return;
                }
                self.previews
                    .remove(&(self.snapshot.active, self.snapshot.tab));
                self.focus = Focus::Files;
                self.nav = false;
            }
        }
    }

    fn key(&mut self, key: Key) {
        if let Key::TerminalReply(bytes) = key {
            if let Some(g) = &mut self.local_graphics {
                g.reply(&bytes);
            }
            return;
        }
        if self.arcade_key(&key) {
            return;
        }
        if self.screensaver.consume(&key, Instant::now()) {
            return;
        }
        if self.update_key(&key)
            || self.search_key(&key)
            || self.remote_tools_key(&key)
            || self.tasks_key(&key)
        {
            return;
        }
        if self.context_key(&key) {
            return;
        }
        if self.pending_close.is_some() && matches!(&key, Key::Bytes(b) if b == b"\x1b") {
            self.cancel_close();
            return;
        }
        if self.confirm_key(&key) {
            return;
        }
        if self.card_popup_key(&key) {
            return;
        }
        if self.pet_key(&key) {
            return;
        }
        if matches!(&key, Key::Bytes(b) if b == b"\x1b") && self.remote_escape() {
            return;
        }
        if let Key::Hover { x, y } = key {
            self.git_hover_at(x, y);
            return;
        }
        if matches!(key, Key::Bytes(_) | Key::Mouse { .. }) {
            self.git_hover = None;
        }
        if matches!(key, Key::Bytes(_) | Key::Paste(_))
            && self.selection.take().is_some()
            && matches!(&key, Key::Bytes(b) if b == b"\x1b")
        {
            return; // Escape clears selection without interrupting the native chat.
        }
        if !self.menu && self.form.is_none() && self.confirm.is_none() && self.image_key(&key) {
            return;
        }
        if !self.menu
            && self.confirm.is_none()
            && self.form.is_none()
            && self.focus == Focus::Files
            && self.prefs.inspector == Inspector::Files
            && matches!(&key, Key::Bytes(b) if b == b"E")
        {
            self.file_editor();
            return;
        }
        if self.form.is_some() {
            self.form_key(key);
            return;
        }
        if let Key::Wheel { x, y, delta, local } = key {
            self.wheel(x, y, delta, local);
            return;
        }
        if let Key::Paste(b) = key {
            if self.menu {
                if self.menu_search {
                    let v = b.strip_prefix(b"\x1b[200~").unwrap_or(&b);
                    let v = v.strip_suffix(b"\x1b[201~").unwrap_or(v);
                    self.append_action_query(v);
                }
                return;
            }
            if b.starts_with(b"\x1b[200~") {
                self.paste_target = if !self.nav
                    && !self.menu
                    && self.confirm.is_none()
                    && self.focus == Focus::Terminal
                    && !self
                        .previews
                        .contains_key(&(self.snapshot.active, self.snapshot.tab))
                {
                    self.snapshot
                        .session()
                        .map(|t| (t.id, t.run.clone(), self.snapshot.bracketed_paste))
                } else {
                    None
                };
            }
            if let Some((id, run, bracketed)) = self.paste_target.clone() {
                self.scrollback = None;
                let bytes = if bracketed {
                    b.as_slice()
                } else {
                    let v = b.strip_prefix(b"\x1b[200~").unwrap_or(&b);
                    v.strip_suffix(b"\x1b[201~").unwrap_or(v)
                };
                self.queue_input(id, &run, bytes);
            }
            if b.ends_with(b"\x1b[201~") {
                self.paste_target = None;
            }
            return;
        }
        if let Key::Drag { x, y } = key {
            if self.pane_divider_drag(x, y) {
                return;
            }
            if let Some(selection) = &mut self.selection {
                if selection.dragging {
                    selection.extend(x, y);
                }
            } else if let Some(left) = self.drag {
                if left {
                    self.prefs.left = x.clamp(16, 70);
                } else {
                    self.prefs.right = self.layout.width.saturating_sub(x).clamp(16, 70);
                }
                self.save_preferences();
            }
            return;
        }
        if let Key::Release { x, y } = key {
            if self.pane_drag.is_some() {
                self.pane_divider_drag(x, y);
                self.pane_drag = None;
                return;
            }
            self.drag = None;
            if let Some(mut selection) = self.selection.take()
                && selection.dragging
                && selection.matches(&self.snapshot, self.terminal_layout())
            {
                selection.extend(x, y);
                selection.dragging = false;
                if let Some(path) = self
                    .snapshot
                    .workspace()
                    .and_then(|w| selection.file_path(Path::new(&w.cwd)))
                {
                    self.terminal_file_open(&path);
                    return;
                }
                match selection.text() {
                    Ok(text) if !text.is_empty() => match selection::clipboard_write(&text) {
                        Ok(sequence) => {
                            self.clipboard = Some(sequence);
                            self.notice = "Selection sent to clipboard".into();
                            self.selection = Some(selection);
                        }
                        Err(e) => self.notice = e.into(),
                    },
                    Err(e) => self.notice = e.into(),
                    _ => {}
                }
            }
            return;
        }
        if let Key::Mouse { x, y } = key {
            if self.confirm.is_some() || self.menu {
                return;
            }
            self.selection = None;
            self.mouse(x, y);
            let l = self.terminal_layout();
            if !self.board
                && self.focus == Focus::Terminal
                && self.confirm.is_none()
                && x >= l.terminal_x
                && x < l.terminal_x + l.cols
                && y >= l.terminal_y
                && y < l.terminal_y + l.rows
                && (self.snapshot.session().is_some()
                    || self
                        .previews
                        .contains_key(&(self.snapshot.active, self.snapshot.tab)))
            {
                let mut selection =
                    selection::Selection::new(&self.snapshot, l, &self.draw(), x, y);
                self.smooth_scroll = None;
                self.selection_history(&mut selection);
                self.selection = Some(selection);
            }
            return;
        }
        let Key::Bytes(mut b) = key else { return };
        if self.menu {
            if self.actions_key(&mut b) {
                return;
            }
            if self.workflow_key(&b) {
                return;
            }
            match b.as_slice() {
                b"n" => self.new_workspace(),
                b"t" => self.new_tab(),
                b"q" => self.quit = true,
                b"b" => {
                    self.history(-1);
                    self.menu = false
                }
                b"f" => {
                    self.history(1);
                    self.menu = false
                }
                b"r" => {
                    if let Some(e) = self.explorers.get_mut(&self.snapshot.active) {
                        e.load(e.root.clone())
                    }
                    self.menu = false
                }
                _ => self.menu = false,
            }
            return;
        }
        self.mouse_hint = None;
        self.nav_help_at = None;
        if b == b"\0" {
            self.nav = if matches!(self.focus, Focus::Cards | Focus::Files) {
                true
            } else {
                !self.nav
            };
            if self.nav {
                self.nav_help_at = Some(Instant::now() + Duration::from_millis(400));
            }
            if !self.nav {
                self.smooth_scroll = None;
                self.focus = Focus::Terminal;
                self.scrollback = None;
            }
            return;
        }
        if self.stopped_key(&b) {
            return;
        }
        if self.inspector_page_key(&b) {
            return;
        }
        if self.git_key(&b) {
            return;
        }
        if self.board && self.board_key(&b) {
            return;
        }
        if self.focus == Focus::Files && self.prefs.inspector == Inspector::Files && b == b"r" {
            if let Some(e) = self.explorers.get_mut(&self.snapshot.active) {
                e.load(e.root.clone());
            }
            return;
        }
        if self.nav {
            if self.card_nav_key(&b) {
                return;
            }
            if self.pane_nav_key(&b) || self.nav_scroll_key(&b) || self.workflow_key(&b) {
                return;
            }
            match b.as_slice() {
                b"\x1b" => {
                    self.smooth_scroll = None;
                    self.nav = false;
                    self.focus = Focus::Terminal;
                    self.scrollback = None;
                }
                b"\r" => {
                    self.smooth_scroll = None;
                    self.nav = false;
                    if self.focus != Focus::Files {
                        self.focus = Focus::Terminal;
                        self.scrollback = None;
                        self.resume_saved_chat();
                    } else {
                        self.inspector_enter(false)
                    }
                }
                b" " => {
                    self.menu = true;
                    self.menu_index = 0;
                    self.menu_query.clear();
                    self.menu_search = false;
                }
                b"q" => self.quit = true,
                b"n" => self.new_workspace(),
                b"t" => self.new_tab(),
                b"b" => self.history(-1),
                b"f" => self.history(1),
                b"\t" if matches!(self.focus, Focus::Terminal | Focus::Tabs) => self.tab(1),
                b"\x1b[Z" if matches!(self.focus, Focus::Terminal | Focus::Tabs) => self.tab(-1),
                b"h" | b"\x1b[D" => {
                    self.focus = match self.focus {
                        Focus::Files => Focus::Terminal,
                        Focus::Tabs => {
                            self.tab(-1);
                            Focus::Tabs
                        }
                        _ => Focus::Cards,
                    }
                }
                b"l" | b"\x1b[C" => {
                    self.focus = match self.focus {
                        Focus::Cards => Focus::Terminal,
                        Focus::Tabs => {
                            self.tab(1);
                            Focus::Tabs
                        }
                        _ => Focus::Files,
                    }
                }
                b"j" | b"\x1b[B" => match self.focus {
                    Focus::Cards => self.workspace(1),
                    Focus::Files => self.inspector_move(1),
                    Focus::Tabs => self.focus = Focus::Terminal,
                    Focus::Terminal => self.focus = Focus::Tabs,
                },
                b"k" | b"\x1b[A" => match self.focus {
                    Focus::Cards => self.workspace(-1),
                    Focus::Files => self.inspector_move(-1),
                    _ => self.focus = Focus::Tabs,
                },
                b"-" | b"\x7f"
                    if self.focus == Focus::Files && self.prefs.inspector == Inspector::Files =>
                {
                    if let Some(e) = self.explorers.get_mut(&self.snapshot.active)
                        && let Some(p) = e.root.parent()
                    {
                        e.load(p.into())
                    }
                }
                b"x" => {
                    if let Some(t) = self.snapshot.session() {
                        self.request_close(t.id);
                    }
                }
                _ => {}
            }
            return;
        }
        let preview_layout = self.terminal_layout();
        if let Some(p) = self
            .previews
            .get_mut(&(self.snapshot.active, self.snapshot.tab))
        {
            let last = p.wrapped(preview_layout.cols).len().saturating_sub(1);
            match b.as_slice() {
                b"a" => {
                    let action = p.action.clone();
                    match action {
                        Some(PreviewAction::Message { wid, id }) if wid == self.snapshot.active => {
                            match self.coordinate(
                                wid,
                                "inbox",
                                &serde_json::json!({"ack_ids":[id]}),
                            ) {
                                Ok(_) => {
                                    self.notice = "Message acknowledged".into();
                                    self.previews.remove(&(wid, self.snapshot.tab));
                                }
                                Err(e) => self.notice = e.to_string(),
                            }
                        }
                        Some(PreviewAction::Decision { wid, value })
                            if wid == self.snapshot.active =>
                        {
                            self.previews.remove(&(wid, self.snapshot.tab));
                            self.answer_form(wid, value);
                        }
                        _ => {}
                    }
                }
                b"q" | b"\x1b" => {
                    let git = matches!(p.action, Some(PreviewAction::Git));
                    self.previews
                        .remove(&(self.snapshot.active, self.snapshot.tab));
                    if git {
                        self.focus = Focus::Files;
                        self.nav = false;
                    }
                }
                b"j" | b"\x1b[B" => p.offset = (p.offset + 1).min(last),
                b"k" | b"\x1b[A" => p.offset = p.offset.saturating_sub(1),
                b"\x1b[6~" => p.offset = (p.offset + preview_layout.rows).min(last),
                b"\x1b[5~" => p.offset = p.offset.saturating_sub(preview_layout.rows),
                _ => {}
            }
            return;
        }
        if self.focus == Focus::Terminal {
            match b.as_slice() {
                b"\x1b[1;2H" | b"\x1b[1;2~" | b"\x1b[7;2~" => {
                    self.scroll_terminal(i64::MIN);
                    return;
                }
                b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" | b"\x1b[7~" if self.scrollback.is_some() => {
                    self.scroll_terminal(i64::MIN);
                    return;
                }
                b"\x1b[5;2~" => {
                    self.scroll_terminal(-(self.terminal_layout().rows as i64 - 1));
                    return;
                }
                b"\x1b[6;2~" => {
                    self.scroll_terminal(self.terminal_layout().rows as i64 - 1);
                    return;
                }
                b"\x1b[5~" if self.scrollback.is_some() => {
                    self.scroll_terminal(-(self.terminal_layout().rows as i64 - 1));
                    return;
                }
                b"\x1b[6~" if self.scrollback.is_some() => {
                    self.scroll_terminal(self.terminal_layout().rows as i64 - 1);
                    return;
                }
                b"\x1b[A" if self.scrollback.is_some() => {
                    self.scroll_terminal(-1);
                    return;
                }
                b"\x1b[B" if self.scrollback.is_some() => {
                    self.scroll_terminal(1);
                    return;
                }
                b"\x1b" | b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" | b"\r"
                    if self.scrollback.is_some() =>
                {
                    self.scrollback = None;
                    return;
                }
                _ => {
                    self.scrollback = None;
                }
            }
        }
        if self.focus == Focus::Files {
            match b.as_slice() {
                b"i" => {
                    self.prefs.inspector = self.prefs.inspector.next();
                    self.save_preferences();
                }
                b"d" => self.inspector_enter(true),
                b"\r" => self.inspector_enter(false),
                b"j" | b"\x1b[B" => self.inspector_move(1),
                b"k" | b"\x1b[A" => self.inspector_move(-1),
                b"-" | b"\x7f" => {
                    if let Some(e) = self.explorers.get_mut(&self.snapshot.active)
                        && let Some(p) = e.root.parent()
                    {
                        e.load(p.into())
                    }
                }
                _ => {}
            }
        } else {
            self.send(&b)
        }
    }
    fn pane_tab_slots(&self, group: usize, l: Layout) -> Vec<(usize, usize, u64, String)> {
        let mut slots = Vec::new();
        let tabs = self.pane_tabs(group);
        let selected = tabs
            .iter()
            .position(|t| t.id == self.pane_selected(group))
            .unwrap_or(0);
        let end = (l.terminal_x + l.cols).saturating_sub(4);
        let mut start = selected.saturating_sub(2);
        loop {
            slots.clear();
            let mut x = l.terminal_x + 1;
            for t in tabs.iter().skip(start) {
                let title: String = wire::passive(&t.title).chars().take(16).collect();
                let text = format!(
                    " {}{} × ",
                    if t.working {
                        workflows::activity_glyph(self.animation_step, self.prefs.reduced_motion)
                    } else if t.kind == "editor" {
                        "▤"
                    } else {
                        ""
                    },
                    title
                );
                let size = text
                    .chars()
                    .map(|c| char_width(c) as usize)
                    .sum::<usize>()
                    .min(end.saturating_sub(x));
                if size < 3 {
                    break;
                }
                slots.push((x, size, t.id, text));
                x += size + 1;
            }
            if slots
                .iter()
                .any(|(_, _, id, _)| *id == self.pane_selected(group))
                || start >= selected
            {
                break;
            }
            start += 1;
        }
        slots
    }
    fn mouse(&mut self, x: usize, y: usize) {
        if self.pet_hit(x, y) {
            self.pet_click(x, y);
            return;
        }
        let l = self.layout;
        if self.board_click(x, y) {
            return;
        }
        if l.left > 0 && x == l.left - 1 {
            self.drag = Some(true);
            return;
        }
        if l.right > 0 && x == l.width - l.right {
            self.drag = Some(false);
            return;
        }
        if ((l.left > 0 && x < l.left) || (l.left == 0 && self.focus == Focus::Cards))
            && !(l.right == 0 && self.focus == Focus::Files)
        {
            self.nav = true;
            self.focus = Focus::Cards;
            if !self.card_click(x, y) {
                self.sidebar_click(y);
            }
            self.show_mouse_hint("Ctrl+Space, h · j/k select workspace");
        } else if (l.right > 0 && x >= l.width - l.right)
            || (l.right == 0 && self.focus == Focus::Files)
        {
            self.focus = Focus::Files;
            self.nav = false;
            if y == 2 {
                if let Some((_, _, inspector, _)) = self
                    .inspector_tabs()
                    .into_iter()
                    .find(|(start, width, _, _)| x >= *start && x < start + width)
                {
                    self.prefs.inspector = inspector;
                    self.save_preferences();
                    self.show_mouse_hint(match inspector {
                        Inspector::Git => "Ctrl+Space, g · Git inspector",
                        Inspector::Details => "Ctrl+Space, v · Details",
                        Inspector::Files => "Ctrl+Space, i · cycle inspector",
                    });
                }
                return;
            }
            if self.prefs.inspector == Inspector::Git && y >= 5 {
                self.git_click(x, y);
                return;
            }
            if self.prefs.inspector == Inspector::Files && y >= 5 && y < 5 + self.file_rows() {
                let rows = self.file_rows();
                if let Some(e) = self.explorers.get_mut(&self.snapshot.active) {
                    let offset = e.start(rows);
                    let index = offset + y - 5;
                    if index < e.entries.len() {
                        e.selected = index;
                        self.show_mouse_hint("j/k select · Enter open · - parent");
                    }
                }
            }
        } else {
            if self.pane_divider_start(x, y) {
                return;
            }
            // Freeze the displayed tab/action before focusing another group:
            // focus performs a socket roundtrip and can reveal newer layout state.
            let group = self.pane_at(x, y, true);
            let tab_layout = group
                .and_then(|g| self.pane_layouts()[g].map(|l| (g, l)))
                .filter(|(_, l)| y == l.terminal_y - 2);
            let slots = tab_layout.map(|(g, l)| self.pane_tab_slots(g, l));
            let plus = slots
                .as_ref()
                .zip(tab_layout)
                .is_some_and(|(slots, (_, l))| {
                    let at = slots
                        .last()
                        .map_or(l.terminal_x + 1, |(x, width, _, _)| x + width + 1);
                    (at..at + 3).contains(&x)
                });
            let clicked = slots.as_ref().and_then(|slots| {
                let (start, width, id, _) = slots
                    .iter()
                    .find(|(start, width, _, _)| x >= *start && x < start + width)?;
                let run = self
                    .snapshot
                    .workspace()?
                    .tabs
                    .iter()
                    .find(|t| t.id == *id)?
                    .run
                    .clone();
                Some((*id, run, x >= start + width.saturating_sub(2)))
            });
            if let Some(group) = group
                && !self.pane_focus(group)
            {
                return;
            }
            if plus {
                self.new_tab();
                self.show_mouse_hint("Ctrl+Space, t · new terminal");
                return;
            }
            if let Some((id, run, close)) = clicked {
                if !self
                    .snapshot
                    .workspace()
                    .is_some_and(|w| w.tabs.iter().any(|t| t.id == id && t.run == run))
                {
                    self.notice = "Tab changed in another attachment; select again".into();
                    return;
                }
                if close {
                    self.request_close(id);
                    return;
                }
                self.command(&[
                    "focus-exact",
                    &self.snapshot.epoch.clone(),
                    &self.snapshot.active.to_string(),
                    &id.to_string(),
                    &run,
                ]);
            }
            self.focus = Focus::Terminal;
            self.nav = false;
        }
    }
    fn frame_canvas(&mut self) -> Canvas {
        if let Some(arcade) = &self.arcade
            && !self.arcade_obscured()
        {
            let mut canvas = Canvas::new(self.layout.width, self.layout.height);
            arcade.draw(&mut canvas);
            return canvas;
        }
        if self.screensaver.active {
            return self.screensaver_canvas();
        }
        let mut canvas = self.draw();
        self.paint_pet(&mut canvas);
        self.draw_card_popup(&mut canvas);
        self.draw_context_menu(&mut canvas);
        self.draw_search(&mut canvas);
        self.draw_update(&mut canvas);
        self.draw_remote_tools(&mut canvas);
        self.draw_tasks(&mut canvas);
        if self.menu
            || self.form.is_some()
            || self.search.is_some()
            || self.update.is_some()
            || self.tasks.is_some()
            || self.remote_tools_open()
            || self.confirm.is_some()
            || self.card_popup_pinned()
            || self.context_menu.is_some()
        {
            canvas.badges.clear();
        }
        canvas
    }
    fn frame_cursor(&self) -> Option<(usize, usize)> {
        let l = self.terminal_layout();
        (self.arcade.is_none()
            && !self.screensaver.active
            && !self.image_view()
            && !self.nav
            && !self.card_popup_pinned()
            && !self.menu
            && self.confirm.is_none()
            && self.context_menu.is_none()
            && self.form.is_none()
            && self.search.is_none()
            && self.update.is_none()
            && self.tasks.is_none()
            && !self.remote_tools_open()
            && !self.board
            && self.focus == Focus::Terminal
            && !self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
            && self.snapshot.cursor
            && self.selection.is_none()
            && self.scrollback.is_none())
        .then(|| {
            (
                l.terminal_x + self.snapshot.x.min(l.cols - 1),
                l.terminal_y + self.snapshot.y.min(l.rows - 1),
            )
        })
        .filter(|(x, y)| !self.card_inside(*x, *y))
    }
    fn draw(&self) -> Canvas {
        if let Some(canvas) = self.image_canvas() {
            return canvas;
        }
        let l = self.layout;
        let mut c = Canvas::new(l.width, l.height);
        c.fill(0, 0, l.width, 1, style(TEXT, PANEL, false));
        c.text(
            2,
            0,
            l.width.saturating_sub(4),
            "FLERE",
            style(CYAN, PANEL, true),
        );
        if l.width >= 100
            && let Some(w) = self.snapshot.workspace()
        {
            let project = if w.meta.project.is_empty() {
                Path::new(&w.cwd)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            } else {
                w.meta.project.clone()
            };
            let branch = self
                .git_views
                .get(&w.id)
                .map(|v| v.branch.split("...").next().unwrap_or(""))
                .unwrap_or(&w.meta.branch);
            let context = if branch.is_empty() {
                project
            } else {
                format!("{project} / {branch}")
            };
            c.text(13, 0, 3, "//", style(MAGENTA, PANEL, false));
            c.text(
                17,
                0,
                l.width.saturating_sub(50),
                &chrome::elide(&context, l.width.saturating_sub(50)),
                style(TEXT, PANEL, false),
            );
        }
        if l.width >= 60 {
            c.text(
                l.width - 31,
                0,
                30,
                &{ self.attention_summary() },
                style(MUTED, PANEL, false),
            );
        }
        let cy = l.height - 2;
        self.draw_sidebar(&mut c);
        let tx = l.left;
        let tw = l.width - l.left - l.right;
        c.text(tx, 1, tw, &"─".repeat(tw), style(BORDER, BG, false));
        c.text(tx, cy, tw, &"─".repeat(tw), style(BORDER, BG, false));
        self.draw_panes(&mut c);
        self.draw_inspector(&mut c);
        self.draw_git_hover(&mut c);
        self.draw_board(&mut c);
        if l.left == 0 && self.focus == Focus::Cards {
            self.draw_sidebar(&mut c);
        }
        let status = if self.context_menu.is_some() {
            " CONTEXT · j/k or arrows · Enter select · Esc return".into()
        } else if self.card_popup_pinned() && self.notice.is_empty() {
            " DETAILS · j/k scroll · Tab actions · Enter activate · Esc return".into()
        } else if self.selection.as_ref().is_some_and(|s| s.dragging) {
            self.selection.as_ref().unwrap().status().into()
        } else if self.scrollback.is_some() {
            if self.nav {
                " NAV SCROLL · ^U/^D page · u/d smooth · Enter live".into()
            } else {
                " SCROLLBACK · Home oldest · PgUp/PgDn · Esc live".into()
            }
        } else if !self.notice.is_empty() {
            self.notice.clone()
        } else if let Some((hint, _)) = &self.mouse_hint {
            hint.clone()
        } else if self.nav && self.nav_help_at.is_some() {
            " NAV".into()
        } else if self.nav {
            if self.focus == Focus::Files && self.prefs.inspector == Inspector::Git {
                if l.width >= 100 {
                    " NAV  Git · h/l panes · j/k select · Enter expand/open · d diff · Space actions"
                } else if l.width >= 60 {
                    " NAV  h/l panes · j/k select · Enter open · Space actions"
                } else if l.width >= 40 {
                    " NAV  h/l panes · j/k · Enter open"
                } else {
                    " NAV  Space actions"
                }.into()
            } else if self.focus == Focus::Files && self.prefs.inspector == Inspector::Details {
                " NAV Details · j/k scroll · PgUp/PgDn · e edit card".into()
            } else if self.focus == Focus::Files && self.prefs.inspector == Inspector::Files {
                " NAV Files · j/k move · Enter open/image · E editor · h back".into()
            } else if self.focus == Focus::Cards {
                if self.card_children_active() {
                    " NAV Terminals · j/k select · Enter activate · h/Esc back to card"
                } else {
                    " NAV Cards · j/k cards · Tab terminals · Enter activate · ? details"
                }
                .into()
            } else if self.focus == Focus::Terminal {
                let directions = if self
                    .snapshot
                    .split
                    .as_ref()
                    .is_some_and(|split| split.axis == crate::panes::SplitAxis::Below)
                {
                    "j/k"
                } else {
                    "h/l"
                };
                format!(
                    " NAV Terminal · Tab/Shift+Tab tabs · {directions} panes · Space actions · ^U/^D page"
                )
            } else {
                " NAV Tabs · Tab/Shift+Tab or h/l tabs · j terminal · Space actions".into()
            }
        } else if self.focus == Focus::Files && self.prefs.inspector == Inspector::Git {
            " GIT · Enter expand/open · d compare · u patch · ? message · F refresh".into()
        } else if self.focus == Focus::Files && self.prefs.inspector == Inspector::Details {
            " DETAILS · j/k scroll · PgUp/PgDn · Ctrl+Space, e edit card".into()
        } else if self.focus == Focus::Files {
            " FILES · j/k move · Enter open/image · E editor · - parent".into()
        } else if self.snapshot.session().is_none() {
            " STOPPED · S start agent · W reopen saved tabs · t shell · Space actions".into()
        } else if self.remote.is_some() {
            " SSH · Ctrl+V image · Ctrl+Space navigate · q detach in NAV".into()
        } else {
            " Ctrl+Space navigate  ·  n workspace / t tab in NAV  ·  q detach in NAV".into()
        };
        c.fill(
            0,
            l.height - 1,
            l.width,
            1,
            style(if self.nav { TEXT } else { MUTED }, PANEL, false),
        );
        c.text(
            0,
            l.height - 1,
            l.width,
            &status,
            style(if self.nav { TEXT } else { MUTED }, PANEL, false),
        );
        if status.starts_with(" NAV") {
            c.text(0, l.height - 1, 5, " NAV ", style(BG, CYAN, true));
        }
        if l.width >= 120 {
            let size = format!(" {}×{} ", l.width, l.height);
            c.text(
                l.width - size.chars().count(),
                l.height - 1,
                size.chars().count(),
                &size,
                style(MUTED, PANEL, false),
            );
        }
        self.draw_actions(&mut c);
        self.draw_form(&mut c);
        c
    }
}
#[derive(Debug)]
enum Key {
    TerminalReply(Vec<u8>),
    Bytes(Vec<u8>),
    Paste(Vec<u8>),
    Hover {
        x: usize,
        y: usize,
    },
    Mouse {
        x: usize,
        y: usize,
    },
    Context {
        x: usize,
        y: usize,
    },
    ContextRelease,
    // Unsupported buttons/modifier combinations still count as human activity,
    // but never become native bytes or actions in Flere's existing handlers.
    PointerActivity,
    Drag {
        x: usize,
        y: usize,
    },
    Release {
        x: usize,
        y: usize,
    },
    Wheel {
        x: usize,
        y: usize,
        delta: i64,
        local: bool,
    },
}
#[derive(Default)]
struct Input {
    buf: Vec<u8>,
    paste: bool,
    reply_discard: bool,
    paste_buf: Vec<u8>,
    since: Option<Instant>,
}
impl Input {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        self.buf.extend_from_slice(bytes);
        let mut keys = Vec::new();
        loop {
            if self.paste {
                if let Some(i) = self.buf.windows(6).position(|s| s == b"\x1b[201~") {
                    self.paste_buf.extend_from_slice(&self.buf[..i + 6]);
                    self.buf.drain(..i + 6);
                    keys.push(Key::Paste(std::mem::take(&mut self.paste_buf)));
                    self.paste = false;
                } else {
                    let n = self.buf.len().saturating_sub(5);
                    self.paste_buf.extend(self.buf.drain(..n));
                    if self.paste_buf.len() > 32768 {
                        keys.push(Key::Paste(std::mem::take(&mut self.paste_buf)))
                    }
                    break;
                }
                continue;
            }
            if self.buf.is_empty() {
                self.since = None;
                break;
            }
            if self.buf.starts_with(b"\x1b[200~") {
                self.paste = true;
                self.paste_buf = self.buf.drain(..6).collect();
                continue;
            }
            if self.buf[0] == 0x1b {
                if self.since.is_none() {
                    self.since = Some(Instant::now())
                }
                if self.buf.len() == 1 {
                    break;
                }
                if self.buf[1] == b'_' {
                    if let Some(end) = self.buf.windows(2).position(|b| b == b"\x1b\\") {
                        let reply = self.buf.drain(..end + 2).collect();
                        if !self.reply_discard {
                            keys.push(Key::TerminalReply(reply));
                        }
                        self.reply_discard = false;
                        self.since = None;
                        continue;
                    }
                    // Keep a bounded APC prefix and possible split ST. An oversized
                    // terminal reply is discarded through its terminator, never typed.
                    if self.buf.len() > 1024 {
                        let last = self.buf.last().copied();
                        self.buf.truncate(2);
                        if last == Some(0x1b) {
                            self.buf.push(0x1b);
                        }
                        self.reply_discard = true;
                    }
                    break;
                }
                if self.buf[1] == b'[' || self.buf[1] == b'O' {
                    if let Some(i) = self
                        .buf
                        .iter()
                        .enumerate()
                        .skip(2)
                        .find(|(_, b)| (0x40..=0x7e).contains(*b))
                        .map(|(i, _)| i)
                    {
                        let b: Vec<u8> = self.buf.drain(..=i).collect();
                        if self.reply_discard {
                            self.reply_discard = false;
                        } else if (b.starts_with(b"\x1b[6;") && b.ends_with(b"t"))
                            || (b.starts_with(b"\x1b[?") && b.ends_with(b"c"))
                        {
                            keys.push(Key::TerminalReply(b));
                        } else if b.starts_with(b"\x1b[<")
                            && (b.ends_with(b"M") || b.ends_with(b"m"))
                        {
                            if let Ok(s) = std::str::from_utf8(&b[3..b.len() - 1]) {
                                let p: Vec<_> = s
                                    .split(';')
                                    .map(|s| s.parse::<usize>().unwrap_or(usize::MAX))
                                    .collect();
                                if p.len() == 3 && p[1] > 0 && p[2] > 0 && !p.contains(&usize::MAX)
                                {
                                    if b.ends_with(b"M") && matches!(p[0] & !28, 64 | 65) {
                                        let delta: i64 = if p[0] & 1 == 0 { -3 } else { 3 };
                                        if let Some(Key::Wheel {
                                            x,
                                            y,
                                            delta: previous,
                                            local,
                                        }) = keys.last_mut()
                                            && (*x, *y) == (p[1] - 1, p[2] - 1)
                                            && previous.signum() == delta.signum()
                                            && *local == (p[0] & 28 != 0)
                                        {
                                            *previous = (*previous + delta).clamp(-500, 500);
                                        } else {
                                            keys.push(Key::Wheel {
                                                x: p[1] - 1,
                                                y: p[2] - 1,
                                                delta,
                                                local: p[0] & 28 != 0,
                                            });
                                        }
                                    } else if b.ends_with(b"M") && p[0] & !28 == 2 {
                                        keys.push(Key::Context {
                                            x: p[1] - 1,
                                            y: p[2] - 1,
                                        });
                                    } else if b.ends_with(b"m") && p[0] & !28 == 2 {
                                        keys.push(Key::ContextRelease);
                                    } else if b.ends_with(b"m") && p[0] & !28 == 0 {
                                        keys.push(Key::Release {
                                            x: p[1].saturating_sub(1),
                                            y: p[2].saturating_sub(1),
                                        });
                                    } else if b.ends_with(b"M") && p[0] == 0 {
                                        keys.push(Key::Mouse {
                                            x: p[1].saturating_sub(1),
                                            y: p[2].saturating_sub(1),
                                        });
                                    } else if b.ends_with(b"M") && p[0] == 35 {
                                        keys.push(Key::Hover {
                                            x: p[1] - 1,
                                            y: p[2] - 1,
                                        });
                                    } else if b.ends_with(b"M") && p[0] == 32 {
                                        keys.push(Key::Drag {
                                            x: p[1].saturating_sub(1),
                                            y: p[2].saturating_sub(1),
                                        });
                                    } else if p[0] < 128 && !(b.ends_with(b"m") && p[0] & 64 != 0) {
                                        keys.push(Key::PointerActivity);
                                    }
                                }
                            }
                        } else if !b.starts_with(b"\x1b[<") {
                            keys.push(Key::Bytes(b))
                        }
                        self.since = None;
                        continue;
                    }
                    if self.buf.len() >= 64 {
                        self.buf.truncate(2);
                        self.reply_discard = true;
                    }
                    break;
                }
                let n = 2.min(self.buf.len());
                keys.push(Key::Bytes(self.buf.drain(..n).collect()));
                self.since = None;
            } else {
                let n = self
                    .buf
                    .iter()
                    .position(|b| *b == 0 || *b == 0x1b)
                    .unwrap_or(self.buf.len());
                if n == 0 {
                    keys.push(Key::Bytes(self.buf.drain(..1).collect()))
                } else {
                    for b in self.buf.drain(..n) {
                        keys.push(Key::Bytes(vec![b]))
                    }
                }
            }
        }
        keys
    }
    fn timeout(&mut self) -> Vec<Key> {
        if !self.paste
            && !self.buf.starts_with(b"\x1b_")
            && !self.buf.starts_with(b"\x1b[")
            && !self.buf.starts_with(b"\x1bO")
            && self
                .since
                .is_some_and(|t| t.elapsed() > Duration::from_millis(40))
        {
            self.since = None;
            vec![Key::Bytes(std::mem::take(&mut self.buf))]
        } else {
            Vec::new()
        }
    }
}
fn ansi_color(c: Color, background: bool, out: &mut String) {
    let base = if background { 48 } else { 38 };
    match c {
        Color::Default => out.push_str(if background { ";49" } else { ";39" }),
        Color::Index(n) => out.push_str(&format!(";{base};5;{n}")),
        Color::Rgb(r, g, b) => out.push_str(&format!(";{base};2;{r};{g};{b}")),
    }
}
fn ansi_style(s: Style, out: &mut String) {
    out.push_str("\x1b[0");
    ansi_color(s.fg, false, out);
    ansi_color(s.bg, true, out);
    if s.bold {
        out.push_str(";1")
    }
    if s.dim {
        out.push_str(";2")
    }
    if s.italic {
        out.push_str(";3")
    }
    if s.strikethrough {
        out.push_str(";9")
    }
    if s.underline {
        out.push_str(";4")
    }
    if s.inverse {
        out.push_str(";7")
    }
    out.push('m');
}
fn render(c: &Canvas, previous: &mut Vec<Cell>) -> String {
    hyperlinks::render(c, previous)
}
// Hide the physical cursor before drawing, then restore it only at the native
// composer. DEC 2026 batches the complete frame on supporting outer terminals;
// terminals that ignore it still never see the cursor at paint positions.
fn display_frame(
    paint: String,
    cursor: Option<(usize, usize)>,
    previous: &mut Option<Option<(usize, usize)>>,
    clipboard: Option<String>,
) -> String {
    if paint.is_empty() && clipboard.is_none() && *previous == Some(cursor) {
        return String::new();
    }
    let mut out = String::from("\x1b[?2026h\x1b[?25l");
    out.push_str(hyperlinks::CLOSE);
    out.push_str(&paint);
    out.push_str(hyperlinks::CLOSE);
    if let Some(copy) = clipboard {
        out.push_str(&copy);
    }
    if let Some((x, y)) = cursor {
        out.push_str(&format!("\x1b[{y};{x}H\x1b[?25h"));
    }
    out.push_str("\x1b[?2026l");
    *previous = Some(cursor);
    out
}
struct DisplayGuard {
    remote: bool,
}
impl Drop for DisplayGuard {
    fn drop(&mut self) {
        let _ = remote::emit(self.remote,
            b"\x18\x1b\\\x1b]8;;\x1b\\\x1b[?2026l\x1b[0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1006l\x1b[>0s\x1b[?2004l\x1b[?7h\x1b[?25h\x1b[?1049l",
        );
    }
}
pub fn intro(state: &Path) -> io::Result<bool> {
    intro::run(state)
}
pub fn attach(state: &Path) -> io::Result<()> {
    attach_mode(state, false)
}
pub fn bridge(state: &Path) -> io::Result<()> {
    attach_mode(state, true)
}
fn attach_mode(state: &Path, is_remote: bool) -> io::Result<()> {
    if let Err(e) = crate::diagnostics::init(
        &state.join("diagnostics"),
        if is_remote { "bridge" } else { "ui" },
    ) {
        eprintln!("Flere diagnostics unavailable: {e}");
    }
    let result = attach_session(state, is_remote);
    match &result {
        Ok(()) => crate::diagnostics::record("exit", "normal"),
        Err(e) => crate::diagnostics::error("exit-error", e),
    }
    if result.is_err()
        && let Some(path) = crate::diagnostics::path()
    {
        eprintln!("Flere diagnostic log: {}", path.display());
    }
    result
}
fn attach_session(state: &Path, is_remote: bool) -> io::Result<()> {
    // Remote framing must use the same unbuffered reader before and after HELLO.
    // Stdin's read-ahead can hide coalesced packets from the descriptor we poll.
    let mut remote_input = if is_remote {
        Some(fs::File::from(io::stdin().as_fd().try_clone_to_owned()?))
    } else {
        None
    };
    let remote = remote_input
        .as_mut()
        .map(remote::Connection::handshake)
        .transpose()?;
    let input_fd = remote_input.as_ref().map_or(0, |input| input.as_raw_fd());
    let (width, height) = remote
        .as_ref()
        .map_or_else(|| os::dimensions(0), |r| r.size);
    crate::diagnostics::record(
        "connected",
        &format!("remote={is_remote} size={width}x{height}"),
    );
    let startup = Instant::now();
    let prefs = workflows::load_preferences(state);
    let layout = Layout::with_preferences(width as usize, height as usize, &prefs);
    let (initial, pane_capable, link_capable) = hyperlinks::initial_snapshot(state)?;
    if pane_capable {
        wire::request(
            state,
            &[
                "resize-panes",
                &initial.epoch,
                &initial.active.to_string(),
                &layout.cols.to_string(),
                &layout.rows.to_string(),
            ],
        )?;
    } else {
        wire::request(
            state,
            &["resize", &layout.cols.to_string(), &layout.rows.to_string()],
        )?;
    }
    let snapshot = Snapshot::decode(&wire::request(
        state,
        &[hyperlinks::snapshot_command(pane_capable, link_capable)],
    )?)?;
    let mut stream = wire::connect(state)?;
    let watch = if pane_capable && wire::request(state, &["frontends"]).is_ok() {
        let metadata = serde_json::json!({"schema_version":1,"pid":std::process::id(),"attachment":os::nonce()?,"remote":is_remote,"build":serde_json::from_str::<serde_json::Value>(crate::build_info::json()).map_err(io::Error::other)?});
        format!(
            "{}\t{}",
            if link_capable {
                "watch-links-build"
            } else {
                "watch-panes-build"
            },
            wire::hex(&serde_json::to_vec(&metadata).map_err(io::Error::other)?)
        )
    } else if link_capable {
        "watch-links".into()
    } else if pane_capable {
        "watch-panes".into()
    } else {
        "watch".into()
    };
    stream.write_all(&wire::frame(watch.as_bytes()))?;
    stream.set_nonblocking(true)?;
    let _raw = if is_remote {
        None
    } else {
        Some(os::RawTerminal::enter(0)?)
    };
    os::signals();
    let _display = DisplayGuard { remote: is_remote };
    // XTSHIFTESCAPE leaves Shift mouse gestures with the outer terminal. This
    // per-terminal mode can survive alternate-screen switches; request it before
    // capture rather than inherit another application's Shift reporting policy.
    remote::emit(
        is_remote,
        b"\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[?2004h\x1b[>0s\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[2J",
    )?;
    let local_graphics = (!is_remote).then(local_graphics::Graphics::new);
    if !is_remote {
        remote::emit(false, local_graphics::Graphics::query().as_bytes())?;
    }
    let mut ui = Ui {
        state: state.into(),
        pane_capable,
        link_capable,
        project_icons: avatars::Projects::new(),
        checkouts: checkout::Checkouts::new(),
        pet: pet::Pet::new(),
        screensaver: screensaver::Saver::new(Instant::now()),
        arcade: None,
        arcade_gate: arcade::Gate::default(),
        arcade_keyboard: arcade::Keyboard::new(is_remote),
        arcade_checked: Instant::now(),
        arcade_nav: false,
        snapshot: snapshot.clone(),
        layout,
        focus: Focus::Terminal,
        side_scroll: None,
        cards: cards::Cards::default(),
        nav: false,
        menu: false,
        menu_index: 0,
        menu_query: Vec::new(),
        menu_search: false,
        nav_help_at: None,
        mouse_hint: None,
        prefs,
        form: None,
        search: None,
        update: None,
        tasks: None,
        board: false,
        board_column: 0,
        git_views: HashMap::new(),
        git_job: None,
        git_checked: Instant::now() - Duration::from_secs(4),
        git_workspace: 0,
        git_history: HashMap::new(),
        history_job: None,
        history_pending: None,
        git_hover: None,
        confirm: None,
        context_menu: None,
        pending_close: None,
        screenshot_job: None,
        explorers: HashMap::new(),
        explorer_loader: explorer::Loader::new(),
        image_loader: image_preview::Loader::new(),
        details_scroll: HashMap::new(),
        previews: HashMap::new(),
        notice: snapshot.notice.clone(),
        history: vec![(
            snapshot.active,
            snapshot.tab,
            snapshot
                .session()
                .map(|t| t.run.clone())
                .unwrap_or_default(),
        )],
        history_index: 0,
        restore_pending: false,
        quit: false,
        refresh: false,
        refresh_to: None,
        refresh_attempt: None,
        update_ack: std::env::var("FLERE_UPDATE_ACK")
            .ok()
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(|s| (s, Instant::now())),
        animation_step: 0,
        animation_checked: Instant::now(),
        drag: None,
        pane_drag: None,
        selection: None,
        clipboard: None,
        scrollback: None,
        parked_scrollback: HashMap::new(),
        smooth_scroll: None,
        coordination: serde_json::json!({}),
        input: Input::default(),
        outgoing: Vec::new(),
        paste_target: None,
        remote,
        remote_tools: Default::default(),
        local_graphics,
    };
    if ui.prefs.history_epoch == snapshot.epoch && !ui.prefs.history.is_empty() {
        ui.history = ui.prefs.history.clone();
        ui.history_index = ui.prefs.history_index.min(ui.history.len() - 1);
    }
    ui.snapshot(snapshot);
    ui.reopen_workspace();
    crate::diagnostics::record(
        "ui-ready",
        &format!("elapsed_ms={}", startup.elapsed().as_millis()),
    );
    let mut previous = Vec::new();
    let mut previous_cursor = None;
    let mut previous_avatars = Vec::new();
    let mut previous_image = false;
    let mut previous_screensaver = false;
    let mut previous_arcade = false;
    let mut previous_pet = None;
    let mut motion = false;
    let mut clear_next = false;
    let mut buffer = Vec::new();
    let mut dirty = true;
    let mut size_checked = Instant::now();
    while !ui.quit && !ui.refresh && !os::stopping() {
        let loop_started = Instant::now();
        let mut fds = [
            os::PollFd {
                fd: input_fd,
                events: os::READ,
                revents: 0,
            },
            os::PollFd {
                fd: stream.as_raw_fd(),
                events: os::READ,
                revents: 0,
            },
        ];
        os::wait(
            &mut fds,
            if dirty {
                0
            } else if ui.smooth_scroll.is_some() {
                20
            } else {
                40
            },
        )?;
        if fds[0].revents & (os::READ | os::HUP) != 0 {
            let mut b = [0; 8192];
            let n = if let Some(input) = &mut remote_input {
                loop {
                    match input.read(&mut b) {
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        result => break result?,
                    }
                }
            } else {
                io::stdin().read(&mut b)?
            };
            if n == 0 {
                crate::diagnostics::record("input-eof", "frontend input closed");
                break;
            }
            ui.screensaver.begin_burst();
            ui.arcade_gate.begin_burst();
            if is_remote {
                ui.remote
                    .as_mut()
                    .unwrap()
                    .buffer
                    .extend_from_slice(&b[..n]);
                while let Some(packet) =
                    crate::remote_protocol::Packet::take(&mut ui.remote.as_mut().unwrap().buffer)?
                {
                    ui.remote_packet(packet)?;
                    if ui.quit || ui.refresh {
                        break;
                    }
                }
            } else {
                ui.feed_input(&b[..n]);
            }
            ui.finish_input_burst();
            dirty = true;
        }
        ui.screensaver.begin_burst();
        ui.arcade_gate.begin_burst();
        for key in ui.input.timeout() {
            ui.key(key);
            dirty = true;
        }
        ui.finish_input_burst();
        ui.flush_input();
        if fds[1].revents & (os::READ | os::HUP) != 0 {
            let mut b = [0; 65536];
            for _ in 0..32 {
                match stream.read(&mut b) {
                    Ok(0) => {
                        return Err(io::Error::other(
                            "supervisor disconnected; no sessions were restarted",
                        ));
                    }
                    Ok(n) => buffer.extend_from_slice(&b[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e),
                }
            }
            while let Some(frame) = wire::take_frame(&mut buffer)? {
                ui.snapshot(Snapshot::decode(&frame)?);
                dirty = true;
            }
            if buffer.len() > wire::MAX + 4 {
                return Err(wire::invalid("watch buffer exceeds bound"));
            }
        }
        if size_checked.elapsed() > Duration::from_millis(200)
            || ui.remote.as_ref().is_some_and(|r| r.force_redraw)
        {
            let (w, h) = ui
                .remote
                .as_ref()
                .map_or_else(|| os::dimensions(0), |r| r.size);
            if let Some(g) = &mut ui.local_graphics {
                if let Some(cell) = os::cell_dimensions(0) {
                    g.update_cell(cell);
                }
                if (w as usize, h as usize) != (ui.layout.width, ui.layout.height) {
                    remote::emit(false, b"\x1b[16t")?;
                }
            }
            let new = Layout::with_preferences(w as usize, h as usize, &ui.prefs);
            if (new.width, new.height) != (ui.layout.width, ui.layout.height) {
                crate::diagnostics::record("resize", &format!("{}x{}", new.width, new.height));
                ui.selection = None;
                ui.scrollback = None;
                ui.layout = new;
                if let Some(arcade) = &mut ui.arcade {
                    arcade.resize(new.width, new.height);
                }
                ui.resize_viewport();
                previous.clear();
                clear_next = true;
                dirty = true;
            }
            size_checked = Instant::now();
        }
        if ui
            .remote
            .as_mut()
            .is_some_and(|r| std::mem::take(&mut r.force_redraw))
        {
            previous.clear();
            clear_next = true;
            dirty = true;
        }
        dirty |= ui.remote_cancel_changed()?;
        dirty |= ui.local_graphics.as_mut().is_some_and(|g| g.poll());
        if ui.image_validate()? {
            // A close/reopen can occur in one input packet with an identical canvas.
            // The companion cleared pixels and text, so restore the complete frame.
            previous.clear();
            clear_next = true;
            dirty = true;
        }
        dirty |= ui.project_icons.poll(&ui.snapshot);
        dirty |= ui.checkouts.poll(&ui.snapshot);
        if ui.pet.controls.is_some() && !ui.pet_visible() {
            ui.pet_close();
            dirty = true;
        }
        ui.pet_sync();
        let pet_animate = ui.pet_visible() && !ui.prefs.reduced_motion;
        dirty |= ui.pet.tick(pet_animate);
        dirty |= ui.tick_polish();
        dirty |= ui.tick_cards();
        dirty |= ui.tick_restore();
        dirty |= ui.tick_close();
        dirty |= ui.tick_screenshot();
        dirty |= ui.tick_explorer();
        dirty |= ui.tick_search();
        dirty |= ui.tick_update();
        dirty |= ui.tick_update_ack();
        dirty |= ui.tick_remote_tools();
        dirty |= ui.tick_update_rpc()?;
        dirty |= ui.tick_tasks();
        dirty |= ui.tick_selection();
        dirty |= ui.tick_scroll();
        dirty |= ui.tick_git();
        dirty |= ui.tick_history();
        dirty |= ui.context_reconcile();
        dirty |= ui.tick_screensaver();
        dirty |= ui.tick_arcade();
        ui.remote_drop_context()?;
        let want_motion = ui.screensaver.active
            || ((ui.layout.left > 0
                || ui.focus == Focus::Cards
                || ui.prefs.inspector == Inspector::Git)
                && !ui.board
                && ui.form.is_none()
                && !ui.menu
                && (ui.layout.left > 0
                    || ui.focus == Focus::Cards
                    || ui.layout.right > 0
                    || ui.focus == Focus::Files));
        if motion != want_motion {
            remote::emit(
                is_remote,
                if want_motion {
                    b"\x1b[?1003h"
                } else {
                    b"\x1b[?1003l\x1b[?1002h"
                },
            )?;
            motion = want_motion;
        }
        if !ui.prefs.reduced_motion
            && ui
                .snapshot
                .workspaces
                .iter()
                .flat_map(|w| &w.tabs)
                .any(|t| t.working)
            && ui.animation_checked.elapsed() > Duration::from_millis(120)
        {
            ui.animation_step = (ui.animation_step + 1) % 60;
            ui.animation_checked = Instant::now();
            dirty = true;
        }
        if dirty {
            if previous_arcade != ui.arcade.is_some() {
                previous.clear();
                clear_next = true;
                previous_arcade = ui.arcade.is_some();
            }
            if previous_screensaver != ui.screensaver.active {
                previous.clear();
                clear_next = true;
                previous_screensaver = ui.screensaver.active;
            }
            let image_view = ui.image_view();
            if previous_image != image_view {
                previous.clear();
                clear_next = true;
                previous_image = image_view;
            }
            let draw_started = Instant::now();
            let c = ui.frame_canvas();
            let avatars = crate::avatar::Layout {
                width: c.width as u16,
                height: c.height as u16,
                badges: c.badges.clone(),
            };
            let avatar_capable = ui.remote.as_ref().is_some_and(|r| r.avatar_capable);
            if (avatar_capable || ui.graphics_cell().is_some()) && previous_avatars != c.badges {
                previous.clear();
                clear_next = true;
                previous_avatars = c.badges.clone();
            }
            let pet_region = if ui.pet_visible() && ui.graphics_cell().is_some() {
                Some((
                    ui.inspector_size(),
                    ui.layout.height,
                    ui.pet_rows(),
                    ui.graphics_cell(),
                ))
            } else {
                None
            };
            if previous_pet != pet_region {
                previous.clear();
                clear_next = true;
                previous_pet = pet_region;
            }
            let mut paint = render(&c, &mut previous);
            if ui.remote.is_some() {
                paint.push_str(&ui.pet_pixels()?);
            }
            if image_view && !paint.is_empty() {
                previous.clear();
                paint = render(&c, &mut previous);
                clear_next = true;
            }
            let graphics_reset = clear_next;
            if clear_next {
                paint.insert_str(0, "\x1b[2J");
                clear_next = false;
            }
            if let Some(g) = &mut ui.local_graphics {
                paint.push_str(&g.frame(
                    &c.badges,
                    &ui.project_icons,
                    (ui.layout.width, ui.layout.height),
                    graphics_reset,
                ));
            }
            if ui.local_graphics.is_some() {
                paint.push_str(&ui.pet_pixels()?);
            }
            let cursor = ui.frame_cursor().map(|(x, y)| (x + 1, y + 1));
            let out = display_frame(paint, cursor, &mut previous_cursor, ui.clipboard.take());
            crate::diagnostics::slow("draw", draw_started);
            if !out.is_empty() {
                remote::emit(is_remote, out.as_bytes())?;
                if avatar_capable {
                    crate::remote_protocol::Packet::new(
                        if ui.remote.as_ref().is_some_and(|r| r.avatar_cell.is_some()) {
                            crate::remote_protocol::AVATAR_LAYOUT_SIZED
                        } else {
                            crate::remote_protocol::AVATAR_LAYOUT
                        },
                        0,
                        if ui.remote.as_ref().is_some_and(|r| r.avatar_cell.is_some()) {
                            avatars.encode_sized()
                        } else {
                            avatars.encode()
                        },
                    )
                    .write(&mut io::stdout().lock())?;
                }
            }
            dirty = false;
        }
        if ui.remote.as_ref().is_some_and(|r| r.avatar_capable) {
            ui.project_icons.emit(&previous_avatars)?;
        }
        ui.image_emit()?;
        crate::diagnostics::slow("ui-loop", loop_started);
    }
    crate::diagnostics::record(
        "leaving",
        &format!(
            "detach={} refresh={} signal={}",
            ui.quit,
            ui.refresh,
            os::stopping()
        ),
    );
    ui.cancel_close();
    ui.arcade_keyboard.stop(Instant::now());
    ui.arcade_keyboard.flush();
    ui.remember_workspace();
    if ui.refresh {
        ui.remote_tools = Default::default();
        ui.remote_escape();
        drop(ui.remote.take());
        drop(ui.local_graphics.take());
        drop(_display);
        drop(_raw);
        drop(stream);
        use std::os::unix::process::CommandExt;
        let mut command =
            std::process::Command::new(ui.refresh_to.take().unwrap_or(os::executable_path()?));
        command.env_remove("FLERE_UPDATE_ACK");
        if let Some(attempt) = ui.refresh_attempt.take() {
            command.env("FLERE_UPDATE_ACK", attempt);
        }
        command
            .arg("--state")
            .arg(state)
            .arg(if is_remote { "_bridge" } else { "attach" });
        crate::diagnostics::record(
            "refresh-exec",
            "replacing frontend; pending input discarded",
        );
        return Err(command.exec());
    }

    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_replies_are_fragment_safe_and_never_become_native_keys() {
        let bytes = b"\x1b_Gi=1380458496;OK\x1b\\\x1b[6;20;8t";
        for split in 0..=bytes.len() {
            let mut input = Input::default();
            let mut keys = input.feed(&bytes[..split]);
            if input.buf.len() > 1 {
                input.since = Some(Instant::now() - Duration::from_secs(1));
                assert!(input.timeout().is_empty());
            }
            keys.extend(input.feed(&bytes[split..]));
            assert_eq!(keys.len(), 2);
            assert!(keys.iter().all(|key| matches!(key, Key::TerminalReply(_))));
        }
        let mut input = Input::default();
        let paste = b"\x1b[200~literal \x1b_Gi=1;OK\x1b\\\x1b[201~";
        assert!(matches!(&input.feed(paste)[..], [Key::Paste(b)] if b == paste));
        input.feed(b"\x1b_G");
        for _ in 0..10 {
            assert!(input.feed(&[b'x'; 1024]).is_empty());
            assert!(input.buf.len() < 1024);
        }
        let keys = input.feed(b"\x1b\\z");
        assert!(matches!(&keys[..], [Key::Bytes(b)] if b == b"z"));
    }
    #[test]
    fn changing_native_row_preserves_badge_pixels_and_text_alignment() {
        let mut canvas = Canvas::new(60, 24);
        canvas.text(3, 21, 3, "ICON", style(TEXT, PANEL, false));
        canvas.badges.push(crate::avatar::Badge {
            x: 3,
            y: 21,
            columns: 3,
            key: "repo-1".into(),
            bg: [18, 26, 41],
        });
        let mut previous = Vec::new();
        assert!(render(&canvas, &mut previous).contains("ICO"));
        canvas.text(30, 21, 12, "draft update", style(TEXT, BG, false));
        let update = render(&canvas, &mut previous);
        assert!(!update.contains("ICO"));
        assert!(update.contains("\x1b[22;7H"));
        let mut term = crate::terminal::Terminal::new(60, 24);
        term.feed(update.as_bytes());
        assert_eq!(&term.grid.line(21)[30..42], "draft update");
        previous.clear();
        assert!(render(&canvas, &mut previous).contains("ICO"));
    }
    #[test]
    fn overlays_remove_covered_avatar_positions() {
        let mut c = Canvas::new(60, 24);
        c.badges.push(crate::avatar::Badge {
            x: 3,
            y: 5,
            columns: 4,
            key: "example".into(),
            bg: [18, 26, 41],
        });
        c.text(8, 5, 20, "Title", Style::default());
        assert_eq!(c.badges.len(), 1);
        c.fill(0, 0, 30, 10, Style::default());
        assert!(c.badges.is_empty());
        c.badges.push(crate::avatar::Badge {
            x: 3,
            y: 5,
            columns: 4,
            key: "example".into(),
            bg: [18, 26, 41],
        });
        c.text(6, 5, 1, "x", Style::default());
        assert!(c.badges.is_empty());
    }
    #[test]
    fn unicode_width_disagreement_does_not_shift_following_panes() {
        let mut canvas = Canvas::new(48, 4);
        canvas.text(0, 0, 48, "left   éééééééé      INSPECTOR", Style::default());
        canvas.text(
            0,
            1,
            48,
            "left   a\u{ad}a\u{ad}a\u{ad}a\u{ad}     INSPECTOR",
            Style::default(),
        );
        canvas.text(0, 2, 48, "left   界界界界      INSPECTOR", Style::default());
        let expected: Vec<_> = (0..3)
            .map(|y| {
                (0..48)
                    .find(|x| canvas.cells[y * 48 + x].text == "I")
                    .unwrap()
            })
            .collect();
        let output = render(&canvas, &mut Vec::new());
        // Model a client which assigns different advance widths than our grid:
        // a narrow glyph becomes wide, soft hyphens take a column, CJK becomes narrow.
        let disagreement = output
            .replace('界', "z")
            .replace('é', "界")
            .replace('\u{ad}', "X");
        let mut outer = crate::terminal::Terminal::new(48, 4);
        outer.feed(b"\x1b[?7l");
        outer.feed(disagreement.as_bytes());
        for (y, x) in expected.into_iter().enumerate() {
            let text: String = outer.grid.cells[y * 48 + x..y * 48 + x + 9]
                .iter()
                .map(|c| c.text.as_str())
                .collect();
            assert_eq!(text, "INSPECTOR");
        }
        assert!(render(&canvas, &mut canvas.cells.clone()).is_empty());
    }
    #[test]
    fn redraw_never_exposes_cursor_at_paint_positions_and_skips_unchanged_frames() {
        let mut terminal = crate::terminal::Terminal::new(60, 20);
        terminal.feed(b"\x1b[17;3H\x1b[?25h");
        let mut cursor = None;
        let frame = display_frame(
            "\x1b[2;1HSidebar\x1b[8;20HContent".into(),
            Some((3, 17)),
            &mut cursor,
            None,
        );
        assert!(frame.starts_with("\x1b[?2026h\x1b[?25l"));
        assert!(frame.ends_with("\x1b[17;3H\x1b[?25h\x1b[?2026l"));
        for byte in frame.bytes() {
            terminal.feed(&[byte]);
            if terminal.cursor {
                assert_eq!((terminal.grid.x, terminal.grid.y), (2, 16));
            }
        }
        assert!(display_frame(String::new(), Some((3, 17)), &mut cursor, None).is_empty());
        let hidden = display_frame(String::new(), None, &mut cursor, None);
        terminal.feed(hidden.as_bytes());
        assert!(!terminal.cursor);
        assert!(display_frame(String::new(), None, &mut cursor, None).is_empty());
    }
    #[test]
    fn layout_stays_inside_narrow_and_wide() {
        for w in [20, 40, 56, 80, 100, 140, 240, 320] {
            for h in [8, 24, 50, 106] {
                let l = Layout::new(w, h);
                assert!(l.terminal_x + l.cols < l.width);
                assert!(l.terminal_y + l.rows <= l.height - 2);
            }
        }
    }
    #[test]
    fn paste_cannot_trigger_navigation() {
        let mut input = Input::default();
        let mut keys = input.feed(b"\x1b[20");
        assert!(keys.is_empty());
        keys.extend(input.feed(b"0~hello\0nq\x1b[201~"));
        assert!(matches!(&keys[..],[Key::Paste(b)] if b==b"\x1b[200~hello\0nq\x1b[201~"));
    }
    #[test]
    fn mouse_is_separate_from_terminal_text() {
        let mut input = Input::default();
        let keys = input.feed(b"\x1b[<0;3;5M");
        assert!(matches!(keys[0], Key::Mouse { x: 2, y: 4 }));
        let keys = input.feed(b"\x1b[<32;8;9M\x1b[<0;8;9m\x1b[<2;8;9m");
        assert!(matches!(
            keys.as_slice(),
            [
                Key::Drag { x: 7, y: 8 },
                Key::Release { x: 7, y: 8 },
                Key::ContextRelease
            ]
        ));
        // Ui::key's first input guard consumes right releases even while awake;
        // they cannot become native bytes or an ordinary left selection release.
        let mut saver = screensaver::Saver::new(Instant::now());
        assert!(!saver.active);
        assert!(saver.consume(&keys[2], Instant::now()));
        assert!(matches!(
            input.feed(b"\x1b[<4;8;9m").as_slice(),
            [Key::Release { x: 7, y: 8 }]
        ));
    }
    #[test]
    fn fragmented_wheel_events_are_bounded_coalesced_and_never_native_bytes() {
        let bytes = b"\x1b[<64;40;12M";
        for split in 0..bytes.len() {
            let mut input = Input::default();
            assert!(input.feed(&bytes[..split]).is_empty());
            assert!(matches!(
                input.feed(&bytes[split..]).as_slice(),
                [Key::Wheel {
                    x: 39,
                    y: 11,
                    delta: -3,
                    local: false
                }]
            ));
        }
        let mut input = Input::default();
        assert!(matches!(
            input.feed(&bytes.repeat(300)).as_slice(),
            [Key::Wheel { delta: -500, .. }]
        ));
        assert!(matches!(
            input.feed(b"\x1b[<68;4;5M\x1b[<65;4;5M").as_slice(),
            [Key::Wheel { delta: -3, .. }, Key::Wheel { delta: 3, .. }]
        ));
        assert!(
            input
                .feed(b"\x1b[<64;0;3M\x1b[<64;4;5m\x1b[<64;1;;5M")
                .is_empty()
        );
    }
    #[test]
    fn unchanged_canvas_writes_nothing() {
        let c = Canvas::new(80, 24);
        let mut old = Vec::new();
        assert!(!render(&c, &mut old).is_empty());
        assert!(render(&c, &mut old).is_empty());
    }
}
