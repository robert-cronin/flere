//! Local choices own a two-component update. Core RPC uses the current bridge;
//! no second SSH login, remotely selected local executable, or native input.
use crate::{connections::Connection, protocol::Packet, remote_update as protocol, update};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::{self, Write},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(4096).collect()
}
#[derive(Clone, Deserialize, Serialize)]
struct Core {
    token: String,
    epoch: String,
    pid: u32,
    current: update::BuildMetadata,
    candidate: update::Manifest,
}
#[derive(Clone, Deserialize, Serialize)]
struct Plan {
    schema_version: u64,
    token: String,
    connection: Vec<String>,
    core: Core,
    companion: update::Prepared,
    phase: String,
    companion_attempt: Option<String>,
    core_attempt: Option<String>,
    detail: String,
}
fn path(token: &str) -> io::Result<PathBuf> {
    if !protocol::token(token) {
        return Err(invalid("invalid coordinated update token"));
    }
    Ok(update::coordinated_dir()?.join(format!("{token}.json")))
}
fn save(plan: &Plan) -> io::Result<()> {
    update::write_json(&path(&plan.token)?, plan)
}
struct Request {
    args: Vec<String>,
    reply: mpsc::SyncSender<io::Result<Value>>,
}
#[derive(Clone)]
struct Rpc(mpsc::SyncSender<Request>);
impl Rpc {
    fn call(&self, args: &[&str]) -> io::Result<Value> {
        let (reply, result) = mpsc::sync_channel(1);
        self.0
            .send(Request {
                args: args.iter().map(|s| (*s).into()).collect(),
                reply,
            })
            .map_err(|_| invalid("update connection closed"))?;
        result
            .recv_timeout(Duration::from_secs(1830))
            .map_err(|_| {
                invalid("update request ended without a result; inspect its retained plan")
            })?
    }
}
struct Reply {
    id: u64,
    sender: mpsc::SyncSender<io::Result<Value>>,
    length: Option<usize>,
    success: bool,
    bytes: Vec<u8>,
}
enum Outcome {
    Prepared(Box<Plan>),
    Restart(Box<Plan>),
    Activate(Box<Plan>),
}
#[derive(PartialEq, Eq)]
enum Phase {
    Form,
    Preparing,
    Review,
    Applying,
    Waiting,
    Failed,
}
struct Flow {
    request: u64,
    fields: [Vec<u8>; 2],
    field: usize,
    phase: Phase,
    status: String,
    plan: Option<Plan>,
    job: Option<mpsc::Receiver<io::Result<Outcome>>>,
    input: Vec<u8>,
    paste: bool,
    displayed: Instant,
    display_pending: bool,
    waiting: Instant,
}
pub struct Coordinator {
    flow: Option<Flow>,
    last: u64,
    counter: u64,
    rpc: Rpc,
    requests: mpsc::Receiver<Request>,
    reply: Option<Reply>,
    last_frame: Vec<u8>,
    released: Option<Instant>,
}
impl Default for Coordinator {
    fn default() -> Self {
        let (tx, requests) = mpsc::sync_channel(1);
        Self {
            flow: None,
            last: 0,
            counter: 0,
            rpc: Rpc(tx),
            requests,
            reply: None,
            last_frame: Vec::new(),
            released: None,
        }
    }
}
fn prepare(connection: &Connection, sources: [String; 2], rpc: &Rpc) -> io::Result<Outcome> {
    let companion = update::prepare(&sources[1])?;
    let remote = rpc.call(&["update-prepare", &sources[0]])?;
    let token = remote["token"]
        .as_str()
        .filter(|s| protocol::token(s))
        .ok_or_else(|| invalid("remote preparation token is invalid"))?;
    let current: update::BuildMetadata =
        serde_json::from_value(remote["runtime"]["build"].clone()).map_err(io::Error::other)?;
    let candidate: update::Manifest =
        serde_json::from_value(remote["candidate"]["manifest"].clone())
            .map_err(io::Error::other)?;
    current.validate()?;
    candidate.validate()?;
    if !companion.package.manifest.build.reads_core(&current)
        || !companion
            .package
            .manifest
            .build
            .reads_core(&candidate.build)
        || !candidate.build.accepts_runtime(&current)
    {
        return Err(invalid(
            "these packages cannot preserve compatibility through the update; installed commands retained",
        ));
    }
    if companion.package.manifest.build.package_version != candidate.build.package_version {
        return Err(invalid(
            "choose core and companion packages from the same release",
        ));
    }
    if let (Some(a), Some(b)) = (&companion.package.manifest.source, &candidate.source)
        && a.source_sha256 != b.source_sha256
    {
        return Err(invalid(
            "core and companion were packaged from different source revisions",
        ));
    }
    let epoch = remote["runtime"]["epoch"]
        .as_str()
        .filter(|s| protocol::token(s))
        .ok_or_else(|| invalid("remote epoch is invalid"))?
        .into();
    let pid = remote["runtime"]["pid"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        .ok_or_else(|| invalid("remote PID is invalid"))? as u32;
    let plan = Plan {
        schema_version: 1,
        token: update::nonce()?,
        connection: connection.args(),
        core: Core {
            token: token.into(),
            epoch,
            pid,
            current,
            candidate,
        },
        companion,
        phase: "prepared".into(),
        companion_attempt: None,
        core_attempt: None,
        detail: "Both packages verified; awaiting local Update action".into(),
    };
    save(&plan)?;
    Ok(Outcome::Prepared(Box::new(plan)))
}
fn apply(mut plan: Plan, rpc: &Rpc) -> io::Result<Outcome> {
    // Never use a remote path as a local executable. The prepared local package
    // is independently verified again by the installer before stable replacement.
    if plan.companion_attempt.is_none() {
        let remote = rpc.call(&["update-plan", &plan.core.token])?;
        if remote["status"] != "prepared"
            || remote["ready"] != true
            || remote["runtime"]["epoch"] != plan.core.epoch
            || remote["runtime"]["pid"] != plan.core.pid
        {
            return Err(invalid("remote plan changed before companion installation"));
        }
        let receipt = update::install_prepared(&plan.companion, true)?;
        plan.companion_attempt = Some(receipt.attempt);
        plan.phase = "companion_installed".into();
        plan.detail = "Companion installed; reconnecting before updating the remote core".into();
        save(&plan)?;
        if !update::running_candidate(&plan.companion) {
            return Ok(Outcome::Restart(Box::new(plan)));
        }
    }
    update::verify_applied(
        &plan.companion,
        plan.companion_attempt
            .as_deref()
            .ok_or_else(|| invalid("companion install attempt is missing"))?,
    )?;
    plan.phase = "applying_core".into();
    plan.detail = "New companion is running; applying the prepared remote core".into();
    save(&plan)?;
    let receipt = rpc.call(&["update-apply", &plan.core.token])?;
    if receipt["activation"]["supervisor"] != "applied"
        || receipt["activation"]["after"]["epoch"] != plan.core.epoch
        || receipt["activation"]["after"]["pid"] != plan.core.pid
        || receipt["current"]["manifest"]["build"]
            != serde_json::to_value(&plan.core.candidate.build).map_err(io::Error::other)?
    {
        plan.phase = "partial".into();
        plan.detail = clean(
            receipt["activation"]["detail"]
                .as_str()
                .unwrap_or("Remote activation was not verified"),
        );
        save(&plan)?;
        return Err(invalid(&format!("Companion retained; {}", plan.detail)));
    }
    plan.core_attempt = Some(
        receipt["attempt"]
            .as_str()
            .filter(|s| protocol::token(s))
            .ok_or_else(|| invalid("remote update attempt is invalid"))?
            .into(),
    );
    plan.phase = "awaiting_frontend".into();
    plan.detail = "Supervisor applied; waiting for the updated remote frontend".into();
    save(&plan)?;
    Ok(Outcome::Activate(Box::new(plan)))
}
impl Coordinator {
    pub fn active(&self) -> bool {
        self.flow.is_some()
    }
    pub fn discard_input(&self, received: Instant) -> bool {
        self.released.is_some_and(|released| received <= released)
    }
    pub fn failed(&mut self, message: &str) {
        if let Some(flow) = &mut self.flow {
            flow.phase = Phase::Failed;
            flow.status = clean(message);
            // Already issued RPCs may finish, but cannot advance this flow or
            // report success after the local installation lost its identity.
            flow.job = None;
            flow.display_pending = true;
        }
    }
    pub fn resume(&mut self, connection: &Connection) -> io::Result<()> {
        let Some(token) = std::env::var_os("FLERE_COORDINATED_UPDATE") else {
            return Ok(());
        };
        let plan: Plan = update::read_json(&path(&token.to_string_lossy())?)?;
        if plan.schema_version != 1
            || plan.connection != connection.args()
            || !matches!(
                plan.phase.as_str(),
                "companion_installed" | "applying_core" | "awaiting_frontend"
            )
            || !update::running_candidate(&plan.companion)
        {
            return Err(invalid(
                "coordinated restart does not match its approved connection and package",
            ));
        }
        update::verify_applied(
            &plan.companion,
            plan.companion_attempt
                .as_deref()
                .ok_or_else(|| invalid("companion install attempt is missing"))?,
        )?;
        let now = Instant::now();
        self.flow = Some(Flow {
            request: 0,
            fields: [Vec::new(), Vec::new()],
            field: 0,
            phase: Phase::Applying,
            status: "Companion updated; completing remote Flere update…".into(),
            plan: Some(plan.clone()),
            job: None,
            input: Vec::new(),
            paste: false,
            displayed: now,
            display_pending: true,
            waiting: now,
        });
        // The first accepted OUTPUT acknowledges the new connection and records
        // its exact installed attempt. Starting the worker here would race that
        // acknowledgement for the local installation lock.
        Ok(())
    }
    pub fn connection_ready(&mut self) {
        let plan = self
            .flow
            .as_ref()
            .filter(|flow| flow.phase == Phase::Applying && flow.job.is_none())
            .and_then(|flow| flow.plan.clone());
        if let Some(plan) = plan {
            self.start_apply(plan);
        }
    }
    pub fn offer(
        &mut self,
        packet: &Packet,
        connection: &Connection,
        queue: &mut VecDeque<Packet>,
    ) -> io::Result<()> {
        if packet.id == 0 || packet.id <= self.last || !packet.data.is_empty() || self.active() {
            queue.push_back(Packet::new(
                protocol::RESULT,
                packet.id,
                serde_json::to_vec(
                    &json!({"message":"Update request is stale or another local action is open"}),
                )
                .unwrap(),
            ));
            return Ok(());
        }
        self.last = packet.id;
        self.last_frame.clear();
        let source = match update::source_default()? {
            Some(update::PackageSource::Public { manifest_url }) => manifest_url,
            _ => String::new(),
        };
        let now = Instant::now();
        self.flow = Some(Flow {
            request: packet.id,
            fields: [Vec::new(), source.into_bytes()],
            field: 0,
            phase: Phase::Form,
            status: format!("{} · Enter prepares both packages", connection.host),
            plan: None,
            job: None,
            input: Vec::new(),
            paste: false,
            displayed: now,
            display_pending: true,
            waiting: now,
        });
        Ok(())
    }
    fn start_apply(&mut self, plan: Plan) {
        let (tx, rx) = mpsc::channel();
        let rpc = self.rpc.clone();
        std::thread::spawn(move || {
            let _ = tx.send(apply(plan, &rpc));
        });
        let flow = self.flow.as_mut().unwrap();
        flow.job = Some(rx);
        flow.phase = Phase::Applying;
        flow.status = "Applying update; terminal sessions remain alive…".into();
    }
    fn cancel(&mut self, queue: &mut VecDeque<Packet>, size: (u16, u16)) {
        if let Some(flow) = self.flow.take() {
            self.released = Some(Instant::now());
            queue.push_back(Packet::new(protocol::RESULT, flow.request, serde_json::to_vec(&json!({"message": if flow.phase == Phase::Failed {flow.status} else {"Update cancelled; installed components retained".into()}})).unwrap()));
            queue.push_back(Packet::new(
                crate::protocol::RESIZE,
                0,
                crate::protocol::size_bytes(size.0, size.1),
            ));
        }
    }
    pub fn input(
        &mut self,
        bytes: &[u8],
        received: Instant,
        connection: &Connection,
        queue: &mut VecDeque<Packet>,
        size: (u16, u16),
    ) -> io::Result<()> {
        let Some(flow) = &mut self.flow else {
            return Ok(());
        };
        if flow.display_pending
            || received <= flow.displayed
            || matches!(
                flow.phase,
                Phase::Preparing | Phase::Applying | Phase::Waiting
            )
        {
            return Ok(());
        }
        flow.input.extend_from_slice(bytes);
        let mut action = 0;
        while !flow.input.is_empty() {
            if flow.input.starts_with(b"\x1b[200~") {
                flow.input.drain(..6);
                flow.paste = true;
                continue;
            }
            if flow.input.starts_with(b"\x1b[201~") {
                flow.input.drain(..6);
                flow.paste = false;
                continue;
            }
            if flow.input[0] == 27 {
                if flow.input.len() == 1 {
                    break;
                }
                if flow.input[1] == b'[' || flow.input[1] == b'O' {
                    if let Some(end) = flow.input[2..]
                        .iter()
                        .position(|b| (0x40..=0x7e).contains(b))
                    {
                        flow.input.drain(..end + 3);
                        continue;
                    }
                    if flow.input.len() > 128 {
                        flow.input.clear();
                    }
                    break;
                }
                if matches!(flow.input[1], b']' | b'P' | b'_') {
                    if let Some(end) = flow.input.windows(2).position(|w| w == b"\x1b\\") {
                        flow.input.drain(..end + 2);
                        continue;
                    }
                    if let Some(end) = flow.input.iter().position(|b| *b == 7) {
                        flow.input.drain(..end + 1);
                        continue;
                    }
                    if flow.input.len() > 4096 {
                        flow.input.clear();
                    }
                    break;
                }
                action = 1;
                break;
            }
            let b = flow.input.remove(0);
            match b {
                3 if !flow.paste => {
                    action = 1;
                    break;
                }
                b'\r' | b'\n' if !flow.paste => {
                    action = 2;
                    break;
                }
                b'\t' if !flow.paste && flow.phase == Phase::Form => flow.field = 1 - flow.field,
                21 if !flow.paste && flow.phase == Phase::Form => flow.fields[flow.field].clear(),
                8 | 127 if !flow.paste && flow.phase == Phase::Form => {
                    flow.fields[flow.field].pop();
                    while !flow.fields[flow.field].is_empty()
                        && std::str::from_utf8(&flow.fields[flow.field]).is_err()
                    {
                        flow.fields[flow.field].pop();
                    }
                }
                b if b >= 32
                    && b != 127
                    && flow.phase == Phase::Form
                    && flow.fields[flow.field].len() < 4096 =>
                {
                    flow.fields[flow.field].push(b)
                }
                _ => {}
            }
        }
        if action == 1 || action == 2 && flow.phase == Phase::Failed {
            self.cancel(queue, size);
            return Ok(());
        }
        if action == 2 {
            flow.input.clear();
            if flow.phase == Phase::Review {
                let plan = flow.plan.clone().unwrap();
                self.start_apply(plan);
            } else if flow.phase == Phase::Form {
                let sources = flow
                    .fields
                    .each_ref()
                    .map(|v| String::from_utf8_lossy(v).trim().to_owned());
                let connection = connection.clone();
                let rpc = self.rpc.clone();
                let (tx, rx) = mpsc::channel();
                flow.job = Some(rx);
                flow.phase = Phase::Preparing;
                flow.status = "Preparing and verifying both packages…".into();
                std::thread::spawn(move || {
                    let _ = tx.send(prepare(&connection, sources, &rpc));
                });
            }
        }
        Ok(())
    }
    pub fn escape(&mut self, queue: &mut VecDeque<Packet>, size: (u16, u16)) {
        if self.flow.as_ref().is_some_and(|f| {
            !f.paste
                && f.input == b"\x1b"
                && matches!(f.phase, Phase::Form | Phase::Review | Phase::Failed)
        }) {
            self.cancel(queue, size);
        }
    }
    pub fn packet(
        &mut self,
        packet: &Packet,
        queue: &mut VecDeque<Packet>,
        size: (u16, u16),
    ) -> io::Result<bool> {
        if packet.tag == protocol::ACK {
            if packet.id != 0 || packet.data.len() > 8192 {
                return Err(invalid("invalid frontend update acknowledgement"));
            }
            let ack: Value = serde_json::from_slice(&packet.data).map_err(io::Error::other)?;
            let Some(flow) = self.flow.as_mut().filter(|f| f.phase == Phase::Waiting) else {
                return Ok(true);
            };
            let plan = flow.plan.as_mut().unwrap();
            if ack["attempt"].as_str() != plan.core_attempt.as_deref()
                || ack["build_id"] != plan.core.candidate.build.build_id
            {
                return Err(invalid("frontend acknowledged another update"));
            }
            if let Err(error) = update::verify_applied(
                &plan.companion,
                plan.companion_attempt
                    .as_deref()
                    .ok_or_else(|| invalid("companion install attempt is missing"))?,
            ) {
                plan.phase = "partial".into();
                plan.detail =
                    format!("Remote core applied; companion verification failed: {error}");
                save(plan)?;
                let detail = plan.detail.clone();
                self.failed(&detail);
                return Ok(true);
            }
            plan.phase = "applied".into();
            plan.detail = "Core supervisor, this frontend and companion applied; exact remote sessions preserved; other windows reported separately".into();
            save(plan)?;
            queue.push_back(Packet::new(protocol::RESULT, flow.request, serde_json::to_vec(&json!({"message":"Updated Flere and this companion; terminal sessions preserved"})).unwrap()));
            queue.push_back(Packet::new(
                crate::protocol::RESIZE,
                0,
                crate::protocol::size_bytes(size.0, size.1),
            ));
            self.flow = None;
            self.released = Some(Instant::now());
            return Ok(true);
        }
        if !matches!(packet.tag, protocol::BEGIN | protocol::DATA | protocol::END) {
            return Ok(false);
        }
        let reply = self
            .reply
            .as_mut()
            .filter(|r| r.id == packet.id)
            .ok_or_else(|| invalid("stale update RPC response"))?;
        match packet.tag {
            protocol::BEGIN => {
                if reply.length.is_some() || packet.data.len() != 9 || packet.data[0] > 1 {
                    return Err(invalid("invalid update response header"));
                }
                let length = u64::from_be_bytes(packet.data[1..].try_into().unwrap());
                if length > protocol::RESPONSE_LIMIT as u64 {
                    return Err(invalid("update response exceeds bound"));
                }
                reply.length = Some(length as usize);
                reply.success = packet.data[0] == 1;
            }
            protocol::DATA => {
                if reply
                    .length
                    .is_none_or(|n| reply.bytes.len() + packet.data.len() > n)
                {
                    return Err(invalid("update response exceeds declared length"));
                }
                reply.bytes.extend_from_slice(&packet.data);
            }
            protocol::END => {
                if !packet.data.is_empty() || reply.length != Some(reply.bytes.len()) {
                    return Err(invalid("update response was incomplete"));
                }
                let reply = self.reply.take().unwrap();
                let result = if reply.success {
                    serde_json::from_slice(&reply.bytes).map_err(io::Error::other)
                } else {
                    Err(invalid(&clean(&String::from_utf8_lossy(&reply.bytes))))
                };
                let _ = reply.sender.send(result);
            }
            _ => unreachable!(),
        }
        Ok(true)
    }
    /// Returns true only after a private, exact-attempt restart request is saved.
    pub fn tick(
        &mut self,
        connection: &Connection,
        queue: &mut VecDeque<Packet>,
    ) -> io::Result<bool> {
        if self.reply.is_none()
            && let Ok(request) = self.requests.try_recv()
        {
            self.counter = self
                .counter
                .checked_add(1)
                .ok_or_else(|| invalid("update RPC counter exhausted"))?;
            queue.push_back(Packet::new(
                protocol::CALL,
                self.counter,
                serde_json::to_vec(&request.args).map_err(io::Error::other)?,
            ));
            self.reply = Some(Reply {
                id: self.counter,
                sender: request.reply,
                length: None,
                success: false,
                bytes: Vec::new(),
            });
        }
        let Some(flow) = &mut self.flow else {
            return Ok(false);
        };
        if flow.phase == Phase::Waiting && flow.waiting.elapsed() > Duration::from_secs(90) {
            flow.phase = Phase::Failed;
            flow.status = "Update remains partial: frontend acknowledgement timed out; inspect install-status".into();
            flow.displayed = Instant::now();
        }
        let Some(job) = &flow.job else {
            return Ok(false);
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(invalid("update worker stopped; inspect retained plan"))
            }
        };
        flow.job = None;
        flow.displayed = Instant::now();
        flow.display_pending = true;
        match result {
            Ok(Outcome::Prepared(plan)) => {
                flow.phase = Phase::Review;
                flow.status = "Packages verified. Enter updates both; sessions kept.".into();
                flow.plan = Some(*plan);
            }
            Ok(Outcome::Restart(plan)) => {
                update::request_restart(
                    connection,
                    Some(&plan.token),
                    plan.companion_attempt
                        .as_deref()
                        .ok_or_else(|| invalid("companion install attempt is missing"))?,
                )?;
                return Ok(true);
            }
            Ok(Outcome::Activate(plan)) => {
                queue.push_back(Packet::new(
                    protocol::ACTIVATE,
                    1,
                    serde_json::to_vec(&json!({"attempt":plan.core_attempt})).unwrap(),
                ));
                flow.phase = Phase::Waiting;
                flow.status = "Supervisor updated; applying this window…".into();
                flow.waiting = Instant::now();
                flow.plan = Some(*plan);
            }
            Err(e) => {
                flow.phase = if flow.phase == Phase::Preparing {
                    Phase::Form
                } else {
                    Phase::Failed
                };
                flow.status = clean(&e.to_string());
            }
        }
        Ok(false)
    }
    pub fn refreshed(&mut self) {
        self.last_frame.clear();
        if !self.active() {
            self.last = 0;
        }
    }
    pub fn draw(&mut self, size: (u16, u16), out: &mut impl Write) -> io::Result<()> {
        let Some(flow) = &self.flow else {
            return Ok(());
        };
        let width = usize::from(size.0).saturating_sub(4);
        let clip = |s: &str| {
            clean(s)
                .chars()
                .scan(0, |n, c| {
                    *n += c.len_utf8();
                    (*n <= width).then_some(c)
                })
                .collect::<String>()
        };
        let mut lines = vec!["LOCAL · Update Flere + companion".into()];
        if let Some(plan) = &flow.plan {
            lines.push(format!(
                "Flere: {} → {} · {}",
                plan.core.current.package_version,
                plan.core.candidate.build.package_version,
                plan.core.candidate.build.target
            ));
            lines.push(format!(
                "Core build: {}",
                plan.core.candidate.build.build_id
            ));
            lines.push(format!(
                "Companion: {} · {}",
                plan.companion.package.manifest.build.package_version,
                plan.companion.package.manifest.build.target
            ));
            lines.push(format!(
                "Companion build: {}",
                plan.companion.package.manifest.build.build_id
            ));
            lines.push("SHA-256 verified · previous packages retained".into());
        } else {
            lines.push(format!(
                "{} Remote core: {}",
                if flow.field == 0 { ">" } else { " " },
                String::from_utf8_lossy(&flow.fields[0])
            ));
            lines.push(format!(
                "{} Local companion: {}",
                if flow.field == 1 { ">" } else { " " },
                String::from_utf8_lossy(&flow.fields[1])
            ));
            lines.push("Package path / HTTPS manifest; blank = saved source".into());
            lines.push("Use --rollback for retained compatible packages".into());
        }
        let height = usize::from(size.1);
        lines.truncate(height.saturating_sub(4));
        lines.push(flow.status.clone());
        lines.push(
            match flow.phase {
                Phase::Form => "Tab field · Enter prepare · Ctrl+U clear · Esc cancel",
                Phase::Review => "Enter Update now · Esc cancel",
                Phase::Failed => "Enter / Esc close · receipt retained for recovery",
                _ => "Update in progress; all input stays in this local view",
            }
            .into(),
        );
        let mut frame = b"\x1b[?2026h\x1b[0m\x1b[?25l\x1b[2J\x1b[H".to_vec();
        for (row, line) in lines.iter().enumerate() {
            write!(frame, "\x1b[{};3H{}", row + 2, clip(line))?;
        }
        frame.extend_from_slice(b"\x1b[?2026l");
        if frame != self.last_frame {
            out.write_all(&frame)?;
            out.flush()?;
            self.last_frame = frame;
        }
        if let Some(flow) = &mut self.flow
            && flow.display_pending
        {
            flow.displayed = Instant::now();
            flow.display_pending = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn connection() -> Connection {
        Connection {
            host: "fixture".into(),
            remote: "flere".into(),
            remote_explicit: true,
            state: None,
            ssh: "ssh".into(),
        }
    }
    fn form() -> Coordinator {
        let now = Instant::now();
        Coordinator {
            flow: Some(Flow {
                request: 1,
                fields: [Vec::new(), Vec::new()],
                field: 0,
                phase: Phase::Form,
                status: "Select packages".into(),
                plan: None,
                job: None,
                input: Vec::new(),
                paste: false,
                displayed: now,
                display_pending: true,
                waiting: now,
            }),
            ..Coordinator::default()
        }
    }
    #[test]
    fn remote_request_cannot_supply_local_package_sources() {
        let mut coordinator = Coordinator::default();
        let mut queue = VecDeque::new();
        coordinator
            .offer(
                &Packet::new(protocol::REQUEST, 1, br#"{"companion":"/hostile"}"#),
                &connection(),
                &mut queue,
            )
            .unwrap();
        assert!(!coordinator.active());
        assert_eq!(queue[0].tag, protocol::RESULT);
        assert!(coordinator.requests.try_recv().is_err());
    }
    #[test]
    fn update_form_consumes_queued_keys_mouse_fragmented_paste_and_cancel_tail() {
        let mut coordinator = form();
        let mut queue = VecDeque::new();
        let before_display = Instant::now();
        coordinator
            .input(
                b"unexpected\r",
                before_display,
                &connection(),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        assert!(coordinator.flow.as_ref().unwrap().fields[0].is_empty());
        let mut rendered = Vec::new();
        coordinator.draw((80, 24), &mut rendered).unwrap();
        coordinator
            .input(
                b"queued\r",
                before_display,
                &connection(),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        assert!(coordinator.flow.as_ref().unwrap().fields[0].is_empty());
        coordinator
            .input(
                b"\x1b[<0;2;2M\x1b[200",
                Instant::now(),
                &connection(),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        coordinator
            .input(
                b"~/local/package\n\t\x1b[201~",
                Instant::now(),
                &connection(),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        let flow = coordinator.flow.as_ref().unwrap();
        assert_eq!(flow.fields[0], b"/local/package");
        assert_eq!(flow.field, 0);
        assert!(flow.job.is_none());
        let queued_tail = Instant::now();
        coordinator
            .input(
                b"\x03discard-tail",
                Instant::now(),
                &connection(),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        assert!(!coordinator.active());
        assert!(coordinator.discard_input(queued_tail));
        assert!(!coordinator.discard_input(Instant::now()));
        assert!(queue.iter().all(|p| p.tag != crate::protocol::KEYS));
    }
    #[test]
    fn update_rpc_accepts_only_owned_complete_bounded_responses() {
        let mut coordinator = Coordinator::default();
        let (sender, result) = mpsc::sync_channel(1);
        coordinator.reply = Some(Reply {
            id: 7,
            sender,
            length: None,
            success: false,
            bytes: Vec::new(),
        });
        let mut queue = VecDeque::new();
        assert!(
            coordinator
                .packet(
                    &Packet::new(protocol::BEGIN, 8, [0; 9]),
                    &mut queue,
                    (80, 24)
                )
                .is_err()
        );
        let mut header = vec![1];
        header.extend(2u64.to_be_bytes());
        coordinator
            .packet(
                &Packet::new(protocol::BEGIN, 7, header),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        assert!(
            coordinator
                .packet(
                    &Packet::new(protocol::DATA, 7, b"too large"),
                    &mut queue,
                    (80, 24)
                )
                .is_err()
        );
        coordinator
            .packet(&Packet::new(protocol::DATA, 7, b"{}"), &mut queue, (80, 24))
            .unwrap();
        coordinator
            .packet(
                &Packet::new(protocol::END, 7, Vec::new()),
                &mut queue,
                (80, 24),
            )
            .unwrap();
        assert_eq!(result.try_recv().unwrap().unwrap(), json!({}));
        assert!(
            coordinator
                .packet(
                    &Packet::new(protocol::END, 7, Vec::new()),
                    &mut queue,
                    (80, 24)
                )
                .is_err()
        );
    }
    #[test]
    fn unchanged_update_overlay_emits_nothing_and_narrow_form_stays_bounded() {
        let mut coordinator = form();
        let mut first = Vec::new();
        coordinator.draw((24, 8), &mut first).unwrap();
        assert!(!first.is_empty());
        let mut repeat = Vec::new();
        coordinator.draw((24, 8), &mut repeat).unwrap();
        assert!(repeat.is_empty());
        assert!(!String::from_utf8(first).unwrap().contains("\x1b[9;"));
    }
}
