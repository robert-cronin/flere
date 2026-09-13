//! Runtime-only task ownership. Ordinary shell restore never replays commands;
//! the optional exec handoff retains this metadata beside the same owned PTYs.
use super::*;
use crate::tasks::{MAX_COMMAND, MAX_LABEL, MAX_PROBLEMS, MAX_TASKS, Parser, Report, Summary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Tasks {
    records: BTreeMap<String, Record>,
    #[serde(default)]
    next: usize,
}
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    report: Report,
    parser: Parser,
    cursor: u64,
    pending: bool,
    #[serde(default)]
    final_scan: bool,
}
impl Tasks {
    pub(super) fn contains(&self, run: &str) -> bool {
        self.records.contains_key(run)
    }
    pub(super) fn remove(&mut self, run: &str) {
        self.records.remove(run);
    }
    pub(super) fn output(&mut self, run: &str) {
        if let Some(record) = self.records.get_mut(run) {
            record.pending = true;
        }
    }
    pub(super) fn exited(&mut self, run: &str, code: Option<i32>) {
        if let Some(record) = self.records.get_mut(run) {
            record.report.task.ended = true;
            record.report.task.exit_code = code;
            record.pending = true;
        }
    }
    pub(super) fn title(&self, run: &str) -> Option<String> {
        self.records
            .get(run)
            .map(|r| format!("Task: {} · {}", r.report.task.label, r.report.task.status()))
    }
    pub(super) fn identities(&self) -> impl Iterator<Item = (u64, u64, &str)> {
        self.records.values().map(|r| {
            (
                r.report.task.workspace,
                r.report.task.session,
                r.report.task.run.as_str(),
            )
        })
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        if self.records.len() > MAX_TASKS {
            return Err(invalid("too many task records"));
        }
        for (run, record) in &self.records {
            let task = &record.report.task;
            if run != &task.run
                || run.len() != 32
                || !run.bytes().all(|b| b.is_ascii_hexdigit())
                || task.workspace == 0
                || task.session == 0
                || !task.cwd.is_absolute()
                || task.cwd.as_os_str().len() > 4096
                || !record.parser.valid()
                || task.problems != record.report.problems.len()
                || task.problems > MAX_PROBLEMS
                || (!task.ended && task.exit_code.is_some())
            {
                return Err(invalid("invalid task record"));
            }
            validate_text(&task.label, &task.command)?;
            for p in &record.report.problems {
                if !p.path.is_absolute()
                    || p.path.as_os_str().len() > 4096
                    || p.path.to_string_lossy().chars().any(char::is_control)
                    || !(1..=10_000_000).contains(&p.line)
                    || !(1..=1_000_000).contains(&p.column)
                    || p.message.len() > 1024
                    || p.message.chars().any(char::is_control)
                {
                    return Err(invalid("invalid task problem"));
                }
            }
        }
        Ok(())
    }
}
fn validate_text(label: &str, command: &str) -> io::Result<()> {
    if label.trim().is_empty()
        || label.len() > MAX_LABEL
        || label.chars().any(char::is_control)
        || command.trim().is_empty()
        || command.len() > MAX_COMMAND
        || command.chars().any(char::is_control)
    {
        return Err(invalid(
            "Task needs a label (80 bytes) and a single-line command (4096 bytes)",
        ));
    }
    Ok(())
}

impl Server {
    pub(super) fn task_operation(&mut self, p: &[&str]) -> io::Result<Vec<u8>> {
        let number = |i: usize| {
            p.get(i)
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| invalid("invalid task identity"))
        };
        match p.first().copied() {
            Some("task-start") if p.len() == 8 => {
                let (wid, tid, revision) = (number(2)?, number(3)?, number(5)?);
                self.validate_pane_origin(p[1], wid, tid, p[4], revision, true)?;
                let (label, command) = (wire::text(p[6])?, wire::text(p[7])?);
                validate_text(&label, &command)?;
                if self.tasks.records.len() >= MAX_TASKS {
                    return Err(invalid(
                        "Close a retained task tab before starting another (limit 64)",
                    ));
                }
                self.pane_tab_capacity(wid)?;
                let w = self.workspaces.iter().find(|w| w.id == wid).unwrap();
                if !w.meta.operation.is_empty() {
                    return Err(invalid("Workspace operation is still in progress"));
                }
                let cwd = w.cwd.clone();
                if cwd.as_os_str().len() > 4096 {
                    return Err(invalid("Task directory exceeds bound"));
                }
                let next = self
                    .next
                    .checked_add(1)
                    .ok_or_else(|| invalid("terminal identity exhausted"))?;
                let run = os::nonce()?;
                self.audit(
                    "task-start-request",
                    wid,
                    &format!("run={run};label={label}"),
                )?;
                let mut child_command = std::process::Command::new(os::shell());
                child_command.arg("-c").arg(&command);
                let (cols, rows) = self.pane_size(wid);
                // No input is written to any existing terminal. The explicit
                // command is the argv of a new shell under its own owned PTY.
                let (master, child) = os::spawn_command_pty(&cwd, &mut child_command, cols, rows)?;
                self.record_workspace_pane_selection(wid);
                let id = self.next;
                self.next = next;
                let task = Summary {
                    workspace: wid,
                    session: id,
                    run: run.clone(),
                    label,
                    command,
                    cwd: cwd.clone(),
                    ended: false,
                    exit_code: None,
                    problems: 0,
                    limited: false,
                };
                let title = format!("Task: {}", task.label);
                self.tasks.records.insert(
                    run.clone(),
                    Record {
                        report: Report {
                            task: task.clone(),
                            problems: Vec::new(),
                        },
                        parser: Parser::default(),
                        cursor: 0,
                        pending: true,
                        final_scan: false,
                    },
                );
                let w = self.workspaces.iter_mut().find(|w| w.id == wid).unwrap();
                w.tabs.push(Session {
                    id,
                    run,
                    master,
                    child,
                    term: Terminal::new(cols as usize, rows as usize),
                    input: VecDeque::new(),
                    alive: true,
                    ended: false,
                    title,
                    kind: "shell".into(),
                    path: String::new(),
                    native: None,
                    working: false,
                });
                w.selected = id;
                self.active = wid;
                self.changed();
                // Persistence stores only an ordinary shell at cwd. An exec
                // handoff separately carries task ownership and observed results.
                if let Err(error) = self.checkpoint_tabs() {
                    self.last_refresh =
                        format!("Task started; shell restore checkpoint failed: {error}");
                }
                serde_json::to_vec(&task).map_err(io::Error::other)
            }
            Some("task-list") if p.len() == 3 => {
                let wid = number(2)?;
                self.task_workspace(p[1], wid)?;
                let mut tasks = self
                    .tasks
                    .records
                    .values()
                    .filter(|r| r.report.task.workspace == wid)
                    .map(|r| &r.report.task)
                    .collect::<Vec<_>>();
                tasks.sort_by_key(|t| std::cmp::Reverse(t.session));
                serde_json::to_vec(&tasks).map_err(io::Error::other)
            }
            Some("task-info") if p.len() == 5 => {
                let (wid, tid) = (number(2)?, number(3)?);
                self.task_workspace(p[1], wid)?;
                self.session(tid, p[4])?;
                let record = self
                    .tasks
                    .records
                    .get(p[4])
                    .filter(|r| r.report.task.workspace == wid && r.report.task.session == tid)
                    .ok_or_else(|| invalid("Task run is no longer available"))?;
                serde_json::to_vec(&record.report).map_err(io::Error::other)
            }
            _ => Err(invalid("invalid task operation arguments")),
        }
    }
    fn task_workspace(&self, epoch: &str, wid: u64) -> io::Result<()> {
        if epoch != self.epoch
            || !self
                .workspaces
                .iter()
                .any(|w| w.id == wid && !w.meta.archived)
        {
            return Err(invalid("Task workspace or epoch changed"));
        }
        Ok(())
    }
    pub(super) fn tasks_tick(&mut self) -> bool {
        let mut changed = false;
        for record in self.tasks.records.values_mut() {
            if !record.final_scan
                && record.report.task.ended
                && self
                    .workspaces
                    .iter()
                    .find(|w| w.id == record.report.task.workspace)
                    .and_then(|w| {
                        w.tabs.iter().find(|t| {
                            t.id == record.report.task.session && t.run == record.report.task.run
                        })
                    })
                    .is_some_and(|t| !t.alive)
            {
                record.pending = true;
            }
        }
        // Eight pages globally, with a rotating start, prevents many noisy
        // tasks from monopolizing the supervisor's PTY and socket drain.
        let pending = self
            .tasks
            .records
            .iter()
            .filter(|(_, r)| r.pending)
            .map(|(run, _)| run.clone())
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return false;
        }
        let start = self.tasks.next % pending.len();
        let take = pending.len().min(8);
        self.tasks.next = (start + take) % pending.len();
        for run in pending.iter().cycle().skip(start).take(take) {
            let record = self.tasks.records.get_mut(run).unwrap();
            let task = &record.report.task;
            let Some(tab) = self
                .workspaces
                .iter()
                .find(|w| w.id == task.workspace)
                .and_then(|w| {
                    w.tabs
                        .iter()
                        .find(|t| t.id == task.session && t.run == task.run)
                })
            else {
                continue;
            };
            let term = &tab.term;
            let complete = tab.ended && !tab.alive;
            // Continue any backlog next tick, and revisit the live screen only
            // after output. A search page contains at most 512 interpreted rows.
            {
                let page = term.search_output(record.cursor, ":");
                let cursor_row =
                    page.end.saturating_sub(term.grid.rows as u64) + term.grid.y as u64;
                for hit in page.hits {
                    // A read chunk can end halfway through a numeric location.
                    // Wait for its newline, or the actual process/PTY end, before
                    // presenting a location the user could act on.
                    if !complete && hit.row >= cursor_row {
                        continue;
                    }
                    if let Some(problem) = record.parser.row(&record.report.task.cwd, &hit.text) {
                        if let Some(old) = record.report.problems.iter_mut().find(|p| {
                            p.path == problem.path
                                && p.line == problem.line
                                && p.column == problem.column
                        }) {
                            if !problem.message.is_empty() && old.message != problem.message {
                                *old = problem;
                                changed = true;
                            }
                        } else if record.report.problems.len() < MAX_PROBLEMS {
                            record.report.problems.push(problem);
                            changed = true;
                        } else if !record.report.task.limited {
                            record.report.task.limited = true;
                            changed = true;
                        }
                    }
                }
                record.cursor = page.next;
                if page.next >= page.end {
                    record.cursor = page.end.saturating_sub(term.grid.rows as u64);
                    record.pending = false;
                    record.final_scan = complete;
                }
            }
            record.report.task.problems = record.report.problems.len();
        }
        changed
    }
}
