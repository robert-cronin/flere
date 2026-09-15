//! Explicit agent dispatch. Durable reservations are never automatically replayed.
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const COORDINATION_WORKFLOW: &str = "When the user asks you to implement issues or put agents to work, reuse existing cards; for a new agent card use prepare_workspace without an initial shell (or new/worktree --no-shell from the CLI), then prepare_worker, authorized start_worker, and worker_status verification. Creating cards/shells is preparation, not working-agent completion. Do not ask again for local work already authorized. If only cards are requested, prepare only. Preserve the actual user request in prepare_worker as context, not a permission credential. Native approval refusals must be reported with report_worker_block; do not retry via raw sockets, terminal input, another tool, or changed settings. A later explicit human clarification can support a newly reviewed attempt. No new assignments launch on navigation or supervisor restart; UI activation may resume an exact saved chat. Report hosted, native-started, assignment-surfaced and assignment-acknowledged separately. A started process or acknowledgement alone is not proof of progress. Use worker_status and checkpoints, and never mark unfinished dispatch complete.";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum Phase {
    Prepared,
    Reserved,
    Hosted,
    Started,
    Exited,
    Failed,
    Blocked,
    Cancelled,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Dispatch {
    id: String,
    request_id: String,
    coordinator: u64,
    workspace: u64,
    epoch: String,
    cwd: PathBuf,
    device: u64,
    inode: u64,
    issue: String,
    branch: String,
    base_sha: String,
    assignment: String,
    user_request: String,
    argv: Vec<String>,
    phase: Phase,
    detail: String,
    session: u64,
    run: String,
    created: u64,
    started: Option<u64>,
    surfaced: Option<u64>,
    acknowledged: Option<u64>,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn text(args: &Value, key: &str, max: usize) -> io::Result<String> {
    let value = args[key]
        .as_str()
        .ok_or_else(|| invalid(&format!("missing {key}")))?;
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        return Err(invalid(&format!(
            "{key} must be nonempty and at most {max} bytes"
        )));
    }
    Ok(value.into())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceUpdate {
    workspace: u64,
    expected_epoch: String,
    expected: Option<ExpectedWorkspace>,
    name: Option<String>,
    project: Option<String>,
    pinned: Option<bool>,
    status: Option<crate::workspace::Workflow>,
    notes: Option<String>,
    issue: Option<String>,
    pr: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedWorkspace {
    name: String,
    // Compare the complete serialized metadata, without CardMeta's defaults:
    // a partial expectation must never accidentally match current defaults.
    meta: Value,
}
impl Dispatch {
    fn prompt(&self) -> String {
        format!(
            "Flere assignment {} in workspace {}. Read Flere get_context and inbox, read the assignment, then acknowledge its exact ID with ack_assignment and carry out that assignment. Read repository instructions first. Preserve native approvals and the assignment's scope. Do not start another agent or publish without separate authorization.",
            self.id, self.workspace
        )
    }
    fn receipt(&self, server: &Server) -> Value {
        let terminal = server
            .workspaces
            .iter()
            .find(|w| w.id == self.workspace)
            .and_then(|w| {
                w.tabs
                    .iter()
                    .find(|t| t.id == self.session && t.run == self.run)
            });
        json!({"id":self.id,"request_id":self.request_id,"workspace":self.workspace,
            "epoch":self.epoch,"cwd":self.cwd,"issue":self.issue,"phase":self.phase,
            "detail":self.detail,"session":self.session,"run":self.run,
            "host_alive":terminal.is_some_and(|t|t.alive && !t.ended),
            "native_hosted":terminal.is_some_and(|t|t.alive && !t.ended && t.native.is_some()),
            "working_observed":terminal.is_some_and(|t|t.working),
            "native_started_at":self.started,"assignment_surfaced_at":self.surfaced,
            "assignment_acknowledged_at":self.acknowledged,"accepted":false})
    }
}
impl coordination::Coordination {
    pub(super) fn validate_dispatches(&self) -> io::Result<()> {
        let mut ids = std::collections::HashSet::new();
        let mut requests = std::collections::HashSet::new();
        for d in &self.dispatches {
            if d.id.len() != 32
                || !d.id.bytes().all(|b| b.is_ascii_hexdigit())
                || !ids.insert(&d.id)
                || !requests.insert((d.coordinator, &d.request_id))
                || d.workspace == 0
                || !d.cwd.is_absolute()
                || d.assignment.len() > 16384
                || d.assignment.trim().is_empty()
                || d.user_request.len() > 4096
                || d.request_id.len() > 128
                || d.detail.len() > 4096
                || d.argv.is_empty()
                || d.argv.iter().any(|a| a.contains('\0'))
                || (d.session != 0
                    && (d.run.len() != 32 || !d.run.bytes().all(|b| b.is_ascii_hexdigit())))
            {
                return Err(invalid("invalid worker dispatch record"));
            }
        }
        Ok(())
    }
}
impl Server {
    fn save_dispatches(&mut self, next: Vec<Dispatch>) -> io::Result<()> {
        let old = std::mem::replace(&mut self.coordination.dispatches, next);
        if let Err(e) = self
            .coordination
            .validate_dispatches()
            .and_then(|_| self.persist())
        {
            self.coordination.dispatches = old;
            return Err(e);
        }
        self.changed();
        Ok(())
    }
    pub(super) fn dispatch_operation(
        &mut self,
        actor: u64,
        native: Option<(u64, &str)>,
        op: &str,
        args: &Value,
    ) -> io::Result<Value> {
        if !self
            .workspaces
            .iter()
            .any(|w| w.id == actor && !w.meta.archived)
        {
            return Err(invalid("unknown or archived coordinator"));
        }
        if op == "add_project" {
            if args
                .as_object()
                .is_none_or(|a| a.keys().any(|k| !matches!(k.as_str(), "cwd" | "name")))
            {
                return Err(invalid("add_project accepts cwd and optional name"));
            }
            let name = match args.get("name") {
                None => "",
                Some(v) => v.as_str().ok_or_else(|| invalid("name must be text"))?,
            };
            let cwd = PathBuf::from(text(args, "cwd", 4096)?);
            let previous = self.active;
            let result = self.add_project(&cwd, name, false);
            self.active = previous;
            return result.and_then(|b| serde_json::from_slice(&b).map_err(io::Error::other));
        }
        if op == "list_workspaces" {
            let data: Value =
                serde_json::from_slice(&self.command("list")?).map_err(io::Error::other)?;
            return Ok(json!({"epoch":self.epoch,"workspaces":data["workspaces"],
                "dispatches":self.coordination.dispatches.iter().rev().take(64).map(|d|d.receipt(self)).collect::<Vec<_>>(),
                "instructions":COORDINATION_WORKFLOW}));
        }
        if op == "update_workspace" {
            if args
                .as_object()
                .is_some_and(|fields| fields.values().any(Value::is_null))
            {
                return Err(invalid(
                    "omit unchanged fields; null is not an update value",
                ));
            }
            let update: WorkspaceUpdate =
                serde_json::from_value(args.clone()).map_err(io::Error::other)?;
            if update.expected_epoch != self.epoch {
                return Err(invalid("stale workspace epoch; read list_workspaces again"));
            }
            let reconciliation = update.status.is_some()
                || update.notes.is_some()
                || update.issue.is_some()
                || update.pr.is_some();
            if !reconciliation
                && update.name.is_none()
                && update.project.is_none()
                && update.pinned.is_none()
            {
                return Err(invalid(
                    "supply name, project, pinned, status, notes, issue or pr",
                ));
            }
            if reconciliation && update.expected.is_none() {
                return Err(invalid(
                    "status/notes/issue/pr require expected {name, meta} from list_workspaces",
                ));
            }
            if update.status == Some(crate::workspace::Workflow::Done) {
                return Err(invalid(
                    "Done is human-controlled; use one of todo/in-progress/needs-me/waiting",
                ));
            }
            let index = self
                .workspaces
                .iter()
                .position(|w| w.id == update.workspace && !w.meta.archived)
                .ok_or_else(|| invalid("unknown or archived workspace"))?;
            let w = &self.workspaces[index];
            if let Some(expected) = update.expected
                && (expected.name != w.name
                    || expected.meta != serde_json::to_value(&w.meta).map_err(io::Error::other)?)
            {
                return Err(invalid(
                    "stale workspace metadata; read list_workspaces and reconcile again",
                ));
            }
            let name = update.name.unwrap_or_else(|| w.name.clone());
            if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(invalid("name must be 1–256 bytes without controls"));
            }
            let mut meta = w.meta.clone();
            if let Some(project) = update.project {
                meta.project = project;
            }
            if let Some(pinned) = update.pinned {
                meta.pinned = pinned;
            }
            if let Some(status) = update.status {
                meta.status = status;
            }
            if let Some(notes) = update.notes {
                meta.notes = notes;
            }
            if let Some(issue) = update.issue {
                meta.issue = issue;
            }
            if let Some(pr) = update.pr {
                meta.pr = pr;
            }
            meta.validate()?;
            self.audit(
                "update-workspace-request",
                update.workspace,
                "partial card metadata update",
            )?;
            let old_name = std::mem::replace(&mut self.workspaces[index].name, name);
            let old_meta = std::mem::replace(&mut self.workspaces[index].meta, meta);
            if let Err(e) = self.persist() {
                self.workspaces[index].name = old_name;
                self.workspaces[index].meta = old_meta;
                return Err(e);
            }
            self.changed();
            let w = &self.workspaces[index];
            return Ok(json!({"epoch":self.epoch,
                "workspace":{"id":w.id,"name":w.name,"cwd":w.cwd,"meta":w.meta}}));
        }
        if op == "prepare_workspace" {
            let name = text(args, "name", 256)?;
            let project = match args.get("project") {
                None => String::new(),
                Some(Value::String(project)) => project.clone(),
                _ => return Err(invalid("project must be a string")),
            };
            let previous = self.active;
            let result = if args.get("repository").is_some() {
                if !args["cwd"].is_null() {
                    return Err(invalid("choose cwd or repository, not both"));
                }
                self.command(
                    &[
                        "worktree-stopped".to_owned(),
                        wire::hex(name.as_bytes()),
                        wire::hex(text(args, "repository", 4096)?.as_bytes()),
                        wire::hex(text(args, "branch", 256)?.as_bytes()),
                        wire::hex(text(args, "base", 256)?.as_bytes()),
                        wire::hex(project.as_bytes()),
                    ]
                    .join("\t"),
                )
            } else {
                if !args["branch"].is_null() || !args["base"].is_null() {
                    return Err(invalid("branch/base apply to repository preparation"));
                }
                self.prepare_project_card(name, PathBuf::from(text(args, "cwd", 4096)?), project)
            };
            self.active = previous;
            return result.and_then(|bytes| serde_json::from_slice::<Value>(&bytes).map_err(io::Error::other))
                .map(|mut created| {
                    created["instruction"] = Value::String("Prepared without an initial shell. Wait for any Git operation to finish, then prepare_worker/start_worker. Keep the returned workspace ID; inspect list_workspaces after a lost reply before repeating creation. Starts no model.".into());
                    created
                });
        } else if op == "prepare_worker" {
            let target = args["workspace"]
                .as_u64()
                .ok_or_else(|| invalid("missing workspace"))?;
            let key = text(args, "request_id", 128)?;
            let assignment = text(args, "assignment", 16384)?;
            let user_request = text(args, "user_request", 4096)?;
            let epoch = text(args, "expected_epoch", 128)?;
            let cwd = PathBuf::from(text(args, "expected_cwd", 4096)?);
            if let Some(d) = self
                .coordination
                .dispatches
                .iter()
                .find(|d| d.coordinator == actor && d.request_id == key)
            {
                if d.workspace != target
                    || d.assignment != assignment
                    || d.user_request != user_request
                    || d.epoch != epoch
                    || d.cwd != cwd
                {
                    return Err(invalid(
                        "request_id already refers to a different immutable assignment",
                    ));
                }
                return Ok(
                    json!({"dispatch":d.receipt(self),"assignment":d.assignment,"user_request":d.user_request,"initial_prompt":d.prompt()}),
                );
            }
            let w = self
                .workspaces
                .iter()
                .find(|w| w.id == target && !w.meta.archived)
                .ok_or_else(|| invalid("unknown or archived worker workspace"))?;
            if target == actor
                || epoch != self.epoch
                || cwd != w.cwd
                || fs::canonicalize(&cwd)? != cwd
            {
                return Err(invalid("stale or invalid worker workspace/epoch/directory"));
            }
            if !w.meta.operation.is_empty()
                || !w.meta.conversations.is_empty()
                || w.tabs.iter().any(|t| t.native.is_some())
            {
                return Err(invalid(
                    "worker needs a ready workspace without a native run or saved conversation; use explicit exact-UUID resume for existing work",
                ));
            }
            if self.coordination.dispatches.iter().any(|d| {
                d.workspace == target
                    && matches!(
                        d.phase,
                        Phase::Prepared | Phase::Reserved | Phase::Hosted | Phase::Started
                    )
            }) {
                return Err(invalid(
                    "workspace already has a pending or started dispatch; inspect/cancel preparation instead of duplicating it",
                ));
            }
            let m = fs::metadata(&cwd)?;
            let mut d=Dispatch{id:os::nonce()?,request_id:key,coordinator:actor,workspace:target,epoch,cwd,
                device:m.dev(),inode:m.ino(),issue:w.meta.issue.clone(),branch:w.meta.branch.clone(),base_sha:w.meta.base_sha.clone(),
                assignment,user_request,argv:crate::native::arguments(&self.state,"codex","")?,phase:Phase::Prepared,
                detail:"Prepared; no worker has been started. User request is context, not an approval credential.".into(),
                session:0,run:String::new(),created:now(),started:None,surfaced:None,acknowledged:None};
            d.argv.push(d.prompt());
            self.audit(
                "prepare-worker",
                target,
                &format!("dispatch={};request={}", d.id, d.request_id),
            )?;
            let mut next = self.coordination.dispatches.clone();
            next.push(d.clone());
            self.save_dispatches(next)?;
            return Ok(
                json!({"dispatch":d.receipt(self),"assignment":d.assignment,"user_request":d.user_request,"initial_prompt":d.prompt()}),
            );
        }
        let id = text(args, "dispatch_id", 64)?;
        let i = self
            .coordination
            .dispatches
            .iter()
            .position(|d| d.id == id)
            .ok_or_else(|| invalid("unknown dispatch"))?;
        let d = self.coordination.dispatches[i].clone();
        if op == "ack_assignment" {
            let (session, run) = native.ok_or_else(|| {
                invalid("only the exact native worker acknowledges its assignment")
            })?;
            if d.workspace != actor
                || d.session != session
                || d.run != run
                || d.epoch != self.epoch
                || d.surfaced.is_none()
            {
                return Err(invalid(
                    "assignment was not surfaced to this exact worker run",
                ));
            }
            if d.acknowledged.is_none() {
                self.audit("ack-assignment", actor, &format!("dispatch={id};run={run}"))?;
                let mut next = self.coordination.dispatches.clone();
                next[i].acknowledged = Some(now());
                self.save_dispatches(next)?;
            }
            return Ok(self.coordination.dispatches[i].receipt(self));
        }
        if d.coordinator != actor && native.is_some() {
            return Err(invalid("dispatch belongs to another coordinator"));
        }
        if op == "worker_status" {
            return Ok(d.receipt(self));
        }
        if op == "report_worker_block" || op == "cancel_worker" {
            if !matches!(d.phase, Phase::Prepared | Phase::Blocked) {
                return Err(invalid(
                    "only unlaunched preparation can be blocked or cancelled; inspect the exact existing run",
                ));
            }
            let reason = text(args, "reason", 4096)?;
            self.audit(op, d.workspace, &format!("dispatch={id};reason={reason}"))?;
            let mut next = self.coordination.dispatches.clone();
            next[i].phase = if op == "cancel_worker" {
                Phase::Cancelled
            } else {
                Phase::Blocked
            };
            next[i].detail = reason;
            self.save_dispatches(next)?;
            return Ok(self.coordination.dispatches[i].receipt(self));
        }
        if op != "start_worker" {
            return Err(invalid("unknown dispatch operation"));
        }
        // A retry after a lost reply reports the recorded attempt, never a second spawn.
        if d.phase != Phase::Prepared {
            return Ok(d.receipt(self));
        }
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == d.workspace && !w.meta.archived)
            .ok_or_else(|| invalid("worker workspace is missing or archived"))?;
        let m = fs::metadata(&w.cwd)?;
        if d.epoch != self.epoch
            || w.cwd != d.cwd
            || fs::canonicalize(&w.cwd)? != d.cwd
            || m.dev() != d.device
            || m.ino() != d.inode
            || w.meta.issue != d.issue
            || w.meta.branch != d.branch
            || w.meta.base_sha != d.base_sha
            || !w.meta.operation.is_empty()
            || !w.meta.conversations.is_empty()
            || w.tabs.iter().any(|t| t.native.is_some())
        {
            return Err(invalid(
                "stale worker target or existing native work; no launch performed",
            ));
        }
        let mut argv = crate::native::arguments(&self.state, "codex", "")?;
        argv.push(d.prompt());
        if argv != d.argv {
            return Err(invalid(
                "native launch configuration changed; prepare and review a new dispatch",
            ));
        }
        let session = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| invalid("identity exhausted"))?;
        let run = os::nonce()?;
        let spec = crate::native::HostSpec {
            launcher: None,
            id: session,
            run: run.clone(),
            harness: "codex".into(),
            argv,
            cwd: d.cwd.clone(),
            shell: os::shell(),
            conversation: String::new(),
            dispatch: id.clone(),
        };
        self.audit(
            "start-worker-request",
            d.workspace,
            &format!("dispatch={id};session={session};run={run}"),
        )?;
        let mut next = self.coordination.dispatches.clone();
        next[i].phase = Phase::Reserved;
        next[i].session = session;
        next[i].run = run;
        next[i].detail="Start reserved; if interrupted, inspect this exact attempt. Never replay automatically.".into();
        self.save_dispatches(next)?;
        let result = crate::native::write_spec(&self.state, &spec)
            .and_then(|_| self.spawn_native(d.workspace, spec, false));
        let mut next = self.coordination.dispatches.clone();
        match result {
            Ok(()) => {
                next[i].phase = Phase::Hosted;
                next[i].detail =
                    "Host created; native startup and assignment handling are separate receipts."
                        .into();
            }
            Err(e) => {
                next[i].phase = Phase::Failed;
                next[i].detail = wire::passive(&e.to_string()).chars().take(1000).collect();
            }
        }
        self.save_dispatches(next)?;
        Ok(self.coordination.dispatches[i].receipt(self))
    }
    pub(super) fn assignment_context(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
    ) -> io::Result<Value> {
        let found = self.coordination.dispatches.iter().rposition(|d| {
            d.workspace == wid && !matches!(d.phase, Phase::Cancelled | Phase::Blocked)
        });
        let Some(i) = found else {
            return Ok(Value::Null);
        };
        let d = &self.coordination.dispatches[i];
        if let Some((session, run)) = native
            && d.session == session
            && d.run == run
            && d.epoch == self.epoch
            && d.surfaced.is_none()
        {
            self.audit(
                "surface-assignment",
                wid,
                &format!("dispatch={};run={run}", d.id),
            )?;
            let mut next = self.coordination.dispatches.clone();
            next[i].surfaced = Some(now());
            self.save_dispatches(next)?;
        }
        let d = &self.coordination.dispatches[i];
        let exact = native.is_some_and(|(session, run)| {
            d.session == session && d.run == run && d.epoch == self.epoch
        });
        Ok(
            json!({"dispatch":d.receipt(self),"body":d.assignment,"user_request":d.user_request,
            "acknowledgement_required":exact && d.acknowledged.is_none(),
            "instruction":"Read the assignment and repository instructions. When acknowledgement_required is true, acknowledge this exact dispatch_id with ack_assignment before starting work. Resumed/other workspace chats can read the retained assignment but cannot acknowledge another run's delivery. Reading/surfacing does not acknowledge handling."}),
        )
    }
    pub(super) fn worker_event(
        &mut self,
        session: u64,
        run: &str,
        event: &str,
        detail: &str,
    ) -> io::Result<()> {
        let spec = self
            .session(session, run)?
            .native
            .as_ref()
            .ok_or_else(|| invalid("worker host is no longer native"))?
            .dispatch
            .clone();
        let i = self
            .coordination
            .dispatches
            .iter()
            .position(|d| {
                d.id == spec && d.session == session && d.run == run && d.epoch == self.epoch
            })
            .ok_or_else(|| invalid("unknown worker host"))?;
        let mut next = self.coordination.dispatches.clone();
        let d = &mut next[i];
        match event {
            "started" if matches!(d.phase, Phase::Reserved | Phase::Hosted) => {
                d.phase = Phase::Started;
                d.started = Some(now());
            }
            "exited" => d.phase = Phase::Exited,
            "failed" => d.phase = Phase::Failed,
            _ => return Err(invalid("invalid or out-of-order worker event")),
        }
        d.detail = wire::passive(detail).chars().take(1000).collect();
        self.audit("worker-event", session, &format!("run={run};event={event}"))?;
        self.save_dispatches(next)
    }
    pub(super) fn spawn_native(
        &mut self,
        wid: u64,
        spec: crate::native::HostSpec,
        focus: bool,
    ) -> io::Result<()> {
        self.pane_tab_capacity(wid)?;
        self.record_workspace_pane_selection(wid);
        let mut command = std::process::Command::new(os::executable_path()?);
        command
            .arg("--state")
            .arg(&self.state)
            .arg("_host")
            .arg(&spec.run);
        let (cols, rows) = self.restoration.order.get(&spec.run).map_or_else(
            || self.pane_size(wid),
            |order| self.restore_size(wid, *order),
        );
        let (master, child) = os::spawn_command_pty(&spec.cwd, &mut command, cols, rows)?;
        let w = self
            .workspaces
            .iter_mut()
            .find(|w| w.id == wid)
            .ok_or_else(|| invalid("workspace disappeared"))?;
        let id = spec.id;
        w.tabs.push(Session {
            shell_run: String::new(),
            id,
            run: spec.run.clone(),
            master,
            child,
            term: Terminal::new(cols as usize, rows as usize),
            input: VecDeque::new(),
            alive: true,
            ended: false,
            title: spec.harness.clone(),
            kind: "agent".into(),
            path: String::new(),
            native: Some(spec),
            working: false,
        });
        if focus || w.selected == 0 {
            w.selected = id;
        }
        if focus {
            self.active = wid;
        }
        self.changed();
        Ok(())
    }
}
