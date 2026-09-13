//! Attachment-owned Kitty keyboard enhancement, negotiated only for Arcade.
//! https://sw.kovidgoyal.net/kitty/keyboard-protocol/
use super::game::KeyEvent;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const QUERY: &[u8] = b"\x1b[?u";
const PUSH: &[u8] = b"\x1b[>11u"; // disambiguate + event types + all keys
const POP: &[u8] = b"\x1b[<u";
const WAIT: Duration = Duration::from_secs(1);
const MAX_PENDING: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum QueryKind {
    Discover,
    Verify,
    Restore,
}
#[derive(Clone, Copy)]
struct Query {
    id: u64,
    generation: u64,
    kind: QueryKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Query,
    Verify,
    Active,
    Drain,
}

pub(in crate::ui) struct Keyboard {
    remote: bool,
    wanted: bool,
    owned: bool,
    phase: Phase,
    deadline: Instant,
    queries: VecDeque<Query>,
    generation: u64,
    next_query: u64,
    restore_fence: Option<u64>,
    discovery_deferred: bool,
    output: Vec<u8>,
}
impl Keyboard {
    pub(in crate::ui) fn new(remote: bool) -> Self {
        Self {
            remote,
            wanted: false,
            owned: false,
            phase: Phase::Idle,
            deadline: Instant::now(),
            queries: VecDeque::new(),
            generation: 0,
            next_query: 0,
            restore_fence: None,
            discovery_deferred: false,
            output: Vec::new(),
        }
    }
    fn query(&mut self, kind: QueryKind) -> u64 {
        // Discover reserves room for both verification and an owned restore.
        debug_assert!(self.queries.len() < MAX_PENDING);
        self.next_query = self.next_query.saturating_add(1);
        let id = self.next_query;
        self.queries.push_back(Query {
            id,
            generation: self.generation,
            kind,
        });
        self.output.extend_from_slice(QUERY);
        id
    }
    fn discover(&mut self, now: Instant) {
        if self.restore_fence.is_some() {
            // Reopening does not shorten the existing all-input drain window.
            self.discovery_deferred = true;
            return;
        }
        self.phase = Phase::Idle;
        if self.queries.len() >= MAX_PENDING - 2 {
            self.discovery_deferred = true;
            return;
        }
        self.discovery_deferred = false;
        self.phase = Phase::Query;
        self.deadline = now + WAIT;
        self.query(QueryKind::Discover);
    }
    fn resume_discovery(&mut self, now: Instant) {
        if self.wanted
            && self.discovery_deferred
            && self.restore_fence.is_none()
            && self.queries.len() < MAX_PENDING - 2
        {
            self.discover(now);
        }
    }
    pub(super) fn sync(&mut self, wanted: bool, now: Instant) {
        if !wanted && self.wanted {
            self.stop(now);
        } else if wanted && !self.wanted {
            self.wanted = true;
            self.generation = self.generation.saturating_add(1);
            self.discover(now);
        }
        if now >= self.deadline {
            match self.phase {
                Phase::Query => self.phase = Phase::Idle, // unsupported: ordinary keys remain usable
                Phase::Verify => self.restore(now),
                // Stop blocking ordinary input after a timeout, but do not bless
                // late structured key presses until this exact restore is fenced.
                Phase::Drain => self.phase = Phase::Idle,
                _ => {}
            }
        }
    }
    fn restore(&mut self, now: Instant) {
        if self.owned {
            self.output.extend_from_slice(POP);
            self.owned = false;
            self.restore_fence = Some(self.query(QueryKind::Restore));
        }
        self.phase = Phase::Drain;
        self.deadline = now + WAIT;
    }
    pub(in crate::ui) fn stop(&mut self, now: Instant) {
        self.wanted = false;
        self.discovery_deferred = false;
        if self.owned {
            self.restore(now);
        } else if self.phase != Phase::Drain {
            self.phase = Phase::Idle;
        }
    }
    pub(super) fn reply(&mut self, bytes: &[u8], now: Instant) -> bool {
        let Some(body) = bytes
            .strip_prefix(b"\x1b[?")
            .and_then(|b| b.strip_suffix(b"u"))
        else {
            return false;
        };
        let Some(flags) = number(body) else {
            return true;
        };
        let Some(query) = self.queries.pop_front() else {
            return true;
        };
        // Terminal replies contain no ID. Their ordered stream attributes them
        // to requests, including requests from an already-closed game. Never
        // discard that older request just because a newer game has opened.
        if query.kind == QueryKind::Restore {
            if self.restore_fence == Some(query.id) {
                self.restore_fence = None;
                if self.phase == Phase::Drain {
                    self.phase = Phase::Idle;
                }
            }
        } else if self.wanted && query.generation == self.generation {
            match (query.kind, self.phase) {
                (QueryKind::Discover, Phase::Query) if now < self.deadline => {
                    self.output.extend_from_slice(PUSH);
                    self.owned = true;
                    self.phase = Phase::Verify;
                    self.deadline = now + WAIT;
                    self.query(QueryKind::Verify);
                }
                (QueryKind::Verify, Phase::Verify) if flags & 11 == 11 => {
                    self.phase = Phase::Active;
                }
                (QueryKind::Verify, Phase::Verify) => self.restore(now),
                _ => {}
            }
        }
        self.resume_discovery(now);
        true // late/invalid replies never become shell input
    }
    pub(super) fn unfenced(&self) -> bool {
        self.restore_fence.is_some()
    }
    pub(super) fn enhanced(&self) -> bool {
        self.owned && matches!(self.phase, Phase::Verify | Phase::Active)
    }
    pub(super) fn drains(&self) -> bool {
        self.phase == Phase::Drain
    }
    pub(in crate::ui) fn flush(&mut self) {
        if !self.output.is_empty() {
            let output = std::mem::take(&mut self.output);
            let _ = super::super::remote::emit(self.remote, &output);
        }
    }
}
impl Drop for Keyboard {
    fn drop(&mut self) {
        self.stop(Instant::now());
        self.flush();
    }
}

fn number(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || bytes.len() > 7 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

pub(super) struct Event {
    pub key: Vec<u8>,
    pub kind: KeyEvent,
}

/// Only complete CSI keyboard reports, never paste or mouse/focus packets.
pub(super) fn event(bytes: &[u8]) -> Option<Event> {
    let bytes = bytes.strip_prefix(b"\x1b[")?;
    if bytes.len() > 64 {
        return None;
    }
    let (&end, parameters) = bytes.split_last()?;
    if !matches!(end, b'u' | b'A'..=b'D') || parameters.is_empty() {
        return None;
    }
    let fields = parameters.split(|b| *b == b';').collect::<Vec<_>>();
    if fields.len() > 3 {
        return None;
    }
    let code = number(fields[0].split(|b| *b == b':').next()?)?;
    let modifiers = fields.get(1).copied().unwrap_or(b"1");
    let mut parts = modifiers.split(|b| *b == b':');
    let modifiers = number(parts.next()?)?.checked_sub(1)?;
    let event_type = match parts.next() {
        Some(value) => number(value)?,
        None => 1,
    };
    let kind = match event_type {
        1 => KeyEvent::Press,
        2 => KeyEvent::Repeat,
        3 => KeyEvent::Release,
        _ => return None,
    };
    if parts.next().is_some() {
        return None;
    }
    let mut key = match (end, code) {
        (b'A'..=b'D', 1) => vec![27, b'[', end],
        (b'u', 57417) => b"\x1b[D".to_vec(),
        (b'u', 57418) => b"\x1b[C".to_vec(),
        (b'u', 57419) => b"\x1b[A".to_vec(),
        (b'u', 57420) => b"\x1b[B".to_vec(),
        (b'u', 57414 | 13) => vec![b'\r'],
        (b'u', 27) => vec![27],
        (b'u', 32..=126) => vec![(code as u8).to_ascii_lowercase()],
        (b'u', _) => Vec::new(), // modifier-only and unused keys are still consumed
        _ => return None,
    };
    // Releasing a binding still clears it if Ctrl/Alt was pressed after it.
    if modifiers & !(1 | 64 | 128) != 0 && kind != KeyEvent::Release {
        key.clear();
    }
    Some(Event { key, kind })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_keyboard_reports_preserve_event_types_and_ignore_modifiers() {
        for (bytes, key, kind) in [
            (
                b"\x1b[1;1:1C".as_slice(),
                b"\x1b[C".as_slice(),
                KeyEvent::Press,
            ),
            (b"\x1b[1;1:2D", b"\x1b[D", KeyEvent::Repeat),
            (b"\x1b[104;5:3u", b"h", KeyEvent::Release),
            (b"\x1b[32u", b" ", KeyEvent::Press),
            (b"\x1b[13;1:3u", b"\r", KeyEvent::Release),
            (b"\x1b[27u", b"\x1b", KeyEvent::Press),
            (b"\x1b[108:76;2u", b"l", KeyEvent::Press),
            (b"\x1b[32;5u", b"", KeyEvent::Press),
        ] {
            let got = event(bytes).unwrap();
            assert_eq!(got.key, key);
            assert_eq!(got.kind, kind);
        }
        for bad in [
            b"\x1b[A".as_slice(),
            b"\x1b[?11u",
            b"\x1b[<0;1;1M",
            b"\x1b[32;1:0u",
            b"\x1b[32;1:4u",
            b"\x1b[32;0u",
        ] {
            assert!(event(bad).is_none(), "{bad:?}");
        }
    }
    #[test]
    fn negotiated_stack_is_owned_and_exit_reply_fences_late_events() {
        let now = Instant::now();
        let mut k = Keyboard::new(false);
        k.sync(true, now);
        assert_eq!(std::mem::take(&mut k.output), QUERY);
        assert!(k.reply(b"\x1b[?0u", now));
        assert_eq!(std::mem::take(&mut k.output), [PUSH, QUERY].concat());
        assert!(k.enhanced());
        k.stop(now);
        assert_eq!(std::mem::take(&mut k.output), [POP, QUERY].concat());
        assert!(k.drains());
        k.reply(b"\x1b[?11u", now); // outstanding verification precedes restore fence
        assert!(k.drains());
        k.reply(b"\x1b[?0u", now);
        assert!(!k.drains());
        k.stop(now);
        assert!(k.output.is_empty());
        let mut unsupported = Keyboard::new(false);
        unsupported.sync(true, now);
        unsupported.output.clear();
        unsupported.sync(true, now + WAIT);
        unsupported.reply(b"\x1b[?0u", now + WAIT);
        unsupported.stop(now + WAIT);
        assert!(unsupported.output.is_empty());
    }
    #[test]
    fn old_discovery_reply_cannot_negotiate_a_quickly_reopened_game() {
        let now = Instant::now();
        let mut k = Keyboard::new(false);
        k.sync(true, now);
        k.stop(now);
        k.sync(true, now);
        assert_eq!(std::mem::take(&mut k.output), [QUERY, QUERY].concat());
        k.reply(b"\x1b[?0u", now); // The first, cancelled discovery.
        assert!(!k.enhanced());
        assert!(k.output.is_empty());
        k.reply(b"\x1b[?0u", now); // Discovery for the current generation.
        assert_eq!(std::mem::take(&mut k.output), [PUSH, QUERY].concat());
        k.reply(b"\x1b[?11u", now);
        assert!(k.enhanced());
        k.stop(now);
        assert_eq!(std::mem::take(&mut k.output), [POP, QUERY].concat());
        k.reply(b"\x1b[?0u", now);
        assert!(!k.unfenced());
        k.stop(now);
        assert!(k.output.is_empty());
    }
    #[test]
    fn restore_timeout_keeps_its_fence_and_defers_reopen_until_the_exact_reply() {
        let now = Instant::now();
        let mut k = Keyboard::new(false);
        k.sync(true, now);
        k.reply(b"\x1b[?0u", now);
        k.output.clear();
        // Verification is still outstanding when the game closes.
        k.stop(now);
        assert_eq!(std::mem::take(&mut k.output), [POP, QUERY].concat());
        let later = now + WAIT + Duration::from_millis(10);
        k.sync(false, later);
        assert!(!k.drains());
        assert!(k.unfenced());
        k.sync(true, later);
        assert!(!k.enhanced());
        assert!(k.output.is_empty());
        k.reply(b"\x1b[?11u", later); // Old verification does not clear restoration.
        assert!(k.unfenced());
        assert!(k.output.is_empty());
        k.reply(b"\x1b[?bad-u", later); // Malformed input cannot retire a request.
        assert!(k.unfenced());
        k.reply(b"\x1b[?0u", later); // The owned restore's matching FIFO entry.
        assert!(!k.unfenced());
        assert_eq!(std::mem::take(&mut k.output), QUERY);
        k.reply(b"\x1b[?0u", later);
        assert_eq!(std::mem::take(&mut k.output), [PUSH, QUERY].concat());
        k.reply(b"\x1b[?11u", later);
        assert!(k.enhanced());
        k.stop(later);
        assert_eq!(std::mem::take(&mut k.output), [POP, QUERY].concat());
        k.reply(b"\x1b[?0u", later);
        assert!(!k.unfenced());
    }
    #[test]
    fn unanswered_discoveries_are_bounded_and_never_push_for_an_old_generation() {
        let now = Instant::now();
        let mut k = Keyboard::new(false);
        for _ in 0..100 {
            k.sync(true, now);
            k.stop(now);
        }
        assert!(k.queries.len() <= MAX_PENDING - 2);
        assert!(!k.owned);
        k.output.clear();
        k.sync(true, now); // Deferred until an old request vacates the bounded FIFO.
        while k.queries.len() > 1 {
            k.reply(b"\x1b[?0u", now);
            assert!(!k.enhanced());
        }
        assert_eq!(std::mem::take(&mut k.output), QUERY);
        k.reply(b"\x1b[?0u", now);
        assert_eq!(std::mem::take(&mut k.output), [PUSH, QUERY].concat());
        k.reply(b"\x1b[?11u", now);
        k.stop(now);
        k.reply(b"\x1b[?0u", now);
        k.output.clear();
    }
}
