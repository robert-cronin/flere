use super::*;
use crate::{
    git::GitView,
    workspace::{CardMeta, Grouping, Inspector, Preferences, Workflow},
};

pub(super) const ACTIONS: &[(u8, &str)] = &[
    (b'G', "Add project"),
    (b'n', "New worktree"),
    (b't', "New terminal"),
    (b'T', "Workspace tabs"),
    (b'|', "Split right (move current tab)"),
    (b'_', "Split below (move current tab)"),
    (b'\\', "Split right (new terminal)"),
    (b'=', "Split below (new terminal)"),
    (b'~', "Move tab to other pane"),
    (b'Z', "Zoom pane / restore split"),
    (b'X', "Remove split (keep tabs)"),
    (b'/', "Find workspace"),
    (b'Q', "Quick Open"),
    (b'H', "Find in Files"),
    (b'?', "Find Terminal Output"),
    (b'!', "Run Task"),
    (b';', "Problems"),
    (b'(', "Previous shell command"),
    (b')', "Next shell command"),
    (b'Y', "Copy command output"),
    (b'P', "Project filter"),
    (b'L', "Group workspaces by Status / Project"),
    (b'g', "Git changes and commits"),
    (b'i', "Cycle Files / Git / Details"),
    (b'F', "Refresh files and Git"),
    (b'v', "Card details"),
    (b'e', "Edit card"),
    (b'p', "Pin / unpin"),
    (b'B', "Project board"),
    (b'J', "Next workspace needing me"),
    (b'C', "Toggle compact workspace rows"),
    (b'S', "Start agent"),
    (b'W', "Reopen saved tabs"),
    (b'w', "All live terminals"),
    (b'b', "Back"),
    (b'f', "Forward"),
    (b'1', "Move to Needs me"),
    (b'2', "Move to In progress"),
    (b'3', "Move to Todo"),
    (b'4', "Move to Waiting"),
    (b'5', "Move to Done"),
    (b'6', "Upload local file"),
    (b'7', "Download selected file"),
    (b'8', "Open selected file locally"),
    (b'9', "Ports"),
    (b'z', "Zoom terminal"),
    (b'r', "Reset layout"),
    (b'R', "Refresh Flere (keep sessions)"),
    (b'K', "Update Flere"),
    (b'M', "Toggle reduced motion"),
    (b'U', "Start screensaver"),
    (b'%', "Mascot: Duck (screensaver + pet)"),
    (b'^', "Mascot: Flere (screensaver + pet)"),
    (
        b'V',
        "Cycle screensaver idle time (5 min / 15 min / manual)",
    ),
    (b'o', "Toggle pet"),
    (b'O', "Play with pets"),
    (b'&', "Arcade: Context Ruins"),
    (b'c', "Read terminal scrollback"),
    (b's', "Copy Flere screenshot"),
    (b'A', "Archive stopped card (keep files)"),
    (b'a', "Restore archived workspace"),
    (b'm', "Message workspace"),
    (b'I', "Read messages"),
    (b'D', "Review decisions"),
    (b'q', "Detach (keep sessions)"),
];
pub(super) struct Form {
    kind: String,
    target: u64,
    values: Vec<(&'static str, Vec<u8>)>,
    selected: usize,
    result: usize,
    meta: CardMeta,
    reference: Option<serde_json::Value>,
    locked: usize,
    pub(super) project: Option<project_picker::Picker>,
}
#[derive(Clone)]
struct Pick {
    label: String,
    wid: u64,
    tab: u64,
    reference: Option<serde_json::Value>,
}
fn picker(kind: &str) -> bool {
    matches!(
        kind,
        "Search"
            | "Workspace tabs"
            | "All terminals"
            | "Messages"
            | "Decisions"
            | "Archived workspaces"
    )
}
pub(super) struct GitResult {
    pub id: u64,
    pub view: GitView,
    pub diff: Option<String>,
}
pub(super) const CARD_HEIGHT: usize = 3;
pub(super) const CARD_PITCH: usize = CARD_HEIGHT + 1;

pub(super) enum SideRow {
    Heading {
        name: String,
        key: String,
        accent: Color,
        count: usize,
    },
    CardBorder {
        id: u64,
        top: bool,
    },
    Card {
        id: u64,
        row: usize,
    },
    Terminal {
        id: u64,
        tab: u64,
        run: String,
    },
    Gap,
}
pub(super) fn tint(status: Workflow) -> (Color, Color) {
    match status {
        Workflow::NeedsMe => (GOLD, Color::Rgb(32, 27, 25)),
        Workflow::InProgress => (WORKING, Color::Rgb(17, 27, 44)),
        Workflow::Todo => (Color::Rgb(174, 157, 255), Color::Rgb(23, 22, 38)),
        Workflow::Waiting => (MAGENTA, Color::Rgb(32, 21, 35)),
        Workflow::Done => (Color::Rgb(125, 218, 176), Color::Rgb(16, 30, 28)),
    }
}
pub(super) fn activity_glyph(step: usize, reduced: bool) -> &'static str {
    if reduced {
        "●"
    } else {
        ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"][step % 10]
    }
}
pub(super) fn activity_signal(
    c: &mut Canvas,
    x: usize,
    y: usize,
    width: usize,
    bg: Color,
    step: usize,
) {
    if width == 0 {
        return;
    }
    let head = step % (width + 5);
    let trail = [
        Color::Rgb(233, 249, 255),
        WORKING,
        Color::Rgb(66, 132, 182),
        Color::Rgb(37, 80, 116),
        BORDER,
    ];
    for i in 0..width {
        let age = head.wrapping_sub(i);
        c.text(
            x + i,
            y,
            1,
            if age == 0 { "◆" } else { "━" },
            style(trail.get(age).copied().unwrap_or(BORDER), bg, age < 2),
        );
    }
}
fn fuzzy(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    let mut chars = text.chars();
    query.to_lowercase().chars().all(|q| chars.any(|c| c == q))
}
pub(super) fn load_preferences(state: &Path) -> Preferences {
    let mut prefs: Preferences = fs::File::open(state.join("ui.json"))
        .ok()
        .and_then(|f| {
            let mut b = Vec::new();
            f.take(65537).read_to_end(&mut b).ok()?;
            if b.len() > 65536 {
                return None;
            }
            serde_json::from_slice(&b).ok()
        })
        .unwrap_or_default();
    prefs.expanded_cards.truncate(512);
    if !matches!(prefs.screensaver_minutes, 0 | 5 | 15) {
        prefs.screensaver_minutes = 5;
    }
    prefs
}
impl Ui {
    pub(super) fn save_preferences(&mut self) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&self.prefs)
            && let Err(e) = crate::workspace::atomic_write(&self.state.join("ui.json"), &bytes)
        {
            self.notice = e.to_string();
        }
        self.relayout();
    }
    pub(super) fn relayout(&mut self) {
        let next = Layout::with_preferences(self.layout.width, self.layout.height, &self.prefs);
        if next != self.layout {
            self.selection = None;
            self.scrollback = None;
        }
        if (next.cols, next.rows) != (self.layout.cols, self.layout.rows) {
            self.layout = next;
            self.resize_viewport();
        } else {
            self.layout = next;
        }
    }
    pub(super) fn visible_cards(&self) -> Vec<u64> {
        self.side_rows()
            .into_iter()
            .filter_map(|r| match r {
                SideRow::Card { id, row: 0 } => Some(id),
                _ => None,
            })
            .collect()
    }
    pub(super) fn side_rows(&self) -> Vec<SideRow> {
        let cards: Vec<_> = self
            .snapshot
            .workspaces
            .iter()
            .filter(|w| {
                !w.meta.archived
                    && (self.prefs.project.is_empty() || w.meta.project == self.prefs.project)
            })
            .collect();
        let mut groups = vec![(
            "Pinned".to_owned(),
            "Pinned".to_owned(),
            CYAN,
            cards
                .iter()
                .copied()
                .filter(|w| w.meta.pinned)
                .collect::<Vec<_>>(),
        )];
        match self.prefs.grouping {
            Grouping::Status => {
                for status in Workflow::ALL {
                    groups.push((
                        status.label().into(),
                        status.label().into(),
                        tint(status).0,
                        cards
                            .iter()
                            .copied()
                            .filter(|w| !w.meta.pinned && w.meta.status == status)
                            .collect(),
                    ));
                }
            }
            Grouping::Project => {
                let mut names: Vec<_> = cards
                    .iter()
                    .filter(|w| !w.meta.pinned)
                    .map(|w| w.meta.project.as_str())
                    .collect();
                names.sort_unstable_by_key(|name| (name.is_empty(), *name));
                names.dedup();
                for name in names {
                    groups.push((
                        if name.is_empty() {
                            "Unassigned".into()
                        } else {
                            name.into()
                        },
                        format!("project:{name}"),
                        if name.is_empty() {
                            MUTED
                        } else {
                            project_color(name)
                        },
                        cards
                            .iter()
                            .copied()
                            .filter(|w| !w.meta.pinned && w.meta.project == name)
                            .collect(),
                    ));
                }
            }
        }
        let mut rows = Vec::new();
        for (name, key, accent, members) in groups {
            if members.is_empty() {
                continue;
            }
            let folded = self.prefs.folds.contains(&key);
            rows.push(SideRow::Heading {
                name,
                key,
                accent,
                count: members.len(),
            });
            if !folded {
                for w in members {
                    rows.push(SideRow::CardBorder {
                        id: w.id,
                        top: true,
                    });
                    for row in 0..self.sidebar_card_height() {
                        rows.push(SideRow::Card { id: w.id, row });
                    }
                    if self.prefs.expanded_cards.contains(&w.id) {
                        for tab in &w.tabs {
                            rows.push(SideRow::Terminal {
                                id: w.id,
                                tab: tab.id,
                                run: tab.run.clone(),
                            });
                        }
                    }
                    rows.push(SideRow::CardBorder {
                        id: w.id,
                        top: false,
                    });
                    rows.push(SideRow::Gap);
                }
            }
        }
        rows
    }
    pub(super) fn side_offset(&self) -> usize {
        let rows = self.side_rows();
        let visible = self.layout.height.saturating_sub(5);
        if rows.is_empty() {
            return 0;
        }
        if let Some(offset) = self.side_scroll {
            return offset.min(rows.len().saturating_sub(visible));
        }
        let cursor = self.card_cursor();
        let active = rows
            .iter()
            .position(|r| match r {
                SideRow::Card { id, row: 0 } => *id == cursor.wid && cursor.tab == 0,
                SideRow::Terminal { id, tab, run } => {
                    *id == cursor.wid && *tab == cursor.tab && *run == cursor.run
                }
                _ => false,
            })
            .unwrap_or(0);
        let top = rows[..=active]
            .iter()
            .rposition(
                |row| matches!(row, SideRow::CardBorder { id, top: true } if *id == cursor.wid),
            )
            .unwrap_or(active);
        let end = rows
            .iter()
            .enumerate()
            .skip(active)
            .find_map(|(i, row)| {
                matches!(row, SideRow::CardBorder { id, top: false } if *id == cursor.wid)
                    .then_some(i + 1)
            })
            .unwrap_or(active + 1);
        if end - top <= visible {
            end.saturating_sub(visible)
        } else {
            // A tall expanded card can exceed the viewport. Follow its concrete
            // cursor row without making any of the clipped parent rows targets.
            let target_end = active
                + if cursor.tab == 0 {
                    self.sidebar_card_height()
                } else {
                    1
                };
            target_end.saturating_sub(visible)
        }
    }
    pub(super) fn sidebar_wheel(&mut self, x: usize, y: usize, delta: i64) -> bool {
        let width = if self.layout.left > 0 {
            self.layout.left
        } else if self.focus == Focus::Cards {
            self.layout.width
        } else {
            return false;
        };
        if x >= width || y < 3 || y >= self.layout.height - 2 {
            return false;
        }
        let last = self
            .side_rows()
            .len()
            .saturating_sub(self.layout.height.saturating_sub(5));
        self.side_scroll = Some(
            self.side_offset()
                .saturating_add_signed(delta as isize)
                .min(last),
        );
        true
    }
    pub(super) fn sidebar_click(&mut self, y: usize) {
        if y == 2 {
            self.toggle_sidebar_grouping();
            return;
        }
        if y < 3 || y >= self.layout.height - 2 {
            return;
        }
        let rows = self.side_rows();
        let index = self.side_offset() + y - 3;
        if let Some(SideRow::Heading { key, .. }) = rows.get(index) {
            if let Some(i) = self.prefs.folds.iter().position(|s| s == key) {
                self.prefs.folds.remove(i);
            } else {
                self.prefs.folds.push(key.clone());
            }
            self.save_preferences();
        }
    }
    pub(super) fn answer_form(&mut self, wid: u64, reference: serde_json::Value) {
        self.open_form("Answer decision");
        if let Some(form) = &mut self.form {
            form.target = wid;
            form.values[0].1 = reference["question"]
                .as_str()
                .unwrap_or_default()
                .as_bytes()
                .to_vec();
            form.values[1].1 = reference["recommendation"]
                .as_str()
                .unwrap_or_default()
                .as_bytes()
                .to_vec();
            form.selected = 2;
            form.locked = 2;
            form.reference = Some(reference);
        }
    }
    pub(super) fn coordinate(
        &mut self,
        wid: u64,
        op: &str,
        args: &serde_json::Value,
    ) -> io::Result<serde_json::Value> {
        let bytes = wire::request(
            &self.state,
            &[
                "coordinate",
                &wid.to_string(),
                op,
                &wire::hex(&serde_json::to_vec(args).map_err(io::Error::other)?),
            ],
        )?;
        serde_json::from_slice(&bytes).map_err(io::Error::other)
    }
    pub(super) fn open_form(&mut self, kind: &str) {
        if matches!(kind, "Messages" | "Decisions") {
            match self.coordinate(self.snapshot.active, "context", &serde_json::json!({})) {
                Ok(v) => self.coordination = v,
                Err(e) => {
                    self.notice = e.to_string();
                    return;
                }
            }
        }
        let w = self.snapshot.workspace();
        let cwd = w.map(|w| w.cwd.clone()).unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .into()
        });
        let meta = w.map(|w| w.meta.clone()).unwrap_or_default();
        let values: Vec<(&'static str, String)> = match kind {
            "Search" => vec![("Find workspace", String::new())],
            "Send message" => vec![
                ("To: workspace ID or user", String::new()),
                ("Message", String::new()),
            ],
            "Answer decision" => vec![
                ("Question (read only)", String::new()),
                ("Recommendation (read only)", String::new()),
                ("Answer", String::new()),
            ],
            "Workspace tabs"
            | "All terminals"
            | "Messages"
            | "Decisions"
            | "Archived workspaces" => {
                vec![("Filter", String::new())]
            }
            "Start agent" => vec![(
                "Harness: codex / claude / copilot",
                if meta.conversations.len() == 1 {
                    meta.conversations[0].harness.clone()
                } else {
                    "codex".into()
                },
            )],
            "New worktree" => vec![
                ("Name", String::new()),
                ("Repository", cwd.clone()),
                ("New branch", String::new()),
                ("Base ref", "HEAD".into()),
                ("Project", meta.project.clone()),
            ],
            "Add project" => vec![(
                "Project directory",
                project_picker::directory_text(Path::new(&cwd)),
            )],
            "Edit workspace" => vec![
                ("Name", w.map(|w| w.name.clone()).unwrap_or_default()),
                ("Project", meta.project.clone()),
                ("Issue URL", meta.issue.clone()),
                ("PR URL", meta.pr.clone()),
            ],
            "Project filter" => vec![("Project (empty = all)", self.prefs.project.clone())],
            _ => return,
        };
        self.form = Some(Form {
            kind: kind.into(),
            target: self.snapshot.active,
            values: values
                .into_iter()
                .map(|(k, v)| (k, v.into_bytes()))
                .collect(),
            selected: 0,
            result: 0,
            meta,
            reference: None,
            locked: 0,
            project: (kind == "Add project")
                .then(|| project_picker::Picker::new(PathBuf::from(cwd))),
        });
        self.menu = false;
    }
    fn picker_items(&self, kind: &str, target: u64, query: &str) -> Vec<Pick> {
        let mut out = Vec::new();
        for w in self.snapshot.workspaces.iter().filter(|w| !w.meta.archived) {
            match kind {
                "Search" => out.push(Pick {
                    label: format!("{} · {}", w.name, w.meta.project),
                    wid: w.id,
                    tab: 0,
                    reference: None,
                }),
                "Workspace tabs" | "All terminals" if kind == "All terminals" || w.id == target => {
                    for t in &w.tabs {
                        out.push(Pick {
                            label: format!("{} · {}", w.name, t.title),
                            wid: w.id,
                            tab: t.id,
                            reference: None,
                        });
                    }
                }
                _ => {}
            }
        }
        if kind == "Archived workspaces" {
            for w in self.snapshot.workspaces.iter().filter(|w| w.meta.archived) {
                out.push(Pick {
                    label: w.name.clone(),
                    wid: w.id,
                    tab: 0,
                    reference: None,
                });
            }
        }
        if matches!(kind, "Messages" | "Decisions") {
            let key = if kind == "Messages" {
                "messages"
            } else {
                "decisions"
            };
            if let Some(items) = self.coordination[key].as_array() {
                for item in items {
                    let label = if kind == "Messages" {
                        format!(
                            "{} · {}",
                            if !item["acknowledged"].is_null() {
                                "acknowledged".to_owned()
                            } else if !item["native_surfaced"].is_null() {
                                "surfaced to agent".to_owned()
                            } else {
                                let state = item["delivery"]["outcome"].as_str().unwrap_or("saved");
                                if item["surfaced"].is_null() {
                                    state.to_owned()
                                } else {
                                    format!("{state} · viewed by user")
                                }
                            },
                            item["body"].as_str().unwrap_or_default()
                        )
                    } else {
                        format!(
                            "{} · {}",
                            if item["answer"].is_null() {
                                "pending"
                            } else {
                                "answered"
                            },
                            item["question"].as_str().unwrap_or_default()
                        )
                    };
                    out.push(Pick {
                        label,
                        wid: target,
                        tab: 0,
                        reference: Some(item.clone()),
                    });
                }
            }
        }
        out.into_iter().filter(|p| fuzzy(query, &p.label)).collect()
    }
    pub(super) fn form_key(&mut self, key: Key) {
        let (b, paste) = match key {
            Key::Bytes(b) => (b, false),
            Key::Paste(b) => (b, true),
            _ => return,
        };
        let mut f = self.form.take().unwrap();
        if !paste && b == b"\x1b" {
            return;
        }
        if let Some(project) = &mut f.project
            && !paste
            && project.key(&b, &mut f.values[0].1)
        {
            self.form = Some(f);
            return;
        }
        if !paste && b == b"\r" {
            let vals: Vec<_> = f
                .values
                .iter()
                .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
                .collect();
            match f.kind.as_str() {
                kind if picker(kind) => {
                    if let Some(pick) = self
                        .picker_items(kind, f.target, &vals[0])
                        .get(f.result)
                        .cloned()
                    {
                        if kind == "Archived workspaces"
                            && let Some(w) =
                                self.snapshot.workspaces.iter().find(|w| w.id == pick.wid)
                        {
                            let mut meta = w.meta.clone();
                            meta.archived = false;
                            self.set_meta(w.id, meta);
                        }
                        self.command(&["focus", &pick.wid.to_string(), &pick.tab.to_string()]);
                        self.focus = Focus::Terminal;
                        self.nav = false;
                        if let Some(reference) = pick.reference {
                            if kind == "Messages" {
                                let body = reference["body"].as_str().unwrap_or_default();
                                let id = reference["id"].as_str().unwrap_or_default().to_owned();
                                if let Err(e) = self.coordinate(
                                    pick.wid,
                                    "read_message",
                                    &serde_json::json!({"id":id}),
                                ) {
                                    self.notice = e.to_string();
                                    self.form = Some(f);
                                    return;
                                }
                                let mut lines=vec![format!("From {} · {}",reference["from"],id),"Read this message, then a to acknowledge. q returns without acknowledgement.".into(),String::new()];
                                lines.extend(body.lines().map(String::from));
                                self.previews.insert(
                                    (pick.wid, self.snapshot.tab),
                                    Preview {
                                        name: "Message · a acknowledges · q returns".into(),
                                        lines,
                                        offset: 0,
                                        action: Some(PreviewAction::Message { wid: pick.wid, id }),
                                    },
                                );
                            } else if reference["answer"].is_null() {
                                let mut lines=vec!["Read the question, recommendation and evidence. a opens the answer form; q cancels.".into(),String::new()];
                                for key in ["question", "recommendation", "evidence"] {
                                    lines.push(key.to_uppercase());
                                    lines.extend(
                                        reference[key]
                                            .as_str()
                                            .unwrap_or_default()
                                            .lines()
                                            .map(String::from),
                                    );
                                    lines.push(String::new());
                                }
                                self.previews.insert(
                                    (pick.wid, self.snapshot.tab),
                                    Preview {
                                        name: "Decision review · a answers · q returns".into(),
                                        lines,
                                        offset: 0,
                                        action: Some(PreviewAction::Decision {
                                            wid: pick.wid,
                                            value: reference,
                                        }),
                                    },
                                );
                            } else {
                                self.notice = format!(
                                    "Answered: {}",
                                    reference["answer"].as_str().unwrap_or_default()
                                );
                            }
                        }
                    }
                }
                "Send message" => {
                    let to = match vals[0].parse::<u64>() {
                        Ok(n) => serde_json::json!(n),
                        Err(_) => serde_json::json!(vals[0]),
                    };
                    match self.coordinate(
                        f.target,
                        "send_message",
                        &serde_json::json!({"to":to,"body":vals[1]}),
                    ) {
                        Ok(_) => {
                            self.notice =
                                "Message saved. Read messages shows its delivery state.".into()
                        }
                        Err(e) => {
                            self.notice = e.to_string();
                            self.form = Some(f);
                            return;
                        }
                    }
                }
                "Answer decision" => {
                    if let Some(reference) = &f.reference {
                        match self.coordinate(
                            f.target,
                            "answer_decision",
                            &serde_json::json!({"id":reference["id"],"answer":vals[2]}),
                        ) {
                            Ok(_) => {
                                self.notice =
                                    "Decision answer recorded; native approvals remain separate."
                                        .into()
                            }
                            Err(e) => {
                                self.notice = e.to_string();
                                self.form = Some(f);
                                return;
                            }
                        }
                    }
                }
                "Start agent" => {
                    self.command(&["native", &f.target.to_string(), &vals[0], ""]);
                    if self.notice.is_empty() {
                        self.focus = Focus::Terminal;
                        self.nav = false;
                    }
                }
                "Project filter" => {
                    self.side_scroll = None;
                    self.prefs.project = vals[0].clone();
                    self.save_preferences();
                }
                "New worktree" => {
                    self.command(&[
                        "worktree",
                        &wire::hex(vals[0].as_bytes()),
                        &wire::hex(vals[1].as_bytes()),
                        &wire::hex(vals[2].as_bytes()),
                        &wire::hex(vals[3].as_bytes()),
                        &wire::hex(vals[4].as_bytes()),
                    ]);
                }
                "Add project" => {
                    if vals[0].is_empty() {
                        self.notice = "Choose a project directory".into();
                        self.form = Some(f);
                        return;
                    }
                    let directory = f.project.as_ref().unwrap().resolve(&vals[0]);
                    self.command(&[
                        "add-project",
                        &wire::hex(directory.to_string_lossy().as_bytes()),
                        "",
                    ]);
                    if self.notice.is_empty() {
                        self.focus = Focus::Terminal;
                        self.nav = false;
                    }
                }
                "Edit workspace" => {
                    self.command(&[
                        "rename",
                        &f.target.to_string(),
                        &wire::hex(vals[0].as_bytes()),
                    ]);
                    if self.notice.is_empty() {
                        let mut meta = f.meta.clone();
                        meta.project = vals[1].clone();
                        meta.issue = vals[2].clone();
                        meta.pr = vals[3].clone();
                        self.set_meta(f.target, meta);
                    }
                }
                _ => {}
            }
            if !self.notice.is_empty()
                && !matches!(f.kind.as_str(), "Send message" | "Answer decision")
            {
                self.form = Some(f);
            }
            return;
        }
        if !paste && b == b"\t" {
            f.selected = ((f.selected + 1) % f.values.len()).max(f.locked);
        } else if !paste && (b == b"\x1b[B" || b == b"\x1b[A") {
            if picker(&f.kind) {
                let count = self
                    .picker_items(&f.kind, f.target, &String::from_utf8_lossy(&f.values[0].1))
                    .len();
                f.result = if b == b"\x1b[B" {
                    (f.result + 1).min(count.saturating_sub(1))
                } else {
                    f.result.saturating_sub(1)
                };
            } else {
                f.selected = if b == b"\x1b[B" {
                    ((f.selected + 1) % f.values.len()).max(f.locked)
                } else {
                    f.selected.saturating_sub(1).max(f.locked)
                };
            }
        } else {
            let value = &mut f.values[f.selected].1;
            if !paste && b == b"\x7f" {
                value.pop();
                while !value.is_empty() && std::str::from_utf8(value).is_err() {
                    value.pop();
                }
            } else if !paste && b == b"\x15" {
                value.clear();
            } else if paste {
                let v = b.strip_prefix(b"\x1b[200~").unwrap_or(&b);
                let v = v.strip_suffix(b"\x1b[201~").unwrap_or(v);
                value.extend(
                    v.iter()
                        .filter(|b| **b >= 32 && **b != 127)
                        .take(4096usize.saturating_sub(value.len())),
                );
            } else if b.len() == 1 && b[0] >= 32 && value.len() < 4096 {
                value.extend_from_slice(&b);
            }
            f.result = 0;
            if let Some(project) = &mut f.project {
                project.update(&f.values[0].1);
            }
        }
        self.form = Some(f);
    }
    pub(super) fn set_meta(&mut self, id: u64, meta: CardMeta) {
        if let Ok(json) = serde_json::to_vec(&meta) {
            self.command(&["metadata", &id.to_string(), &wire::hex(&json)]);
        }
    }
    pub(super) fn workflow_key(&mut self, b: &[u8]) -> bool {
        match b {
            b"&" => self.open_arcade(),
            b"|" => {
                self.pane_action("split-right", Some("move"));
            }
            b"_" => {
                self.pane_action("split-below", Some("move"));
            }
            b"\\" => {
                self.pane_action("split-right", Some("new"));
            }
            b"=" => {
                self.pane_action("split-below", Some("new"));
            }
            b"~" => {
                self.pane_action("move", None);
            }
            b"Z" => {
                self.pane_action("zoom", None);
            }
            b"X" => {
                self.pane_action("merge", None);
            }
            b"s" => self.copy_screenshot(),
            b"/" => self.open_form("Search"),
            b"Q" => self.open_search(search::Kind::Files),
            b"H" => self.open_search(search::Kind::Contents),
            b"?" => self.open_search(search::Kind::Output),
            b"!" => self.open_task(),
            b";" => self.open_problems(),
            b"(" => self.command_history("previous"),
            b")" => self.command_history("next"),
            b"Y" => self.command_history("copy"),
            b"S" => self.open_form("Start agent"),
            b"N" => self.open_form("New worktree"),
            b"G" => self.open_form("Add project"),
            b"F" => {
                if let Some(e) = self.explorers.get_mut(&self.snapshot.active) {
                    e.load(e.root.clone());
                }
                self.git_checked = Instant::now() - Duration::from_secs(4);
                self.history_refresh();
            }
            b"c" => {
                self.previews
                    .remove(&(self.snapshot.active, self.snapshot.tab));
                self.focus = Focus::Terminal;
                self.nav = false;
                self.scroll_terminal(-(self.terminal_layout().rows as i64 - 1));
            }
            b"T" => self.open_form("Workspace tabs"),
            b"w" => self.open_form("All terminals"),
            b"W" => self.resume_saved_chat(),
            b"m" => self.open_form("Send message"),
            b"I" => self.open_form("Messages"),
            b"D" => self.open_form("Decisions"),
            b"a" => self.open_form("Archived workspaces"),
            b"A" => {
                if let Some(w) = self.snapshot.workspace() {
                    let mut meta = w.meta.clone();
                    meta.archived = true;
                    self.set_meta(w.id, meta);
                }
            }
            b"R" => {
                if let Ok(path) = os::executable_path() {
                    self.command(&["refresh", &wire::hex(path.to_string_lossy().as_bytes())]);
                    if self.notice.is_empty() {
                        self.refresh = true;
                    }
                }
            }
            b"K" => self.open_update(),
            b"6" => self.open_remote_tool(remote_tools::Action::Upload),
            b"7" => self.open_remote_tool(remote_tools::Action::Download),
            b"8" => self.open_remote_tool(remote_tools::Action::OpenLocal),
            b"9" => self.open_remote_tool(remote_tools::Action::Ports),
            b"<" => self.history(-1),
            b">" => self.history(1),
            b"e" => self.open_form("Edit workspace"),
            b"P" => self.open_form("Project filter"),
            b"L" => self.toggle_sidebar_grouping(),
            b"O" => self.pet_open(),
            b"o" => {
                self.prefs.pet = !self.prefs.pet;
                self.save_preferences();
            }
            b"M" => {
                self.prefs.reduced_motion = !self.prefs.reduced_motion;
                self.save_preferences();
            }
            b"U" => self.start_screensaver(),
            b"%" | b"^" => {
                self.prefs.pet_kind = if b == b"^" {
                    crate::pet::Kind::Flere
                } else {
                    crate::pet::Kind::Duck
                };
                self.prefs.screensaver_mascot = if b == b"^" {
                    crate::pet::ScreensaverMascot::Flere
                } else {
                    crate::pet::ScreensaverMascot::Duck
                };
                self.notice = if b == b"^" {
                    "Mascot: Flere-Imsaho · click to pet, drag to throw".into()
                } else {
                    "Mascot: Duck".into()
                };
                self.save_preferences();
                self.start_screensaver();
            }
            b"V" => self.cycle_screensaver(),
            b"p" => {
                if let Some(w) = self.snapshot.workspace() {
                    let mut meta = w.meta.clone();
                    meta.pinned = !meta.pinned;
                    self.set_meta(w.id, meta);
                }
            }
            b"1" | b"2" | b"3" | b"4" | b"5" => {
                if let Some(w) = self.snapshot.workspace() {
                    let mut meta = w.meta.clone();
                    meta.status = Workflow::ALL[(b[0] - b'1') as usize];
                    self.set_meta(w.id, meta);
                }
            }
            b"i" => {
                self.prefs.inspector = self.prefs.inspector.next();
                self.save_preferences();
            }
            b"g" => {
                self.prefs.inspector = Inspector::Git;
                self.focus = Focus::Files;
                self.save_preferences();
                self.git_checked = Instant::now() - Duration::from_secs(4);
            }
            b"v" => {
                self.prefs.inspector = Inspector::Details;
                self.focus = Focus::Files;
                self.save_preferences();
            }
            b"z" => {
                self.prefs.zoom = !self.prefs.zoom;
                self.save_preferences();
            }
            b"[" | b"]" | b"{" | b"}" => {
                let value = if b == b"[" || b == b"]" {
                    &mut self.prefs.left
                } else {
                    &mut self.prefs.right
                };
                *value = if b == b"[" || b == b"{" {
                    value.saturating_sub(2).max(16)
                } else {
                    (*value + 2).min(70)
                };
                self.save_preferences();
            }
            b"B" => {
                self.board = !self.board;
            }
            b"r" => {
                self.prefs = Preferences::default();
                self.save_preferences();
            }
            b"C" => {
                self.prefs.compact_cards = !self.prefs.compact_cards;
                self.side_scroll = None;
                self.save_preferences();
            }
            b"J" => self.next_attention(),
            _ => return false,
        }
        self.menu = false;
        true
    }
    pub(super) fn tick_git(&mut self) -> bool {
        let mut dirty = false;
        if let Some(rx) = &self.git_job
            && let Ok(result) = rx.try_recv()
        {
            self.git_views.insert(result.id, result.view);
            if let Some(diff) = result.diff {
                self.previews.insert(
                    (result.id, self.snapshot.tab),
                    Preview {
                        name: "Git diff · q returns".into(),
                        lines: diff.lines().map(String::from).collect(),
                        offset: 0,
                        action: None,
                    },
                );
            }
            self.git_job = None;
            dirty = true;
        }
        if self.prefs.inspector == Inspector::Git
            && self.git_job.is_none()
            && (self.git_workspace != self.snapshot.active
                || self.git_checked.elapsed() > Duration::from_secs(3))
            && let Some(w) = self.snapshot.workspace()
        {
            let id = w.id;
            let cwd = PathBuf::from(&w.cwd);
            let (tx, rx) = std::sync::mpsc::channel();
            self.git_job = Some(rx);
            std::thread::spawn(move || {
                let view = crate::git::inspect(&cwd);
                let _ = tx.send(GitResult {
                    id,
                    view,
                    diff: None,
                });
            });
            self.git_workspace = id;
            self.git_checked = Instant::now();
        }
        dirty
    }
    pub(super) fn inspector_move(&mut self, delta: isize) {
        if self.prefs.inspector == Inspector::Files {
            let rows = self.file_rows();
            if let Some(e) = self.explorers.get_mut(&self.snapshot.active) {
                e.selected = (e.selected as isize + delta)
                    .max(0)
                    .min(e.entries.len().saturating_sub(1) as isize)
                    as usize;
                e.start(rows);
            }
        } else if self.prefs.inspector == Inspector::Git {
            self.history_move(delta);
        } else {
            let max = self.detail_lines().len().saturating_sub(self.detail_rows());
            let offset = self.details_scroll.entry(self.snapshot.active).or_default();
            *offset = offset.saturating_add_signed(delta).min(max);
        }
    }
    pub(super) fn inspector_enter(&mut self, diff: bool) {
        if self.prefs.inspector == Inspector::Files {
            self.file_enter();
            return;
        }
        if self.prefs.inspector != Inspector::Git {
            return;
        }
        self.git_activate(diff);
    }
    pub(super) fn draw_form(&self, c: &mut Canvas) {
        let Some(f) = &self.form else {
            return;
        };
        if let Some(project) = &f.project {
            project.draw(c, self.layout, &f.values[0].1);
            return;
        }
        let l = self.layout;
        let w = 70.min(l.width);
        let is_picker = picker(&f.kind);
        // At the minimum eight-row viewport, use the spare top margin so a
        // picker still has separate filter, selected-result and help rows.
        let available = (l.height - 2).max(if is_picker { 7 } else { 0 });
        let h = (f.values.len() * 2 + 6 + if is_picker { 8 } else { 0 }).min(available);
        let x = (l.width - w) / 2;
        let y = (l.height - h) / 2;
        c.fill(x, y, w, h, style(TEXT, PANEL, false));
        c.border(x, y, w, h, BORDER);
        c.text(x + 2, y + 1, w - 4, &f.kind, style(CYAN, PANEL, true));
        let visible_fields = (h.saturating_sub(5) / 2).max(1);
        let field_start = f.selected.saturating_sub(visible_fields - 1);
        let label_width = f
            .values
            .iter()
            .map(|(label, _)| label.chars().count())
            .max()
            .unwrap_or(0)
            .min((w - 6) / 2);
        for (i, (index, (label, value))) in f
            .values
            .iter()
            .enumerate()
            .skip(field_start)
            .take(visible_fields)
            .enumerate()
        {
            let row = y + 3 + i * 2;
            if row >= y + h - 2 {
                break;
            }
            let selected = index == f.selected;
            let bg = if selected { ACTIVE_BG } else { PANEL };
            c.fill(x + 1, row, w - 2, 1, style(TEXT, bg, false));
            if selected {
                c.text(x + 1, row, 1, "▌", style(CYAN, bg, true));
            }
            let stacked = label.chars().count() > 18 || w < 45;
            let value_width = if stacked {
                w - 6
            } else {
                w.saturating_sub(label_width + 7)
            };
            c.text(
                x + 3,
                if stacked { row - 1 } else { row },
                if stacked { w - 6 } else { label_width },
                &chrome::elide(label, if stacked { w - 6 } else { label_width }),
                style(MUTED, if stacked { PANEL } else { bg }, false),
            );
            let value = String::from_utf8_lossy(value);
            let shown = chrome::tail(&value, value_width);
            c.text(
                if stacked { x + 3 } else { x + 5 + label_width },
                row,
                value_width,
                &shown,
                style(TEXT, bg, selected),
            );
        }
        if is_picker {
            let query = String::from_utf8_lossy(&f.values[0].1);
            let result_y = if h < 10 { 4 } else { 5 };
            let visible = h.saturating_sub(result_y + 2);
            let start = f.result.saturating_sub(visible.saturating_sub(1));
            let items = self.picker_items(&f.kind, f.target, &query);
            if items.is_empty() && visible > 0 {
                c.text(
                    x + 3,
                    y + result_y,
                    w - 6,
                    "No matches",
                    style(MUTED, PANEL, false),
                );
            }
            for (i, pick) in items.iter().enumerate().skip(start).take(visible) {
                let row = y + result_y + i - start;
                let selected = i == f.result;
                let bg = if selected { ACTIVE_BG } else { PANEL };
                c.fill(x + 1, row, w - 2, 1, style(TEXT, bg, false));
                if selected {
                    c.text(x + 1, row, 1, "▌", style(CYAN, bg, true));
                }
                c.text(
                    x + 3,
                    row,
                    w - 6,
                    &chrome::elide(
                        &format!("{} {}", if selected { "›" } else { " " }, pick.label),
                        w - 6,
                    ),
                    style(if selected { CYAN } else { TEXT }, bg, selected),
                );
            }
        }
        c.text(
            x + 2,
            y + h - 2,
            w - 4,
            if is_picker {
                "Type to search · ↑/↓ select · Enter open · Esc back"
            } else {
                "Tab field · Ctrl+U clear · Enter apply · Esc cancel"
            },
            style(MUTED, PANEL, false),
        );
    }

    fn board_cards(&self, status: Workflow) -> Vec<&crate::model::WorkspaceView> {
        self.snapshot
            .workspaces
            .iter()
            .filter(|w| {
                !w.meta.archived
                    && w.meta.status == status
                    && (self.prefs.project.is_empty() || w.meta.project == self.prefs.project)
            })
            .collect()
    }
    fn board_visible(&self) -> usize {
        // Three content rows per card; the final card needs no trailing spacer.
        self.layout.height.saturating_sub(5) / CARD_PITCH
    }
    fn board_offset(&self, cards: &[&crate::model::WorkspaceView]) -> usize {
        let visible = self.board_visible().max(1);
        cards
            .iter()
            .position(|w| w.id == self.snapshot.active)
            .unwrap_or(0)
            .saturating_sub(visible - 1)
    }
    pub(super) fn draw_board(&self, c: &mut Canvas) {
        if !self.board {
            return;
        }
        let l = self.layout;
        let width = l.width - l.left - l.right;
        let cols = if width >= 100 { 5 } else { 1 };
        let slot = width / cols;
        c.fill(l.left, 1, width, l.height - 2, style(TEXT, BG, false));
        for i in 0..cols {
            let status = if cols == 1 {
                Workflow::ALL[self.board_column]
            } else {
                Workflow::ALL[i]
            };
            let x = l.left + i * slot;
            let (fg, _) = tint(status);
            c.border(
                x,
                1,
                slot,
                l.height - 2,
                if status == Workflow::ALL[self.board_column] {
                    CYAN
                } else {
                    BORDER
                },
            );
            let cards = self.board_cards(status);
            c.text(
                x + 2,
                2,
                slot - 4,
                &format!("{} · {}", status.label(), cards.len()),
                style(fg, BG, true),
            );
            let offset = self.board_offset(&cards);
            for (j, w) in cards
                .iter()
                .skip(offset)
                .take(self.board_visible())
                .enumerate()
            {
                let y = 4 + j * CARD_PITCH;
                self.draw_workspace_card(c, w, x + 1, y, slot - 2);
            }
        }
    }
    pub(super) fn board_click(&mut self, x: usize, y: usize) -> bool {
        if !self.board || x < self.layout.left || x >= self.layout.width - self.layout.right {
            return false;
        }
        let width = self.layout.width - self.layout.left - self.layout.right;
        let cols = if width >= 100 { 5 } else { 1 };
        if cols == 5 {
            self.board_column = ((x - self.layout.left) / (width / 5)).min(4);
        }
        let visible = self.board_visible();
        if y >= 4 && y < 4 + visible * CARD_PITCH && (y - 4) % CARD_PITCH < CARD_HEIGHT {
            let cards = self.board_cards(Workflow::ALL[self.board_column]);
            let index = self.board_offset(&cards) + (y - 4) / CARD_PITCH;
            if let Some(id) = cards.get(index).map(|w| w.id) {
                self.command(&["focus", &id.to_string(), "0"]);
            }
        }
        true
    }
    pub(super) fn board_key(&mut self, b: &[u8]) -> bool {
        if !self.board {
            return false;
        }
        match b {
            b"h" | b"\x1b[D" => self.board_column = self.board_column.saturating_sub(1),
            b"l" | b"\x1b[C" => self.board_column = (self.board_column + 1).min(4),
            b"j" | b"k" | b"\x1b[A" | b"\x1b[B" => {
                let ids: Vec<_> = self
                    .board_cards(Workflow::ALL[self.board_column])
                    .iter()
                    .map(|w| w.id)
                    .collect();
                if !ids.is_empty() {
                    let next = match ids.iter().position(|id| *id == self.snapshot.active) {
                        Some(current) if b == b"j" || b == b"\x1b[B" => (current + 1) % ids.len(),
                        Some(current) => current.saturating_sub(1),
                        None => 0,
                    };
                    self.command(&["focus", &ids[next].to_string(), "0"]);
                }
            }
            b"\r" | b"\x1b" | b"B" => {
                self.board = false;
                self.focus = Focus::Terminal;
                self.nav = false;
            }
            _ => return false,
        }
        true
    }
}

pub(super) fn project_color(name: &str) -> Color {
    let colors = [
        Color::Rgb(139, 182, 242),
        Color::Rgb(168, 157, 221),
        Color::Rgb(120, 193, 179),
        Color::Rgb(208, 154, 184),
        Color::Rgb(198, 178, 136),
    ];
    if name.is_empty() {
        MUTED
    } else {
        colors[name
            .bytes()
            .fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(b as usize))
            % colors.len()]
    }
}
