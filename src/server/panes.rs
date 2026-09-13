//! Shared workspace pane groups and their exact terminal dimensions.
use super::*;
use crate::{
    model::{PaneBuffer, SplitView},
    panes::{PaneGroup, PaneLayout, SplitAxis, geometry},
};

impl Server {
    pub(super) fn validate_pane_origin(
        &self,
        epoch: &str,
        wid: u64,
        tid: u64,
        run: &str,
        revision: u64,
        allow_empty: bool,
    ) -> io::Result<()> {
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == wid && !w.meta.archived)
            .ok_or_else(|| invalid("workspace unavailable"))?;
        let identity = w.tabs.iter().any(|t| t.id == tid && t.run == run)
            || (allow_empty && tid == 0 && run.is_empty() && w.tabs.is_empty());
        if epoch != self.epoch
            || self.active != wid
            || w.selected != tid
            || !identity
            || self.pane_revision(wid) != revision
        {
            return Err(invalid("stale pane workspace, session, run or revision"));
        }
        Ok(())
    }
    pub(super) fn pane_tab_capacity(&self, wid: u64) -> io::Result<()> {
        if let Some(w) = self.workspaces.iter().find(|w| w.id == wid)
            && (w.split.is_some() || self.restoration.panes.contains_key(&wid))
            && w.tabs.len()
                + self
                    .restoration
                    .pending
                    .iter()
                    .filter(|p| p.workspace == wid)
                    .count()
                >= 512
            && !self.restoration.busy
        {
            return Err(invalid("split workspace is limited to 512 saved tabs"));
        }
        Ok(())
    }
    pub(super) fn pane_revision(&self, wid: u64) -> u64 {
        self.workspaces
            .iter()
            .find(|w| w.id == wid)
            .and_then(|w| w.split.as_ref())
            .map_or(0, |s| s.revision)
    }
    pub(super) fn split_view(&self, w: &Workspace) -> Option<SplitView> {
        let split = w.split.as_ref()?;
        let other = w
            .tabs
            .iter()
            .find(|t| t.id == split.groups[usize::from(1 - split.focused)].selected)?;
        Some(SplitView {
            axis: split.axis,
            ratio: split.ratio,
            zoomed: split.zoomed,
            focused: split.focused,
            revision: split.revision,
            groups: split.groups.clone(),
            other: PaneBuffer {
                tab: other.id,
                cols: other.term.grid.cols,
                rows: other.term.grid.rows,
                x: other.term.grid.x,
                y: other.term.grid.y,
                cursor: other.alive && other.term.cursor,
                bracketed_paste: other.term.bracketed_paste,
                app_cursor: other.term.app_cursor,
                cells: other.term.grid.cells.clone(),
            },
        })
    }
    pub(super) fn saved_split(&self, w: &Workspace) -> Option<PaneLayout> {
        let mut saved = if let Some(split) = &w.split {
            let mut saved = split.clone();
            for group in &mut saved.groups {
                for id in &mut group.tabs {
                    *id = self.tab_order(w.tabs.iter().find(|t| t.id == *id)?);
                }
                group.selected = w
                    .tabs
                    .iter()
                    .find(|t| t.id == group.selected)
                    .map_or(0, |t| self.tab_order(t));
            }
            if let Some(template) = self.restoration.panes.get(&w.id) {
                let mapped = saved
                    .groups
                    .iter()
                    .flat_map(|g| g.tabs.iter().copied())
                    .collect::<Vec<_>>();
                for (group, old) in saved.groups.iter_mut().zip(&template.groups) {
                    // Keep missing saved entries beside their original neighbors,
                    // while accepting explicit live-tab moves and new tab order.
                    for (position, id) in old.tabs.iter().enumerate().rev() {
                        if !mapped.contains(id) {
                            let before = old.tabs[position + 1..]
                                .iter()
                                .find_map(|next| group.tabs.iter().position(|id| id == next))
                                .unwrap_or(group.tabs.len());
                            group.tabs.insert(before, *id);
                        }
                    }
                    if !mapped.contains(&old.selected) {
                        group.selected = old.selected;
                    }
                }
            }
            saved
        } else {
            self.restoration.panes.get(&w.id)?.clone()
        };
        // Failed restores retain their stable identity in the saved group. New
        // tabs join its focused group; explicitly closed tabs leave the layout.
        let ids: Vec<_> = self.saved_tabs(w).iter().map(|t| t.order).collect();
        for group in &mut saved.groups {
            group.tabs.retain(|id| ids.contains(id));
        }
        for id in &ids {
            if !saved.groups.iter().any(|g| g.tabs.contains(id)) {
                saved.groups[usize::from(saved.focused)].tabs.push(*id);
            }
        }
        for group in &mut saved.groups {
            if !group.tabs.contains(&group.selected) {
                group.selected = *group.tabs.first()?;
            }
        }
        saved.validate(&ids).ok()?;
        Some(saved)
    }
    pub(super) fn restore_panes(&mut self, wid: u64) {
        if !self.restoration.panes.contains_key(&wid) {
            return;
        }
        let Some(w) = self.workspaces.iter().find(|w| w.id == wid) else {
            return;
        };
        let Some(saved) = self.saved_split(w) else {
            return;
        };
        let mut split = saved.clone();
        let mut complete = true;
        for group in &mut split.groups {
            group.tabs = group
                .tabs
                .iter()
                .filter_map(|order| {
                    let tab = w.tabs.iter().find(|t| self.tab_order(t) == *order);
                    complete &= tab.is_some();
                    tab.map(|t| t.id)
                })
                .collect();
            if group.tabs.is_empty() {
                return;
            }
            group.selected = w
                .tabs
                .iter()
                .find(|t| self.tab_order(t) == group.selected)
                .map_or(group.tabs[0], |t| t.id);
        }
        for t in &w.tabs {
            if !split.groups.iter().any(|g| g.tabs.contains(&t.id)) {
                split.groups[usize::from(split.focused)].tabs.push(t.id);
            }
        }
        let w = self.workspaces.iter_mut().find(|w| w.id == wid).unwrap();
        w.selected = split.groups[usize::from(split.focused)].selected;
        split.revision = self.generation.saturating_add(1).max(1);
        w.split = Some(split);
        if complete {
            self.restoration.panes.remove(&wid);
        } else {
            self.restoration.panes.insert(wid, saved);
        }
    }
    pub(super) fn record_pane_selection(&mut self) {
        self.record_workspace_pane_selection(self.active);
    }
    pub(super) fn record_workspace_pane_selection(&mut self, wid: u64) {
        if self.restoration.busy {
            return;
        }
        let Some(w) = self.workspaces.iter().find(|w| w.id == wid) else {
            return;
        };
        let Some(t) = w.tabs.iter().find(|t| t.id == w.selected) else {
            return;
        };
        let order = self.tab_order(t);
        if let Some(template) = self.restoration.panes.get_mut(&w.id)
            && let Some(index) = template.groups.iter().position(|g| g.tabs.contains(&order))
        {
            template.focused = index as u8;
            template.groups[index].selected = order;
        }
    }
    fn remember_pane_order(&mut self, wid: u64, ids: &[u64]) {
        if let Some(w) = self.workspaces.iter().find(|w| w.id == wid) {
            let orders = ids
                .iter()
                .filter_map(|id| {
                    w.tabs
                        .iter()
                        .find(|t| t.id == *id)
                        .map(|t| self.tab_order(t))
                })
                .collect();
            self.restoration.sequence.insert(wid, orders);
        }
        if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == wid) {
            w.tabs
                .sort_by_key(|t| ids.iter().position(|id| *id == t.id).unwrap_or(usize::MAX));
        }
    }
    pub(super) fn sync_panes(&mut self) {
        if self.restoration.busy {
            return;
        }
        let mut merged = Vec::new();
        for w in &mut self.workspaces {
            let Some(split) = &mut w.split else {
                continue;
            };
            let before = split.clone();
            let ids: Vec<_> = w.tabs.iter().map(|t| t.id).collect();
            for group in &mut split.groups {
                group.tabs.retain(|id| ids.contains(id));
                if !group.tabs.contains(&group.selected) {
                    group.selected = group.tabs.first().copied().unwrap_or(0);
                }
            }
            for id in ids {
                if !split.groups.iter().any(|g| g.tabs.contains(&id)) {
                    split.groups[usize::from(split.focused)].tabs.push(id);
                }
            }
            if let Some(index) = split
                .groups
                .iter()
                .position(|g| g.tabs.contains(&w.selected))
            {
                split.focused = index as u8;
                split.groups[index].selected = w.selected;
            }
            if split.groups.iter().any(|g| g.tabs.is_empty()) {
                let ids = split
                    .groups
                    .iter()
                    .flat_map(|g| g.tabs.iter().copied())
                    .collect::<Vec<_>>();
                if !ids.contains(&w.selected) {
                    w.selected = ids.first().copied().unwrap_or(0);
                }
                merged.push((w.id, ids));
            } else {
                w.selected = split.groups[usize::from(split.focused)].selected;
                if *split != before {
                    split.revision = self
                        .generation
                        .saturating_add(1)
                        .max(split.revision.saturating_add(1));
                }
            }
        }
        for (wid, ids) in merged {
            let saved = self
                .workspaces
                .iter()
                .find(|w| w.id == wid)
                .and_then(|w| self.saved_split(w));
            if let Some(saved) = saved {
                self.restoration.panes.insert(wid, saved);
            } else {
                self.restoration.panes.remove(&wid);
            }
            self.workspaces
                .iter_mut()
                .find(|w| w.id == wid)
                .unwrap()
                .split = None;
            self.remember_pane_order(wid, &ids);
        }
        if !self.restoration.busy {
            let pending = self
                .workspaces
                .iter()
                .filter(|w| self.restoration.panes.contains_key(&w.id))
                .map(|w| (w.id, self.saved_split(w)))
                .collect::<Vec<_>>();
            for (wid, split) in pending {
                if let Some(split) = split {
                    self.restoration.panes.insert(wid, split);
                } else {
                    self.restoration.panes.remove(&wid);
                }
            }
        }
    }
    pub(super) fn pane_size(&self, wid: u64) -> (u16, u16) {
        self.workspaces
            .iter()
            .find(|w| w.id == wid)
            .and_then(|w| w.split.as_ref())
            .and_then(|split| {
                geometry(
                    self.cols as usize,
                    self.rows as usize,
                    split.axis,
                    split.ratio,
                    split.zoomed,
                    split.focused,
                )[usize::from(split.focused)]
            })
            .map_or((self.cols, self.rows), |r| (r.cols as u16, r.rows as u16))
    }
    pub(super) fn restore_size(&self, wid: u64, order: u64) -> (u16, u16) {
        self.restoration
            .panes
            .get(&wid)
            .and_then(|split| {
                let group = split.groups.iter().position(|g| g.tabs.contains(&order))?;
                geometry(
                    self.cols as usize,
                    self.rows as usize,
                    split.axis,
                    split.ratio,
                    split.zoomed,
                    split.focused,
                )[group]
            })
            .map_or_else(|| self.pane_size(wid), |r| (r.cols as u16, r.rows as u16))
    }
    pub(super) fn resize_sessions(&mut self) -> io::Result<()> {
        for w in &mut self.workspaces {
            let rectangles = w.split.as_ref().map(|split| {
                geometry(
                    self.cols as usize,
                    self.rows as usize,
                    split.axis,
                    split.ratio,
                    split.zoomed,
                    split.focused,
                )
            });
            for t in &mut w.tabs {
                let size = if let (Some(split), Some(rectangles)) = (&w.split, rectangles) {
                    split
                        .groups
                        .iter()
                        .position(|g| g.tabs.contains(&t.id))
                        .and_then(|i| rectangles[i])
                        .map(|r| (r.cols, r.rows))
                } else {
                    Some((self.cols as usize, self.rows as usize))
                };
                // A hidden pane retains its last useful geometry and its live process.
                if let Some((cols, rows)) = size
                    && (t.term.grid.cols, t.term.grid.rows) != (cols, rows)
                {
                    if t.alive {
                        os::resize(t.master.as_raw_fd(), cols as u16, rows as u16)?;
                    }
                    t.term.resize(cols, rows);
                }
            }
        }
        Ok(())
    }
    pub(super) fn pane_command(&mut self, p: &[&str]) -> io::Result<Vec<u8>> {
        if !(7..=8).contains(&p.len()) {
            return Err(invalid("invalid pane command arguments"));
        }
        let number = |i: usize| {
            p[i].parse::<u64>()
                .map_err(|_| invalid("invalid pane identity or value"))
        };
        let wid = number(2)?;
        let tid = number(3)?;
        let revision = number(5)?;
        self.validate_pane_origin(p[1], wid, tid, p[4], revision, false)?;
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == wid && !w.meta.archived)
            .ok_or_else(|| invalid("workspace unavailable"))?;
        let op = p[6];
        let arg = p.get(7).copied();
        let next_revision = self
            .generation
            .saturating_add(1)
            .max(revision.saturating_add(1));
        let merged_orders = (op == "merge")
            .then(|| self.saved_split(w))
            .flatten()
            .map(|s| {
                s.groups
                    .into_iter()
                    .flat_map(|g| g.tabs)
                    .collect::<Vec<_>>()
            });
        match op {
            "new-tab" if arg.is_none() => {
                self.audit("start-shell-request", wid, "pane")?;
                self.spawn(wid)?;
            }
            "split-right" | "split-below" => {
                if w.split.is_some() || self.restoration.panes.contains_key(&wid) {
                    return Err(invalid("workspace already has two pane groups"));
                }
                if !matches!(arg, Some("move" | "new")) {
                    return Err(invalid("split requires move or new"));
                }
                if w.tabs.len()
                    + self
                        .restoration
                        .pending
                        .iter()
                        .filter(|p| p.workspace == wid)
                        .count()
                    + usize::from(arg == Some("new"))
                    > 512
                {
                    return Err(invalid("split workspace is limited to 512 saved tabs"));
                }
                if arg == Some("move") && w.tabs.len() < 2 {
                    return Err(invalid("split move needs another tab; choose a new shell"));
                }
                let mut first = w.tabs.iter().map(|t| t.id).collect::<Vec<_>>();
                let moved = if arg == Some("new") {
                    self.spawn(wid)?
                } else {
                    first.retain(|id| *id != tid);
                    tid
                };
                let first_selected = if first.contains(&tid) { tid } else { first[0] };
                let w = self.workspaces.iter_mut().find(|w| w.id == wid).unwrap();
                w.split = Some(PaneLayout {
                    axis: if op == "split-right" {
                        SplitAxis::Right
                    } else {
                        SplitAxis::Below
                    },
                    ratio: 500,
                    zoomed: false,
                    focused: 1,
                    revision: next_revision,
                    groups: [
                        PaneGroup {
                            tabs: first,
                            selected: first_selected,
                        },
                        PaneGroup {
                            tabs: vec![moved],
                            selected: moved,
                        },
                    ],
                });
                w.selected = moved;
            }
            "focus" | "move" | "ratio" | "zoom" | "merge" => {
                if w.split.is_none() {
                    return Err(invalid("workspace is not split"));
                }
                match op {
                    "focus" if arg.is_some_and(|a| matches!(a, "0" | "1")) => {}
                    "ratio"
                        if arg.is_some_and(|a| {
                            a.parse::<u16>().is_ok_and(|n| (200..=800).contains(&n))
                        }) => {}
                    "move" | "zoom" | "merge" if arg.is_none() => {}
                    _ => return Err(invalid("invalid pane operation value")),
                }
                let w = self.workspaces.iter_mut().find(|w| w.id == wid).unwrap();
                let split = w.split.as_mut().unwrap();
                let from = usize::from(split.focused);
                match op {
                    "focus" => {
                        split.focused = arg.unwrap().parse().unwrap();
                        w.selected = split.groups[usize::from(split.focused)].selected;
                    }
                    "move" => {
                        split.groups[from].tabs.retain(|id| *id != tid);
                        split.groups[from].selected =
                            split.groups[from].tabs.first().copied().unwrap_or(0);
                        split.groups[1 - from].tabs.push(tid);
                        split.groups[1 - from].selected = tid;
                        split.focused = (1 - from) as u8;
                    }
                    "ratio" => split.ratio = arg.unwrap().parse().unwrap(),
                    "zoom" => split.zoomed = !split.zoomed,
                    "merge" => {
                        let ids = split
                            .groups
                            .iter()
                            .flat_map(|g| g.tabs.iter().copied())
                            .collect::<Vec<_>>();
                        w.split = None;
                        self.remember_pane_order(wid, &ids);
                        self.restoration.panes.remove(&wid);
                        if let Some(orders) = merged_orders {
                            self.restoration.sequence.insert(wid, orders);
                        }
                    }
                    _ => unreachable!(),
                }
                if let Some(split) = self
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == wid)
                    .and_then(|w| w.split.as_mut())
                {
                    split.revision = next_revision;
                }
            }
            _ => return Err(invalid("unknown pane operation")),
        }
        self.audit("pane", wid, op)?;
        self.changed();
        Ok(self.pane_revision(wid).to_string().into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(Server);
    impl Drop for Fixture {
        fn drop(&mut self) {
            for tab in self.0.workspaces.iter_mut().flat_map(|w| &mut w.tabs) {
                let _ = tab.child.kill();
                drop(std::mem::replace(
                    &mut tab.master,
                    File::open("/dev/null").unwrap(),
                ));
                let _ = tab.child.wait();
            }
        }
    }
    fn pane(server: &mut Server, op: &str, arg: Option<&str>) {
        let snapshot = server.snapshot();
        let tab = snapshot.session().unwrap();
        let mut request = format!(
            "pane\t{}\t{}\t{}\t{}\t{}\t{op}",
            snapshot.epoch,
            snapshot.active,
            tab.id,
            tab.run,
            server.pane_revision(snapshot.active)
        );
        if let Some(arg) = arg {
            request.push('\t');
            request.push_str(arg);
        }
        server.command(&request).unwrap();
    }
    #[test]
    fn retained_stopped_origins_allow_view_shell_and_editor_actions_but_not_input() {
        // Run this fixture in its own test process so shell/editor HOME and
        // configuration are private, without changing global environment state.
        let Some(root) = std::env::var_os("FLERE_STOPPED_ORIGIN_FIXTURE") else {
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!("stp-{}", &os::nonce().unwrap()[..12]));
            fs::create_dir_all(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "server::panes::tests::retained_stopped_origins_allow_view_shell_and_editor_actions_but_not_input", "--nocapture"])
                .env("FLERE_STOPPED_ORIGIN_FIXTURE", &root).env("HOME", &root).env("SHELL", "/bin/sh")
                .env("ENV", "").env("VISUAL", "/usr/bin/vim -Nu NONE -n --noplugin").env("TMPDIR", &root)
                .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while child.try_wait().unwrap().is_none() {
                if Instant::now() >= deadline {
                    child.kill().unwrap();
                    let _ = child.wait();
                    panic!(
                        "stopped-origin fixture exceeded its ten-second bound: {}",
                        root.display()
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            fs::remove_dir_all(root).unwrap();
            return;
        };
        let root = PathBuf::from(root);
        let state = root.join("s");
        private_state(&state).unwrap();
        let mut fixture = Fixture(Server::load(&state).unwrap());
        let server = &mut fixture.0;
        server
            .create_workspace(
                "stopped origin".into(),
                root.clone(),
                CardMeta::default(),
                true,
            )
            .unwrap();
        let wid = server.active;
        let first = server.snapshot().tab;
        let stopped = server.spawn(wid).unwrap();
        pane(server, "split-right", Some("move"));
        let run = server.snapshot().session().unwrap().run.clone();
        // Freeze the legitimate interval between reaping a child and pruning its
        // retained terminal record; platform PTY hangup timing cannot race it.
        let session = server.session(stopped, &run).unwrap();
        session.child.kill().unwrap();
        drop(std::mem::replace(
            &mut session.master,
            File::open("/dev/null").unwrap(),
        ));
        session.child.wait().unwrap();
        session.ended = true;
        session.alive = false;
        let reject_input = format!("input\t{stopped}\t{run}\t{}", wire::hex(b"MUST_NOT_SEND"));
        assert!(
            server
                .command(&reject_input)
                .unwrap_err()
                .to_string()
                .contains("exited")
        );
        pane(server, "new-tab", None);
        let new_shell = server.snapshot().tab;
        assert_ne!(new_shell, stopped);
        assert_eq!(server.snapshot().session().unwrap().kind, "shell");
        let focus_stopped = format!("focus-exact\t{}\t{wid}\t{stopped}\t{run}", server.epoch);
        server.command(&focus_stopped).unwrap();
        pane(server, "focus", Some("0"));
        assert_eq!(server.snapshot().tab, first);
        server.command(&focus_stopped).unwrap();
        pane(server, "merge", None);
        assert!(server.snapshot().split.is_none());
        assert_eq!(server.snapshot().tab, stopped);
        pane(server, "split-below", Some("move"));
        let file = root.join("stopped-origin.txt");
        fs::write(&file, "private editor fixture\n").unwrap();
        server
            .command(&format!(
                "open-pane-epoch\t{}\t{wid}\t{stopped}\t{run}\t{}\t{}",
                server.epoch,
                server.pane_revision(wid),
                wire::hex(file.to_str().unwrap().as_bytes())
            ))
            .unwrap();
        let opened = server.snapshot();
        assert_eq!(opened.session().unwrap().kind, "editor");
        assert!(opened.split.unwrap().groups[1].tabs.contains(&opened.tab));
        assert!(
            server
                .command(&reject_input)
                .unwrap_err()
                .to_string()
                .contains("exited")
        );
        assert!(server.session(stopped, "wrong-run").is_err());
    }
}
