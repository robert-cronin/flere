//! Shared workspace views; terminal processes remain supervisor-owned sessions.
use super::*;
use crate::{model::TabView, panes::SplitAxis};

pub(super) struct DividerDrag {
    epoch: String,
    workspace: u64,
    tab: u64,
    run: String,
    revision: u64,
    layout: Layout,
}

#[derive(Clone)]
pub(super) struct EditorOrigin {
    pub epoch: String,
    pub workspace: u64,
    pub tab: u64,
    pub run: String,
    pub revision: u64,
}

pub(super) fn layouts(base: Layout, snapshot: &Snapshot) -> [Option<Layout>; 2] {
    let Some(split) = &snapshot.split else {
        return [Some(base), None];
    };
    crate::panes::geometry(
        base.cols,
        base.rows,
        split.axis,
        split.ratio,
        split.zoomed,
        split.focused,
    )
    .map(|rect| {
        rect.map(|rect| Layout {
            terminal_x: base.terminal_x + rect.x,
            terminal_y: base.terminal_y + rect.y,
            cols: rect.cols,
            rows: rect.rows,
            ..base
        })
    })
}

pub(super) fn terminal_layout(base: Layout, snapshot: &Snapshot) -> Layout {
    let group = snapshot.split.as_ref().map_or(0, |s| s.focused as usize);
    layouts(base, snapshot)[group].unwrap_or(base)
}

impl Ui {
    pub(super) fn editor_open_at(
        &mut self,
        origin: &EditorOrigin,
        path: &Path,
        line: usize,
        column: usize,
    ) -> bool {
        if !self.pane_capable {
            self.notice = "Update Flere to enable exact file/line jumps".into();
            return false;
        }
        self.command(&[
            "open-at-pane-epoch",
            &origin.epoch,
            &origin.workspace.to_string(),
            &origin.tab.to_string(),
            &origin.run,
            &origin.revision.to_string(),
            &wire::hex(path.to_string_lossy().as_bytes()),
            &line.to_string(),
            &column.to_string(),
        ]);
        self.notice.is_empty()
    }
    pub(super) fn editor_origin(&self) -> EditorOrigin {
        EditorOrigin {
            epoch: self.snapshot.epoch.clone(),
            workspace: self.snapshot.active,
            tab: self.snapshot.tab,
            run: self
                .snapshot
                .session()
                .map(|t| t.run.clone())
                .unwrap_or_default(),
            revision: self.pane_revision(),
        }
    }

    pub(super) fn editor_open(
        &mut self,
        origin: &EditorOrigin,
        path: &Path,
        before: Option<&Path>,
    ) -> bool {
        let operation = match (self.pane_capable, before.is_some()) {
            (true, false) => "open-pane-epoch",
            (true, true) => "open-diff-pane-epoch",
            (false, false) => "open-epoch",
            (false, true) => "open-diff-epoch",
        };
        let mut fields = vec![
            operation.into(),
            origin.epoch.clone(),
            origin.workspace.to_string(),
        ];
        if self.pane_capable {
            fields.extend([
                origin.tab.to_string(),
                origin.run.clone(),
                origin.revision.to_string(),
            ]);
        }
        if let Some(before) = before {
            fields.push(wire::hex(before.to_string_lossy().as_bytes()));
        }
        fields.push(wire::hex(path.to_string_lossy().as_bytes()));
        self.command(&fields.iter().map(String::as_str).collect::<Vec<_>>());
        self.notice.is_empty()
    }

    pub(super) fn read_snapshot(&self) -> io::Result<Snapshot> {
        Snapshot::decode(&wire::request(
            &self.state,
            &[hyperlinks::snapshot_command(
                self.pane_capable,
                self.link_capable,
            )],
        )?)
    }

    pub(super) fn terminal_layout(&self) -> Layout {
        terminal_layout(self.layout, &self.snapshot)
    }

    pub(super) fn pane_layouts(&self) -> [Option<Layout>; 2] {
        layouts(self.layout, &self.snapshot)
    }

    pub(super) fn pane_group(&self) -> usize {
        self.snapshot
            .split
            .as_ref()
            .map_or(0, |s| s.focused as usize)
    }

    pub(super) fn pane_revision(&self) -> u64 {
        self.snapshot.split.as_ref().map_or(0, |s| s.revision)
    }

    pub(super) fn pane_tabs(&self, group: usize) -> Vec<&TabView> {
        let Some(workspace) = self.snapshot.workspace() else {
            return Vec::new();
        };
        if let Some(split) = &self.snapshot.split {
            split.groups[group]
                .tabs
                .iter()
                .filter_map(|id| workspace.tabs.iter().find(|t| t.id == *id))
                .collect()
        } else {
            workspace.tabs.iter().collect()
        }
    }

    pub(super) fn pane_selected(&self, group: usize) -> u64 {
        self.snapshot
            .split
            .as_ref()
            .map_or(self.snapshot.tab, |s| s.groups[group].selected)
    }

    pub(super) fn resize_viewport(&mut self) {
        let cols = self.layout.cols.to_string();
        let rows = self.layout.rows.to_string();
        if self.pane_capable {
            self.command(&[
                "resize-panes",
                &self.snapshot.epoch.clone(),
                &self.snapshot.active.to_string(),
                &cols,
                &rows,
            ]);
        } else {
            self.command(&["resize", &cols, &rows]);
        }
    }

    pub(super) fn pane_action(&mut self, operation: &str, argument: Option<&str>) -> bool {
        self.menu = false;
        if !self.pane_capable {
            self.notice = "Refresh Flere to use split panes".into();
            return false;
        }
        let Some(tab) = self.snapshot.session() else {
            self.notice = "Open a terminal before splitting the workspace".into();
            return false;
        };
        let mut fields = vec![
            "pane".to_string(),
            self.snapshot.epoch.clone(),
            self.snapshot.active.to_string(),
            tab.id.to_string(),
            tab.run.clone(),
            self.pane_revision().to_string(),
            operation.into(),
        ];
        if let Some(argument) = argument {
            fields.push(argument.into());
        }
        self.flush_input();
        let result = wire::request(
            &self.state,
            &fields.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .and_then(|reply| {
            let revision = std::str::from_utf8(&reply)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| io::Error::other("Invalid pane operation receipt"))?;
            let snapshot = self.read_snapshot()?;
            self.snapshot(snapshot);
            if self.pane_revision() != revision
                || self.snapshot.epoch != fields[1]
                || self.snapshot.active.to_string() != fields[2]
            {
                return Err(io::Error::other(
                    "Pane changed in another attachment; select again",
                ));
            }
            Ok(())
        });
        let success = result.is_ok();
        self.notice = result
            .err()
            .map_or_else(String::new, |e| wire::passive(&e.to_string()));
        if success {
            self.focus = Focus::Terminal;
            self.selection = None;
            self.smooth_scroll = None;
        }
        success
    }

    pub(super) fn pane_focus(&mut self, group: usize) -> bool {
        if group == self.pane_group() {
            return true;
        }
        let origin = (self.snapshot.epoch.clone(), self.snapshot.active);
        let Some(before) = self.snapshot.split.clone() else {
            return false;
        };
        let destination = before.groups[group].selected;
        let run = self.snapshot.workspace().and_then(|w| {
            w.tabs
                .iter()
                .find(|t| t.id == destination)
                .map(|t| t.run.clone())
        });
        let visible_size =
            self.pane_layouts()[group].map(|_| (before.other.cols, before.other.rows));
        if !self.pane_action("focus", Some(&group.to_string())) {
            return false;
        }
        let matched = self.pane_group() == group
            && origin == (self.snapshot.epoch.clone(), self.snapshot.active)
            && self.snapshot.tab == destination
            && self.snapshot.session().map(|t| &t.run) == run.as_ref()
            && self.snapshot.split.as_ref().is_some_and(|after| {
                (after.axis, after.ratio, after.zoomed, &after.groups)
                    == (before.axis, before.ratio, before.zoomed, &before.groups)
            })
            && visible_size.is_none_or(|size| size == (self.snapshot.cols, self.snapshot.rows));
        if !matched {
            self.notice = "Pane changed in another attachment; select again".into();
        }
        matched
    }

    pub(super) fn pane_nav_key(&mut self, key: &[u8]) -> bool {
        if self.focus != Focus::Terminal {
            return false;
        }
        let Some(split) = &self.snapshot.split else {
            return false;
        };
        let direction = match (split.axis, key) {
            (SplitAxis::Right, b"h" | b"\x1b[D" | b"\x1bOD") => -1,
            (SplitAxis::Right, b"l" | b"\x1b[C" | b"\x1bOC") => 1,
            (SplitAxis::Below, b"k" | b"\x1b[A" | b"\x1bOA") => -1,
            (SplitAxis::Below, b"j" | b"\x1b[B" | b"\x1bOB") => 1,
            _ => return false,
        };
        let next = split.focused as isize + direction;
        if !(0..=1).contains(&next) {
            return false;
        }
        self.pane_focus(next as usize);
        true
    }

    pub(super) fn pane_at(&self, x: usize, y: usize, tabs: bool) -> Option<usize> {
        self.pane_layouts()
            .iter()
            .enumerate()
            .find_map(|(group, layout)| {
                let l = layout.as_ref()?;
                (x >= l.terminal_x
                    && x < l.terminal_x + l.cols
                    && y >= l.terminal_y.saturating_sub(if tabs { 2 } else { 0 })
                    && y < l.terminal_y + l.rows)
                    .then_some(group)
            })
    }

    pub(super) fn pane_tab_at(&self, x: usize, y: usize) -> Option<u64> {
        self.pane_layouts()
            .iter()
            .enumerate()
            .find_map(|(group, layout)| {
                let l = layout.as_ref()?;
                if y != l.terminal_y - 2 {
                    return None;
                }
                self.pane_tab_slots(group, *l)
                    .into_iter()
                    .find_map(|(start, width, id, _)| {
                        (x >= start && x < start + width).then_some(id)
                    })
            })
    }

    pub(super) fn pane_divider(&self) -> Option<(SplitAxis, usize)> {
        let split = self.snapshot.split.as_ref()?;
        let [Some(first), Some(_)] = self.pane_layouts() else {
            return None;
        };
        Some((
            split.axis,
            match split.axis {
                SplitAxis::Right => first.terminal_x + first.cols,
                SplitAxis::Below => first.terminal_y + first.rows,
            },
        ))
    }

    pub(super) fn pane_divider_start(&mut self, x: usize, y: usize) -> bool {
        let Some((axis, position)) = self.pane_divider() else {
            return false;
        };
        let l = self.layout;
        let hit = match axis {
            SplitAxis::Right => x == position && y >= 2 && y < l.terminal_y + l.rows,
            SplitAxis::Below => y == position && x >= l.terminal_x && x < l.terminal_x + l.cols,
        };
        if hit && let Some(tab) = self.snapshot.session() {
            self.pane_drag = Some(DividerDrag {
                epoch: self.snapshot.epoch.clone(),
                workspace: self.snapshot.active,
                tab: tab.id,
                run: tab.run.clone(),
                revision: self.pane_revision(),
                layout: l,
            });
            self.selection = None;
            self.smooth_scroll = None;
        }
        hit
    }

    pub(super) fn pane_divider_drag(&mut self, x: usize, y: usize) -> bool {
        let Some(drag) = self.pane_drag.take() else {
            return false;
        };
        let valid = drag.epoch == self.snapshot.epoch
            && drag.workspace == self.snapshot.active
            && drag.tab == self.snapshot.tab
            && drag.layout == self.layout
            && drag.revision == self.pane_revision()
            && self.snapshot.session().is_some_and(|t| t.run == drag.run);
        if !valid {
            return true;
        }
        let Some(split) = &self.snapshot.split else {
            return true;
        };
        let (offset, total) = match split.axis {
            SplitAxis::Right => (
                x.saturating_sub(self.layout.terminal_x),
                self.layout.cols.saturating_sub(1),
            ),
            SplitAxis::Below => (
                y.saturating_sub(self.layout.terminal_y),
                self.layout.rows.saturating_sub(3),
            ),
        };
        let ratio = (offset * 1000 / total.max(1)).clamp(200, 800) as u16;
        if ratio == split.ratio || self.pane_action("ratio", Some(&ratio.to_string())) {
            self.pane_drag = Some(DividerDrag {
                revision: self.pane_revision(),
                ..drag
            });
        }
        true
    }

    pub(super) fn draw_panes(&self, c: &mut Canvas) {
        let current = self.pane_group();
        for (group, layout) in self.pane_layouts().into_iter().enumerate() {
            let Some(l) = layout else {
                continue;
            };
            let y = l.terminal_y - 2;
            c.fill(l.terminal_x, y, l.cols, 1, style(MUTED, PANEL, false));
            c.text(
                l.terminal_x,
                y + 1,
                l.cols,
                &"─".repeat(l.cols),
                style(BORDER, BG, false),
            );
            let slots = self.pane_tab_slots(group, l);
            let new_tab_x = slots
                .last()
                .map_or(l.terminal_x + 1, |(x, width, _, _)| x + width + 1);
            for (x, width, id, text) in slots {
                let active = id == self.pane_selected(group);
                let accent = if group == current { CYAN } else { MUTED };
                let color = if active {
                    accent
                } else if self
                    .snapshot
                    .workspace()
                    .is_some_and(|w| w.tabs.iter().any(|t| t.id == id && t.working))
                {
                    WORKING
                } else {
                    MUTED
                };
                if active {
                    c.text(
                        x,
                        y + 1,
                        width,
                        &"━".repeat(width),
                        style(accent, BG, group == current),
                    );
                }
                c.text(x, y, width, &text, style(color, BG, false));
            }
            c.text(
                new_tab_x,
                y,
                3.min((l.terminal_x + l.cols).saturating_sub(new_tab_x)),
                " + ",
                style(MUTED, PANEL, false),
            );
            if group != current
                && let Some(split) = &self.snapshot.split
            {
                let other = &split.other;
                c.fill(
                    l.terminal_x,
                    l.terminal_y,
                    l.cols,
                    l.rows,
                    style(
                        crate::terminal::DEFAULT_FG,
                        crate::terminal::DEFAULT_BG,
                        false,
                    ),
                );
                for yy in 0..l.rows.min(other.rows) {
                    for xx in 0..l.cols.min(other.cols) {
                        let mut cell = other.cells[yy * other.cols + xx].clone();
                        if cell.style.fg == Color::Default {
                            cell.style.fg = crate::terminal::DEFAULT_FG;
                        }
                        if cell.style.bg == Color::Default {
                            cell.style.bg = crate::terminal::DEFAULT_BG;
                        }
                        c.cells[(l.terminal_y + yy) * c.width + l.terminal_x + xx] = cell;
                    }
                }
                if let Some(tab) = self
                    .snapshot
                    .workspace()
                    .and_then(|w| w.tabs.iter().find(|t| t.id == other.tab))
                    && let Some(view) =
                        self.parked_scrollback
                            .get(&(self.snapshot.active, tab.id, tab.run.clone()))
                    && view.matches_at(
                        &self.snapshot.epoch,
                        self.snapshot.active,
                        tab.id,
                        &tab.run,
                        l,
                        (other.cols, other.rows),
                    )
                {
                    view.paint(c);
                }
                if let Some(preview) = self.previews.get(&(self.snapshot.active, other.tab)) {
                    self.paint_preview(c, l, preview);
                }
            }
        }
        if let Some((axis, position)) = self.pane_divider() {
            match axis {
                SplitAxis::Right => {
                    for y in 2..self.layout.terminal_y + self.layout.rows {
                        c.text(position, y, 1, "│", style(BORDER, BG, false));
                    }
                }
                SplitAxis::Below => c.text(
                    self.layout.terminal_x,
                    position,
                    self.layout.cols,
                    &"─".repeat(self.layout.cols),
                    style(BORDER, BG, false),
                ),
            }
        }
        self.paint_buffer(c);
    }
}
