//! An attachment-local overlay, never a second event loop or a session transition.
use super::*;

pub(super) struct Saver {
    pub active: bool,
    last_activity: Instant,
    started: Instant,
    frame: usize,
    // A wake consumes this read burst and any incomplete sequence/paste it started.
    burst: bool,
    partial: bool,
    mouse: bool,
    context_mouse: bool,
    utf8_remaining: u8,
    duck: intro::Duck,
    flere: intro::Flere,
    mascot: crate::pet::ScreensaverMascot,
}
impl Saver {
    pub fn new(now: Instant) -> Self {
        Self {
            active: false,
            last_activity: now,
            started: now,
            frame: 0,
            burst: false,
            partial: false,
            mouse: false,
            context_mouse: false,
            utf8_remaining: 0,
            duck: intro::Duck::new(now),
            flere: intro::Flere::new(now),
            mascot: crate::pet::ScreensaverMascot::default(),
        }
    }
    pub fn begin_burst(&mut self) {
        self.burst = self.active || self.partial;
    }
    pub fn end_burst(&mut self, input: &Input, pending_packet: bool) {
        self.partial = self.burst
            && (input.paste || !input.buf.is_empty() || pending_packet || self.utf8_remaining != 0);
        self.burst = false;
    }
    fn enter(&mut self, now: Instant) {
        self.active = true;
        self.started = now;
        self.frame = 0;
        self.duck = intro::Duck::new(now);
        self.flere.reset(now);
    }
    fn wake(&mut self, now: Instant) {
        self.active = false;
        self.last_activity = now;
        self.burst = true;
    }
    pub fn consume(&mut self, key: &Key, now: Instant) -> bool {
        if matches!(key, Key::TerminalReply(_))
            || matches!(key, Key::Bytes(b) if b == b"\x1b[I" || b == b"\x1b[O")
        {
            return self.active || self.burst;
        }
        self.last_activity = now;
        if self.active {
            if matches!(key, Key::Bytes(_) | Key::Paste(_)) {
                self.wake(now);
            } else {
                if self.mascot == crate::pet::ScreensaverMascot::Flere {
                    self.flere.pointer(key, now);
                } else {
                    self.duck.pointer(key, now);
                }
            }
        }
        // A fresh press ends ownership of a prior press whose release was lost
        // (for example when the terminal lost focus). Do not trap the new drag.
        if !self.burst && matches!(key, Key::Mouse { .. }) {
            self.mouse = false;
        }
        if !self.burst && matches!(key, Key::Context { .. }) {
            self.context_mouse = false;
        }
        let consume = self.active
            || self.burst
            || (self.mouse && matches!(key, Key::Drag { .. } | Key::Release { .. }))
            || ((self.mouse || self.context_mouse) && matches!(key, Key::Hover { .. }))
            || matches!(key, Key::ContextRelease | Key::PointerActivity);
        if consume && matches!(key, Key::Mouse { .. }) {
            self.mouse = true;
        }
        if matches!(key, Key::Release { .. }) {
            self.mouse = false;
        }
        if consume && matches!(key, Key::Context { .. }) {
            self.context_mouse = true;
        }
        if matches!(key, Key::ContextRelease) {
            self.context_mouse = false;
        }
        if consume && let Key::Bytes(bytes) = key {
            // Input normally forwards UTF-8 bytes unchanged. A wake can straddle
            // reads inside one codepoint, so retain its remaining continuation bytes.
            for byte in bytes {
                self.utf8_remaining = match byte {
                    0x80..=0xbf => self.utf8_remaining.saturating_sub(1),
                    0xc2..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf4 => 3,
                    _ => 0,
                };
            }
        }
        consume
    }
    fn due(&self, now: Instant, minutes: u16, safe: bool) -> bool {
        !self.active
            && safe
            && !self.partial
            && !self.mouse
            && !self.context_mouse
            && minutes != 0
            && now.saturating_duration_since(self.last_activity)
                >= Duration::from_secs(u64::from(minutes) * 60)
    }
}

impl Ui {
    pub(super) fn finish_input_burst(&mut self) {
        self.arcade_gate.end_burst(
            &self.input,
            self.remote.as_ref().is_some_and(|r| !r.buffer.is_empty()),
        );
        self.screensaver.end_burst(
            &self.input,
            self.remote.as_ref().is_some_and(|r| !r.buffer.is_empty()),
        );
    }
    pub(super) fn feed_input(&mut self, bytes: &[u8]) {
        for key in self.input.feed(bytes) {
            self.key(key);
        }
        // The parser buffers short bracketed pastes until their terminator. Wake
        // immediately on their opening, but keep ownership through every fragment.
        if self.input.paste && self.screensaver.active {
            self.screensaver.wake(Instant::now());
        }
    }
    fn screensaver_safe(&self) -> bool {
        self.arcade.is_none()
            && !self.menu
            && self.form.is_none()
            && self.search.is_none()
            && self.update.is_none()
            && self.tasks.is_none()
            && !self.remote_tools_busy()
            && self.confirm.is_none()
            && self.context_menu.is_none()
            && self.pending_close.is_none()
            && self.restore_pending.is_none()
            && self.screenshot_job.is_none()
            && !self.card_popup_pinned()
            && self.pet.controls.is_none()
            && !self.pet_interacting()
            && !self.image_view()
            && !self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
            && self.drag.is_none()
            && self.pane_drag.is_none()
            && self.selection.is_none()
            && self.smooth_scroll.is_none()
            && self.history_pending.is_none()
            && self.history_job.is_none()
            && self.paste_target.is_none()
            && !self.input.paste
            && self.input.buf.is_empty()
            && self.outgoing.is_empty()
            && self.clipboard.is_none()
            && !self.remote.as_ref().is_some_and(|r| r.input_busy())
    }
    fn enter_screensaver(&mut self, now: Instant) {
        self.card_close();
        self.git_hover = None;
        self.screensaver.mascot = self.prefs.screensaver_mascot;
        self.screensaver.enter(now);
    }
    pub(super) fn start_screensaver(&mut self) {
        // Actions is the entry point, not an unfinished modal action.
        self.menu = false;
        if self.screensaver_safe() {
            self.nav = false;
            self.focus = Focus::Terminal;
            self.enter_screensaver(Instant::now());
        } else {
            self.notice = "Finish the current action before starting the screensaver".into();
        }
    }
    pub(super) fn cycle_screensaver(&mut self) {
        self.prefs.screensaver_minutes = match self.prefs.screensaver_minutes {
            5 => 15,
            15 => 0,
            _ => 5,
        };
        self.notice = match self.prefs.screensaver_minutes {
            0 => "Screensaver: manual only".into(),
            n => format!("Screensaver: after {n} minutes idle"),
        };
        self.save_preferences();
    }
    pub(super) fn tick_screensaver(&mut self) -> bool {
        let now = Instant::now();
        if self
            .screensaver
            .due(now, self.prefs.screensaver_minutes, self.screensaver_safe())
        {
            self.enter_screensaver(now);
            return true;
        }
        let duck_dirty = self.screensaver.active
            && if self.screensaver.mascot == crate::pet::ScreensaverMascot::Flere {
                self.screensaver.flere.tick(now)
            } else {
                self.screensaver.duck.tick(now)
            };
        if self.screensaver.active && !self.prefs.reduced_motion {
            let frame = now.duration_since(self.screensaver.started).as_millis() / 100;
            let frame = frame as usize;
            if self.screensaver.frame != frame {
                self.screensaver.frame = frame;
                return true;
            }
        }
        duck_dirty
    }
    pub(super) fn screensaver_canvas(&mut self) -> Canvas {
        if self.screensaver.mascot == crate::pet::ScreensaverMascot::Flere {
            return self.screensaver.flere.canvas(
                self.layout.width,
                self.layout.height,
                Instant::now(),
                self.prefs.reduced_motion,
            );
        }
        intro::interactive_canvas(
            self.layout.width,
            self.layout.height,
            self.screensaver.frame,
            self.prefs.reduced_motion,
            &mut self.screensaver.duck,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_deadline_uses_only_human_activity_and_safe_idle_state() {
        let now = Instant::now();
        let mut saver = Saver::new(now);
        assert!(!saver.due(now + Duration::from_secs(299), 5, true));
        assert!(!saver.due(now + Duration::from_secs(301), 5, false));
        assert!(!saver.due(now + Duration::from_secs(900), 0, true));
        for key in [
            Key::TerminalReply(b"\x1b[6;20;8t".to_vec()),
            Key::Bytes(b"\x1b[I".to_vec()),
        ] {
            saver.consume(&key, now + Duration::from_secs(299));
        }
        assert!(saver.due(now + Duration::from_secs(300), 5, true));
        saver.consume(&Key::Hover { x: 2, y: 2 }, now + Duration::from_secs(299));
        assert!(!saver.due(now + Duration::from_secs(300), 5, true));
        assert!(saver.due(now + Duration::from_secs(599), 5, true));
        assert!(!saver.due(now + Duration::from_secs(599), 15, true));
    }
    #[test]
    fn wake_owns_burst_paste_fragments_and_matching_mouse_release() {
        let now = Instant::now();
        let mut saver = Saver::new(now);
        let mut input = Input::default();
        saver.enter(now);
        saver.begin_burst();
        for key in input.feed(b"x\0q\r\x1b[200~fragment") {
            assert!(saver.consume(&key, now));
        }
        saver.end_burst(&input, false);
        assert!(!saver.active);
        assert!(saver.partial);
        saver.begin_burst();
        for key in input.feed(b" tail\x1b[201~\r") {
            assert!(saver.consume(&key, now));
        }
        saver.end_burst(&input, false);
        assert!(!saver.partial);
        saver.begin_burst();
        assert!(!saver.consume(&Key::Bytes(b"next".to_vec()), now));
        saver.end_burst(&input, false);
        saver.enter(now);
        saver.begin_burst();
        assert!(saver.consume(&Key::Mouse { x: 3, y: 4 }, now));
        assert!(saver.active);
        saver.end_burst(&input, false);
        saver.begin_burst();
        assert!(saver.consume(&Key::Drag { x: 8, y: 9 }, now));
        assert!(saver.consume(&Key::Release { x: 8, y: 9 }, now));
        assert!(saver.consume(&Key::Hover { x: 1, y: 1 }, now));
        assert!(saver.consume(&Key::Context { x: 1, y: 1 }, now));
        assert!(saver.active);
        assert!(saver.consume(&Key::Bytes(b"q".to_vec()), now));
        assert!(!saver.active);
        assert!(saver.consume(&Key::ContextRelease, now));
        saver.end_burst(&input, false);
        assert!(!saver.mouse);
    }
    #[test]
    fn fragmented_unicode_escape_and_remote_packet_tails_remain_owned_after_wake() {
        let now = Instant::now();
        for fragments in [
            vec![b"\xf0".as_slice(), b"\x9f\xa6", b"\x86"],
            vec![b"x\x1b[".as_slice(), b"1;", b"2A"],
        ] {
            let mut saver = Saver::new(now);
            let mut input = Input::default();
            saver.enter(now);
            for (i, fragment) in fragments.iter().enumerate() {
                saver.begin_burst();
                for key in input.feed(fragment) {
                    assert!(saver.consume(&key, now));
                }
                saver.end_burst(&input, false);
                assert_eq!(saver.partial, i + 1 < fragments.len());
            }
            saver.begin_burst();
            assert!(!saver.consume(&Key::Bytes(b"next".to_vec()), now));
        }
        let mut saver = Saver::new(now);
        let input = Input::default();
        saver.enter(now);
        saver.begin_burst();
        assert!(saver.consume(&Key::Context { x: 4, y: 5 }, now));
        saver.end_burst(&input, true);
        saver.begin_burst();
        assert!(saver.consume(&Key::Bytes(b"packet tail".to_vec()), now));
        saver.end_burst(&input, false);
        saver.begin_burst();
        assert!(saver.consume(&Key::Hover { x: 5, y: 6 }, now));
        assert!(saver.consume(&Key::ContextRelease, now));
        assert!(!saver.context_mouse);
        assert!(!saver.consume(&Key::Bytes(b"next".to_vec()), now));
        saver.enter(now);
        saver.begin_burst();
        assert!(saver.consume(&Key::Mouse { x: 1, y: 1 }, now));
        assert!(saver.consume(&Key::Bytes(b"x".to_vec()), now));
        saver.end_burst(&input, false);
        saver.begin_burst();
        assert!(!saver.consume(&Key::Mouse { x: 2, y: 2 }, now));
        assert!(!saver.consume(&Key::Release { x: 2, y: 2 }, now));
        saver.enter(now);
        assert!(saver.consume(&Key::PointerActivity, now));
        assert!(saver.active);
    }
}
