//! Git inspector presentation and bounded asynchronous history actions.
use super::*;
use crate::git::history::{self, Details, FileChange, History};
use std::collections::HashSet;
mod tree;
use tree::{Entry, Group};

#[derive(Default)]
pub(super) struct Pane {
    key: String,
    viewport: viewport::Viewport,
    changes_closed: bool,
    commits_closed: bool,
    branch_closed: bool,
    closed: HashSet<String>,
    requested: bool,
    history: Option<History>,
    expanded: Option<Details>,
    peek: Option<Details>,
    error: String,
}
#[derive(Clone)]
enum Action {
    Page(Option<Vec<String>>, usize),
    Details(String, bool),
    Message(String),
    CommitDiff(String),
    Diff(Details, Option<FileChange>, bool),
    Working(crate::git::Change, bool),
}
#[derive(Clone)]
pub(super) struct Request {
    epoch: String,
    workspace: u64,
    tab: u64,
    run: String,
    revision: u64,
    cwd: String,
    root: PathBuf,
    key: String,
    action: Action,
}
enum Payload {
    Page(History),
    Details(Details, bool),
    Message(Details),
    Diff(String, Option<crate::editor::DiffDocument>),
    Comparison(crate::editor::ManagedComparison),
}
pub(super) struct Result {
    request: Request,
    result: io::Result<Payload>,
}
#[derive(Clone)]
enum Kind {
    Changes,
    Group(Group),
    Folder(String),
    Branch,
    BranchFile(usize),
    Commits,
    Change(usize, Group),
    Commit(String),
    File(String, usize),
    All(String),
    Previous,
    Next,
    Inert,
}
struct Row {
    key: String,
    text: String,
    graph: String,
    status: Option<(usize, String)>,
    kind: Kind,
}
pub(super) struct ContextRow {
    pub key: String,
    pub text: String,
    pub preview: bool,
    pub sha: Option<String>,
}
impl Row {
    fn new(key: impl Into<String>, text: impl Into<String>, kind: Kind) -> Self {
        Self {
            key: key.into(),
            text: text.into(),
            graph: String::new(),
            status: None,
            kind,
        }
    }
    fn with_graph(mut self, graph: String) -> Self {
        self.graph = graph;
        self
    }
    fn selectable(&self) -> bool {
        !matches!(self.kind, Kind::Inert)
    }
    fn sha(&self) -> Option<&str> {
        match &self.kind {
            Kind::Commit(s) | Kind::File(s, _) | Kind::All(s) => Some(s),
            _ => None,
        }
    }
}
impl Pane {
    fn rows(&self, view: Option<&crate::git::GitView>) -> Vec<Row> {
        let mut rows = vec![Row::new(
            "changes",
            format!(
                "{} Working tree · {}",
                if self.changes_closed { "▸" } else { "▾" },
                view.map_or(0, |v| v.changes.len())
            ),
            Kind::Changes,
        )];
        if !self.changes_closed
            && let Some(view) = view
        {
            if !view.error.is_empty() {
                rows.push(Row::new("", &view.error, Kind::Inert));
            } else if view.changes.is_empty() {
                rows.push(Row::new("", "  Working tree clean", Kind::Inert));
            }
            for group in Group::ALL {
                let entries: Vec<_> = view
                    .changes
                    .iter()
                    .enumerate()
                    .filter_map(|(i, change)| {
                        group.change(change).map(|c| Entry {
                            path: c.path,
                            old_path: c.old_path,
                            status: c.status,
                            kind: Kind::Change(i, group),
                        })
                    })
                    .collect();
                if entries.is_empty() {
                    continue;
                }
                let key = format!("group:{}", group.label());
                let closed = self.closed.contains(&key);
                rows.push(Row::new(
                    &key,
                    format!(
                        "{} {} · {}",
                        if closed { "▸" } else { "▾" },
                        group.label(),
                        entries.len()
                    ),
                    Kind::Group(group),
                ));
                if !closed {
                    rows.extend(tree::rows(group.label(), entries, &self.closed));
                }
            }
        }
        if let Some(view) = view {
            if let Some(branch) = &view.comparison
                && !branch.details.files.is_empty()
            {
                rows.push(Row::new("", "", Kind::Inert));
                rows.push(Row::new(
                    "branch",
                    format!(
                        "{} Committed on branch · {}",
                        if self.branch_closed { "▸" } else { "▾" },
                        branch.details.files.len()
                    ),
                    Kind::Branch,
                ));
                if !self.branch_closed {
                    let entries = branch
                        .details
                        .files
                        .iter()
                        .enumerate()
                        .map(|(i, f)| Entry {
                            path: f.path.clone(),
                            old_path: f.old_path.clone(),
                            status: f.status.clone(),
                            kind: Kind::BranchFile(i),
                        })
                        .collect();
                    rows.extend(tree::rows("branch", entries, &self.closed));
                }
            }
            if !view.comparison_error.is_empty() {
                rows.push(Row::new(
                    "",
                    format!("Base: {}", view.comparison_error),
                    Kind::Inert,
                ));
            }
        }
        let range = self
            .history
            .as_ref()
            .map(|h| {
                if h.commits.is_empty() {
                    String::new()
                } else {
                    format!(
                        " {}–{}",
                        h.page * history::PAGE_SIZE + 1,
                        h.page * history::PAGE_SIZE + h.commits.len()
                    )
                }
            })
            .unwrap_or_default();
        rows.push(Row::new("", "", Kind::Inert));
        rows.push(Row::new(
            "commits",
            format!(
                "{} Commits{range}",
                if self.commits_closed { "▸" } else { "▾" }
            ),
            Kind::Commits,
        ));
        if !self.commits_closed {
            if let Some(h) = &self.history {
                if h.page > 0 {
                    rows.push(Row::new(
                        "previous",
                        "↑ Previous 50 commits",
                        Kind::Previous,
                    ));
                }
                if h.commits.is_empty() {
                    rows.push(Row::new("", "  No commits yet", Kind::Inert));
                }
                for commit in &h.commits {
                    let expanded = self.expanded.as_ref().filter(|d| d.sha == commit.sha);
                    rows.push(
                        Row::new(
                            format!("commit:{}", commit.sha),
                            format!(
                                "{} {} {}",
                                if expanded.is_some() { "▾" } else { "▸" },
                                &commit.sha[..7],
                                commit.subject
                            ),
                            Kind::Commit(commit.sha.clone()),
                        )
                        .with_graph(commit.graph.clone()),
                    );
                    if !commit.refs.is_empty() {
                        rows.push(
                            Row::new(
                                format!("refs:{}", commit.sha),
                                format!("  [{}]", commit.refs),
                                Kind::Commit(commit.sha.clone()),
                            )
                            .with_graph(commit.graph.replace('*', "|")),
                        );
                    }
                    if let Some(details) = expanded {
                        if details.files.is_empty() {
                            rows.push(Row::new("", "    No file changes", Kind::Inert));
                        } else {
                            let entries = details
                                .files
                                .iter()
                                .enumerate()
                                .map(|(i, f)| Entry {
                                    path: f.path.clone(),
                                    old_path: f.old_path.clone(),
                                    status: f.status.clone(),
                                    kind: Kind::File(commit.sha.clone(), i),
                                })
                                .collect();
                            rows.extend(
                                tree::rows(&commit.sha, entries, &self.closed)
                                    .into_iter()
                                    .map(|r| r.with_graph(commit.graph.replace('*', "|"))),
                            );
                            rows.push(
                                Row::new(
                                    format!("all:{}", commit.sha),
                                    "  ╰ Open all changes",
                                    Kind::All(commit.sha.clone()),
                                )
                                .with_graph(commit.graph.replace('*', "|")),
                            );
                        }
                    }
                    for edge in &commit.edges {
                        rows.push(Row::new("", "", Kind::Inert).with_graph(edge.clone()));
                    }
                }
                if h.more {
                    rows.push(Row::new("next", "↓ Next 50 commits", Kind::Next));
                }
            } else {
                rows.push(Row::new("", "  Reading history…", Kind::Inert));
            }
        }
        if !self.error.is_empty() {
            rows.push(Row::new(
                "",
                format!("{} · F retries", self.error),
                Kind::Inert,
            ));
        }
        rows
    }
    fn selected(&self, rows: &[Row]) -> usize {
        rows.iter()
            .position(|r| r.selectable() && r.key == self.key)
            .unwrap_or(0)
    }
    fn details(&self, sha: &str) -> Option<&Details> {
        self.expanded
            .as_ref()
            .filter(|d| d.sha == sha)
            .or_else(|| self.peek.as_ref().filter(|d| d.sha == sha))
    }
}
impl Ui {
    fn git_rows(&self) -> Vec<Row> {
        self.git_history
            .get(&self.snapshot.active)
            .map(|p| p.rows(self.git_views.get(&self.snapshot.active)))
            .unwrap_or_default()
    }
    fn git_selected_row(&self) -> Option<Row> {
        let rows = self.git_rows();
        let index = self.git_history.get(&self.snapshot.active)?.selected(&rows);
        rows.into_iter().nth(index)
    }
    fn git_geometry(&self) -> (usize, usize, usize) {
        let l = self.layout;
        let width = if l.right == 0 { l.width } else { l.right };
        (
            l.width - width,
            width,
            self.inspector_height().saturating_sub(8).max(1),
        )
    }
    fn git_start(&self, rows: &[Row]) -> usize {
        let selected = self
            .git_history
            .get(&self.snapshot.active)
            .map_or(0, |p| p.selected(rows));
        let count = self.git_geometry().2;
        let expanding = rows.get(selected).and_then(Row::sha).is_some_and(|sha| {
            matches!(rows[selected].kind, Kind::Commit(_))
                && self
                    .git_history
                    .get(&self.snapshot.active)
                    .and_then(|p| p.expanded.as_ref())
                    .is_some_and(|d| d.sha == sha)
        });
        let below = if expanding {
            3.min(count.saturating_sub(1))
        } else {
            0
        };
        self.git_history
            .get(&self.snapshot.active)
            .map_or(0, |p| p.viewport.start(selected, rows.len(), count, below))
    }
    pub(super) fn history_move(&mut self, delta: isize) {
        self.git_hover = None;
        let rows = self.git_rows();
        let Some(pane) = self.git_history.get_mut(&self.snapshot.active) else {
            return;
        };
        let positions: Vec<_> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .map(|(i, _)| i)
            .collect();
        if positions.is_empty() {
            return;
        }
        let index = positions
            .iter()
            .position(|i| *i == pane.selected(&rows))
            .unwrap_or(0);
        let next = (index as isize)
            .saturating_add(delta)
            .clamp(0, positions.len() as isize - 1) as usize;
        pane.key = rows[positions[next]].key.clone();
        self.git_start(&rows);
    }
    pub(super) fn history_refresh(&mut self) {
        self.git_hover = None;
        self.history_request(Action::Page(None, 0));
    }
    fn history_request(&mut self, action: Action) {
        let Some(w) = self.snapshot.workspace() else {
            return;
        };
        let Some(view) = self
            .git_views
            .get(&w.id)
            .filter(|v| v.error.is_empty() && !v.root.as_os_str().is_empty())
        else {
            return;
        };
        let pane = self.git_history.entry(w.id).or_default();
        pane.error.clear();
        self.history_pending = Some(Request {
            epoch: self.snapshot.epoch.clone(),
            workspace: w.id,
            tab: self.snapshot.tab,
            run: self
                .snapshot
                .session()
                .map(|t| t.run.clone())
                .unwrap_or_default(),
            revision: self.snapshot.split.as_ref().map_or(0, |s| s.revision),
            cwd: w.cwd.clone(),
            root: view.root.clone(),
            key: pane.key.clone(),
            action,
        });
    }
    pub(super) fn tick_history(&mut self) -> bool {
        let mut dirty = false;
        if let Some(rx) = &self.history_job {
            match rx.try_recv() {
                Ok(result) => {
                    self.history_job = None;
                    dirty = true;
                    let request = result.request;
                    if request.epoch == self.snapshot.epoch
                        && self
                            .snapshot
                            .workspaces
                            .iter()
                            .any(|w| w.id == request.workspace && w.cwd == request.cwd)
                    {
                        let pane = self.git_history.entry(request.workspace).or_default();
                        let active = request.workspace == self.snapshot.active
                            && request.tab == self.snapshot.tab
                            && request.revision
                                == self.snapshot.split.as_ref().map_or(0, |s| s.revision)
                            && self
                                .snapshot
                                .session()
                                .map(|t| t.run.as_str())
                                .unwrap_or_default()
                                == request.run
                            && pane.key == request.key
                            && self.prefs.inspector == Inspector::Git
                            && self.focus == Focus::Files
                            && !self.board;
                        match result.result {
                            Ok(Payload::Page(history)) => {
                                let page_changed = pane
                                    .history
                                    .as_ref()
                                    .is_some_and(|h| h.page != history.page);
                                if page_changed {
                                    pane.key = history
                                        .commits
                                        .first()
                                        .map(|c| format!("commit:{}", c.sha))
                                        .unwrap_or_else(|| "commits".into());
                                }
                                pane.history = Some(history);
                                pane.expanded = None;
                                pane.peek = None;
                                pane.error.clear();
                            }
                            Ok(Payload::Details(details, expand)) => {
                                if expand && active {
                                    pane.expanded = Some(details);
                                } else {
                                    pane.peek = Some(details);
                                }
                            }
                            Ok(Payload::Message(details)) => {
                                let text = details.message.clone();
                                pane.peek = Some(details);
                                if active {
                                    self.git_preview("Commit message", &text);
                                }
                            }
                            Ok(Payload::Comparison(comparison)) if active => {
                                let opened = self.editor_open(
                                    &panes::EditorOrigin {
                                        epoch: request.epoch.clone(),
                                        workspace: request.workspace,
                                        tab: request.tab,
                                        run: request.run.clone(),
                                        revision: request.revision,
                                    },
                                    &comparison.after,
                                    Some(&comparison.before),
                                );
                                if opened
                                    && self.snapshot.session().is_some_and(|t| {
                                        t.path == comparison.after.to_string_lossy()
                                    })
                                {
                                    self.previews
                                        .remove(&(request.workspace, self.snapshot.tab));
                                    self.focus = Focus::Terminal;
                                    self.nav = false;
                                }
                            }
                            Ok(Payload::Diff(text, file)) if active => {
                                if let Some(document) = file {
                                    if self.editor_open(
                                        &panes::EditorOrigin {
                                            epoch: request.epoch.clone(),
                                            workspace: request.workspace,
                                            tab: request.tab,
                                            run: request.run.clone(),
                                            revision: request.revision,
                                        },
                                        &document.path,
                                        None,
                                    ) {
                                        self.previews
                                            .remove(&(request.workspace, self.snapshot.tab));
                                        self.focus = Focus::Files;
                                        self.nav = false;
                                    }
                                } else {
                                    self.git_preview("Git diff", &text);
                                }
                            }
                            Ok(_) => {}
                            Err(e) => {
                                pane.error = wire::passive(&e.to_string());
                                if request.workspace == self.snapshot.active {
                                    self.notice = format!("Git: {}", pane.error);
                                }
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.history_job = None;
                    self.notice = "Git history worker disconnected; F retries".into();
                    dirty = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.prefs.inspector == Inspector::Git {
            let id = self.snapshot.active;
            if !self.git_history.entry(id).or_default().requested
                && self.history_pending.is_none()
                && self
                    .git_views
                    .get(&id)
                    .is_some_and(|v| v.error.is_empty() && v.root.is_absolute())
            {
                self.history_request(Action::Page(None, 0));
                dirty = true;
            }
            if let Some((_, _, since, shown)) = &mut self.git_hover
                && !*shown
                && since.elapsed() >= Duration::from_millis(350)
            {
                *shown = true;
                dirty = true;
            }
            if let Some((sha, _, since, _)) = &self.git_hover
                && since.elapsed() >= Duration::from_millis(350)
                && self.history_job.is_none()
                && self.history_pending.is_none()
                && self
                    .git_history
                    .get(&id)
                    .is_some_and(|p| p.details(sha).is_none() && p.error.is_empty())
            {
                self.history_request(Action::Details(sha.clone(), false));
            }
        }
        if self.history_job.is_none()
            && let Some(request) = self.history_pending.take()
            && request.epoch == self.snapshot.epoch
            && request.workspace == self.snapshot.active
        {
            // A queued page may be discarded when its card loses focus. Mark
            // it requested only once it starts, so returning retries that page.
            if matches!(request.action, Action::Page(..)) {
                self.git_history
                    .entry(request.workspace)
                    .or_default()
                    .requested = true;
            }
            let (tx, rx) = std::sync::mpsc::channel();
            self.history_job = Some(rx);
            let state = self.state.clone();
            std::thread::spawn(move || {
                let result = match &request.action {
                    Action::Page(tips, page) => {
                        history::history(&request.root, tips.as_deref(), *page).map(Payload::Page)
                    }
                    Action::Details(sha, expand) => {
                        history::details(&request.root, sha).map(|d| Payload::Details(d, *expand))
                    }
                    Action::Message(sha) => {
                        history::details(&request.root, sha).map(Payload::Message)
                    }
                    Action::CommitDiff(sha) => history::details(&request.root, sha)
                        .and_then(|d| history::diff(&request.root, &d, None))
                        .map(|s| Payload::Diff(s, None)),
                    Action::Diff(details, Some(file), true) => {
                        crate::git::versions::commit(&request.root, details, file)
                            .and_then(|v| crate::editor::managed_comparison(&state, &v))
                            .map(Payload::Comparison)
                    }
                    Action::Diff(details, file, editor) => {
                        history::diff(&request.root, details, file.as_ref()).and_then(|text| {
                            let path = if *editor {
                                let file_name = file
                                    .as_ref()
                                    .and_then(|f| Path::new(&f.path).file_name())
                                    .and_then(|p| p.to_str())
                                    .unwrap_or("all-changes");
                                Some(crate::editor::managed_diff_file(
                                    &state,
                                    &text,
                                    &format!("{}-{file_name}", &details.sha[..8]),
                                )?)
                            } else {
                                None
                            };
                            Ok(Payload::Diff(text, path))
                        })
                    }
                    Action::Working(change, true) => {
                        crate::git::versions::working(&request.root, change)
                            .and_then(|v| crate::editor::managed_comparison(&state, &v))
                            .map(Payload::Comparison)
                    }
                    Action::Working(change, false) => {
                        crate::git::diff_change(&request.root, change)
                            .map(|s| Payload::Diff(s, None))
                    }
                };
                let _ = tx.send(Result { request, result });
            });
            dirty = true;
        }
        dirty
    }
    fn git_preview(&mut self, title: &str, text: &str) {
        self.previews.insert(
            (self.snapshot.active, self.snapshot.tab),
            Preview {
                name: title.into(),
                lines: text.lines().map(String::from).collect(),
                offset: 0,
                action: Some(PreviewAction::Git),
            },
        );
        self.focus = Focus::Terminal;
        self.nav = false;
        self.git_hover = None;
    }
    pub(super) fn git_activate(&mut self, preview: bool) {
        self.git_hover = None;
        let id = self.snapshot.active;
        let Some(row) = self.git_selected_row() else {
            return;
        };
        match row.kind {
            Kind::Changes => {
                if let Some(p) = self.git_history.get_mut(&id) {
                    p.changes_closed = !p.changes_closed;
                }
            }
            Kind::Group(group) => {
                let p = self.git_history.get_mut(&id).unwrap();
                let key = format!("group:{}", group.label());
                if !p.closed.remove(&key) {
                    p.closed.insert(key);
                }
            }
            Kind::Folder(key) => {
                let p = self.git_history.get_mut(&id).unwrap();
                if !p.closed.remove(&key) {
                    p.closed.insert(key);
                }
            }
            Kind::Branch => {
                let p = self.git_history.get_mut(&id).unwrap();
                p.branch_closed = !p.branch_closed;
            }
            Kind::BranchFile(i) => {
                if let Some(branch) = self.git_views.get(&id).and_then(|v| v.comparison.as_ref())
                    && let Some(file) = branch.details.files.get(i).cloned()
                {
                    self.history_request(Action::Diff(
                        branch.details.clone(),
                        Some(file),
                        !preview,
                    ));
                }
            }
            Kind::Commits => {
                if let Some(p) = self.git_history.get_mut(&id) {
                    p.commits_closed = !p.commits_closed;
                }
            }
            Kind::Change(i, group) => {
                if let Some(view) = self.git_views.get(&id)
                    && let Some(change) = view.changes.get(i).and_then(|c| group.change(c))
                {
                    if preview {
                        self.history_request(Action::Working(change.clone(), false));
                    } else {
                        let path = view.root.join(&change.path);
                        self.command(&[
                            "open",
                            &id.to_string(),
                            &wire::hex(path.to_string_lossy().as_bytes()),
                        ]);
                        if self
                            .snapshot
                            .session()
                            .is_some_and(|t| t.path == path.to_string_lossy())
                        {
                            self.focus = Focus::Terminal;
                            self.nav = false;
                        }
                    }
                }
            }
            Kind::Commit(sha) => {
                let pane = self.git_history.get_mut(&id).unwrap();
                if !preview && pane.expanded.as_ref().is_some_and(|d| d.sha == sha) {
                    pane.expanded = None;
                } else if let Some(details) = pane.details(&sha).cloned() {
                    if preview {
                        self.history_request(Action::Diff(details, None, false));
                    } else {
                        pane.expanded = Some(details);
                    }
                } else if preview {
                    self.history_request(Action::CommitDiff(sha));
                } else {
                    self.history_request(Action::Details(sha, true));
                }
            }
            kind @ (Kind::File(_, _) | Kind::All(_)) => {
                let (sha, index) = match kind {
                    Kind::File(sha, i) => (sha, Some(i)),
                    Kind::All(sha) => (sha, None),
                    _ => unreachable!(),
                };
                if let Some(details) = self
                    .git_history
                    .get(&id)
                    .and_then(|p| p.details(&sha))
                    .cloned()
                {
                    let file = index.and_then(|i| details.files.get(i).cloned());
                    self.history_request(Action::Diff(details, file, !preview));
                }
            }
            kind @ (Kind::Previous | Kind::Next) => {
                if let Some(h) = self.git_history.get(&id).and_then(|p| p.history.as_ref()) {
                    let page = if matches!(kind, Kind::Previous) {
                        h.page.saturating_sub(1)
                    } else {
                        h.page + 1
                    };
                    self.history_request(Action::Page(Some(h.tips.clone()), page));
                }
            }
            Kind::Inert => {}
        }
    }
    fn git_compare_selected(&mut self) {
        if let Some(row) = self.git_selected_row() {
            match row.kind {
                Kind::Change(i, group) => {
                    if let Some(change) = self
                        .git_views
                        .get(&self.snapshot.active)
                        .and_then(|v| v.changes.get(i))
                        .and_then(|c| group.change(c))
                    {
                        self.nav = false;
                        self.history_request(Action::Working(change, true));
                    }
                }
                Kind::File(_, _) | Kind::All(_) | Kind::BranchFile(_) => self.git_activate(false),
                Kind::Commit(_) => self.git_activate(true),
                _ => {}
            }
        }
    }
    pub(super) fn git_key(&mut self, b: &[u8]) -> bool {
        if self.prefs.inspector != Inspector::Git || self.focus != Focus::Files || self.board {
            return false;
        }
        match b {
            b"j" | b"\x1b[B" => self.history_move(1),
            b"k" | b"\x1b[A" => self.history_move(-1),
            b"\x1b[6~" => self.history_move(self.git_geometry().2 as isize),
            b"\x1b[5~" => self.history_move(-(self.git_geometry().2 as isize)),
            b"\x1b[H" => self.history_move(isize::MIN),
            b"\x1b[F" => self.history_move(isize::MAX),
            b"\r" => self.git_activate(false),
            b"d" => self.git_compare_selected(),
            b"u" => self.git_activate(true),
            b"F" => {
                self.git_checked = Instant::now() - Duration::from_secs(4);
                self.history_refresh();
            }
            b"G" => {
                if let Some(p) = self.git_history.get_mut(&self.snapshot.active) {
                    p.key = "commits".into();
                    p.commits_closed = false;
                }
            }
            b"?" => {
                if let Some(row) = self.git_selected_row()
                    && let Some(sha) = row.sha()
                {
                    if let Some(details) = self
                        .git_history
                        .get(&self.snapshot.active)
                        .and_then(|p| p.details(sha))
                    {
                        let message = details.message.clone();
                        self.git_preview("Commit message", &message);
                    } else {
                        self.history_request(Action::Message(sha.into()));
                    }
                }
            }
            _ => return false,
        }
        true
    }
    fn git_hit(&self, x: usize, y: usize) -> Option<Row> {
        if self.prefs.inspector != Inspector::Git
            || self.board
            || self.menu
            || self.form.is_some()
            || (self.layout.right == 0 && self.focus != Focus::Files)
        {
            return None;
        }
        let (left, width, count) = self.git_geometry();
        if x <= left || x >= left + width - 1 || y < 5 || y >= 5 + count {
            return None;
        }
        let rows = self.git_rows();
        let start = self.git_start(&rows);
        rows.into_iter().nth(start + y - 5)
    }
    pub(super) fn git_click(&mut self, x: usize, y: usize) {
        if let Some(row) = self.git_hit(x, y)
            && row.selectable()
        {
            self.git_history
                .entry(self.snapshot.active)
                .or_default()
                .key = row.key;
            self.git_activate(false);
        }
    }
    pub(super) fn git_context_row(&self, x: usize, y: usize) -> Option<ContextRow> {
        let row = self.git_hit(x, y).filter(Row::selectable)?;
        Some(ContextRow {
            preview: matches!(
                row.kind,
                Kind::Change(..)
                    | Kind::Commit(_)
                    | Kind::File(..)
                    | Kind::All(_)
                    | Kind::BranchFile(_)
            ),
            sha: row.sha().map(String::from),
            key: row.key,
            text: row.text,
        })
    }
    pub(super) fn git_context_exists(&self, key: &str, text: &str) -> bool {
        self.git_rows()
            .iter()
            .any(|row| row.key == key && row.text == text && row.selectable())
    }
    pub(super) fn git_context_open(&mut self, key: &str, preview: bool) {
        self.git_history
            .entry(self.snapshot.active)
            .or_default()
            .key = key.into();
        self.focus = Focus::Files;
        self.nav = false;
        self.git_activate(preview);
    }
    pub(super) fn git_hover_at(&mut self, x: usize, y: usize) {
        let next = self.git_hit(x, y).and_then(|r| r.sha().map(String::from));
        if self.git_hover.as_ref().map(|v| &v.0) != next.as_ref() {
            self.git_hover = next.map(|s| (s, y, Instant::now(), false));
        }
    }
    pub(super) fn git_wheel(&mut self, x: usize, y: usize, delta: i64) -> bool {
        if self.git_hit(x, y).is_some() {
            self.history_move(delta as isize);
            return true;
        }
        false
    }
    pub(super) fn draw_git(&self, c: &mut Canvas) {
        let (x, width, count) = self.git_geometry();
        let view = self.git_views.get(&self.snapshot.active);
        if let Some(view) = view {
            c.text(
                x + 2,
                3,
                width.saturating_sub(4),
                &chrome::elide(&view.branch, width.saturating_sub(4)),
                style(CYAN, PANEL, true),
            );
        }
        if let Some(branch) = view.and_then(|v| v.comparison.as_ref()) {
            c.text(
                x + 2,
                4,
                width.saturating_sub(4),
                &chrome::elide(
                    &format!("compared with {}", branch.target),
                    width.saturating_sub(4),
                ),
                style(MUTED, PANEL, false),
            );
        }
        let rows = self.git_rows();
        let start = self.git_start(&rows);
        let selected = self
            .git_history
            .get(&self.snapshot.active)
            .map_or(0, |p| p.selected(&rows));
        for (i, row) in rows.iter().enumerate().skip(start).take(count) {
            let y = 5 + i - start;
            let chosen = i == selected && self.chrome_focused(Focus::Files);
            let bg = if chosen { ACTIVE_BG } else { PANEL };
            let fg = if chosen {
                CYAN
            } else if matches!(
                row.kind,
                Kind::Changes | Kind::Commits | Kind::Branch | Kind::Group(_)
            ) {
                TEXT
            } else if matches!(row.kind, Kind::Folder(_)) {
                MUTED
            } else if row.key.starts_with("refs:") {
                MAGENTA
            } else {
                TEXT
            };
            if row.text.is_empty() && row.graph.is_empty() {
                c.text(
                    x + 2,
                    y,
                    width.saturating_sub(4),
                    &"─".repeat(width.saturating_sub(4)),
                    style(BORDER, PANEL, false),
                );
                continue;
            }
            c.fill(x + 1, y, width.saturating_sub(2), 1, style(fg, bg, false));
            if chosen {
                c.text(x + 1, y, 1, "▌", style(CYAN, bg, true));
            }
            let max_graph = width.saturating_sub(16).min(24);
            let graph_width = row.graph.len().min(max_graph);
            for (col, glyph) in row.graph.chars().take(graph_width).enumerate() {
                let text = match glyph {
                    '*' => "●",
                    '|' => "│",
                    '/' => "╱",
                    '\\' => "╲",
                    '-' | '_' => "─",
                    _ => " ",
                };
                let colors = [CYAN, MAGENTA, Color::Rgb(125, 218, 176), GOLD];
                c.text(
                    x + 2 + col,
                    y,
                    1,
                    text,
                    style(colors[(col / 2) % colors.len()], bg, true),
                );
            }
            c.text(
                x + 2 + graph_width,
                y,
                width.saturating_sub(4 + graph_width),
                &row.text,
                style(fg, bg, false),
            );
            if let Some((column, status)) = &row.status {
                let available = width.saturating_sub(4 + graph_width + column);
                let color = match status.as_bytes().first() {
                    Some(b'A') => Color::Rgb(125, 218, 176),
                    Some(b'D') => Color::Rgb(240, 143, 151),
                    Some(b'?') => MUTED,
                    _ => GOLD,
                };
                c.text(
                    x + 2 + graph_width + column,
                    y,
                    available.min(status.len()),
                    status,
                    style(color, bg, false),
                );
            }
        }
        if self.inspector_height() > 8 {
            c.text(
                x + 2,
                self.inspector_height() - 3,
                width.saturating_sub(4),
                if width >= 40 {
                    "Enter open/fold · d diff · u patch"
                } else {
                    "Enter open · d diff · u patch"
                },
                style(MUTED, PANEL, false),
            );
        }
    }
    pub(super) fn draw_git_hover(&self, c: &mut Canvas) {
        if self.prefs.inspector != Inspector::Git
            || self.board
            || self.menu
            || self.form.is_some()
            || self.confirm.is_some()
        {
            return;
        }
        let Some((sha, row, since, _)) = &self.git_hover else {
            return;
        };
        if since.elapsed() < Duration::from_millis(350) {
            return;
        }
        let Some(details) = self
            .git_history
            .get(&self.snapshot.active)
            .and_then(|p| p.details(sha))
        else {
            return;
        };
        let width = self.layout.width.saturating_sub(4).min(74);
        if width < 16 || self.layout.height < 10 {
            return;
        }
        let body = details
            .message
            .split_once("\n\n")
            .map_or(details.message.as_str(), |(_, body)| body);
        let preview = Preview {
            name: String::new(),
            lines: body.lines().map(String::from).collect(),
            offset: 0,
            action: None,
        };
        let lines = preview.wrapped(width.saturating_sub(4));
        let height = (lines.len() + 3).min(12).min(self.layout.height - 3);
        let x = self
            .layout
            .width
            .saturating_sub(self.layout.right.max(width) + 2);
        let y = (row + 1).min(self.layout.height.saturating_sub(height + 1));
        c.fill(x + 1, y + 1, width, height, style(TEXT, BG, false));
        c.fill(x, y, width, height, style(TEXT, PANEL, false));
        c.border(x, y, width, height, MAGENTA);
        for (i, line) in lines.iter().take(height - 3).enumerate() {
            c.text(x + 2, y + 1 + i, width - 4, line, style(TEXT, PANEL, false));
        }
        c.text(
            x + 2,
            y + height - 2,
            width - 4,
            "Select commit · ? full message",
            style(CYAN, PANEL, false),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{Change, GitView, branch::BranchDiff};

    #[test]
    fn grouped_tree_rows_preserve_selection_identity_and_fold_scopes() {
        let mut view = GitView {
            changes: vec![Change {
                status: "MM".into(),
                path: "src/界\t.rs".into(),
                old_path: None,
            }],
            ..GitView::default()
        };
        let mut pane = Pane::default();
        let first = pane.rows(Some(&view));
        let staged = first
            .iter()
            .find(|r| matches!(r.kind, Kind::Change(_, Group::Staged)))
            .unwrap();
        let unstaged = first
            .iter()
            .find(|r| matches!(r.kind, Kind::Change(_, Group::Unstaged)))
            .unwrap();
        assert_ne!(staged.key, unstaged.key);
        assert!(staged.key.ends_with("src/界\t.rs"));
        assert!(!staged.text.contains('\t'));
        pane.key = unstaged.key.clone();
        view.changes.insert(
            0,
            Change {
                status: "??".into(),
                path: "a.txt".into(),
                old_path: None,
            },
        );
        pane.closed.insert("dir:Staged:src/".into());
        let folded = pane.rows(Some(&view));
        assert!(
            !folded
                .iter()
                .any(|r| matches!(r.kind, Kind::Change(_, Group::Staged)))
        );
        assert!(matches!(
            folded[pane.selected(&folded)].kind,
            Kind::Change(1, Group::Unstaged)
        ));
        pane.closed.clear();
        let restored = pane.rows(Some(&view));
        assert_eq!(restored[pane.selected(&restored)].key, unstaged.key);
        assert!(restored.iter().any(|r| r.key == staged.key));
    }

    #[test]
    fn committed_heading_exists_only_for_net_branch_files_and_disappearing_selection_is_safe() {
        let mut view = GitView {
            comparison: Some(BranchDiff {
                target: "main".into(),
                details: Details {
                    sha: "a".repeat(40),
                    parent: Some("b".repeat(40)),
                    message: String::new(),
                    files: vec![],
                },
            }),
            ..GitView::default()
        };
        let mut pane = Pane::default();
        assert!(
            !pane
                .rows(Some(&view))
                .iter()
                .any(|r| matches!(r.kind, Kind::Branch))
        );
        view.comparison
            .as_mut()
            .unwrap()
            .details
            .files
            .push(FileChange {
                status: "M".into(),
                path: "src/file.rs".into(),
                old_path: None,
            });
        let rows = pane.rows(Some(&view));
        pane.key = rows
            .iter()
            .find(|r| matches!(r.kind, Kind::BranchFile(_)))
            .unwrap()
            .key
            .clone();
        assert!(rows.iter().any(|r| r.text.contains("Committed on branch")));
        view.comparison.as_mut().unwrap().details.files.clear();
        let clean = pane.rows(Some(&view));
        assert!(matches!(clean[pane.selected(&clean)].kind, Kind::Changes));
        assert!(!clean.iter().any(|r| r.text.contains("Committed on branch")));
        view.comparison = None;
        view.comparison_error = "comparison base unavailable locally".into();
        assert!(
            pane.rows(Some(&view))
                .iter()
                .any(|r| r.text.contains("unavailable locally"))
        );
    }
}
