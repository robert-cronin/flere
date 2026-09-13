//! Attachment-local search with frozen editor/session ownership.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Files,
    Contents,
    Output,
}
impl Kind {
    fn title(self) -> &'static str {
        match self {
            Self::Files => "Quick Open",
            Self::Contents => "Find in Files",
            Self::Output => "Find Terminal Output",
        }
    }
}
struct Hit {
    label: String,
    file: Option<crate::search::Hit>,
    row: Option<u64>,
    text: String,
}
struct ResultSet {
    hits: Vec<Hit>,
    status: String,
}
struct Job {
    query: Vec<u8>,
    cancel: Arc<AtomicBool>,
    receiver: mpsc::Receiver<ResultSet>,
}
pub(super) struct Search {
    kind: Kind,
    origin: panes::EditorOrigin,
    root: PathBuf,
    query: Vec<u8>,
    selected: usize,
    hits: Vec<Hit>,
    status: String,
    changed: Instant,
    pending: bool,
    job: Option<Job>,
}
impl Drop for Search {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}
impl Ui {
    pub(super) fn open_search(&mut self, kind: Kind) {
        let Some(workspace) = self.snapshot.workspace() else {
            return;
        };
        self.search = Some(Search {
            kind,
            origin: self.editor_origin(),
            root: PathBuf::from(&workspace.cwd),
            query: Vec::new(),
            selected: 0,
            hits: Vec::new(),
            status: "Type to search".into(),
            changed: Instant::now() - Duration::from_secs(1),
            pending: kind == Kind::Files,
            job: None,
        });
        self.menu = false;
        self.selection = None;
        self.smooth_scroll = None;
    }
    pub(super) fn tick_search(&mut self) -> bool {
        let Some(search) = &mut self.search else {
            return false;
        };
        let origin = &search.origin;
        if origin.epoch != self.snapshot.epoch
            || origin.workspace != self.snapshot.active
            || origin.tab != self.snapshot.tab
            || origin.revision != self.snapshot.split.as_ref().map_or(0, |s| s.revision)
            || origin.run
                != self
                    .snapshot
                    .session()
                    .map(|t| t.run.as_str())
                    .unwrap_or("")
        {
            self.search = None;
            self.notice = "Search closed because its workspace or pane changed".into();
            return true;
        }
        let mut dirty = false;
        if let Some(job) = &search.job {
            match job.receiver.try_recv() {
                Ok(result) => {
                    if job.query == search.query {
                        search.hits = result.hits;
                        search.status = result.status;
                        search.selected = search.selected.min(search.hits.len().saturating_sub(1));
                        dirty = true;
                    }
                    search.job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    search.job = None;
                    search.status = "Search worker stopped".into();
                    dirty = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !search.pending
            || search.job.is_some()
            || search.changed.elapsed() < Duration::from_millis(120)
        {
            return dirty;
        }
        search.pending = false;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        let query = search.query.clone();
        let (kind, root, state, origin) = (
            search.kind,
            search.root.clone(),
            self.state.clone(),
            search.origin.clone(),
        );
        search.status = "Searching…".into();
        search.job = Some(Job {
            query: query.clone(),
            cancel,
            receiver,
        });
        std::thread::spawn(move || {
            let query = String::from_utf8_lossy(&query);
            let result = if kind == Kind::Output {
                output(&state, &origin, &query, &worker_cancel)
            } else {
                crate::search::workspace(
                    &root,
                    if kind == Kind::Files {
                        crate::search::Kind::Files
                    } else {
                        crate::search::Kind::Contents
                    },
                    &query,
                    &worker_cancel,
                )
                .map(|result| {
                    let status = format!(
                        "{} matches{}{}",
                        result.hits.len(),
                        if result.limited {
                            " · search limit reached"
                        } else {
                            ""
                        },
                        if result.skipped > 0 {
                            " · binary/large/unreadable files skipped"
                        } else {
                            ""
                        }
                    );
                    let hits = result
                        .hits
                        .into_iter()
                        .map(|hit| {
                            let path = hit
                                .path
                                .strip_prefix(&root)
                                .unwrap_or(&hit.path)
                                .to_string_lossy();
                            let label = if kind == Kind::Files {
                                path.into_owned()
                            } else {
                                format!("{}:{}:{}  {}", path, hit.line, hit.column, hit.text)
                            };
                            Hit {
                                label,
                                text: hit.text.clone(),
                                file: Some(hit),
                                row: None,
                            }
                        })
                        .collect();
                    ResultSet { hits, status }
                })
            };
            let result = result.unwrap_or_else(|e| ResultSet {
                hits: Vec::new(),
                status: wire::passive(&e.to_string()),
            });
            let _ = sender.send(result);
        });
        true
    }
    pub(super) fn search_key(&mut self, key: &Key) -> bool {
        let Some(mut search) = self.search.take() else {
            return false;
        };
        match key {
            Key::Bytes(b) if b == b"\x1b" || b == b"\0" => return true,
            Key::Bytes(b) if b == b"\r" || b == b"\n" || b == b"\x03" => {
                if let Some(hit) = search.hits.get(search.selected) {
                    if b == b"\x03" {
                        self.clipboard = Some(if hit.file.is_some() {
                            hit.label.clone()
                        } else {
                            hit.text.clone()
                        });
                        self.notice = "Search result copied".into();
                    } else if let Some(file) = &hit.file {
                        let ok = if search.kind == Kind::Files {
                            self.editor_open(&search.origin, &file.path, None)
                        } else {
                            self.editor_open_at(&search.origin, &file.path, file.line, file.column)
                        };
                        if ok {
                            self.focus = Focus::Terminal;
                            self.nav = false;
                            return true;
                        }
                    } else if let Some(row) = hit.row
                        && self.editor_origin_matches(&search.origin)
                    {
                        match wire::request(
                            &self.state,
                            &[
                                "scrollback",
                                &search.origin.tab.to_string(),
                                &search.origin.run,
                                &row.to_string(),
                                "0",
                            ],
                        )
                        .and_then(|bytes| crate::model::ScrollbackPage::decode(&bytes))
                        {
                            Ok(page)
                                if page.available
                                    && (page.cols, page.rows)
                                        == (self.snapshot.cols, self.snapshot.rows) =>
                            {
                                self.scrollback = Some(scrollback::Scrollback::new(
                                    &self.snapshot,
                                    self.terminal_layout(),
                                    page,
                                ));
                                self.focus = Focus::Terminal;
                                self.nav = true;
                                return true;
                            }
                            Ok(_) => self.notice = "Output view changed; search again".into(),
                            Err(e) => self.notice = wire::passive(&e.to_string()),
                        }
                    }
                }
            }
            Key::Bytes(b) if matches!(b.as_slice(), b"\x1b[A" | b"\x10" | b"\x1b[5~") => {
                search.selected =
                    search
                        .selected
                        .saturating_sub(if b == b"\x1b[5~" { 8 } else { 1 })
            }
            Key::Bytes(b) if matches!(b.as_slice(), b"\x1b[B" | b"\x0e" | b"\x1b[6~") => {
                search.selected = (search.selected + if b == b"\x1b[6~" { 8 } else { 1 })
                    .min(search.hits.len().saturating_sub(1))
            }
            Key::Wheel { delta, .. } => {
                search.selected = search
                    .selected
                    .saturating_add_signed(*delta as isize)
                    .min(search.hits.len().saturating_sub(1))
            }
            Key::Bytes(b) | Key::Paste(b) => {
                let old = search.query.clone();
                if matches!(key, Key::Bytes(_)) && (b == b"\x7f" || b == b"\x08") {
                    search.query.pop();
                    while !search.query.is_empty() && std::str::from_utf8(&search.query).is_err() {
                        search.query.pop();
                    }
                } else if matches!(key, Key::Bytes(_)) && b == b"\x15" {
                    search.query.clear();
                } else if matches!(key, Key::Paste(_)) || !b.starts_with(b"\x1b") {
                    let bytes = b.strip_prefix(b"\x1b[200~").unwrap_or(b);
                    let bytes = bytes.strip_suffix(b"\x1b[201~").unwrap_or(bytes);
                    search.query.extend(
                        bytes
                            .iter()
                            .filter(|b| **b >= 32 && **b != 127)
                            .take(1024usize.saturating_sub(search.query.len())),
                    );
                }
                if old != search.query {
                    if let Some(job) = &search.job {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    search.hits.clear();
                    search.selected = 0;
                    search.pending = true;
                    search.changed = Instant::now();
                    search.status = "Searching…".into();
                }
            }
            _ => {}
        }
        self.search = Some(search);
        true
    }
    fn editor_origin_matches(&self, origin: &panes::EditorOrigin) -> bool {
        origin.epoch == self.snapshot.epoch
            && origin.workspace == self.snapshot.active
            && origin.tab == self.snapshot.tab
            && origin.revision == self.pane_revision()
            && origin.run
                == self
                    .snapshot
                    .session()
                    .map(|t| t.run.as_str())
                    .unwrap_or("")
    }
    pub(super) fn draw_search(&self, c: &mut Canvas) {
        let Some(search) = &self.search else {
            return;
        };
        let w = self.layout.width.saturating_sub(2).clamp(8, 110);
        let h = self.layout.height.saturating_sub(2).clamp(6, 28);
        let (x, y) = ((self.layout.width - w) / 2, (self.layout.height - h) / 2);
        c.fill(x, y, w, h, style(TEXT, PANEL, false));
        c.border(x, y, w, h, BORDER);
        c.text(
            x + 2,
            y + 1,
            w - 4,
            search.kind.title(),
            style(CYAN, PANEL, true),
        );
        c.text(
            x + 2,
            y + 2,
            w - 4,
            &format!(
                "> {}",
                chrome::tail(&String::from_utf8_lossy(&search.query), w.saturating_sub(6))
            ),
            style(TEXT, ACTIVE_BG, true),
        );
        let visible = h.saturating_sub(6);
        let start = search.selected.saturating_sub(visible.saturating_sub(1));
        for (index, hit) in search.hits.iter().enumerate().skip(start).take(visible) {
            let selected = index == search.selected;
            let bg = if selected { ACTIVE_BG } else { PANEL };
            c.fill(
                x + 1,
                y + 3 + index - start,
                w - 2,
                1,
                style(TEXT, bg, false),
            );
            c.text(
                x + 2,
                y + 3 + index - start,
                w - 4,
                &chrome::elide(&wire::passive(&hit.label), w - 4),
                style(if selected { CYAN } else { TEXT }, bg, selected),
            );
        }
        c.text(
            x + 2,
            y + h - 3,
            w - 4,
            &search.status,
            style(MUTED, PANEL, false),
        );
        c.text(
            x + 2,
            y + h - 2,
            w - 4,
            "↑/↓ select · Enter open · Ctrl+C copy · Esc cancel",
            style(MUTED, PANEL, false),
        );
    }
}
fn output(
    state: &Path,
    origin: &panes::EditorOrigin,
    query: &str,
    cancel: &AtomicBool,
) -> io::Result<ResultSet> {
    let mut hits = Vec::new();
    if query.is_empty() {
        return Ok(ResultSet {
            hits,
            status: "Type to search retained terminal output".into(),
        });
    }
    let mut from = 0;
    let until = Instant::now() + Duration::from_secs(4);
    let mut limited = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let bytes = wire::request(
            state,
            &[
                "search-output",
                &origin.tab.to_string(),
                &origin.run,
                &from.to_string(),
                &wire::hex(query.as_bytes()),
            ],
        )?;
        let page: crate::terminal::search::Page =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        for hit in page.hits {
            hits.push(Hit {
                label: format!("{}  {}", hit.row, hit.text),
                text: hit.text,
                file: None,
                row: Some(hit.row),
            });
            if hits.len() >= 200 {
                limited = true;
                break;
            }
        }
        if limited || page.next >= page.end {
            break;
        }
        if page.next <= from || Instant::now() >= until {
            limited = true;
            break;
        }
        from = page.next;
    }
    let status = format!(
        "{} matches{}",
        hits.len(),
        if limited {
            " · search limit reached"
        } else {
            ""
        }
    );
    Ok(ResultSet { hits, status })
}
