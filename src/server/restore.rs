//! Durable tab layout. Only a UI request reopens programs, one tab per request.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Tab {
    pub order: u64,
    pub kind: String,
    pub cwd: PathBuf,
    pub harness: String,
    pub conversation: String,
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub owner: Option<(u32, String)>,
}
impl Tab {
    pub fn validate(&self) -> io::Result<()> {
        if self.order == 0
            || self.order == u64::MAX
            || !self.cwd.is_absolute()
            || self.cwd.to_str().is_none()
            || self.path.len() > 8192
            || self.title.len() > 4096
            || !matches!(self.kind.as_str(), "shell" | "agent" | "editor")
            || (self.kind == "agent"
                && (!matches!(self.harness.as_str(), "codex" | "claude" | "copilot")
                    || (!self.conversation.is_empty()
                        && !crate::native::valid_uuid(&self.conversation))))
            || (self.kind == "editor" && !Path::new(&self.path).is_absolute())
        {
            return Err(invalid("invalid saved tab layout"));
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Pending {
    pub workspace: u64,
    pub tab: Tab,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct State {
    pub pending: Vec<Pending>,
    pub selected: BTreeMap<u64, u64>,
    pub panes: BTreeMap<u64, crate::panes::PaneLayout>,
    pub sequence: BTreeMap<u64, Vec<u64>>,
    pub order: BTreeMap<String, u64>,
    pub cwd: BTreeMap<String, PathBuf>,
    pub owners: BTreeMap<String, (u32, String)>,
    pub closed: BTreeSet<String>,
    pub attempted: BTreeSet<(u64, u64)>,
    #[serde(skip)]
    pub busy: bool,
    #[serde(skip)]
    checkpoint: Vec<u8>,
}
impl Server {
    pub(super) fn tab_order(&self, t: &Session) -> u64 {
        self.restoration.order.get(&t.run).copied().unwrap_or(t.id)
    }
    pub(super) fn saved_tabs(&self, w: &Workspace) -> Vec<Tab> {
        let mut tabs: Vec<_> = self
            .restoration
            .pending
            .iter()
            .filter(|p| p.workspace == w.id)
            .map(|p| p.tab.clone())
            .collect();
        for t in w
            .tabs
            .iter()
            .filter(|t| t.alive && !t.ended && !self.restoration.closed.contains(&t.run))
        {
            let cwd = t
                .native
                .as_ref()
                .map(|n| n.cwd.clone())
                .or_else(|| self.restoration.cwd.get(&t.run).cloned())
                .unwrap_or_else(|| w.cwd.clone());
            tabs.push(Tab {
                order: self.tab_order(t),
                kind: t.kind.clone(),
                cwd,
                harness: t
                    .native
                    .as_ref()
                    .map(|n| n.harness.clone())
                    .unwrap_or_default(),
                conversation: t
                    .native
                    .as_ref()
                    .map(|n| n.conversation.clone())
                    .unwrap_or_default(),
                path: t.path.clone(),
                title: t.title.chars().take(1024).collect(),
                owner: self.restoration.owners.get(&t.run).cloned(),
            });
        }
        tabs.sort_by_key(|t| {
            (
                self.restoration
                    .sequence
                    .get(&w.id)
                    .and_then(|ids| ids.iter().position(|id| *id == t.order))
                    .unwrap_or(usize::MAX),
                t.order,
            )
        });
        tabs
    }
    pub(super) fn saved_selection(&self, w: &Workspace) -> u64 {
        w.tabs
            .iter()
            .find(|t| t.id == w.selected)
            .map(|t| self.tab_order(t))
            .or_else(|| self.restoration.selected.get(&w.id).copied())
            .unwrap_or(0)
    }
    pub(super) fn sample_tab_directories(&mut self) {
        let runs: BTreeSet<_> = self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .map(|t| t.run.clone())
            .collect();
        self.restoration.order.retain(|run, _| runs.contains(run));
        self.restoration.cwd.retain(|run, _| runs.contains(run));
        self.restoration.owners.retain(|run, _| runs.contains(run));
        self.restoration.closed.retain(|run| runs.contains(run));
        self.restoration.attempted.retain(|(wid, order)| {
            self.restoration
                .pending
                .iter()
                .any(|p| p.workspace == *wid && p.tab.order == *order)
        });
        for t in self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.alive && !t.ended)
        {
            if let Ok(cwd) = os::process_cwd(t.child.id())
                && cwd.is_absolute()
                && cwd.to_str().is_some()
            {
                self.restoration.cwd.insert(t.run.clone(), cwd);
            }
        }
    }
    pub(super) fn checkpoint_tabs(&mut self) -> io::Result<()> {
        if self.restoration.busy {
            return Ok(());
        }
        for t in self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.alive && !t.ended)
        {
            if !self.restoration.owners.contains_key(&t.run)
                && let Ok(owner) = os::child_identity(t.child.id())
            {
                self.restoration
                    .owners
                    .insert(t.run.clone(), (t.child.id(), owner.1));
            }
        }
        let layout: Vec<_> = self
            .workspaces
            .iter()
            .map(|w| {
                (
                    w.id,
                    self.saved_selection(w),
                    self.saved_tabs(w),
                    self.saved_split(w),
                )
            })
            .collect();
        let bytes = serde_json::to_vec(&(self.active, layout)).map_err(io::Error::other)?;
        if bytes != self.restoration.checkpoint {
            self.persist().map_err(|e| {
                invalid(&format!(
                    "Tabs remain open, but their layout could not be saved: {e}"
                ))
            })?;
            self.restoration.checkpoint = bytes;
        }
        // The busy early-return above deliberately skips this release. A live
        // editor whose layout failed to save must keep its independent cache pin.
        let saved_paths: BTreeSet<_> = self
            .workspaces
            .iter()
            .flat_map(|w| self.saved_tabs(w))
            .filter(|t| t.kind == "editor")
            .map(|t| t.path)
            .collect();
        let retained_paths: Vec<_> = self
            .workspaces
            .iter()
            .flat_map(|w| &w.tabs)
            .filter(|t| t.kind == "editor" && !saved_paths.contains(&t.path))
            .map(|t| Path::new(&t.path))
            .collect();
        let pins = std::mem::take(self.cache_reservations.get_mut());
        for pin in pins {
            if retained_paths.iter().any(|path| pin.protects(path)) {
                self.cache_reservations.get_mut().push(pin);
            } else {
                pin.release();
            }
        }
        Ok(())
    }
    fn restore_tab(&mut self, wid: u64, tab: &Tab) -> io::Result<u64> {
        tab.validate()?;
        let (cols, rows) = self.restore_size(wid, tab.order);
        if let Some(t) = self.workspaces.iter().flat_map(|w| &w.tabs).find(|t| {
            t.alive
                && t.native.as_ref().is_some_and(|n| {
                    tab.kind == "agent"
                        && !tab.conversation.is_empty()
                        && n.harness == tab.harness
                        && n.conversation == tab.conversation
                })
        }) {
            let id = t.id;
            if self
                .workspaces
                .iter()
                .find(|w| w.id == wid)
                .is_some_and(|w| w.tabs.iter().any(|t| t.id == id))
            {
                return Ok(id);
            }
            return Err(invalid(
                "this saved chat is already open in another workspace",
            ));
        }
        if let Some((pid, start)) = &tab.owner
            && os::child_identity(*pid).is_ok_and(|current| current.1 == *start)
        {
            return Err(invalid(
                "the previous tab process is still running; inspect it before reopening",
            ));
        }
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == wid && !w.meta.archived)
            .ok_or_else(|| invalid("workspace unavailable"))?;
        if !w.meta.operation.is_empty() {
            return Err(invalid(&w.meta.operation));
        }
        if !tab.cwd.is_dir() {
            return Err(invalid("saved tab directory is unavailable"));
        }
        let id = self.next;
        if tab.kind == "agent" {
            let argv =
                crate::native::resume_arguments(&self.state, &tab.harness, &tab.conversation)?;
            let run = os::nonce()?;
            let spec = crate::native::HostSpec {
                id,
                run,
                harness: tab.harness.clone(),
                argv,
                cwd: tab.cwd.clone(),
                shell: os::shell(),
                conversation: tab.conversation.clone(),
                dispatch: String::new(),
            };
            crate::native::write_spec(&self.state, &spec)?;
            self.restoration.order.insert(spec.run.clone(), tab.order);
            self.next = self
                .next
                .checked_add(1)
                .ok_or_else(|| invalid("identity exhausted"))?;
            self.spawn_native(wid, spec, false)?;
        } else {
            let run = os::nonce()?;
            let (master, child) = if tab.kind == "editor" {
                let path = crate::editor::file(&tab.cwd, &tab.path)?;
                if let Some(pin) = crate::editor::cache::reserve_open(&self.state, &path)? {
                    self.cache_reservations.borrow_mut().push(pin);
                }
                os::spawn_command_pty(
                    &tab.cwd,
                    &mut crate::close::editor_command(
                        &self.state,
                        &run,
                        crate::editor::command(&path)?,
                    )?,
                    cols,
                    rows,
                )?
            } else {
                os::spawn_command_pty(
                    &tab.cwd,
                    &mut crate::close::shell_command(&self.state, &run, &os::shell())?,
                    cols,
                    rows,
                )?
            };
            self.next = self
                .next
                .checked_add(1)
                .ok_or_else(|| invalid("identity exhausted"))?;
            self.workspaces
                .iter_mut()
                .find(|w| w.id == wid)
                .unwrap()
                .tabs
                .push(Session {
                    id,
                    run,
                    master,
                    child,
                    term: Terminal::new(cols as usize, rows as usize),
                    input: VecDeque::new(),
                    alive: true,
                    ended: false,
                    title: tab.title.clone(),
                    kind: tab.kind.clone(),
                    path: tab.path.clone(),
                    native: None,
                    working: false,
                });
        }
        Ok(id)
    }
    pub(super) fn restore_next(&mut self, epoch: &str) -> io::Result<Vec<u8>> {
        if epoch != self.epoch {
            return Err(invalid("workspace epoch changed; reopen Flere"));
        }
        let next = self.restoration.pending.iter().position(|p| {
            !self
                .restoration
                .attempted
                .contains(&(p.workspace, p.tab.order))
                && self
                    .workspaces
                    .iter()
                    .any(|w| w.id == p.workspace && !w.meta.archived)
        });
        let mut error = String::new();
        if let Some(index) = next {
            let p = self.restoration.pending[index].clone();
            let selected = self
                .workspaces
                .iter()
                .find(|w| w.id == p.workspace)
                .map_or(0, |w| w.selected);
            let active = self.active;
            self.restoration
                .attempted
                .insert((p.workspace, p.tab.order));
            self.restoration.busy = true;
            let result = self.restore_tab(p.workspace, &p.tab);
            self.restoration.busy = false;
            self.active = active;
            match result {
                Ok(id) => {
                    let w = self
                        .workspaces
                        .iter_mut()
                        .find(|w| w.id == p.workspace)
                        .unwrap();
                    let t = w.tabs.iter().find(|t| t.id == id).unwrap();
                    self.restoration.order.insert(t.run.clone(), p.tab.order);
                    self.restoration.cwd.insert(t.run.clone(), p.tab.cwd);
                    w.selected = if selected != 0 {
                        selected
                    } else if self.restoration.selected.get(&w.id) == Some(&p.tab.order) {
                        id
                    } else {
                        0
                    };
                    w.tabs.sort_by_key(|t| {
                        let order = self.restoration.order.get(&t.run).copied().unwrap_or(t.id);
                        (
                            self.restoration
                                .sequence
                                .get(&w.id)
                                .and_then(|ids| ids.iter().position(|id| *id == order))
                                .unwrap_or(usize::MAX),
                            order,
                        )
                    });
                    self.restoration.pending.remove(index);
                }
                Err(e) => {
                    error = format!(
                        "Could not reopen {}: {e}. W retries saved tabs.",
                        p.tab.title
                    );
                }
            }
            let more_for_workspace = self.restoration.pending.iter().any(|p2| {
                p2.workspace == p.workspace
                    && !self
                        .restoration
                        .attempted
                        .contains(&(p2.workspace, p2.tab.order))
            });
            if !more_for_workspace {
                let w = self
                    .workspaces
                    .iter_mut()
                    .find(|w| w.id == p.workspace)
                    .unwrap();
                if w.selected == 0 {
                    w.selected = w.tabs.first().map_or(0, |t| t.id);
                }
            }
            self.restore_panes(p.workspace);
            self.changed();
            self.checkpoint_tabs()?;
        }
        let remaining = self
            .restoration
            .pending
            .iter()
            .filter(|p| {
                !self
                    .restoration
                    .attempted
                    .contains(&(p.workspace, p.tab.order))
                    && self
                        .workspaces
                        .iter()
                        .any(|w| w.id == p.workspace && !w.meta.archived)
            })
            .count();
        serde_json::to_vec(&serde_json::json!({"remaining":remaining,"error":error}))
            .map_err(io::Error::other)
    }
}
