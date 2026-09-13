//! Workspace trees and one keyboard-first, optionally passive details surface.
use super::github::{Github, Link};
use super::*;
use crate::model::TabView;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Cursor {
    pub wid: u64,
    pub tab: u64,
    pub run: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Issue,
    Pr,
    CopyIssue,
    CopyPr,
    Path,
    Notes,
    Refresh,
}
impl Action {
    fn key(self) -> u8 {
        match self {
            Self::Issue => b'i',
            Self::Pr => b'p',
            Self::CopyIssue => b'y',
            Self::CopyPr => b'Y',
            Self::Path => b'd',
            Self::Notes => b'e',
            Self::Refresh => b'r',
        }
    }
    fn label(self, remote: bool) -> &'static str {
        match self {
            Self::Issue if remote => "Copy issue",
            Self::Pr if remote => "Copy PR",
            Self::Issue => "Open issue",
            Self::Pr => "Open PR",
            Self::CopyIssue => "Copy issue",
            Self::CopyPr => "Copy PR",
            Self::Path => "Copy path",
            Self::Notes => "Edit notes",
            Self::Refresh => "Refresh GitHub",
        }
    }
}
struct Edit {
    expected: String,
    bytes: Vec<u8>,
    error: String,
}
struct Popup {
    epoch: String,
    wid: u64,
    issue: String,
    pr: String,
    pinned: bool,
    anchor: Option<(usize, usize)>,
    offset: usize,
    action: usize,
    pressed: Option<(Action, usize, usize)>,
    edit: Option<Edit>,
}
#[derive(Clone)]
struct Hover {
    epoch: String,
    wid: u64,
    at: Instant,
    x: usize,
    y: usize,
}
#[derive(Default)]
pub(super) struct Cards {
    pub cursor: Option<Cursor>,
    children: Option<u64>,
    popup: Option<Popup>,
    hover: Option<Hover>,
    leave: Option<Instant>,
    github: Github,
}
pub(super) fn tab_status(t: &TabView) -> &'static str {
    if !t.alive {
        "stopped"
    } else if t.working {
        "working"
    } else {
        "running"
    }
}
fn actions(p: &Popup) -> Vec<Action> {
    let mut out = Vec::new();
    if Link::parse(&p.issue).is_some() {
        out.extend([Action::Issue, Action::CopyIssue]);
    }
    if Link::parse(&p.pr).is_some() {
        out.extend([Action::Pr, Action::CopyPr]);
    }
    out.extend([Action::Path, Action::Notes]);
    if Link::parse(&p.issue).is_some() || Link::parse(&p.pr).is_some() {
        out.push(Action::Refresh);
    }
    out
}
impl Ui {
    pub(super) fn card_cursor(&self) -> Cursor {
        self.cards
            .cursor
            .clone()
            .filter(|c| {
                self.side_rows().iter().any(|r| match r {
                    workflows::SideRow::Card { id, row: 0 } => c.wid == *id && c.tab == 0,
                    workflows::SideRow::Terminal { id, tab, run } => {
                        self.cards.children == Some(*id)
                            && c.wid == *id
                            && c.tab == *tab
                            && c.run == *run
                    }
                    _ => false,
                })
            })
            .unwrap_or(Cursor {
                wid: self.snapshot.active,
                tab: 0,
                run: String::new(),
            })
    }
    pub(super) fn card_rows(&self) -> Vec<Cursor> {
        self.side_rows()
            .into_iter()
            .filter_map(|r| match r {
                workflows::SideRow::Card { id, row: 0 } if self.cards.children.is_none() => {
                    Some(Cursor {
                        wid: id,
                        tab: 0,
                        run: String::new(),
                    })
                }
                workflows::SideRow::Terminal { id, tab, run }
                    if self.cards.children == Some(id) =>
                {
                    Some(Cursor { wid: id, tab, run })
                }
                _ => None,
            })
            .collect()
    }
    pub(super) fn card_move(&mut self, delta: isize) {
        let rows = self.card_rows();
        if rows.is_empty() {
            return;
        }
        let at = rows
            .iter()
            .position(|r| *r == self.card_cursor())
            .unwrap_or(0);
        let target = rows[(at as isize + delta).rem_euclid(rows.len() as isize) as usize].clone();
        // Header selection retains Flere's existing workspace-preview behavior.
        // Terminal children are local until Enter and never send child input.
        if target.tab == 0 {
            self.card_focus(&target, false);
        }
        self.cards.cursor = Some(target);
        self.side_scroll = None;
    }
    fn card_focus(&mut self, target: &Cursor, enter: bool) {
        self.command(&[
            "focus-exact",
            &self.snapshot.epoch.clone(),
            &target.wid.to_string(),
            &target.tab.to_string(),
            &target.run,
        ]);
        if enter && self.notice.is_empty() {
            self.cards.children = None;
            self.focus = Focus::Terminal;
            self.nav = false;
            self.scrollback = None;
            self.resume_saved_chat();
        }
    }
    pub(super) fn card_toggle(&mut self, wid: u64) {
        self.cards.children = None;
        if let Some(i) = self.prefs.expanded_cards.iter().position(|id| *id == wid) {
            self.prefs.expanded_cards.remove(i);
        } else {
            self.prefs.expanded_cards.push(wid);
            if self.prefs.expanded_cards.len() > 512 {
                self.prefs.expanded_cards.remove(0);
            }
        }
        self.cards.cursor = Some(Cursor {
            wid,
            tab: 0,
            run: String::new(),
        });
        self.side_scroll = None;
        self.save_preferences();
    }
    fn card_enter_children(&mut self) {
        let wid = self.card_cursor().wid;
        let Some(workspace) = self.snapshot.workspaces.iter().find(|w| w.id == wid) else {
            return;
        };
        let Some(tab) = workspace
            .tabs
            .iter()
            .find(|t| wid == self.snapshot.active && t.id == self.snapshot.tab)
            .or_else(|| workspace.tabs.first())
        else {
            return;
        };
        let cursor = Cursor {
            wid,
            tab: tab.id,
            run: tab.run.clone(),
        };
        if !self.prefs.expanded_cards.contains(&wid) {
            self.card_toggle(wid);
        }
        self.cards.children = Some(wid);
        self.cards.cursor = Some(cursor);
        self.side_scroll = None;
    }
    fn card_leave_children(&mut self) {
        if self.cards.children.take().is_some() {
            self.cards.cursor = Some(Cursor {
                wid: self.snapshot.active,
                tab: 0,
                run: String::new(),
            });
            self.side_scroll = None;
        }
    }
    pub(super) fn card_children_active(&self) -> bool {
        self.cards.children.is_some()
    }
    pub(super) fn card_nav_key(&mut self, bytes: &[u8]) -> bool {
        if !self.nav || self.focus != Focus::Cards || self.board {
            self.cards.children = None;
            return false;
        }
        if self.cards.children.is_some() && self.card_rows().is_empty() {
            self.card_leave_children();
        }
        match bytes {
            b"\t" if self.cards.children.is_some() => self.card_leave_children(),
            b"\t" => self.card_enter_children(),
            b"h" | b"\x1b[D" | b"\x1bOD" | b"\x1b" if self.cards.children.is_some() => {
                self.card_leave_children();
            }
            b"l" | b"\x1b[C" | b"\x1bOC" if self.cards.children.is_some() => {
                self.card_leave_children();
                return false;
            }
            b"?" => self.card_open(self.card_cursor().wid, true),
            b"\r" => self.card_focus(&self.card_cursor(), true),
            b"j" | b"\x1b[B" | b"\x1bOB" => self.card_move(1),
            b"k" | b"\x1b[A" | b"\x1bOA" => self.card_move(-1),
            _ => return false,
        }
        true
    }
    pub(super) fn card_at(&self, x: usize, y: usize) -> Option<Cursor> {
        let width = if self.layout.left > 0 {
            self.layout.left
        } else if self.focus == Focus::Cards {
            self.layout.width
        } else {
            0
        };
        let slots = sidebar::Metrics::new(width);
        if x <= slots.left || x >= slots.right || y < 3 || y >= self.layout.height - 2 {
            return None;
        }
        let offset = self.side_offset();
        let index = offset + y - 3;
        match self.side_rows().get(index)? {
            workflows::SideRow::Card { id, row }
                if self.sidebar_content_visible(index, *row, offset) =>
            {
                Some(Cursor {
                    wid: *id,
                    tab: 0,
                    run: String::new(),
                })
            }
            workflows::SideRow::Terminal { id, tab, run } => Some(Cursor {
                wid: *id,
                tab: *tab,
                run: run.clone(),
            }),
            _ => None,
        }
    }
    pub(super) fn card_click(&mut self, x: usize, y: usize) -> bool {
        let Some(target) = self.card_at(x, y) else {
            return false;
        };
        let header = matches!(
            self.side_rows().get(self.side_offset() + y - 3),
            Some(workflows::SideRow::Card { row: 0, .. })
        );
        self.cards.children = None;
        self.cards.cursor = Some(target.clone());
        if target.tab != 0 {
            self.card_focus(&target, true);
        } else if header && x == sidebar::Metrics::new(self.sidebar_width()).fold {
            self.card_toggle(target.wid);
        } else if header && x == sidebar::Metrics::new(self.sidebar_width()).detail {
            self.card_open(target.wid, true);
        } else {
            self.card_focus(&target, false);
            if self.notice.is_empty() {
                self.resume_saved_chat();
            }
        }
        true
    }
    pub(super) fn card_popup_visible(&self) -> bool {
        self.cards.popup.is_some()
    }
    pub(super) fn card_popup_pinned(&self) -> bool {
        self.cards.popup.as_ref().is_some_and(|p| p.pinned)
    }
    fn card_rect(&self) -> (usize, usize, usize, usize) {
        let l = self.layout;
        if let Some(p) = &self.cards.popup
            && !p.pinned
        {
            let (ax, ay) = p.anchor.unwrap_or((self.sidebar_width(), 3));
            let mut height = 14.min(l.height.saturating_sub(3));
            let right = self.sidebar_width().saturating_add(1);
            // Prefer the card's right edge. In a full-width Cards view keep the
            // three-cell logo gutter exposed when a readable tooltip still fits.
            let (x, width) = if l.width.saturating_sub(right + 1) >= 28 {
                (right, 56.min(l.width - right - 1))
            } else if l.width >= 36 {
                let x = ax.saturating_add(2).max(8).min(l.width - 29);
                (x, 56.min(l.width - x - 1))
            } else {
                (1, l.width.saturating_sub(2))
            };
            // Very narrow screens cannot fit beside the logo. Prefer a shorter
            // tooltip below or above its row before covering the hovered card.
            if x <= 6 {
                let below = l.height.saturating_sub(ay + 3);
                let above = ay.saturating_sub(2);
                if below.max(above) >= 6 {
                    height = height.min(below.max(above));
                }
            }
            let last_y = l.height.saturating_sub(height + 1).max(1);
            let y = if x <= 6 && ay + 2 <= last_y {
                ay + 2
            } else if x <= 6 && ay > height + 1 {
                ay - height - 1
            } else {
                ay.saturating_sub(1).clamp(1, last_y)
            };
            return (x, y, width, height);
        }
        if l.right >= 30 {
            (l.width - l.right, 2, l.right, l.height - 3)
        } else if l.width >= 90 {
            (l.left + 1, 2, (l.width - l.left - 2).min(64), l.height - 3)
        } else {
            (0, 1, l.width, l.height - 2)
        }
    }
    fn card_page_rows(&self) -> usize {
        let h = self.card_rect().3;
        h.saturating_sub(if h < 8 { 5 } else { 6 }).max(1)
    }
    pub(super) fn card_inside(&self, x: usize, y: usize) -> bool {
        if !self.card_popup_visible() {
            return false;
        }
        let (px, py, w, h) = self.card_rect();
        x >= px && x < px + w && y >= py && y < py + h
    }
    pub(super) fn card_popup_overlaps(&self, x: usize, y: usize, w: usize, h: usize) -> bool {
        if !self.card_popup_visible() {
            return false;
        }
        let (px, py, pw, ph) = self.card_rect();
        px < x + w && px + pw > x && py < y + h && py + ph > y
    }
    fn card_links(&self) -> Vec<Link> {
        self.cards
            .popup
            .as_ref()
            .map(|p| {
                [&p.issue, &p.pr]
                    .into_iter()
                    .filter_map(|s| Link::parse(s))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub(super) fn card_open(&mut self, wid: u64, pinned: bool) {
        if self.paste_target.is_some() {
            return;
        }
        let Some(w) = self
            .snapshot
            .workspaces
            .iter()
            .find(|w| w.id == wid && !w.meta.archived)
        else {
            return;
        };
        self.cards.popup = Some(Popup {
            epoch: self.snapshot.epoch.clone(),
            wid,
            issue: w.meta.issue.clone(),
            pr: w.meta.pr.clone(),
            pinned,
            anchor: self
                .cards
                .hover
                .as_ref()
                .filter(|h| h.wid == wid)
                .map(|h| (h.x, h.y)),
            offset: 0,
            action: 0,
            pressed: None,
            edit: None,
        });
        self.cards.leave = None;
        self.git_hover = None;
        if pinned {
            self.selection = None;
            self.drag = None;
            self.cards.github.request(self.card_links(), false);
        }
    }
    pub(super) fn card_close(&mut self) {
        self.cards.popup = None;
        self.cards.hover = None;
        self.cards.leave = None;
        self.cards.github.cancel();
    }
    pub(super) fn card_reconcile(&mut self) {
        if self.cards.children.is_some_and(|wid| {
            wid != self.snapshot.active
                || !self.prefs.expanded_cards.contains(&wid)
                || !self
                    .snapshot
                    .workspaces
                    .iter()
                    .any(|w| w.id == wid && !w.tabs.is_empty())
        }) {
            self.card_leave_children();
        }
        if self
            .cards
            .hover
            .as_ref()
            .is_some_and(|hover| hover.epoch != self.snapshot.epoch)
        {
            self.cards.hover = None;
        }
        if self.cards.popup.as_ref().is_some_and(|p| {
            p.epoch != self.snapshot.epoch
                || !self.snapshot.workspaces.iter().any(|w| {
                    w.id == p.wid
                        && !w.meta.archived
                        && w.meta.issue == p.issue
                        && w.meta.pr == p.pr
                })
        }) {
            self.card_close();
            self.notice = "Workspace details changed; reopen to inspect the current target".into();
        }
        if let Some(cursor) = &self.cards.cursor
            && !self.snapshot.workspaces.iter().any(|w| {
                w.id == cursor.wid
                    && !w.meta.archived
                    && (cursor.tab == 0
                        || w.tabs
                            .iter()
                            .any(|t| t.id == cursor.tab && t.run == cursor.run))
            })
        {
            self.cards.cursor = None;
            self.card_leave_children();
        }
    }
    fn card_hover(&mut self, x: usize, y: usize) {
        if self.card_popup_pinned()
            || self.form.is_some()
            || self.menu
            || self.board
            || self.confirm.is_some()
            || self.pet.controls.is_some()
            || self.image_view()
            || (self.layout.right == 0 && self.focus == Focus::Files)
        {
            self.cards.hover = None;
            return;
        }
        if self.card_popup_visible() && self.card_inside(x, y) {
            self.cards.leave = None;
            return;
        }
        if let Some(target) = self.card_at(x, y) {
            let at = self
                .cards
                .hover
                .as_ref()
                .filter(|h| h.wid == target.wid)
                .map_or_else(Instant::now, |h| h.at);
            self.cards.hover = Some(Hover {
                epoch: self.snapshot.epoch.clone(),
                wid: target.wid,
                at,
                x,
                y,
            });
            self.cards.leave = None;
        } else {
            self.cards.hover = None;
            if self.card_popup_visible() {
                self.cards.leave.get_or_insert(Instant::now());
            }
        }
    }
    pub(super) fn tick_cards(&mut self) -> bool {
        let mut dirty = self.cards.github.tick();
        if !self.card_popup_pinned() {
            if let Some(hover) = &self.cards.hover
                && hover.at.elapsed() >= Duration::from_millis(450)
                && self.cards.popup.as_ref().map(|p| p.wid) != Some(hover.wid)
            {
                self.card_open(hover.wid, false);
                dirty = true;
            }
            if self
                .cards
                .leave
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(220))
            {
                self.card_close();
                dirty = true;
            }
        }
        dirty
    }
    fn card_action(&mut self, action: Action) {
        // Re-read before acting. Changed links/epochs invalidate the frozen popup.
        if let Ok(snapshot) = self.read_snapshot() {
            self.snapshot(snapshot);
        } else {
            self.notice = "Unable to verify workspace; action cancelled".into();
            return;
        }
        let Some(p) = &self.cards.popup else {
            return;
        };
        if !p.pinned || !actions(p).contains(&action) {
            return;
        }
        let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == p.wid) else {
            return;
        };
        let url = match action {
            Action::Issue | Action::CopyIssue => p.issue.clone(),
            Action::Pr | Action::CopyPr => p.pr.clone(),
            _ => w.cwd.clone(),
        };
        match action {
            Action::Notes => {
                let notes = w.meta.notes.clone();
                self.cards.popup.as_mut().unwrap().edit = Some(Edit {
                    expected: notes.clone(),
                    bytes: notes.into_bytes(),
                    error: String::new(),
                });
            }
            Action::Refresh => self.cards.github.request(self.card_links(), true),
            Action::Issue | Action::Pr if self.remote.is_none() => {
                if let Some(mut command) = github::opener(&url) {
                    match command.spawn() {
                        Ok(mut child) => {
                            std::thread::spawn(move || {
                                let _ = child.wait();
                            });
                            self.notice = "Opening GitHub link".into();
                        }
                        Err(_) => {
                            self.notice = "Could not open browser; use the copy URL action".into()
                        }
                    }
                }
            }
            _ => match selection::clipboard_write(&url) {
                Ok(sequence) => {
                    self.clipboard = Some(sequence);
                    self.notice = "Sent to clipboard".into();
                }
                Err(e) => self.notice = e.into(),
            },
        }
    }
    fn card_edit_key(&mut self, key: &Key) {
        let (_, y, _, height) = self.card_rect();
        if let Key::Mouse { x: hit_x, y: hit } = key {
            if !self.card_inside(*hit_x, *hit) {
                return;
            }
            if *hit == y + height - 3 {
                self.card_edit_key(&Key::Bytes(b"\r".to_vec()));
            } else if *hit == y + height - 2 {
                self.card_edit_key(&Key::Bytes(b"\x1b".to_vec()));
            }
            return;
        }
        if matches!(key, Key::Bytes(b) if b == b"\x19") {
            if let Some(edit) = self.cards.popup.as_ref().and_then(|p| p.edit.as_ref()) {
                match selection::clipboard_write(&String::from_utf8_lossy(&edit.bytes)) {
                    Ok(sequence) => {
                        self.clipboard = Some(sequence);
                        self.notice = "Edit sent to clipboard".into();
                    }
                    Err(e) => self.notice = e.into(),
                }
            }
            return;
        }
        let Some(p) = &mut self.cards.popup else {
            return;
        };
        let Some(edit) = &mut p.edit else {
            return;
        };
        match key {
            Key::Bytes(b) if b == b"\x1b" => {
                p.edit = None;
            }
            Key::Bytes(b) if b == b"\r" => {
                let epoch = p.epoch.clone();
                let wid = p.wid;
                let old = wire::hex(edit.expected.as_bytes());
                let text = String::from_utf8_lossy(&edit.bytes).into_owned();
                let new = wire::hex(text.as_bytes());
                self.command(&["notes", &epoch, &wid.to_string(), &old, &new]);
                if let Some(p) = &mut self.cards.popup {
                    if self.notice.is_empty() {
                        p.edit = None;
                    } else if let Some(edit) = &mut p.edit {
                        edit.error = self.notice.clone();
                    }
                }
            }
            Key::Bytes(b) if b == b"\x7f" => {
                edit.bytes.pop();
                while std::str::from_utf8(&edit.bytes).is_err() && !edit.bytes.is_empty() {
                    edit.bytes.pop();
                }
            }
            Key::Bytes(b) if b == b"\x15" => edit.bytes.clear(),
            Key::Bytes(b) if b == b"\n" && edit.bytes.len() < 65536 => edit.bytes.push(b'\n'),
            Key::Bytes(b) if b.len() == 1 && b[0] >= 32 && edit.bytes.len() < 65536 => {
                edit.bytes.extend_from_slice(b)
            }
            Key::Paste(b) => {
                let b = b.strip_prefix(b"\x1b[200~").unwrap_or(b);
                let b = b.strip_suffix(b"\x1b[201~").unwrap_or(b);
                for byte in b {
                    if edit.bytes.len() >= 65536 {
                        break;
                    }
                    if *byte >= 32 && *byte != 127 || matches!(*byte, b'\n' | b'\t') {
                        edit.bytes.push(*byte);
                    } else if *byte == b'\r' {
                        edit.bytes.push(b'\n');
                    }
                }
            }
            _ => {}
        }
    }
    pub(super) fn card_popup_key(&mut self, key: &Key) -> bool {
        if let Key::Hover { x, y } = key {
            self.card_hover(*x, *y);
            return self.card_popup_visible();
        }
        if self.cards.popup.is_none() {
            self.cards.hover = None;
        }
        if self.cards.popup.as_ref().is_some_and(|p| p.edit.is_some()) {
            self.card_edit_key(key);
            return true;
        }
        if !self.card_popup_pinned() {
            if self.card_popup_visible() {
                if let Key::Mouse { x, y } = key
                    && self.card_inside(*x, *y)
                {
                    let wid = self.cards.popup.as_ref().unwrap().wid;
                    self.card_open(wid, true);
                    return true;
                }
                if let Key::Wheel { x, y, delta, .. } = key
                    && self.card_inside(*x, *y)
                {
                    let last = self
                        .card_content()
                        .len()
                        .saturating_sub(self.card_page_rows());
                    let popup = self.cards.popup.as_mut().unwrap();
                    popup.offset = popup
                        .offset
                        .saturating_add_signed(*delta as isize)
                        .min(last);
                    return true;
                }
                if matches!(key, Key::Release { x, y } | Key::Drag { x, y }
                    if self.card_inside(*x, *y))
                {
                    return true;
                }
                if matches!(
                    key,
                    Key::Bytes(_) | Key::Paste(_) | Key::Mouse { .. } | Key::Wheel { .. }
                ) {
                    self.card_close();
                }
            }
            return false;
        }
        let choices = actions(self.cards.popup.as_ref().unwrap());
        let (px, py, width, h) = self.card_rect();
        let page_rows = self.card_page_rows();
        match key {
            Key::Bytes(b) => match b.as_slice() {
                b"\x1b" => self.card_close(),
                b"\t" | b"\x1b[Z" => {
                    let p = self.cards.popup.as_mut().unwrap();
                    p.action = (p.action as isize + if b == b"\t" { 1 } else { -1 })
                        .rem_euclid(choices.len() as isize) as usize;
                    p.pressed = None;
                }
                b"\r" => {
                    let index = self
                        .cards
                        .popup
                        .as_ref()
                        .unwrap()
                        .action
                        .min(choices.len() - 1);
                    self.card_action(choices[index]);
                }
                b"j" | b"\x1b[B" | b"\x1b[6~" => {
                    let last = self
                        .card_content()
                        .len()
                        .saturating_sub(self.card_page_rows());
                    let p = self.cards.popup.as_mut().unwrap();
                    p.offset =
                        (p.offset + if b == b"\x1b[6~" { page_rows.max(1) } else { 1 }).min(last);
                }
                b"k" | b"\x1b[A" | b"\x1b[5~" => {
                    let p = self.cards.popup.as_mut().unwrap();
                    p.offset =
                        p.offset
                            .saturating_sub(if b == b"\x1b[5~" { page_rows.max(1) } else { 1 });
                }
                _ => {
                    if b.len() == 1
                        && let Some(a) = choices.iter().find(|a| a.key() == b[0])
                    {
                        self.card_action(*a);
                    }
                }
            },
            Key::Wheel { delta, .. } => {
                let last = self
                    .card_content()
                    .len()
                    .saturating_sub(self.card_page_rows());
                let p = self.cards.popup.as_mut().unwrap();
                p.offset = p.offset.saturating_add_signed(*delta as isize).min(last);
            }
            Key::Mouse { x, y } => {
                if !self.card_inside(*x, *y) {
                    self.card_close();
                } else if *y == py + h - 3
                    || (*y == py + h - 2
                        && (18..30).contains(&width)
                        && *x >= px + 7
                        && *x < px + 12)
                {
                    let p = self.cards.popup.as_mut().unwrap();
                    p.pressed = Some((choices[p.action.min(choices.len() - 1)], *x, *y));
                } else if *y == py + h - 2 && *x >= px + width - if width < 30 { 5 } else { 11 } {
                    self.card_close();
                } else if *y == py + h - 2 && *x < px + if width < 30 { 5 } else { 10 } {
                    let p = self.cards.popup.as_mut().unwrap();
                    p.action = (p.action + 1) % choices.len();
                    p.pressed = None;
                }
            }
            Key::Release { x, y } => {
                let p = self.cards.popup.as_mut().unwrap();
                if let Some((a, ax, ay)) = p.pressed.take()
                    && ax == *x
                    && ay == *y
                    && (*y == py + h - 3
                        || (*y == py + h - 2
                            && (18..30).contains(&width)
                            && *x >= px + 7
                            && *x < px + 12))
                    && self.card_inside(*x, *y)
                {
                    self.card_action(a);
                }
            }
            Key::Drag { .. } => self.cards.popup.as_mut().unwrap().pressed = None,
            _ => {}
        }
        true // Pinned overlays own every key and paste, including unknown shortcuts.
    }
    fn card_content(&self) -> Vec<String> {
        let Some(p) = &self.cards.popup else {
            return Vec::new();
        };
        let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == p.wid) else {
            return Vec::new();
        };
        let mut lines = vec![
            format!(
                "Project: {}",
                if w.meta.project.is_empty() {
                    "—"
                } else {
                    &w.meta.project
                }
            ),
            format!("Directory: {}", w.cwd),
            format!(
                "Branch: {}",
                if w.meta.branch.is_empty() {
                    "not recorded"
                } else {
                    &w.meta.branch
                }
            ),
            format!("Status: {}", w.meta.status.label()),
            String::new(),
            format!("Terminals ({})", w.tabs.len()),
        ];
        if !w.meta.base_sha.is_empty() {
            lines.insert(3, format!("Base: {}", w.meta.base_sha));
        }
        if self.is_main_checkout(w) {
            lines.insert(2, "Checkout: primary".into());
        }
        if w.tabs.is_empty() {
            lines.push("No terminals · stopped".into());
        }
        for t in &w.tabs {
            lines.push(format!(
                "{} {} · {} · {}",
                if w.id == self.snapshot.active && t.id == self.snapshot.tab {
                    "*"
                } else {
                    " "
                },
                t.title,
                t.kind,
                tab_status(t)
            ));
        }
        for (label, value) in [("Issue", &p.issue), ("PR", &p.pr)] {
            if value.is_empty() {
                continue;
            }
            lines.push(String::new());
            if let Some(link) = Link::parse(value) {
                lines.push(format!("{label} #{}", link.number));
                lines.extend(self.cards.github.lines(&link, p.pinned));
                lines.push(link.url);
            } else {
                lines.push(format!("{label}: unsupported GitHub URL"));
                lines.push(wire::passive(value));
            }
        }
        lines.extend([String::new(), "Notes".into()]);
        if w.meta.notes.is_empty() {
            lines.push("No notes yet · e to add".into());
        } else {
            lines.extend(w.meta.notes.lines().map(String::from));
        }
        lines.push(String::new());
        for a in actions(p) {
            lines.push(format!(
                "[{}] {}",
                a.key() as char,
                a.label(self.remote.is_some())
            ));
        }
        let width = self.card_rect().2.saturating_sub(4).max(1);
        lines
            .into_iter()
            .flat_map(|line| chrome::wrap(&line, width, 65536))
            .collect()
    }
    pub(super) fn draw_card_popup(&self, c: &mut Canvas) {
        let Some(p) = &self.cards.popup else {
            return;
        };
        let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == p.wid) else {
            return;
        };
        let (x, y, width, height) = self.card_rect();
        let inner = width.saturating_sub(4).max(1);
        c.fill(x, y, width, height, style(TEXT, BG, false));
        c.border(x, y, width, height, if p.pinned { CYAN } else { BORDER });
        c.text(
            x + 2,
            y + 1,
            inner,
            &chrome::elide(
                &format!(
                    "{} · {}",
                    if p.edit.is_some() {
                        "Edit notes"
                    } else {
                        "Details"
                    },
                    w.name
                ),
                inner,
            ),
            style(if p.pinned { CYAN } else { TEXT }, BG, true),
        );
        if let Some(edit) = &p.edit {
            let body: Vec<_> = String::from_utf8_lossy(&edit.bytes)
                .split('\n')
                .flat_map(|s| chrome::wrap(s, inner, 65536))
                .collect();
            let visible = self.card_page_rows();
            for (i, line) in body
                .iter()
                .skip(body.len().saturating_sub(visible))
                .take(visible)
                .enumerate()
            {
                c.text(x + 2, y + 2 + i, inner, line, style(TEXT, BG, false));
            }
            c.text(
                x + 2,
                y + height - 4,
                inner,
                &edit.error,
                style(GOLD, BG, false),
            );
            c.text(
                x + 2,
                y + height - 3,
                inner,
                "[Enter] Save notes · Ctrl+Y copy",
                style(CYAN, BG, false),
            );
            c.text(
                x + 2,
                y + height - 2,
                inner,
                "[Esc] Cancel · Ctrl+J newline",
                style(MUTED, BG, false),
            );
            return;
        }
        let content = self.card_content();
        let visible = self.card_page_rows();
        let offset = p.offset.min(content.len().saturating_sub(visible));
        for (i, line) in content.iter().skip(offset).take(visible).enumerate() {
            c.text(x + 2, y + 2 + i, inner, line, style(TEXT, BG, false));
        }
        if content.len() > visible && height >= 8 {
            c.text(
                x + 2,
                y + height - 4,
                inner,
                &format!(
                    "{}–{} / {} · {} scroll",
                    offset + 1,
                    (offset + visible).min(content.len()),
                    content.len(),
                    if p.pinned { "j/k" } else { "wheel" }
                ),
                style(MUTED, BG, false),
            );
        }
        if p.pinned {
            let choices = actions(p);
            let a = choices[p.action.min(choices.len() - 1)];
            c.fill(
                x + 1,
                y + height - 3,
                width - 2,
                1,
                style(TEXT, ACTIVE_BG, false),
            );
            c.text(
                x + 2,
                y + height - 3,
                inner,
                &format!(
                    "> [{}] {}{}",
                    a.key() as char,
                    a.label(self.remote.is_some()),
                    if width >= 30 { " · Enter" } else { "" }
                ),
                style(CYAN, ACTIVE_BG, true),
            );
            let small = width < 30;
            c.text(
                x + 2,
                y + height - 2,
                if small { 3 } else { 8 },
                if small { "Tab" } else { "Tab next" },
                style(MUTED, BG, false),
            );
            c.text(
                x + width - if small { 5 } else { 11 },
                y + height - 2,
                if small { 3 } else { 9 },
                if small { "Esc" } else { "Esc close" },
                style(MUTED, BG, false),
            );
            if small && width >= 18 {
                c.text(x + 7, y + height - 2, 5, "Enter", style(MUTED, BG, false));
            }
        } else {
            c.text(
                x + 2,
                y + height - 3,
                inner,
                if width < 32 {
                    "Click for details"
                } else {
                    "Preview · click to focus"
                },
                style(MUTED, BG, false),
            );
            c.text(
                x + 2,
                y + height - 2,
                inner,
                if width < 32 {
                    "Move away to close"
                } else {
                    "Cards: ? details · Tab terminals"
                },
                style(MUTED, BG, false),
            );
        }
    }
}
