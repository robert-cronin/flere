//! Supervisor-owned mailbox delivery. Native queue acceptance is not handling.
use super::*;
use crate::{
    inbox_hook::{Input, notice},
    native::delivery::Target,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct State {
    pub hooks: Vec<Hook>,
    pub focus: Vec<Focus>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Hook {
    epoch: String,
    session: u64,
    run: String,
    target: Target,
    event: String,
    turn: String,
    observed: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Focus {
    epoch: String,
    session: u64,
    run: String,
    until: u64,
    reason: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Receipt {
    pub outcome: String,
    pub detail: String,
    pub epoch: String,
    pub session: u64,
    pub run: String,
    pub conversation: String,
    pub updated: u64,
    pub lease: String,
    pub expires: u64,
}
pub(super) struct Job {
    id: String,
    child: std::process::Child,
    started: Instant,
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn pending(m: &coordination::Message) -> bool {
    m.acknowledged.is_none()
        && m.native_surfaced.is_none()
        && m.delivery.as_ref().is_none_or(|d| {
            !matches!(
                d.outcome.as_str(),
                "queue-prepared"
                    | "queued"
                    | "unknown"
                    | "queue-submitted"
                    | "hook-prepared"
                    | "hook-emitted"
                    | "mcp-returned"
            )
        })
}

impl State {
    pub(super) fn validate(&self) -> io::Result<()> {
        let token = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
        if self.hooks.len() > 4096 || self.focus.len() > 4096 {
            return Err(invalid("mailbox observation limit reached"));
        }
        if self.hooks.iter().any(|h| {
            !token(&h.epoch)
                || !token(&h.run)
                || h.session == 0
                || h.turn.len() > 1024
                || !crate::inbox_hook::EVENTS.contains(&h.event.as_str())
                || !crate::native::valid_uuid(&h.target.uuid)
                || !h.target.transcript.is_absolute()
                || !h.target.cwd.is_absolute()
                || !h.target.exe.is_absolute()
        }) || self
            .focus
            .iter()
            .any(|f| !token(&f.epoch) || !token(&f.run) || f.session == 0 || f.reason.len() > 1024)
        {
            return Err(invalid("invalid mailbox observation identity"));
        }
        Ok(())
    }
}
impl Server {
    pub(super) fn save_mailbox(&mut self, next: coordination::Coordination) -> io::Result<()> {
        next.delivery.validate()?;
        next.validate_chat_messages()?;
        let old = std::mem::replace(&mut self.coordination, next);
        if let Err(e) = self.persist() {
            self.coordination = old;
            return Err(e);
        }
        self.changed();
        Ok(())
    }
    pub(super) fn native_scope(
        &self,
        session: u64,
        run: &str,
    ) -> io::Result<(u64, &Session, &Workspace)> {
        for w in &self.workspaces {
            if !w.meta.archived
                && let Some(s) = w.tabs.iter().find(|s| {
                    s.id == session && s.run == run && s.alive && !s.ended && s.native.is_some()
                })
            {
                return Ok((w.id, s, w));
            }
        }
        Err(invalid("stale or non-native mailbox target"))
    }
    pub(super) fn proof(&self, session: u64, run: &str) -> io::Result<Target> {
        let (_, s, _) = self.native_scope(session, run)?;
        let spec = s.native.as_ref().unwrap();
        if spec.harness != "codex" {
            return Err(invalid("automatic delivery currently requires Codex"));
        }
        let proof = crate::native::delivery::inspect(s.child.id(), &spec.cwd, session, run)?;
        if !spec.conversation.is_empty() && spec.conversation != proof.uuid {
            return Err(invalid("native conversation changed"));
        }
        Ok(proof)
    }
    pub(super) fn recipient(&self, wid: u64) -> io::Result<(u64, String)> {
        let w = self
            .workspaces
            .iter()
            .find(|w| w.id == wid && !w.meta.archived)
            .ok_or_else(|| invalid("recipient workspace is stopped or archived"))?;
        let tabs: Vec<_> = w
            .tabs
            .iter()
            .filter(|s| s.alive && !s.ended && s.native.is_some())
            .collect();
        if tabs.len() != 1 {
            return Err(invalid(
                "recipient needs exactly one live native run; no target selected",
            ));
        }
        Ok((tabs[0].id, tabs[0].run.clone()))
    }
    fn focus(&self, session: u64, run: &str) -> Option<&Focus> {
        self.coordination.delivery.focus.iter().find(|f| {
            f.epoch == self.epoch && f.session == session && f.run == run && f.until > now()
        })
    }
    fn observation(&self, session: u64, run: &str) -> Option<&Hook> {
        self.coordination
            .delivery
            .hooks
            .iter()
            .find(|h| h.epoch == self.epoch && h.session == session && h.run == run)
    }
    fn hooks_configured(&self, session: u64, run: &str) -> bool {
        self.native_scope(session, run)
            .ok()
            .and_then(|(_, s, _)| s.native.as_ref())
            .is_some_and(|s| {
                crate::inbox_hook::EVENTS.iter().all(|event| {
                    s.argv
                        .iter()
                        .any(|a| a.starts_with(&format!("hooks.{event}=")))
                })
            })
    }
    pub(super) fn activation_at(&self, wid: u64, recipient: Option<(u64, String)>) -> Value {
        let Some((session, run)) = recipient else {
            return json!({"state":"target-unavailable","detail":"Select exactly one live native recipient. Stopped work is never launched by delivery."});
        };
        let proof = self.proof(session, &run).ok();
        let configured = self.hooks_configured(session, &run);
        let observed = self.observation(session, &run);
        json!({"state":if observed.is_some(){"observed"}else if configured{"unobserved"}else{"activation-required"},
            "workspace":wid,"session":session,"run":run,"configured":configured,
            "conversation":proof.as_ref().map(|p|&p.uuid),"observation":observed,"focus":self.focus(session,&run),
            "chat_messages":{"operation":"send_chat_message","address":"workspace and verified native conversation","retry":"reuse request_id","cli_fallback":"flere --state \"$FLERE_STATE\" agent-call send_chat_message '<JSON arguments>'"},
            "instructions":"Configured hooks may not run until the first turn. A verified empty idle composer can receive its first notice through the native queue before any hook is observed. Later delivery respects observed activity and permission events. If hooks are absent, explicitly resume this exact UUID with the current launcher. Review native trust prompts yourself; configuration does not prove trust. Flere tool replies can surface pending notices. Messaging never restarts stopped chats."})
    }
    pub(super) fn delivery_operation(
        &mut self,
        wid: u64,
        native: Option<(u64, &str)>,
        op: &str,
        args: &Value,
    ) -> io::Result<Value> {
        if op == "messaging_activation" {
            return Ok(self.activation_at(
                wid,
                native
                    .map(|(s, r)| (s, r.to_owned()))
                    .or_else(|| self.recipient(wid).ok()),
            ));
        }
        if op == "set_focus" {
            let (session, run) = native
                .map(|(s, r)| (s, r.to_owned()))
                .ok_or_else(|| invalid("set_focus requires your exact native run"))?;
            let seconds = args["seconds"]
                .as_u64()
                .filter(|s| *s <= 1800)
                .ok_or_else(|| invalid("seconds must be 0..1800"))?;
            let reason = args["reason"].as_str().unwrap_or("");
            if reason.len() > 1024 || (seconds > 0 && reason.trim().is_empty()) {
                return Err(invalid("DND requires a bounded reason"));
            }
            let mut next = self.coordination.clone();
            next.delivery
                .focus
                .retain(|f| f.run != run || f.epoch != self.epoch);
            if seconds > 0 {
                next.delivery.focus.push(Focus {
                    epoch: self.epoch.clone(),
                    session,
                    run: run.clone(),
                    until: now() + seconds * 1000,
                    reason: reason.into(),
                });
            }
            self.audit(
                "mailbox-focus",
                wid,
                &format!("session={session};run={run};seconds={seconds};reason={reason}"),
            )?;
            self.save_mailbox(next)?;
            return Ok(json!({"focus":self.focus(session,&run)}));
        }
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("missing message id"))?;
        let m = self
            .coordination
            .messages
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| invalid("unknown message"))?;
        let conversation = self.mailbox_conversation(wid, native);
        let recipient = m.to == wid && m.for_conversation(conversation.as_deref());
        let sender = m.from == wid && m.sent_by_conversation(conversation.as_deref());
        if native.is_some() && !recipient && !sender {
            return Err(invalid(
                "message is outside your workspace or native conversation",
            ));
        }
        if op == "message_status" {
            if native.is_some() && recipient && m.native_surfaced.is_none() {
                let mut next = self.coordination.clone();
                let m = next.messages.iter_mut().find(|x| x.id == id).unwrap();
                m.surfaced.get_or_insert(now() / 1000);
                m.native_surfaced = Some(now() / 1000);
                self.save_mailbox(next)?;
            }
            return Ok(
                json!({"message":self.coordination.messages.iter().find(|x|x.id==id),"activation":self.activation_at(m.to,self.message_recipient(&m).ok())}),
            );
        }
        if op == "deliver_message" {
            let session = args["session"]
                .as_u64()
                .ok_or_else(|| invalid("fresh exact recipient session required"))?;
            let run = args["run"]
                .as_str()
                .ok_or_else(|| invalid("fresh exact recipient run required"))?;
            let (actual, s, _) = self.native_scope(session, run)?;
            if actual != m.to || s.run != run {
                return Err(invalid("wrong recipient"));
            }
            if self.message_recipient(&m)? != (session, run.to_owned()) {
                return Err(invalid("ambiguous recipient"));
            }
            self.try_delivery(id)?;
            return Ok(json!({"message":self.coordination.messages.iter().find(|x|x.id==id)}));
        }
        Err(invalid("unknown delivery operation"))
    }
    pub(super) fn observe_hook(
        &mut self,
        session: u64,
        run: &str,
        event: Input,
    ) -> io::Result<Value> {
        if !event.agent_id.is_empty() {
            return Ok(json!({"output":{}}));
        }
        if event.turn_id.len() > 1024
            || event.transcript_path.len() > 4096
            || event.tool_name.len() > 1024
            || event.session_id.len() != 36
        {
            return Err(invalid("native hook identity exceeds bounds"));
        }
        if !crate::inbox_hook::EVENTS.contains(&event.hook_event_name.as_str()) {
            return Err(invalid("unknown hook event"));
        }
        let (wid, _, _) = self.native_scope(session, run)?;
        let target = self.proof(session, run)?;
        if event.session_id != target.uuid || Path::new(&event.transcript_path) != target.transcript
        {
            return Err(invalid(
                "hook does not belong to exact native conversation/transcript",
            ));
        }
        let previous = self.observation(session, run);
        let name = event.hook_event_name.as_str();
        if matches!(name, "PreToolUse" | "PostToolUse" | "PermissionRequest")
            && (event.turn_id.is_empty() || previous.is_none_or(|h| h.turn != event.turn_id))
        {
            return Err(invalid("tool hook is outside the observed main turn"));
        }
        if matches!(name, "UserPromptSubmit" | "Stop" | "Interrupt") && event.turn_id.is_empty() {
            return Err(invalid("missing main turn identity"));
        }
        if matches!(name, "Stop" | "Interrupt")
            && previous.is_some_and(|h| !h.turn.is_empty() && h.turn != event.turn_id)
        {
            return Err(invalid("stale main turn hook"));
        }
        if name == "SessionStart" && previous.is_some_and(|h| !h.turn.is_empty()) {
            return Ok(json!({"output":{}}));
        }
        let turn = if event.turn_id.is_empty()
            && matches!(name, "PreCompact" | "PostCompact" | "SessionEnd")
        {
            previous.map(|h| h.turn.clone()).unwrap_or_default()
        } else {
            event.turn_id
        };
        let mut next = self.coordination.clone();
        if name == "UserPromptSubmit"
            && let Some(submitted) = event.queue_notice
            && submitted.workspace == wid
            && let Some(m) = next.messages.iter_mut().find(|m| {
                m.id == submitted.id && m.to == wid && m.for_conversation(Some(&target.uuid))
            })
            && let Some(d) = &mut m.delivery
            && d.conversation == target.uuid
            && d.session == submitted.session
            && d.run == submitted.run
            && matches!(d.outcome.as_str(), "queue-prepared" | "queued" | "unknown")
        {
            // The input reached a native hook, even if another hook blocks it or
            // the model's subsequent tools fail. This is not a payload read,
            // model-consumption receipt, acknowledgment, or human approval.
            // The original attempt identity survives a same-conversation resume.
            d.outcome = "queue-submitted".into();
            d.detail = "Exact queued notice observed at native prompt submission; reading and handling remain unconfirmed.".into();
            d.updated = now();
            self.audit(
                "native-queue-submitted",
                wid,
                &format!("message={};session={session};run={run};turn={turn}", m.id),
            )?;
        }
        next.delivery
            .hooks
            .retain(|h| h.epoch != self.epoch || h.run != run);
        next.delivery.hooks.push(Hook {
            epoch: self.epoch.clone(),
            session,
            run: run.into(),
            target,
            event: name.into(),
            turn,
            observed: now(),
        });
        self.save_mailbox(next)?;
        if self.focus(session, run).is_some()
            || !matches!(name, "PostToolUse" | "Stop")
            || (name == "Stop" && event.stop_hook_active)
            || (name == "PostToolUse"
                && (event.tool_name.ends_with("flere__inbox")
                    || event.tool_name.ends_with("flere__get_context")))
        {
            return Ok(json!({"output":{}}));
        }
        let conversation = self.mailbox_conversation(wid, Some((session, run)));
        let ids: Vec<_> = self
            .coordination
            .messages
            .iter()
            .filter(|m| {
                m.to == wid
                    && m.for_conversation(conversation.as_deref())
                    && m.acknowledged.is_none()
                    && m.native_surfaced.is_none()
                    && (pending(m)
                        || m.delivery
                            .as_ref()
                            .is_some_and(|d| d.outcome == "hook-prepared" && d.expires < now()))
                    && (name != "Stop" || m.intent == "interrupt")
            })
            .take(8)
            .map(|m| m.id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(json!({"output":{}}));
        }
        let lease = os::nonce()?;
        let mut next = self.coordination.clone();
        for m in next.messages.iter_mut().filter(|m| ids.contains(&m.id)) {
            m.delivery = Some(Receipt {
                outcome: "hook-prepared".into(),
                detail: "Native hook context prepared; not yet confirmed emitted.".into(),
                epoch: self.epoch.clone(),
                session,
                run: run.into(),
                conversation: event.session_id.clone(),
                updated: now(),
                lease: lease.clone(),
                expires: now() + 6000,
            });
        }
        self.audit(
            "prepare-hook-notice",
            wid,
            &format!(
                "session={session};run={run};lease={lease};ids={}",
                ids.join(",")
            ),
        )?;
        self.save_mailbox(next)?;
        let text = notice(wid, session, run, &ids);
        let output = if name == "Stop" {
            json!({"decision":"block","reason":text})
        } else {
            json!({"hookSpecificOutput":{"hookEventName":name,"additionalContext":text}})
        };
        Ok(json!({"output":output,"lease":lease}))
    }
    pub(super) fn confirm_notice(
        &mut self,
        session: u64,
        run: &str,
        lease: &str,
    ) -> io::Result<()> {
        let (wid, _, _) = self.native_scope(session, run)?;
        let conversation = self.mailbox_conversation(wid, Some((session, run)));
        let mut next = self.coordination.clone();
        let mut found = false;
        for m in &mut next.messages {
            if m.to == wid
                && m.for_conversation(conversation.as_deref())
                && let Some(d) = &mut m.delivery
                && d.epoch == self.epoch
                && d.session == session
                && d.run == run
                && d.lease == lease
                && d.outcome == "hook-prepared"
            {
                d.outcome = "hook-emitted".into();
                d.detail =
                    "Hook context emitted; handling still requires explicit acknowledgement."
                        .into();
                d.updated = now();
                m.surfaced.get_or_insert(now() / 1000);
                m.native_surfaced.get_or_insert(now() / 1000);
                found = true;
            }
        }
        if !found {
            return Err(invalid("unknown or stale hook notice lease"));
        }
        self.audit(
            "confirm-hook-notice",
            wid,
            &format!("session={session};run={run};lease={lease}"),
        )?;
        self.save_mailbox(next)
    }
    /// Old MCP adapters still receive this field through their ordinary exact-run response.
    pub(super) fn attach_notice(
        &mut self,
        wid: u64,
        session: u64,
        run: &str,
        value: &mut Value,
    ) -> io::Result<()> {
        if self.focus(session, run).is_some() || !value.is_object() {
            return Ok(());
        }
        let conversation = self.mailbox_conversation(wid, Some((session, run)));
        let ids: Vec<_> = self
            .coordination
            .messages
            .iter()
            .filter(|m| m.to == wid && m.for_conversation(conversation.as_deref()) && pending(m))
            .take(8)
            .map(|m| m.id.clone())
            .collect();
        if ids.is_empty() {
            return Ok(());
        }
        let mut next = self.coordination.clone();
        for m in next.messages.iter_mut().filter(|m| ids.contains(&m.id)) {
            m.surfaced.get_or_insert(now() / 1000);
            m.native_surfaced = Some(now() / 1000);
            m.delivery = Some(Receipt {
                outcome: "mcp-returned".into(),
                detail:
                    "Notice returned in an exact-run tool response; handling is not acknowledged."
                        .into(),
                epoch: self.epoch.clone(),
                session,
                run: run.into(),
                conversation: String::new(),
                updated: now(),
                lease: String::new(),
                expires: 0,
            });
        }
        self.save_mailbox(next)?;
        value["mailbox_notice"] = json!(notice(wid, session, run, &ids));
        Ok(())
    }
    fn waiting(&mut self, id: &str, outcome: &str, detail: &str) -> io::Result<()> {
        let m = self
            .coordination
            .messages
            .iter()
            .find(|m| m.id == id)
            .unwrap();
        if !pending(m)
            || m.delivery
                .as_ref()
                .is_some_and(|d| d.outcome == outcome && d.detail == detail)
        {
            return Ok(());
        }
        let mut next = self.coordination.clone();
        next.messages
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .delivery = Some(Receipt {
            outcome: outcome.into(),
            detail: detail.into(),
            epoch: self.epoch.clone(),
            session: 0,
            run: String::new(),
            conversation: String::new(),
            updated: now(),
            lease: String::new(),
            expires: 0,
        });
        self.save_mailbox(next)
    }
    pub(super) fn try_delivery(&mut self, id: &str) -> io::Result<()> {
        let m = self
            .coordination
            .messages
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| invalid("unknown message"))?;
        if m.to == 0 || !pending(&m) {
            return Ok(());
        }
        if self.delivery_jobs.len() >= 4 {
            return self.waiting(
                id,
                "waiting-for-capacity",
                "Four native queue helpers are in flight; delivery will retry.",
            );
        }
        let (session, run) = match self.message_recipient(&m) {
            Ok(recipient) => recipient,
            Err(e) => {
                return self.waiting(id, "target-unavailable", &wire::passive(&e.to_string()));
            }
        };
        if self.focus(session, &run).is_some() {
            return self.waiting(
                id,
                "deferred",
                "Recipient has an active do-not-disturb interval.",
            );
        }
        let hook = self.observation(session, &run);
        if let Some(hook) = hook {
            if hook.event == "PermissionRequest" {
                return self.waiting(
                    id,
                    "waiting-for-human",
                    "Native permission prompt remains human-controlled.",
                );
            }
            if !matches!(hook.event.as_str(), "Stop" | "Interrupt" | "SessionStart") {
                return self.waiting(
                    id,
                    "waiting-for-hook",
                    "Recipient is active or activity is unknown; wait for a native hook boundary.",
                );
            }
            if now().saturating_sub(hook.observed) < 1000 {
                return self.waiting(id, "waiting-for-idle", "Idle boundary is settling.");
            }
        } else if !self.hooks_configured(session, &run) {
            return self.waiting(
                id,
                "activation-required",
                "Native activity hooks are not configured for this run. Read messaging_activation.",
            );
        }
        // Codex defers SessionStart until its first turn. Requiring that hook
        // before queuing the first notice deadlocks a freshly resumed idle chat.
        // The same exact-owner, composer, DND and receipt guards still apply;
        // no observation or native trust is inferred from configured flags.
        let target = match self.proof(session, &run) {
            Ok(t) if hook.is_none_or(|h| t == h.target) && m.for_conversation(Some(&t.uuid)) => t,
            _ => {
                return self.waiting(
                    id,
                    "target-unavailable",
                    "Exact native ownership changed or cannot be verified.",
                );
            }
        };
        let (_, s, _) = self.native_scope(session, &run)?;
        if !s.input.is_empty() {
            return self.waiting(
                id,
                "waiting-for-idle",
                "User input is still pending; no native message submitted.",
            );
        }
        if let Some(reason) = crate::native::delivery::composer_block(&s.term) {
            return self.waiting(id, "waiting-for-idle", reason);
        }
        let recipient_workspace = m.to;
        if self.coordination.messages.iter().any(|m| {
            m.acknowledged.is_none()
                && m.native_surfaced.is_none()
                && m.delivery.as_ref().is_some_and(|d| {
                    (d.run == run || (m.to == recipient_workspace && d.conversation == target.uuid))
                        && matches!(d.outcome.as_str(), "queue-prepared" | "queued" | "unknown")
                })
        }) {
            return self.waiting(id,"waiting-for-receipt","An earlier native handoff to this conversation is awaiting a surface receipt; no duplicate wake-up.");
        }
        let text = notice(m.to, session, &run, std::slice::from_ref(&m.id));
        let mut command =
            crate::native::delivery::queue_command(&target, &self.state, session, &run, &text)?;
        // Recheck after environment/proof preparation. The supervisor serializes DND, ack and launch.
        if !pending(
            self.coordination
                .messages
                .iter()
                .find(|m| m.id == id)
                .unwrap(),
        ) || self.focus(session, &run).is_some()
            || self.proof(session, &run)? != target
        {
            return Ok(());
        }
        let mut next = self.coordination.clone();
        next.messages.iter_mut().find(|m|m.id==id).unwrap().delivery=Some(Receipt{outcome:"queue-prepared".into(),detail:"Native queue handoff reserved; uncertain outcomes are never automatically replayed.".into(),epoch:self.epoch.clone(),session,run:run.clone(),conversation:target.uuid,updated:now(),lease:String::new(),expires:0});
        self.audit(
            "native-queue-request",
            m.to,
            &format!(
                "message={id};session={session};run={run};pid={}",
                target.pid
            ),
        )?;
        self.save_mailbox(next)?;
        match command.spawn() {
            Ok(child) => self.delivery_jobs.push(Job {
                id: id.into(),
                child,
                started: Instant::now(),
            }),
            Err(_) => self.finish_delivery(
                id,
                "unknown",
                "Native queue could not start; inspect this attempt before retrying.",
            )?,
        }
        Ok(())
    }
    fn finish_delivery(&mut self, id: &str, outcome: &str, detail: &str) -> io::Result<()> {
        let mut next = self.coordination.clone();
        let m = next
            .messages
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| invalid("unknown queue message"))?;
        if let Some(d) = &mut m.delivery
            && d.outcome == "queue-prepared"
        {
            d.outcome = outcome.into();
            d.detail = detail.into();
            d.updated = now();
        }
        self.audit(
            "native-queue-result",
            m.to,
            &format!("message={id};outcome={outcome}"),
        )?;
        self.save_mailbox(next)
    }
    pub(super) fn delivery_tick(&mut self) {
        let mut i = 0;
        while i < self.delivery_jobs.len() {
            let job = &mut self.delivery_jobs[i];
            let result = job.child.try_wait();
            let finished = match result {
                Ok(Some(s)) => Some(s.success()),
                Ok(None) if job.started.elapsed() < Duration::from_secs(5) => None,
                _ => Some(false),
            };
            if let Some(ok) = finished {
                let mut job = self.delivery_jobs.remove(i);
                if !ok {
                    let _ = job.child.kill();
                    let _ = job.child.wait();
                }
                let _ = self.finish_delivery(
                    &job.id,
                    if ok { "queued" } else { "unknown" },
                    if ok {
                        "Native accepted the queue request; not yet surfaced or acknowledged."
                    } else {
                        "Native queue outcome is uncertain; no automatic retry or input fallback."
                    },
                );
            } else {
                i += 1;
            }
        }
        if self.stop || self.refresh.is_some() || os::stopping() {
            return;
        }
        if self.delivery_checked.elapsed() < Duration::from_millis(500) {
            return;
        }
        self.delivery_checked = Instant::now();
        let len = self.coordination.messages.len();
        if len == 0 {
            return;
        }
        let start = self.delivery_cursor % len;
        let ids: Vec<_> = (0..len)
            .map(|offset| (start + offset) % len)
            .filter_map(|index| {
                let m = &self.coordination.messages[index];
                (m.to != 0 && pending(m)).then(|| (index, m.id.clone()))
            })
            .take(16)
            .collect();
        for (index, id) in ids {
            if self.delivery_jobs.len() >= 4 {
                break;
            }
            self.delivery_cursor = (index + 1) % len;
            if let Err(e) = self.try_delivery(&id) {
                let _ = self.waiting(&id, "target-unavailable", &wire::passive(&e.to_string()));
            }
        }
    }
    pub(super) fn stop_delivery_jobs(&mut self) {
        for mut job in self.delivery_jobs.drain(..) {
            let _ = job.child.kill();
            let _ = job.child.wait();
        }
    }
}
