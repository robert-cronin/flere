//! Frozen, exact-target context actions. Opening a menu never selects a child or
//! forwards pointer/key bytes; an explicit choice revalidates a fresh snapshot.
use super::*;

#[derive(Clone)]
enum Target {
    Mascot(crate::pet::Kind),
    Workspace(u64),
    Tab {
        workspace: u64,
        tab: u64,
        run: String,
    },
    File {
        workspace: u64,
        root: PathBuf,
        path: PathBuf,
        directory: bool,
    },
    Git {
        key: String,
        text: String,
        sha: Option<String>,
    },
    Terminal,
}
#[derive(Clone, Copy)]
enum Action {
    Arcade,
    Pet,
    FocusWorkspace,
    Expand(bool),
    Pin(bool),
    Details,
    ActivateTab,
    CloseTab,
    OpenDirectory,
    OpenEditor,
    PreviewImage,
    CopyPath,
    Screenshot,
    Actions,
    GitOpen,
    GitPreview,
    CopySha,
    RefreshGit,
}
struct Item {
    label: String,
    action: Action,
}
impl Item {
    fn new(label: impl Into<String>, action: Action) -> Self {
        Self {
            label: label.into(),
            action,
        }
    }
}
#[derive(Clone, Copy)]
struct Rect {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}
pub(super) struct Menu {
    epoch: String,
    origin: (u64, u64, String),
    focus: Focus,
    layout: Layout,
    pane_revision: Option<u64>,
    target: Target,
    title: &'static str,
    subtitle: String,
    items: Vec<Item>,
    selected: usize,
    pressed: Option<usize>,
    rect: Rect,
}
impl Menu {
    fn visible(&self) -> usize {
        self.rect.height.saturating_sub(5).max(1)
    }
    fn start(&self) -> usize {
        self.selected.saturating_sub(self.visible() - 1)
    }
    fn hit(&self, x: usize, y: usize) -> Option<usize> {
        let r = self.rect;
        if x <= r.x || x >= r.x + r.width - 1 || y < r.y + 4 || y >= r.y + r.height - 1 {
            return None;
        }
        let item = self.start() + y - r.y - 4;
        (item < self.items.len()).then_some(item)
    }
    fn contains(&self, x: usize, y: usize) -> bool {
        let r = self.rect;
        (r.x..r.x + r.width).contains(&x) && (r.y..r.y + r.height).contains(&y)
    }
    fn matches(&self, ui: &Ui) -> bool {
        if self.epoch != ui.snapshot.epoch
            || self.layout != ui.layout
            || self.pane_revision != ui.snapshot.split.as_ref().map(|s| s.revision)
            || self.focus != ui.focus
            || self.origin != ui.context_location()
            || ui.menu
            || ui.arcade.is_some()
            || ui.form.is_some()
            || ui.confirm.is_some()
            || ui.card_popup_pinned()
            || ui.image_view()
            || ui.pet.controls.is_some()
        {
            return false;
        }
        let workspace = |id| {
            ui.snapshot
                .workspaces
                .iter()
                .find(|w| w.id == id && !w.meta.archived)
        };
        match &self.target {
            Target::Mascot(kind) => ui.pet_room() && ui.prefs.pet_kind == *kind,
            Target::Workspace(id) => workspace(*id).is_some(),
            Target::Tab {
                workspace: wid,
                tab,
                run,
            } => workspace(*wid)
                .is_some_and(|w| w.tabs.iter().any(|t| t.id == *tab && t.run == *run)),
            Target::File {
                workspace: wid,
                root,
                path,
                directory,
            } => {
                workspace(*wid).is_some()
                    && ui.explorers.get(wid).is_some_and(|e| {
                        e.root == *root
                            && e.entries
                                .iter()
                                .any(|(_, p, dir)| p == path && dir == directory)
                    })
            }
            Target::Git { key, text, .. } => {
                ui.prefs.inspector == Inspector::Git && ui.git_context_exists(key, text)
            }
            Target::Terminal => workspace(self.origin.0).is_some(),
        }
    }
}
impl Ui {
    fn context_location(&self) -> (u64, u64, String) {
        (
            self.snapshot.active,
            self.snapshot.tab,
            self.snapshot
                .session()
                .map(|t| t.run.clone())
                .unwrap_or_default(),
        )
    }
    pub(super) fn context_reconcile(&mut self) -> bool {
        if self
            .context_menu
            .as_ref()
            .is_some_and(|menu| !menu.matches(self))
        {
            self.context_menu = None;
            return true;
        }
        false
    }
    pub(super) fn context_key(&mut self, key: &Key) -> bool {
        if let Key::Context { x, y } = key {
            self.context_menu = None;
            self.open_context_menu(*x, *y);
            return true;
        }
        let Some(mut menu) = self.context_menu.take() else {
            return false;
        };
        if !menu.matches(self) {
            return true;
        }
        if matches!(key, Key::Bytes(_) | Key::Paste(_)) {
            menu.pressed = None;
        }
        let mut selected = None;
        match key {
            Key::Bytes(bytes) => match bytes.as_slice() {
                b"\x1b" => return true,
                b"j" | b"\x1b[B" | b"\x1bOB" => {
                    menu.selected = (menu.selected + 1) % menu.items.len()
                }
                b"k" | b"\x1b[A" | b"\x1bOA" => {
                    menu.selected = (menu.selected + menu.items.len() - 1) % menu.items.len()
                }
                b"\x1b[H" | b"\x1bOH" => menu.selected = 0,
                b"\x1b[F" | b"\x1bOF" => menu.selected = menu.items.len() - 1,
                b"\r" => selected = Some(menu.selected),
                _ => {}
            },
            Key::Mouse { x, y } => {
                if !menu.contains(*x, *y) {
                    return true;
                }
                menu.pressed = menu.hit(*x, *y);
                if let Some(index) = menu.pressed {
                    menu.selected = index;
                }
            }
            Key::Release { x, y } => {
                if let Some(pressed) = menu.pressed.take()
                    && menu.hit(*x, *y) == Some(pressed)
                {
                    selected = Some(pressed);
                }
            }
            Key::Hover { x, y } => {
                if let Some(index) = menu.hit(*x, *y) {
                    menu.selected = index;
                }
            }
            Key::Drag { .. } => menu.pressed = None,
            Key::Wheel { delta, .. } => {
                menu.selected = menu
                    .selected
                    .saturating_add_signed(*delta as isize)
                    .min(menu.items.len() - 1);
                menu.pressed = None;
            }
            _ => {}
        }
        if let Some(index) = selected {
            self.context_action(&menu, menu.items[index].action);
        } else {
            self.context_menu = Some(menu);
        }
        true
    }
    fn open_context_menu(&mut self, x: usize, y: usize) {
        if self.menu
            || self.form.is_some()
            || self.confirm.is_some()
            || self.pending_close.is_some()
            || self.card_popup_pinned()
            || self.pet.controls.is_some()
            || self.image_view()
            || self.paste_target.is_some()
            || self.board
            || x >= self.layout.width
            || y >= self.layout.height
        {
            return;
        }
        if self.card_popup_visible() && self.card_inside(x, y) {
            self.card_close();
            return; // Dismiss the foreground popup; never target a concealed row.
        }
        let mut title = "Workspace";
        let mut subtitle;
        let mut items = Vec::new();
        let target;
        let cards_covered = self.layout.right == 0 && self.focus == Focus::Files;
        if self.pet_visible() && self.pet_hit(x, y) {
            title = "Mascot";
            subtitle = self.prefs.pet_kind.label().into();
            target = Target::Mascot(self.prefs.pet_kind);
            items.extend([
                Item::new("Play Context Ruins", Action::Arcade),
                Item::new("Play with mascot", Action::Pet),
            ]);
        } else if let Some(cursor) = self.card_at(x, y).filter(|_| !cards_covered) {
            let Some(w) = self.snapshot.workspaces.iter().find(|w| w.id == cursor.wid) else {
                return;
            };
            subtitle = w.name.clone();
            if cursor.tab == 0 {
                target = Target::Workspace(w.id);
                let expanded = self.prefs.expanded_cards.contains(&w.id);
                items.extend([
                    Item::new("Focus workspace", Action::FocusWorkspace),
                    Item::new(
                        if expanded {
                            "Collapse tabs"
                        } else {
                            "Expand tabs"
                        },
                        Action::Expand(!expanded),
                    ),
                    Item::new(
                        if w.meta.pinned {
                            "Unpin workspace"
                        } else {
                            "Pin workspace"
                        },
                        Action::Pin(!w.meta.pinned),
                    ),
                    Item::new("Workspace details", Action::Details),
                ]);
            } else {
                let Some(tab) = w
                    .tabs
                    .iter()
                    .find(|t| t.id == cursor.tab && t.run == cursor.run)
                else {
                    return;
                };
                title = "Tab";
                subtitle = format!("{} · {}", tab.title, w.name);
                target = Target::Tab {
                    workspace: w.id,
                    tab: tab.id,
                    run: tab.run.clone(),
                };
            }
        } else {
            let l = self.layout;
            if l.left == 0 && self.focus == Focus::Cards {
                return; // The full-width Cards overlay owns its empty space too.
            }
            let (inspector_x, inspector_width) = self.inspector_size();
            let in_inspector = (l.right > 0 || self.focus == Focus::Files)
                && x > inspector_x
                && x < inspector_x + inspector_width - 1;
            if in_inspector
                && self.prefs.inspector == Inspector::Files
                && y >= 5
                && y < 5 + self.file_rows()
            {
                let Some(explorer) = self.explorers.get(&self.snapshot.active) else {
                    return;
                };
                let index = explorer.start(self.file_rows()) + y - 5;
                let Some((name, path, directory)) = explorer.entries.get(index) else {
                    return;
                };
                title = if *directory { "Directory" } else { "File" };
                subtitle = name.clone();
                target = Target::File {
                    workspace: self.snapshot.active,
                    root: explorer.root.clone(),
                    path: path.clone(),
                    directory: *directory,
                };
                if *directory {
                    items.push(Item::new("Open directory", Action::OpenDirectory));
                } else {
                    if image_preview::image_path(path) {
                        items.push(Item::new("Preview image", Action::PreviewImage));
                    }
                    items.push(Item::new("Open in editor", Action::OpenEditor));
                }
                items.push(Item::new("Copy path", Action::CopyPath));
            } else if in_inspector && self.prefs.inspector == Inspector::Git {
                let Some(row) = self.git_context_row(x, y) else {
                    return;
                };
                title = "Git";
                subtitle = row.text.clone();
                items.push(Item::new("Open / expand", Action::GitOpen));
                if row.preview {
                    items.push(Item::new("Preview diff", Action::GitPreview));
                }
                if row.sha.is_some() {
                    items.push(Item::new("Copy commit SHA", Action::CopySha));
                }
                items.push(Item::new("Refresh Git", Action::RefreshGit));
                target = Target::Git {
                    key: row.key,
                    text: row.text,
                    sha: row.sha,
                };
            } else if !in_inspector && let Some(id) = self.pane_tab_at(x, y) {
                let Some(w) = self.snapshot.workspace() else {
                    return;
                };
                let Some(tab) = w.tabs.iter().find(|t| t.id == id) else {
                    return;
                };
                title = "Tab";
                subtitle = format!("{} · {}", tab.title, w.name);
                target = Target::Tab {
                    workspace: w.id,
                    tab: tab.id,
                    run: tab.run.clone(),
                };
            } else if !in_inspector && self.pane_at(x, y, false).is_some() {
                title = "Terminal";
                subtitle = self
                    .snapshot
                    .workspace()
                    .map(|w| w.name.clone())
                    .unwrap_or_default();
                target = Target::Terminal;
                items.extend([
                    Item::new("Copy Flere screenshot", Action::Screenshot),
                    Item::new("Flere actions", Action::Actions),
                ]);
            } else {
                return;
            }
        }
        if matches!(target, Target::Tab { .. }) {
            items.extend([
                Item::new("Activate tab", Action::ActivateTab),
                Item::new("Close tab", Action::CloseTab),
                Item::new("Workspace details", Action::Details),
            ]);
        }
        let label_width = items
            .iter()
            .map(|item| item.label.chars().count())
            .max()
            .unwrap_or(0);
        let width = (label_width + 6)
            .clamp(24, 36)
            .min(self.layout.width.saturating_sub(2));
        let height = (items.len() + 5).min(self.layout.height);
        let rect = Rect {
            x: x.min(self.layout.width - width),
            y: y.min(self.layout.height - height),
            width,
            height,
        };
        self.card_close();
        self.selection = None;
        self.drag = None;
        self.smooth_scroll = None;
        self.paste_target = None;
        self.git_hover = None;
        self.context_menu = Some(Menu {
            epoch: self.snapshot.epoch.clone(),
            origin: self.context_location(),
            focus: self.focus,
            layout: self.layout,
            pane_revision: self.snapshot.split.as_ref().map(|s| s.revision),
            target,
            title,
            subtitle: wire::passive(&subtitle),
            items,
            selected: 0,
            pressed: None,
            rect,
        });
    }
    fn context_copy(&mut self, text: &str) {
        match selection::clipboard_write(text) {
            Ok(sequence) => {
                self.clipboard = Some(sequence);
                self.notice = "Sent to clipboard".into();
            }
            Err(error) => self.notice = error.into(),
        }
    }
    fn context_focus_tab(&mut self, menu: &Menu, workspace: u64, tab: u64, run: &str) -> bool {
        self.command(&[
            "focus-exact",
            &menu.epoch,
            &workspace.to_string(),
            &tab.to_string(),
            run,
        ]);
        let exact = self.notice.is_empty()
            && self.snapshot.epoch == menu.epoch
            && self.snapshot.active == workspace
            && (tab == 0
                || (self.snapshot.tab == tab
                    && self
                        .snapshot
                        .session()
                        .is_some_and(|t| t.id == tab && t.run == run)));
        if !exact && self.notice.is_empty() {
            self.notice = "Context target changed; open the menu again".into();
        }
        exact
    }
    fn context_action(&mut self, menu: &Menu, action: Action) {
        let fresh = self.read_snapshot();
        let Ok(fresh) = fresh else {
            self.notice = "Could not recheck context target".into();
            return;
        };
        self.snapshot(fresh);
        if !menu.matches(self) {
            self.notice = "Context target changed; open the menu again".into();
            return;
        }
        match (&menu.target, action) {
            (Target::Mascot(_), Action::Arcade) => self.open_arcade(),
            (Target::Mascot(_), Action::Pet) => self.pet_open(),
            (Target::Workspace(id), Action::FocusWorkspace) => {
                if self.context_focus_tab(menu, *id, 0, "") {
                    self.cards.cursor = Some(cards::Cursor {
                        wid: *id,
                        tab: 0,
                        run: String::new(),
                    });
                    self.side_scroll = None;
                    self.focus = Focus::Cards;
                    self.nav = true;
                }
            }
            (Target::Workspace(id), Action::Expand(expanded)) => {
                if self.prefs.expanded_cards.contains(id) != expanded {
                    self.card_toggle(*id);
                }
            }
            (Target::Workspace(id), Action::Pin(pinned)) => {
                let result = self.coordinate(
                    self.snapshot.active,
                    "update_workspace",
                    &serde_json::json!({
                        "workspace": id, "expected_epoch": menu.epoch, "pinned": pinned,
                    }),
                );
                match result {
                    Ok(_) => {
                        if let Ok(fresh) = self.read_snapshot() {
                            self.snapshot(fresh);
                        }
                    }
                    Err(error) => self.notice = error.to_string(),
                }
            }
            (Target::Workspace(id), Action::Details)
            | (Target::Tab { workspace: id, .. }, Action::Details) => self.card_open(*id, true),
            (
                Target::Tab {
                    workspace,
                    tab,
                    run,
                },
                Action::ActivateTab | Action::CloseTab,
            ) => {
                if self.context_focus_tab(menu, *workspace, *tab, run) {
                    if matches!(action, Action::CloseTab) {
                        self.request_close(*tab);
                    } else {
                        self.focus = Focus::Terminal;
                        self.nav = false;
                        self.scrollback = None;
                    }
                }
            }
            (
                Target::File {
                    workspace,
                    path,
                    directory,
                    ..
                },
                action,
            ) => {
                // Checking existence is part of the explicit path action, never
                // an idle-menu poll or an inferred selection of another entry.
                if fs::symlink_metadata(path).is_err() {
                    self.notice = "File no longer exists; refresh Files".into();
                    return;
                }
                match action {
                    Action::OpenDirectory if *directory => {
                        if let Some(explorer) = self.explorers.get_mut(workspace) {
                            explorer.load(path.clone());
                        }
                        self.focus = Focus::Files;
                    }
                    Action::PreviewImage if !*directory => self.image_open(path),
                    Action::OpenEditor if !*directory => {
                        let origin = panes::EditorOrigin {
                            epoch: menu.epoch.clone(),
                            workspace: *workspace,
                            tab: menu.origin.1,
                            run: menu.origin.2.clone(),
                            revision: menu.pane_revision.unwrap_or(0),
                        };
                        if self.editor_open(&origin, path, None) {
                            self.previews.remove(&(*workspace, self.snapshot.tab));
                            self.focus = Focus::Files;
                            self.nav = false;
                        }
                    }
                    Action::CopyPath => self.context_copy(&path.to_string_lossy()),
                    _ => {}
                }
            }
            (Target::Git { key, .. }, Action::GitOpen | Action::GitPreview) => {
                self.git_context_open(key, matches!(action, Action::GitPreview))
            }
            (Target::Git { sha: Some(sha), .. }, Action::CopySha) => self.context_copy(sha),
            (Target::Git { .. }, Action::RefreshGit) => {
                self.git_checked = Instant::now() - Duration::from_secs(4);
                self.history_refresh();
            }
            (Target::Terminal, Action::Screenshot) => self.copy_screenshot(),
            (Target::Terminal, Action::Actions) => {
                self.menu = true;
                self.menu_index = 0;
                self.menu_query.clear();
                self.menu_search = false;
            }
            _ => {}
        }
    }
    pub(super) fn draw_context_menu(&self, c: &mut Canvas) {
        let Some(menu) = &self.context_menu else {
            return;
        };
        let r = menu.rect;
        c.fill(r.x, r.y, r.width, r.height, style(TEXT, PANEL, false));
        c.border(r.x, r.y, r.width, r.height, CYAN);
        let width = r.width.saturating_sub(4);
        c.text(
            r.x + 2,
            r.y + 1,
            width,
            menu.title,
            style(CYAN, PANEL, true),
        );
        c.text(
            r.x + 2,
            r.y + 2,
            width,
            &chrome::elide(&menu.subtitle, width),
            style(MUTED, PANEL, false),
        );
        c.text(
            r.x + 1,
            r.y + 3,
            r.width - 2,
            &"─".repeat(r.width - 2),
            style(BORDER, PANEL, false),
        );
        for (index, item) in menu
            .items
            .iter()
            .enumerate()
            .skip(menu.start())
            .take(menu.visible())
        {
            let y = r.y + 4 + index - menu.start();
            let selected = index == menu.selected;
            let bg = if selected { ACTIVE_BG } else { PANEL };
            c.fill(r.x + 1, y, r.width - 2, 1, style(TEXT, bg, false));
            if selected {
                c.text(r.x + 1, y, 1, "▸", style(CYAN, bg, true));
            }
            c.text(
                r.x + 3,
                y,
                r.width - 5,
                &chrome::elide(&item.label, r.width - 5),
                style(TEXT, bg, selected),
            );
        }
    }
}
