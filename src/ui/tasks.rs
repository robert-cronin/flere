//! Explicit command entry and passive Problems, bound to the opening pane.
use super::*;
use crate::tasks::{MAX_COMMAND, MAX_LABEL, Report, Summary};
use std::sync::mpsc;

pub(super) struct Panel {
    origin: panes::EditorOrigin,
    root: PathBuf,
    mode: Mode,
    status: String,
}
enum Mode {
    Run {
        label: Vec<u8>,
        command: Vec<u8>,
        field: usize,
    },
    Problems(Problems),
}
struct Problems {
    tasks: Vec<Summary>,
    task: usize,
    selected: usize,
    report: Option<Report>,
    checked: Option<Instant>,
    job: Option<mpsc::Receiver<io::Result<Loaded>>>,
}
struct Loaded {
    tasks: Vec<Summary>,
    task: usize,
    report: Option<Report>,
}
impl Ui {
    pub(super) fn open_task(&mut self) {
        self.task_panel(Mode::Run {
            label: b"Task".to_vec(),
            command: Vec::new(),
            field: 1,
        });
    }
    pub(super) fn open_problems(&mut self) {
        self.task_panel(Mode::Problems(Problems {
            tasks: Vec::new(),
            task: 0,
            selected: 0,
            report: None,
            checked: None,
            job: None,
        }));
    }
    fn task_panel(&mut self, mode: Mode) {
        self.menu = false;
        if !self.pane_capable {
            self.notice = "Update Flere to use Tasks and Problems".into();
            return;
        }
        let Some(w) = self.snapshot.workspace() else {
            self.notice = "Select a workspace first".into();
            return;
        };
        self.tasks = Some(Panel {
            origin: self.editor_origin(),
            root: PathBuf::from(&w.cwd),
            mode,
            status: String::new(),
        });
        self.selection = None;
        self.smooth_scroll = None;
        self.notice.clear();
    }
    fn task_origin_matches(&self, origin: &panes::EditorOrigin) -> bool {
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
    pub(super) fn tick_tasks(&mut self) -> bool {
        let Some(mut panel) = self.tasks.take() else {
            return false;
        };
        if !self.task_origin_matches(&panel.origin) {
            self.notice = "Tasks closed because the workspace or pane changed".into();
            return true;
        }
        let mut dirty = false;
        if let Mode::Problems(problems) = &mut panel.mode {
            if let Some(result) = problems.job.as_ref().and_then(|job| job.try_recv().ok()) {
                problems.job = None;
                problems.checked = Some(Instant::now());
                match result {
                    Ok(loaded) => {
                        problems.tasks = loaded.tasks;
                        problems.task = loaded.task;
                        problems.report = loaded.report;
                        problems.selected = problems.selected.min(
                            problems
                                .report
                                .as_ref()
                                .map_or(0, |r| r.problems.len().saturating_sub(1)),
                        );
                        panel.status = if problems.tasks.is_empty() {
                            "No task results in this workspace. Run a task with ! in NAV.".into()
                        } else {
                            String::new()
                        };
                    }
                    Err(e) => {
                        problems.report = None;
                        panel.status = wire::passive(&e.to_string());
                    }
                }
                dirty = true;
            }
            if problems.job.is_none()
                && problems
                    .checked
                    .is_none_or(|at| at.elapsed() >= Duration::from_millis(400))
            {
                let (sender, receiver) = mpsc::channel();
                problems.job = Some(receiver);
                let state = self.state.clone();
                let origin = panel.origin.clone();
                let selected = problems
                    .tasks
                    .get(problems.task)
                    .map(|t| (t.session, t.run.clone()));
                std::thread::spawn(move || {
                    let _ = sender.send(load(&state, &origin, selected));
                });
            }
        }
        self.tasks = Some(panel);
        dirty
    }
    pub(super) fn tasks_key(&mut self, key: &Key) -> bool {
        let Some(mut panel) = self.tasks.take() else {
            return false;
        };
        if !self.task_origin_matches(&panel.origin) {
            self.notice = "Tasks closed because the workspace or pane changed".into();
            return true;
        }
        if matches!(key, Key::Bytes(b) if b == b"\x1b" || b == b"\0") {
            return true;
        }
        match &mut panel.mode {
            Mode::Run {
                label,
                command,
                field,
            } => match key {
                Key::Bytes(b)
                    if matches!(b.as_slice(), b"\t" | b"\x1b[Z" | b"\x1b[A" | b"\x1b[B") =>
                {
                    *field = 1 - *field;
                }
                Key::Bytes(b) if b == b"\r" || b == b"\n" => {
                    self.flush_input();
                    let origin = &panel.origin;
                    let result = wire::request(
                        &self.state,
                        &[
                            "task-start",
                            &origin.epoch,
                            &origin.workspace.to_string(),
                            &origin.tab.to_string(),
                            &origin.run,
                            &origin.revision.to_string(),
                            &wire::hex(label),
                            &wire::hex(command),
                        ],
                    )
                    .and_then(|bytes| {
                        serde_json::from_slice::<Summary>(&bytes).map_err(io::Error::other)
                    });
                    match result {
                        Ok(task) => {
                            if let Ok(snapshot) = self.read_snapshot() {
                                self.snapshot(snapshot);
                            }
                            self.focus = Focus::Terminal;
                            self.nav = false;
                            self.notice =
                                format!("Task started: {} · NAV ; opens Problems", task.label);
                            return true;
                        }
                        Err(e) => panel.status = wire::passive(&e.to_string()),
                    }
                }
                Key::Bytes(bytes) | Key::Paste(bytes) => {
                    let (value, limit) = if *field == 0 {
                        (label, MAX_LABEL)
                    } else {
                        (command, MAX_COMMAND)
                    };
                    edit(value, bytes, limit, matches!(key, Key::Paste(_)));
                }
                _ => {}
            },
            Mode::Problems(problems) => {
                match key {
                    Key::Bytes(b)
                        if matches!(b.as_slice(), b"\t" | b"\x1b[C" | b"\x1b[Z" | b"\x1b[D") =>
                    {
                        if !problems.tasks.is_empty() {
                            let delta = if b == b"\x1b[Z" || b == b"\x1b[D" {
                                -1
                            } else {
                                1
                            };
                            problems.task = (problems.task as isize + delta)
                                .rem_euclid(problems.tasks.len() as isize)
                                as usize;
                            problems.selected = 0;
                            problems.report = None;
                            problems.job = None;
                            problems.checked = None;
                        }
                    }
                    Key::Bytes(b) if matches!(b.as_slice(), b"k" | b"\x1b[A" | b"\x1bOA") => {
                        problems.selected = problems.selected.saturating_sub(1);
                    }
                    Key::Bytes(b) if matches!(b.as_slice(), b"j" | b"\x1b[B" | b"\x1bOB") => {
                        problems.selected = problems.selected.saturating_add(1);
                    }
                    Key::Wheel { delta, .. } => {
                        problems.selected =
                            problems.selected.saturating_add_signed(*delta as isize);
                    }
                    Key::Bytes(b) if b == b"\r" || b == b"\n" => {
                        if let Some(report) = &problems.report
                            && let Some(problem) = report.problems.get(problems.selected)
                        {
                            // Revalidate the exact task run and chosen location
                            // before the existing exact-pane native editor jump.
                            let task = &report.task;
                            let current = wire::request(
                                &self.state,
                                &[
                                    "task-info",
                                    &panel.origin.epoch,
                                    &task.workspace.to_string(),
                                    &task.session.to_string(),
                                    &task.run,
                                ],
                            )
                            .and_then(|bytes| {
                                serde_json::from_slice::<Report>(&bytes).map_err(io::Error::other)
                            });
                            match current {
                                Ok(report)
                                    if report.problems.iter().any(|p| {
                                        p.path == problem.path
                                            && p.line == problem.line
                                            && p.column == problem.column
                                    }) =>
                                {
                                    if self.editor_open_at(
                                        &panel.origin,
                                        &problem.path,
                                        problem.line,
                                        problem.column,
                                    ) {
                                        self.focus = Focus::Terminal;
                                        self.nav = false;
                                        return true;
                                    }
                                    panel.status = self.notice.clone();
                                }
                                Ok(_) => {
                                    panel.status = "Task location changed; select it again".into()
                                }
                                Err(e) => panel.status = wire::passive(&e.to_string()),
                            }
                        }
                    }
                    _ => {}
                }
                problems.selected = problems.selected.min(
                    problems
                        .report
                        .as_ref()
                        .map_or(0, |r| r.problems.len().saturating_sub(1)),
                );
            }
        }
        // Every mouse, paste and key event belongs to this panel. Only explicit
        // keyboard submission starts a task or opens a diagnostic location.
        self.tasks = Some(panel);
        true
    }
    pub(super) fn draw_tasks(&self, canvas: &mut Canvas) {
        let Some(panel) = &self.tasks else {
            return;
        };
        let w = self.layout.width.saturating_sub(2).min(112);
        let h =
            self.layout
                .height
                .saturating_sub(2)
                .min(if matches!(panel.mode, Mode::Run { .. }) {
                    12
                } else {
                    28
                });
        if w < 8 || h < 6 {
            return;
        }
        let (x, y) = ((self.layout.width - w) / 2, (self.layout.height - h) / 2);
        canvas.fill(x, y, w, h, style(TEXT, PANEL, false));
        canvas.border(x, y, w, h, BORDER);
        canvas.text(
            x + 2,
            y + 1,
            w - 4,
            if matches!(panel.mode, Mode::Run { .. }) {
                "Run Task"
            } else {
                "Problems"
            },
            style(CYAN, PANEL, true),
        );
        match &panel.mode {
            Mode::Run {
                label,
                command,
                field,
            } => {
                for (row, (name, value)) in [("Label", label), ("Command", command)]
                    .into_iter()
                    .enumerate()
                {
                    canvas.text(
                        x + 2,
                        y + 3 + row * 2,
                        w - 4,
                        &format!(
                            "{name}: {}",
                            chrome::tail(
                                &String::from_utf8_lossy(value),
                                w.saturating_sub(name.len() + 7)
                            )
                        ),
                        style(
                            TEXT,
                            if *field == row { ACTIVE_BG } else { PANEL },
                            *field == row,
                        ),
                    );
                }
                if h >= 10 {
                    canvas.text(
                        x + 2,
                        y + 7,
                        w - 4,
                        &format!("Directory: {}", panel.root.display()),
                        style(MUTED, PANEL, false),
                    );
                    canvas.text(
                        x + 2,
                        y + 8,
                        w - 4,
                        "Runs once in a new terminal. Restart restores a shell, without rerunning.",
                        style(MUTED, PANEL, false),
                    );
                }
            }
            Mode::Problems(problems) => {
                if let Some(task) = problems.tasks.get(problems.task) {
                    canvas.text(
                        x + 2,
                        y + 2,
                        w - 4,
                        &format!(
                            "{} / {} · {} · {} · {} locations{}",
                            problems.task + 1,
                            problems.tasks.len(),
                            task.label,
                            task.status(),
                            task.problems,
                            if task.limited { " (limit reached)" } else { "" }
                        ),
                        style(TEXT, PANEL, true),
                    );
                }
                let visible = h.saturating_sub(6);
                let start = problems.selected.saturating_sub(visible.saturating_sub(1));
                if let Some(report) = &problems.report {
                    for (index, problem) in
                        report.problems.iter().enumerate().skip(start).take(visible)
                    {
                        let bg = if index == problems.selected {
                            ACTIVE_BG
                        } else {
                            PANEL
                        };
                        canvas.fill(
                            x + 1,
                            y + 3 + index - start,
                            w - 2,
                            1,
                            style(TEXT, bg, false),
                        );
                        let path = problem
                            .path
                            .strip_prefix(&panel.root)
                            .unwrap_or(&problem.path);
                        canvas.text(
                            x + 2,
                            y + 3 + index - start,
                            w - 4,
                            &format!(
                                "{}:{}:{}  {}",
                                path.display(),
                                problem.line,
                                problem.column,
                                problem.message
                            ),
                            style(TEXT, bg, index == problems.selected),
                        );
                    }
                    if report.problems.is_empty() {
                        canvas.text(
                            x + 2,
                            y + 4,
                            w - 4,
                            "No recognized locations. Read the task terminal for complete output.",
                            style(MUTED, PANEL, false),
                        );
                    }
                }
            }
        }
        if !panel.status.is_empty() {
            canvas.text(
                x + 2,
                y + h - 3,
                w - 4,
                &panel.status,
                style(CYAN, PANEL, false),
            );
        }
        canvas.text(
            x + 2,
            y + h - 2,
            w - 4,
            if matches!(panel.mode, Mode::Run { .. }) {
                "Tab field · Enter run · Esc cancel"
            } else {
                "j/k location · Tab task · Enter open file · Esc close"
            },
            style(MUTED, PANEL, false),
        );
    }
}
fn edit(value: &mut Vec<u8>, bytes: &[u8], limit: usize, paste: bool) {
    if !paste && matches!(bytes, b"\x7f" | b"\x08") {
        value.pop();
        while !value.is_empty() && std::str::from_utf8(value).is_err() {
            value.pop();
        }
    } else if !paste && bytes == b"\x15" {
        value.clear();
    } else if paste || !bytes.starts_with(b"\x1b") {
        let bytes = bytes.strip_prefix(b"\x1b[200~").unwrap_or(bytes);
        let bytes = bytes.strip_suffix(b"\x1b[201~").unwrap_or(bytes);
        value.extend(
            bytes
                .iter()
                .filter(|b| **b >= 32 && **b != 127)
                .take(limit.saturating_sub(value.len())),
        );
    }
}
fn load(
    state: &Path,
    origin: &panes::EditorOrigin,
    selected: Option<(u64, String)>,
) -> io::Result<Loaded> {
    let bytes = wire::request(
        state,
        &["task-list", &origin.epoch, &origin.workspace.to_string()],
    )?;
    let tasks: Vec<Summary> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let task = tasks
        .iter()
        .position(|t| {
            selected
                .as_ref()
                .is_some_and(|(id, run)| t.session == *id && t.run == *run)
        })
        .or_else(|| {
            tasks
                .iter()
                .position(|t| t.session == origin.tab && t.run == origin.run)
        })
        .unwrap_or(0);
    let report = tasks
        .get(task)
        .map(|t| {
            wire::request(
                state,
                &[
                    "task-info",
                    &origin.epoch,
                    &origin.workspace.to_string(),
                    &t.session.to_string(),
                    &t.run,
                ],
            )
            .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other))
        })
        .transpose()?;
    Ok(Loaded {
        tasks,
        task,
        report,
    })
}
