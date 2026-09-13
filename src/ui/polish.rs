//! Small, attachment-local discoverability features. No child input or polling jobs.
use super::*;
use crate::workspace::Workflow;
impl Ui {
    pub(super) fn sidebar_card_height(&self) -> usize {
        if self.prefs.compact_cards {
            2
        } else {
            workflows::CARD_HEIGHT
        }
    }
    fn attention_cards(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self
            .snapshot
            .workspaces
            .iter()
            .filter(|w| {
                !w.meta.archived
                    && w.meta.status == Workflow::NeedsMe
                    && (self.prefs.project.is_empty() || w.meta.project == self.prefs.project)
            })
            .map(|w| w.id)
            .collect();
        ids.sort_unstable(); // Creation identity stays stable as activity and groups change.
        ids
    }
    pub(super) fn attention_summary(&self) -> String {
        let needs = self.attention_cards().len();
        let working = self
            .snapshot
            .workspaces
            .iter()
            .filter(|w| {
                !w.meta.archived
                    && (self.prefs.project.is_empty() || w.meta.project == self.prefs.project)
            })
            .filter(|w| w.tabs.iter().any(|t| t.working))
            .count();
        format!("{needs} need me · {working} working")
    }
    pub(super) fn next_attention(&mut self) {
        let ids = self.attention_cards();
        if let Some(id) = ids
            .iter()
            .copied()
            .find(|id| *id > self.snapshot.active)
            .or_else(|| ids.first().copied())
        {
            self.side_scroll = None;
            if let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == id) {
                let group = self.sidebar_group_key(w);
                if self.prefs.folds.iter().any(|fold| fold == &group) {
                    self.prefs.folds.retain(|fold| fold != &group);
                    self.save_preferences();
                }
            }
            self.command(&["focus", &id.to_string(), "0"]);
            self.focus = Focus::Cards;
        } else {
            self.show_mouse_hint("No workspaces need you in this project");
        }
    }
    pub(super) fn show_mouse_hint(&mut self, text: &str) {
        self.mouse_hint = Some((text.into(), Instant::now() + Duration::from_secs(3)));
    }
    pub(super) fn tick_polish(&mut self) -> bool {
        let now = Instant::now();
        let mut dirty = false;
        if self.nav_help_at.is_some_and(|at| now >= at) {
            self.nav_help_at = None;
            dirty = self.nav;
        }
        if self.mouse_hint.as_ref().is_some_and(|(_, at)| now >= *at) {
            self.mouse_hint = None;
            dirty = true;
        }
        dirty
    }
    fn action_matches(&self) -> Vec<(u8, &'static str)> {
        let query = String::from_utf8_lossy(&self.menu_query).to_lowercase();
        workflows::ACTIONS
            .iter()
            .copied()
            .filter(|(_, label)| {
                let lower = label.to_lowercase();
                query.split_whitespace().all(|word| lower.contains(word))
            })
            .collect()
    }
    pub(super) fn append_action_query(&mut self, bytes: &[u8]) {
        self.menu_query.extend(
            bytes
                .iter()
                .copied()
                .filter(|b| *b >= 32 && *b != 127)
                .take(256usize.saturating_sub(self.menu_query.len())),
        );
        self.menu_index = 0;
    }
    /// True consumes the key. False replaces Enter with the selected actual shortcut.
    pub(super) fn actions_key(&mut self, b: &mut Vec<u8>) -> bool {
        let items = self.action_matches();
        match b.as_slice() {
            b"\x1b" => {
                if self.menu_search {
                    self.menu_search = false;
                    self.menu_query.clear();
                    self.menu_index = 0;
                } else {
                    self.menu = false;
                }
            }
            b"/" if !self.menu_search => {
                self.menu_search = true;
                self.menu_index = 0;
            }
            b"\x1b[B" => self.menu_index = (self.menu_index + 1).min(items.len().saturating_sub(1)),
            b"\x1b[A" => self.menu_index = self.menu_index.saturating_sub(1),
            b"j" if !self.menu_search => {
                self.menu_index = (self.menu_index + 1) % items.len().max(1)
            }
            b"k" if !self.menu_search => self.menu_index = self.menu_index.saturating_sub(1),
            b"\r" => {
                if let Some((key, _)) = items.get(self.menu_index) {
                    *b = vec![*key];
                    return false;
                }
            }
            b"\x7f" if self.menu_search => {
                self.menu_query.pop();
                while !self.menu_query.is_empty() && std::str::from_utf8(&self.menu_query).is_err()
                {
                    self.menu_query.pop();
                }
                self.menu_index = 0;
            }
            b"\x15" if self.menu_search => {
                self.menu_query.clear();
                self.menu_index = 0;
            }
            _ if self.menu_search => {
                if b.len() == 1 {
                    self.append_action_query(b);
                }
            }
            _ => return false,
        }
        true
    }
    pub(super) fn draw_actions(&self, c: &mut Canvas) {
        if let Some(confirm) = &self.confirm {
            confirm.paint(c, self.layout);
            return;
        }
        if !self.menu {
            return;
        }
        let l = self.layout;
        let w = 66.min(l.width);
        let h = (workflows::ACTIONS.len() + 7).min(20).min(l.height - 2);
        let x = (l.width - w) / 2;
        let y = (l.height - h) / 2;
        c.fill(x, y, w, h, style(TEXT, PANEL, false));
        c.border(x, y, w, h, BORDER);
        c.text(
            x + 2,
            y + 1,
            w - 4,
            "FLERE ACTIONS",
            style(CYAN, PANEL, true),
        );
        c.text(
            x + 2,
            y + 2,
            w - 4,
            &if self.menu_search {
                format!(
                    "/ {}",
                    chrome::tail(&String::from_utf8_lossy(&self.menu_query), w - 7)
                )
            } else {
                "Press / to search; shortcuts also work here".into()
            },
            style(MUTED, PANEL, false),
        );
        let items = self.action_matches();
        let visible = h.saturating_sub(5);
        let start = self.menu_index.saturating_sub(visible.saturating_sub(1));
        if items.is_empty() {
            c.text(
                x + 3,
                y + 3,
                w - 6,
                "No matching actions",
                style(MUTED, PANEL, false),
            );
        }
        for (i, (key, label)) in items.iter().enumerate().skip(start).take(visible) {
            let row = y + 3 + i - start;
            let selected = i == self.menu_index;
            let bg = if selected { ACTIVE_BG } else { PANEL };
            c.fill(x + 1, row, w - 2, 1, style(TEXT, bg, false));
            if selected {
                c.text(x + 1, row, 1, "▌", style(CYAN, bg, true));
            }
            c.text(
                x + 3,
                row,
                w - 9,
                &chrome::elide(label, w - 9),
                style(if selected { CYAN } else { TEXT }, bg, selected),
            );
            c.text(
                x + w - 4,
                row,
                1,
                &(*key as char).to_string(),
                style(MUTED, bg, false),
            );
        }
        c.text(
            x + 2,
            y + h - 2,
            w - 4,
            if self.menu_search {
                "Type query · ↑/↓ select · Enter run · Esc back"
            } else {
                "/ search · j/k select · Enter run · Esc back"
            },
            style(MUTED, PANEL, false),
        );
    }
}
