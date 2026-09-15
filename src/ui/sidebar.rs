//! Sidebar-only card geometry. Logical rows also own scrolling and mouse targets.
use super::*;
use crate::{model::WorkspaceView, workspace::Grouping};
use workflows::{SideRow, activity_glyph, tint};

const SELECTED: Color = Color::Rgb(10, 31, 42);
const PRIMARY_BADGE: &str = "[primary]";

fn draw_primary_badge(c: &mut Canvas, x: usize, y: usize) {
    c.text(
        x,
        y,
        PRIMARY_BADGE.len(),
        PRIMARY_BADGE,
        style(Color::Rgb(176, 184, 193), Color::Rgb(31, 43, 52), false),
    );
}

#[derive(Clone, Copy)]
pub(super) struct Metrics {
    pub left: usize,
    pub right: usize,
    pub fold: usize,
    pub detail: usize,
    title: usize,
    child: usize,
    icons: bool,
    wide: bool,
}
impl Metrics {
    pub fn new(width: usize) -> Self {
        let icons = width >= 24;
        Self {
            left: 1,
            right: width.saturating_sub(2),
            fold: 2,
            detail: width.saturating_sub(3),
            title: if icons { 8 } else { 4 },
            child: if icons { 10 } else { 5 },
            icons,
            wide: width >= 36,
        }
    }
    fn text_width(self) -> usize {
        (self.detail + 1).saturating_sub(self.title)
    }
    fn inline_primary(self, name: &str) -> bool {
        let name_width: usize = name.chars().map(|ch| char_width(ch) as usize).sum();
        self.detail.saturating_sub(self.title + 1) >= name_width.min(8) + 1 + PRIMARY_BADGE.len()
    }
}
fn workflow_glyph(status: crate::workspace::Workflow) -> &'static str {
    use crate::workspace::Workflow;
    match status {
        Workflow::NeedsMe => "◆",
        Workflow::Done => "✓",
        Workflow::Waiting => "…",
        Workflow::InProgress => "●",
        Workflow::Todo => "·",
    }
}
fn heading_background(accent: Color) -> Color {
    if let Color::Rgb(r, g, b) = accent {
        Color::Rgb(
            ((9 * 6 + u16::from(r)) / 7) as u8,
            ((18 * 6 + u16::from(g)) / 7) as u8,
            ((27 * 6 + u16::from(b)) / 7) as u8,
        )
    } else {
        ACTIVE_BG
    }
}
impl Ui {
    pub(super) fn sidebar_width(&self) -> usize {
        if self.layout.left > 0 {
            self.layout.left
        } else if self.focus == Focus::Cards {
            self.layout.width
        } else {
            0
        }
    }
    pub(super) fn sidebar_content_visible(&self, index: usize, row: usize, offset: usize) -> bool {
        let start = index.saturating_sub(row);
        start >= offset
            && start + self.sidebar_card_height() <= offset + self.layout.height.saturating_sub(5)
    }
    pub(super) fn sidebar_group_key(&self, w: &WorkspaceView) -> String {
        if w.meta.pinned {
            "Pinned".into()
        } else {
            match self.prefs.grouping {
                Grouping::Status => w.meta.status.label().into(),
                Grouping::Project => format!("project:{}", w.meta.project),
            }
        }
    }
    pub(super) fn toggle_sidebar_grouping(&mut self) {
        self.prefs.grouping = match self.prefs.grouping {
            Grouping::Status => Grouping::Project,
            Grouping::Project => Grouping::Status,
        };
        self.side_scroll = None;
        self.save_preferences();
    }
    fn sidebar_surface(&self, w: &WorkspaceView) -> (Color, Color) {
        if self.snapshot.active == w.id {
            (SELECTED, CYAN)
        } else if w.meta.status == crate::workspace::Workflow::NeedsMe {
            (tint(w.meta.status).1, GOLD)
        } else {
            (PANEL, BORDER)
        }
    }
    fn sidebar_runtime(&self, w: &WorkspaceView) -> (&'static str, Color, bool) {
        if w.tabs.iter().any(|t| t.alive && t.working) {
            (
                activity_glyph(self.animation_step, self.prefs.reduced_motion),
                WORKING,
                true,
            )
        } else if w.tabs.iter().any(|t| t.alive) {
            ("●", MUTED, false)
        } else {
            ("○", MUTED, false)
        }
    }
    fn sidebar_body(&self, c: &mut Canvas, w: &WorkspaceView, y: usize, s: Metrics, bg: Color) {
        let border = self.sidebar_surface(w).1;
        c.fill(s.left, y, s.right - s.left + 1, 1, style(TEXT, bg, false));
        c.text(
            s.left,
            y,
            1,
            if self.snapshot.active == w.id {
                "▌"
            } else {
                "│"
            },
            style(border, bg, false),
        );
        c.text(s.right, y, 1, "│", style(border, bg, false));
    }
    fn sidebar_border(&self, c: &mut Canvas, w: &WorkspaceView, y: usize, s: Metrics, top: bool) {
        let (bg, border) = self.sidebar_surface(w);
        c.fill(s.left, y, s.right - s.left + 1, 1, style(TEXT, bg, false));
        c.text(
            s.left,
            y,
            1,
            if top { "┌" } else { "└" },
            style(border, bg, false),
        );
        c.text(
            s.left + 1,
            y,
            s.right - s.left - 1,
            &"─".repeat(s.right - s.left - 1),
            style(border, bg, false),
        );
        c.text(
            s.right,
            y,
            1,
            if top { "┐" } else { "┘" },
            style(border, bg, false),
        );
        if top {
            // Workflow, observed work and selection are independent signals,
            // including in compact mode and in the Pinned/Project groups.
            c.text(
                s.detail - 3,
                y,
                1,
                workflow_glyph(w.meta.status),
                style(tint(w.meta.status).0, bg, true),
            );
            let (glyph, color, working) = self.sidebar_runtime(w);
            c.text(s.detail - 1, y, 1, glyph, style(color, bg, working));
        } else if self.is_main_checkout(w) && !s.inline_primary(&w.name) {
            // A narrow card retains its readable title. Its small directory
            // badge moves to the inert bottom rule, outside the metadata row.
            draw_primary_badge(c, s.left + 2, y);
        }
    }
    fn sidebar_icon(&self, c: &mut Canvas, w: &WorkspaceView, y: usize, bg: Color) {
        let key = self.project_icons.get(w);
        let label = if w.meta.project.is_empty() {
            Path::new(&w.cwd)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
        } else {
            &w.meta.project
        };
        let initials: String = label
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(2)
            .collect();
        let initials = initials.to_uppercase();
        let local_image = key.as_ref().is_some_and(|key| {
            self.local_graphics
                .as_ref()
                .is_some_and(|g| g.capable() && g.has_icon(key))
        });
        c.text(
            4,
            y,
            3,
            if local_image { "   " } else { &initials },
            style(MUTED, bg, true),
        );
        if let Some(key) = key
            && c.badges.len() < crate::avatar::LIMIT
        {
            // A stable three-cell slot fits proportionally in both graphics
            // backends. The fallback reserves the same cells; image arrival and
            // source aspect never move workspace text.
            c.badges.push(crate::avatar::Badge {
                x: 4,
                y: y as u16,
                columns: 3,
                key,
                bg: match bg {
                    Color::Rgb(r, g, b) => [r, g, b],
                    _ => [9, 18, 27],
                },
            });
        }
    }
    fn sidebar_card_row(
        &self,
        c: &mut Canvas,
        w: &WorkspaceView,
        y: usize,
        row: usize,
        s: Metrics,
    ) {
        let bg = self.sidebar_surface(w).0;
        match row {
            0 => {
                let cursor = self.card_cursor();
                let focused =
                    self.chrome_focused(Focus::Cards) && cursor.wid == w.id && cursor.tab == 0;
                c.text(
                    s.fold,
                    y,
                    1,
                    if self.prefs.expanded_cards.contains(&w.id) {
                        "▾"
                    } else {
                        "▸"
                    },
                    style(if focused { CYAN } else { MUTED }, bg, focused),
                );
                if s.icons {
                    self.sidebar_icon(c, w, y, bg);
                }
                let primary = self.is_main_checkout(w) && s.inline_primary(&w.name);
                let width = s.detail.saturating_sub(s.title + 1)
                    - if primary { PRIMARY_BADGE.len() + 1 } else { 0 };
                let name = chrome::elide(&w.name, width);
                c.text(s.title, y, width, &name, style(TEXT, bg, true));
                if primary {
                    let used: usize = name.chars().map(|ch| char_width(ch) as usize).sum();
                    draw_primary_badge(c, s.title + used + 1, y);
                }
                c.text(s.detail, y, 1, "?", style(MUTED, bg, false));
            }
            1 => {
                let context = if !w.meta.branch.is_empty() {
                    format!("↳ {}", w.meta.branch)
                } else if !w.meta.project.is_empty() {
                    w.meta.project.clone()
                } else {
                    Path::new(&w.cwd)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
                c.text(
                    s.title,
                    y,
                    s.text_width(),
                    &chrome::elide(&context, s.text_width()),
                    style(MUTED, bg, false),
                );
            }
            _ => {
                let (glyph, color, working) = self.sidebar_runtime(w);
                let runtime = if s.wide {
                    if working {
                        format!("Working · {}", chrome::agent_label(w))
                    } else {
                        chrome::agent_label(w)
                    }
                } else if s.text_width() >= 16 && !w.tabs.iter().any(|t| t.alive) {
                    "stopped".into()
                } else {
                    glyph.into()
                };
                let runtime_width = runtime
                    .chars()
                    .map(|ch| char_width(ch) as usize)
                    .sum::<usize>()
                    .min(s.text_width().saturating_sub(9).max(1));
                let right = s.detail + 1 - runtime_width;
                let workflow = if s.icons {
                    format!(
                        "{} {}",
                        workflow_glyph(w.meta.status),
                        w.meta.status.label()
                    )
                } else {
                    w.meta.status.label().into()
                };
                let width = right.saturating_sub(s.title + 1);
                c.text(
                    s.title,
                    y,
                    width,
                    &chrome::elide(&workflow, width),
                    style(tint(w.meta.status).0, bg, false),
                );
                c.text(
                    right,
                    y,
                    runtime_width,
                    &chrome::elide(&runtime, runtime_width),
                    style(color, bg, working),
                );
            }
        }
    }
    fn sidebar_terminal(&self, c: &mut Canvas, w: &WorkspaceView, tab: u64, y: usize, s: Metrics) {
        let Some(t) = w.tabs.iter().find(|t| t.id == tab) else {
            return;
        };
        let cursor = self.card_cursor();
        let focused = self.chrome_focused(Focus::Cards)
            && cursor.wid == w.id
            && cursor.tab == tab
            && cursor.run == t.run;
        let current = self.snapshot.active == w.id && self.snapshot.tab == tab;
        let bg = if current || focused {
            ACTIVE_BG
        } else {
            self.sidebar_surface(w).0
        };
        self.sidebar_body(c, w, y, s, bg);
        if s.icons {
            c.text(
                4,
                y,
                2,
                if w.tabs.last().is_some_and(|last| last.id == tab) {
                    "└─"
                } else {
                    "├─"
                },
                style(BORDER, bg, false),
            );
        }
        let marker = if current || focused { ">" } else { " " };
        c.text(2, y, 1, marker, style(CYAN, bg, focused));
        c.text(
            if s.icons { 8 } else { 3 },
            y,
            1,
            match t.kind.as_str() {
                "editor" => "≡",
                "agent" => "●",
                _ => "$",
            },
            style(MUTED, bg, false),
        );
        let tail_start = if s.wide { s.detail - 7 } else { s.detail };
        let width = tail_start.saturating_sub(s.child + 1);
        let label = if s.wide && t.title != t.kind {
            format!("{} · {}", t.title, t.kind)
        } else {
            t.title.clone()
        };
        c.text(
            s.child,
            y,
            width,
            &chrome::elide(&label, width),
            style(TEXT, bg, current || focused),
        );
        let working = t.alive && t.working;
        let state = if s.wide {
            cards::tab_status(t)
        } else if !t.alive {
            "○"
        } else if working {
            activity_glyph(self.animation_step, self.prefs.reduced_motion)
        } else {
            "●"
        };
        let len = state
            .chars()
            .map(|ch| char_width(ch) as usize)
            .sum::<usize>();
        c.text(
            s.detail + 1 - len,
            y,
            len,
            state,
            style(if working { WORKING } else { MUTED }, bg, working),
        );
    }
    pub(super) fn draw_workspace_sidebar(&self, c: &mut Canvas) {
        let width = self.sidebar_width();
        if width == 0 {
            return;
        }
        let s = Metrics::new(width);
        let height = self.layout.height;
        c.fill(0, 1, width, height - 2, style(TEXT, PANEL, false));
        for y in [1, height - 2] {
            c.text(0, y, width, &"─".repeat(width), style(BORDER, PANEL, false));
        }
        for y in 2..height - 2 {
            c.text(width - 1, y, 1, "│", style(BORDER, PANEL, false));
        }
        let total = self
            .snapshot
            .workspaces
            .iter()
            .filter(|w| {
                !w.meta.archived
                    && (self.prefs.project.is_empty() || w.meta.project == self.prefs.project)
            })
            .count()
            .to_string();
        let count_x = s.detail + 1 - total.len();
        let grouping = format!("{} ▾", self.prefs.grouping.label());
        let group_width = grouping.chars().count();
        let group_x = count_x.saturating_sub(group_width + 2).max(2);
        if width >= 28 {
            let title = if self.prefs.project.is_empty() {
                "Workspaces"
            } else {
                &self.prefs.project
            };
            let available = group_x.saturating_sub(3);
            c.text(
                2,
                2,
                available,
                &chrome::elide(title, available),
                style(TEXT, PANEL, true),
            );
        }
        let group_x = if width >= 28 { group_x } else { 2 };
        let group_width = group_width.min(count_x.saturating_sub(group_x + 1));
        c.text(
            group_x,
            2,
            group_width,
            &chrome::elide(&grouping, group_width),
            style(CYAN, PANEL, true),
        );
        c.text(count_x, 2, total.len(), &total, style(CYAN, PANEL, true));
        let rows = self.side_rows();
        let offset = self.side_offset();
        let visible = height.saturating_sub(5);
        for (index, row) in rows.iter().enumerate().skip(offset).take(visible) {
            let y = 3 + index - offset;
            match row {
                SideRow::Heading {
                    name,
                    key,
                    accent,
                    count,
                } => {
                    let bg = heading_background(*accent);
                    c.fill(s.left, y, s.right - s.left + 1, 1, style(*accent, bg, true));
                    c.text(
                        s.fold,
                        y,
                        1,
                        if self.prefs.folds.contains(key) {
                            "▸"
                        } else {
                            "▾"
                        },
                        style(*accent, bg, true),
                    );
                    let count = count.to_string();
                    let end = s.detail + 1 - count.len();
                    let width = end.saturating_sub(5);
                    c.text(
                        4,
                        y,
                        width,
                        &chrome::elide(name, width),
                        style(*accent, bg, true),
                    );
                    c.text(end, y, count.len(), &count, style(*accent, bg, true));
                }
                SideRow::CardBorder { id, top } => {
                    if let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == *id) {
                        self.sidebar_border(c, w, y, s, *top);
                    }
                }
                SideRow::Card { id, row } => {
                    if let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == *id) {
                        self.sidebar_body(c, w, y, s, self.sidebar_surface(w).0);
                        if self.sidebar_content_visible(index, *row, offset) {
                            self.sidebar_card_row(c, w, y, *row, s);
                        }
                    }
                }
                SideRow::Terminal { id, tab, .. } => {
                    if let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == *id) {
                        self.sidebar_terminal(c, w, *tab, y, s);
                    }
                }
            }
        }
        let above = rows
            .iter()
            .take(offset)
            .filter(|r| matches!(r, SideRow::Card { row: 0, .. }))
            .count();
        let below = rows
            .iter()
            .enumerate()
            .filter(|(i, r)| {
                matches!(r, SideRow::Card { row: 0, .. })
                    && *i >= offset
                    && *i + self.sidebar_card_height() > offset + visible
            })
            .count();
        let overflow = match (above, below) {
            (0, 0) => String::new(),
            (0, n) => format!("↓ {n} more"),
            (n, 0) => format!("↑ {n} above"),
            (a, b) => format!("↑ {a} above · ↓ {b} more"),
        };
        if !overflow.is_empty() {
            c.text(
                2,
                height - 2,
                width - 4,
                &chrome::elide(&overflow, width - 4),
                style(MUTED, PANEL, false),
            );
        }
    }
}
