//! Workbench chrome derived from existing workspace state; never native input.
use super::*;
use crate::{model::WorkspaceView, workspace::Workflow};
use workflows::{CARD_HEIGHT, activity_glyph, project_color, tint};

const SURFACE: Color = Color::Rgb(12, 27, 37);
const SOFT: Color = Color::Rgb(91, 111, 142);
const GREEN: Color = Color::Rgb(125, 218, 176);

fn cells(text: &str) -> usize {
    text.chars().map(|c| char_width(c) as usize).sum()
}
pub(super) fn elide(text: &str, width: usize) -> String {
    let text = wire::passive(text);
    if cells(&text) <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let size = char_width(ch) as usize;
        if used + size >= width {
            break;
        }
        out.push(ch);
        used += size;
    }
    out.push('…');
    out
}
// Preserve explicit note lines. Prefer word boundaries, but also bound long paths.
pub(super) fn wrap(text: &str, width: usize, limit: usize) -> Vec<String> {
    if width == 0 || limit == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for paragraph in text.split('\n') {
        let clean = wire::passive(paragraph);
        let mut rest = clean.trim();
        if rest.is_empty() {
            rows.push(String::new());
        }
        while !rest.is_empty() {
            let mut used = 0;
            let mut end = rest.len();
            let mut space = None;
            for (i, ch) in rest.char_indices() {
                let size = char_width(ch) as usize;
                if used + size > width {
                    end = if used >= width / 2 {
                        space.unwrap_or(i)
                    } else {
                        i
                    };
                    break;
                }
                used += size;
                if ch == ' ' && used >= width / 2 {
                    space = Some(i);
                }
            }
            if end == 0 {
                // A two-cell glyph cannot fit in a one-cell column.
                rows.push("…".into());
                rest = &rest[rest.chars().next().unwrap().len_utf8()..];
            } else {
                rows.push(rest[..end].trim_end().into());
                rest = rest[end..].trim_start();
            }
            if rows.len() > limit {
                rows.truncate(limit);
                let last = rows.last_mut().unwrap();
                *last = elide(&format!("{last} …"), width);
                return rows;
            }
        }
        if rows.len() > limit {
            rows.truncate(limit);
            let last = rows.last_mut().unwrap();
            *last = elide(&format!("{last} …"), width);
            return rows;
        }
    }
    rows
}
fn file_kind(name: &str, dir: bool) -> (&'static str, &'static str, Color) {
    if name == ".." && dir {
        return ("↰", "Parent directory", MUTED);
    }
    if dir {
        return ("▸", "Directory", WORKING);
    }
    match Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => ("rs", "Rust source", GOLD),
        "md" | "txt" => ("≡", "Text / documentation", CYAN),
        "json" | "toml" | "yaml" | "yml" | "lock" => ("{}", "Configuration / data", MAGENTA),
        "png" | "jpg" | "jpeg" => ("◇", "Image · Enter preview · E editor", MAGENTA),
        "py" | "js" | "ts" | "tsx" | "jsx" | "sh" => ("λ", "Source code", GREEN),
        "html" | "css" | "svg" => ("<> ", "Web document", CYAN),
        _ => ("·", "File", MUTED),
    }
}
pub(super) fn agent_label(w: &WorkspaceView) -> String {
    let agents: Vec<_> = w
        .tabs
        .iter()
        .filter(|t| t.alive && t.kind == "agent")
        .collect();
    if agents.len() > 1 {
        return format!("{} agents", agents.len());
    }
    if let Some(agent) = agents.first() {
        return elide(&agent.title, 12);
    }
    if !w.meta.issue.is_empty() || !w.meta.conversations.is_empty() {
        "no agent".into()
    } else {
        let live = w.tabs.iter().filter(|t| t.alive).count();
        match live {
            0 => "stopped".into(),
            1 => "1 tab".into(),
            n => format!("{n} terminals"),
        }
    }
}

impl Ui {
    pub(super) fn chrome_controls_active(&self) -> bool {
        !self.menu
            && !self.card_popup_pinned()
            && self.form.is_none()
            && self.confirm.is_none()
            && self.pet.controls.is_none()
            && self.scrollback.is_none()
            && !self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
    }
    pub(super) fn chrome_focused(&self, focus: Focus) -> bool {
        self.focus == focus && !self.board && self.chrome_controls_active()
    }
    pub(super) fn draw_workspace_card(
        &self,
        c: &mut Canvas,
        w: &WorkspaceView,
        x: usize,
        y: usize,
        width: usize,
    ) {
        if width < 4 {
            return;
        }
        let active = w.id == self.snapshot.active;
        let working = w.tabs.iter().filter(|t| t.working).count();
        let (accent, _) = tint(w.meta.status);
        let cursor = self.card_cursor();
        let focused = (if self.board {
            active
        } else {
            cursor.wid == w.id && cursor.tab == 0
        }) && self.chrome_controls_active()
            && if self.board {
                x >= self.layout.left && x < self.layout.width - self.layout.right
            } else {
                self.focus == Focus::Cards
            };
        let bg = if focused {
            ACTIVE_BG
        } else if active {
            SURFACE
        } else {
            PANEL
        };
        let inner = width.saturating_sub(4);
        let compact = self.prefs.compact_cards && !self.board;
        let height = if compact { 2 } else { CARD_HEIGHT };
        c.fill(x, y, width, height, style(TEXT, bg, false));
        for row in 0..height {
            c.text(
                x,
                y + row,
                1,
                if focused { "▌" } else { " " },
                style(if focused { CYAN } else { BORDER }, bg, false),
            );
        }
        if active {
            c.text(
                x + 1,
                y,
                1,
                "›",
                style(if focused { CYAN } else { MUTED }, bg, false),
            );
        }
        let key = self.project_icons.get(w);
        let label = if !w.meta.project.is_empty() {
            w.meta.project.as_str()
        } else {
            Path::new(&w.cwd)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
        };
        let columns = key
            .as_ref()
            .and_then(|key| {
                self.graphics_cell().map(|cell| {
                    self.project_icons
                        .columns(key, cell, inner.saturating_sub(9))
                })
            })
            .unwrap_or(2);
        let inset = if width >= 14 && !label.is_empty() {
            usize::from(columns) + 1
        } else {
            0
        };
        if inset > 0 {
            let initials: String = label
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .collect();
            c.text(
                x + 2,
                y,
                usize::from(columns),
                &if key.as_ref().is_some_and(|key| {
                    self.local_graphics
                        .as_ref()
                        .is_some_and(|g| g.capable() && g.has_icon(key))
                }) {
                    " ".repeat(usize::from(columns))
                } else {
                    initials.to_uppercase()
                },
                style(MUTED, bg, false),
            );
            if let Some(key) = key
                && c.badges.len() < crate::avatar::LIMIT
                && x + 2 + usize::from(columns) <= c.width
                && y < c.height
            {
                c.badges.push(crate::avatar::Badge {
                    x: (x + 2) as u16,
                    y: y as u16,
                    columns,
                    key,
                    bg: match bg {
                        Color::Rgb(r, g, b) => [r, g, b],
                        _ => [9, 18, 27],
                    },
                });
            }
        }
        let status = if working > 0 {
            "●"
        } else if w.meta.status == Workflow::NeedsMe {
            "◆"
        } else if w.meta.status == Workflow::Done {
            "✓"
        } else {
            ""
        };
        let title_width = inner.saturating_sub(
            inset
                + if compact {
                    6
                } else if self.board {
                    0
                } else {
                    2
                },
        );
        let title = if compact {
            vec![elide(&w.name, title_width)]
        } else {
            wrap(&w.name, title_width, 2)
        };
        for (row, line) in title.iter().enumerate() {
            c.text(
                x + 2 + inset,
                y + row,
                title_width,
                line,
                style(TEXT, bg, row == 0),
            );
        }
        if compact {
            c.text(
                x + width - 7,
                y,
                1,
                status,
                style(if working > 0 { WORKING } else { accent }, bg, false),
            );
        }
        if !self.board {
            c.text(
                x + 1,
                y,
                1,
                if self.prefs.expanded_cards.contains(&w.id) {
                    "▾"
                } else {
                    "▸"
                },
                style(if focused { CYAN } else { MUTED }, bg, false),
            );
            if compact {
                let count = w.tabs.len().to_string();
                c.text(
                    x + width.saturating_sub(4 + count.len()),
                    y,
                    count.len(),
                    &count,
                    style(MUTED, bg, false),
                );
            }
            if active {
                c.text(
                    x,
                    y,
                    1,
                    "›",
                    style(if focused { CYAN } else { MUTED }, bg, false),
                );
            }
            c.text(x + width - 2, y, 1, "?", style(MUTED, bg, false));
        }
        if title.len() < 2 {
            let context = if !w.meta.branch.is_empty() {
                format!("↳ {}", w.meta.branch)
            } else if !w.meta.project.is_empty() {
                w.meta.project.clone()
            } else {
                Path::new(&w.cwd)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into()
            };
            c.text(
                x + 2,
                y + 1,
                inner,
                &elide(&context, inner),
                style(SOFT, bg, false),
            );
        }
        if compact {
            return;
        }
        let icon = if working > 0 {
            activity_glyph(self.animation_step, self.prefs.reduced_motion)
        } else if w.meta.status == Workflow::NeedsMe {
            "◆"
        } else if w.meta.status == Workflow::Done {
            "✓"
        } else if w.tabs.iter().any(|t| t.alive) {
            "●"
        } else {
            "○"
        };
        let runtime = agent_label(w);
        let label = if working > 0 {
            format!("Working · {runtime}")
        } else if self.git_views.get(&w.id).is_some_and(|g| g.primary) {
            format!("{runtime} · [primary]")
        } else if !w.meta.pinned && !w.meta.project.is_empty() {
            format!("{runtime} · {}", w.meta.project)
        } else {
            format!("{runtime} · {}", w.meta.status.label())
        };
        c.text(
            x + 2,
            y + 2,
            1,
            icon,
            style(if working > 0 { WORKING } else { accent }, bg, working > 0),
        );
        c.text(
            x + 4,
            y + 2,
            width.saturating_sub(5),
            &elide(&label, width.saturating_sub(5)),
            style(if working > 0 { WORKING } else { MUTED }, bg, working > 0),
        );
        if working > 0 && width >= 42 && !self.prefs.reduced_motion {
            workflows::activity_signal(
                c,
                x + width - 7,
                y + 2,
                6,
                bg,
                self.animation_step + w.id as usize % 7,
            );
        }
    }
    pub(super) fn draw_sidebar(&self, c: &mut Canvas) {
        self.draw_workspace_sidebar(c);
    }
    pub(super) fn inspector_size(&self) -> (usize, usize) {
        let width = if self.layout.right == 0 {
            self.layout.width
        } else {
            self.layout.right
        };
        (self.layout.width - width, width)
    }
    pub(super) fn inspector_tabs(&self) -> Vec<(usize, usize, Inspector, &'static str)> {
        let (x, width) = self.inspector_size();
        let mut start = x + 2;
        [Inspector::Files, Inspector::Git, Inspector::Details]
            .into_iter()
            .map(|kind| {
                let label = if width >= 23 {
                    kind.label()
                } else {
                    match kind {
                        Inspector::Files => "F",
                        Inspector::Git => "G",
                        Inspector::Details => "D",
                    }
                };
                let slot = (start, label.len(), kind, label);
                start += label.len() + 2;
                slot
            })
            .collect()
    }
    pub(super) fn file_rows(&self) -> usize {
        self.inspector_height().saturating_sub(10).max(1)
    }
    pub(super) fn detail_rows(&self) -> usize {
        self.inspector_height().saturating_sub(8).max(1)
    }
    pub(super) fn inspector_page_key(&mut self, key: &[u8]) -> bool {
        if self.focus != Focus::Files || self.prefs.inspector == Inspector::Git || self.board {
            return false;
        }
        let page = if self.prefs.inspector == Inspector::Files {
            self.file_rows()
        } else {
            self.detail_rows()
        } as isize;
        let delta = match key {
            b"\x1b[5~" => -page,
            b"\x1b[6~" => page,
            b"\x1b[H" | b"\x1bOH" | b"\x1b[1~" => -1_000_000,
            b"\x1b[F" | b"\x1bOF" | b"\x1b[4~" => 1_000_000,
            _ => return false,
        };
        self.inspector_move(delta);
        true
    }
    pub(super) fn inspector_wheel(&mut self, x: usize, y: usize, delta: i64) -> bool {
        let (start, width) = self.inspector_size();
        if self.prefs.inspector == Inspector::Git
            || (self.layout.right == 0 && self.focus != Focus::Files)
            || x <= start
            || x >= start + width - 1
            || y < 5
            || y >= self.inspector_height() - 2
        {
            return false;
        }
        self.inspector_move(delta.clamp(-1000, 1000) as isize);
        true
    }
    pub(super) fn draw_inspector(&self, c: &mut Canvas) {
        if self.layout.right == 0 && self.focus != Focus::Files {
            return;
        }
        let (x, width) = self.inspector_size();
        let height = self.layout.height;
        c.fill(x, 1, width, height - 2, style(TEXT, PANEL, false));
        c.text(x, 1, width, &"─".repeat(width), style(BORDER, PANEL, false));
        c.text(
            x,
            height - 2,
            width,
            &"─".repeat(width),
            style(BORDER, PANEL, false),
        );
        for y in 2..height - 2 {
            c.text(x, y, 1, "│", style(BORDER, PANEL, false));
        }
        for (start, size, kind, label) in self.inspector_tabs() {
            let active = kind == self.prefs.inspector;
            c.text(
                start,
                2,
                size,
                label,
                style(if active { CYAN } else { SOFT }, PANEL, false),
            );
        }
        if self.chrome_focused(Focus::Files) {
            c.text(x, 2, 1, "▌", style(CYAN, PANEL, true));
        }
        match self.prefs.inspector {
            Inspector::Files => self.draw_files(c),
            Inspector::Git => self.draw_git(c),
            Inspector::Details => self.draw_details(c),
        }
    }
    fn draw_files(&self, c: &mut Canvas) {
        let (x, width) = self.inspector_size();
        let inner = width.saturating_sub(4);
        let height = self.inspector_height();
        let Some(e) = self.explorers.get(&self.snapshot.active) else {
            return;
        };
        let path = e
            .root
            .file_name()
            .filter(|s| !s.is_empty())
            .unwrap_or(e.root.as_os_str())
            .to_string_lossy();
        c.text(
            x + 2,
            3,
            inner,
            &elide(&format!("/ {path}"), inner),
            style(TEXT, PANEL, true),
        );
        let count = e
            .entries
            .len()
            .saturating_sub(usize::from(e.entries.first().is_some_and(|e| e.0 == "..")));
        c.text(
            x + 2,
            4,
            inner,
            &if e.loading {
                "Loading directory…".into()
            } else if e.error.is_some() {
                "Folder unavailable · r retry".into()
            } else {
                format!(
                    "{count} items  ·  {}",
                    if self.remote.is_some() {
                        "remote"
                    } else {
                        "local"
                    }
                )
            },
            style(SOFT, PANEL, false),
        );
        let visible = self.file_rows();
        let start = e.start(visible);
        for (i, (name, _, dir)) in e.entries.iter().skip(start).take(visible).enumerate() {
            let selected = start + i == e.selected;
            let bg = if selected {
                if self.chrome_focused(Focus::Files) {
                    ACTIVE_BG
                } else {
                    SURFACE
                }
            } else {
                PANEL
            };
            let (icon, _, fg) = file_kind(name, *dir);
            c.fill(x + 1, 5 + i, width - 2, 1, style(TEXT, bg, false));
            if selected {
                c.text(
                    x + 1,
                    5 + i,
                    1,
                    "▌",
                    style(
                        if self.chrome_focused(Focus::Files) {
                            CYAN
                        } else {
                            SOFT
                        },
                        bg,
                        true,
                    ),
                );
            }
            c.text(x + 2, 5 + i, 2.min(inner), icon, style(fg, bg, *dir));
            c.text(
                x + 5,
                5 + i,
                inner.saturating_sub(3),
                &elide(name, inner.saturating_sub(3)),
                style(
                    if name.starts_with('.') { MUTED } else { TEXT },
                    bg,
                    selected,
                ),
            );
        }
        if let Some(error) = &e.error {
            let mut message = error.clone();
            if cfg!(target_os = "macos") && error == "Folder access denied" {
                message.push_str(
                    ". Check this terminal's Files and Folders access in System Settings.",
                );
            }
            for (i, line) in wrap(&message, inner, visible.saturating_sub(2))
                .iter()
                .enumerate()
            {
                c.text(x + 2, 7 + i, inner, line, style(GOLD, PANEL, false));
            }
        }
        if height >= 12 {
            let y = height - 5;
            c.text(
                x + 1,
                y,
                width - 2,
                &"─".repeat(width - 2),
                style(BORDER, PANEL, false),
            );
            if let Some((name, _, dir)) = e.entries.get(e.selected) {
                let (_, kind, fg) = file_kind(name, *dir);
                c.text(
                    x + 2,
                    y + 1,
                    inner,
                    &elide(&format!("{name} · {kind}"), inner),
                    style(fg, PANEL, false),
                );
            }
        }
        if height >= 10 {
            c.text(
                x + 2,
                height - 3,
                inner,
                if e.loading || e.error.is_some() {
                    "r retry  ·  - parent"
                } else {
                    "Enter open  ·  - parent"
                },
                style(MUTED, PANEL, false),
            );
        }
    }
    pub(super) fn detail_lines(&self) -> Vec<(String, Color, bool)> {
        let Some(w) = self.snapshot.workspace() else {
            return Vec::new();
        };
        let width = self.inspector_size().1.saturating_sub(4);
        let mut rows = Vec::new();
        let mut section = |title: &str, value: &str, color: Color| {
            if value.is_empty() {
                return;
            }
            if !rows.is_empty() {
                rows.push((String::new(), MUTED, false));
            }
            rows.push((title.into(), SOFT, true));
            for line in wrap(value, width, 512) {
                rows.push((line, color, false));
            }
        };
        section("WORKSPACE", &w.name, TEXT);
        section(
            "WORKFLOW",
            &format!(
                "{}{}",
                w.meta.status.label(),
                if w.meta.pinned { " · Pinned" } else { "" }
            ),
            tint(w.meta.status).0,
        );
        let working = w.tabs.iter().filter(|t| t.working).count();
        let process = if working > 0 {
            format!("{working} Working · {}", agent_label(w))
        } else {
            agent_label(w)
        };
        section(
            "RUNTIME",
            &process,
            if working > 0 { WORKING } else { TEXT },
        );
        section("PROJECT", &w.meta.project, project_color(&w.meta.project));
        section("BRANCH", &w.meta.branch, CYAN);
        section("TASK NOTES", &w.meta.notes, TEXT);
        section("ISSUE", &w.meta.issue, CYAN);
        section("PULL REQUEST", &w.meta.pr, CYAN);
        section("DIRECTORY", &w.cwd, MUTED);
        section("OPERATION", &w.meta.operation, GOLD);
        let tabs = w
            .tabs
            .iter()
            .map(|t| {
                format!(
                    "{} {} · {}",
                    if t.id == self.snapshot.tab {
                        "›"
                    } else {
                        "·"
                    },
                    t.title,
                    if t.working {
                        "Working"
                    } else if t.alive {
                        "live"
                    } else {
                        "stopped"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        section("TERMINALS", &tabs, TEXT);
        section(
            "CONNECTION",
            match &self.remote {
                Some(remote) if remote.preview_capable => "SSH companion\nImage previews available",
                Some(_) => "SSH companion\nImage previews unavailable",
                None if self.image_capable() => {
                    "Direct terminal attachment\nNative images available"
                }
                None => "Direct terminal attachment\nTerminal image support unavailable",
            },
            MUTED,
        );
        rows
    }
    fn draw_details(&self, c: &mut Canvas) {
        let (x, width) = self.inspector_size();
        let inner = width.saturating_sub(4);
        c.text(
            x + 2,
            3,
            inner,
            "Workspace overview",
            style(TEXT, PANEL, true),
        );
        let rows = self.detail_lines();
        let visible = self.detail_rows();
        let start = self
            .details_scroll
            .get(&self.snapshot.active)
            .copied()
            .unwrap_or(0)
            .min(rows.len().saturating_sub(visible));
        for (i, (line, fg, bold)) in rows.iter().skip(start).take(visible).enumerate() {
            c.text(x + 2, 5 + i, inner, line, style(*fg, PANEL, *bold));
        }
        if self.inspector_height() >= 10 {
            c.text(
                x + 2,
                self.inspector_height() - 3,
                inner,
                &format!("j/k scroll  ·  {}/{}", start + 1, rows.len().max(1)),
                style(MUTED, PANEL, false),
            );
        }
    }
}

pub(super) fn tail(value: &str, width: usize) -> String {
    if value.chars().map(|c| char_width(c) as usize).sum::<usize>() <= width {
        return value.into();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = Vec::new();
    let mut used = 1;
    for ch in value.chars().rev() {
        let n = char_width(ch) as usize;
        if used + n > width {
            break;
        }
        result.push(ch);
        used += n;
    }
    format!("…{}", result.into_iter().rev().collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapped_metadata_stays_in_cells_and_keeps_controls_passive() {
        for width in 1..40 {
            let lines = wrap(
                "東京 notes e\u{301} and a-very-long-filename.rs\nsecond line\x1b[31m",
                width,
                8,
            );
            assert!(lines.len() <= 8);
            assert!(lines.iter().all(|s| cells(s) <= width));
            assert!(lines.iter().all(|s| !s.contains('\x1b')));
            assert!(cells(&elide("東京 extended title e\u{301}", width)) <= width);
        }
        assert_eq!(
            wrap("first line\nsecond line", 20, 3),
            ["first line", "second line"]
        );
        assert!(
            wrap("one two three four five", 8, 2)
                .last()
                .unwrap()
                .ends_with('…')
        );
    }
}
