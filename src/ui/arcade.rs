//! Context Ruins: an attachment-local platformer on the ordinary cell canvas.
//!
//! The UI adapter owns input isolation and the existing screenshot launcher.
//! Rules in `game` have no I/O, preferences, or access to native sessions.
use super::{Canvas, Input, Key, Ui, style};
use crate::{
    pet::{self, Kind},
    terminal::Color,
};
use std::time::{Duration, Instant};

mod game;
use game::{Game, Phase};
mod keyboard;
pub(super) use keyboard::Keyboard;
mod render;

const BG: Color = Color::Rgb(8, 14, 23);
const PANEL: Color = Color::Rgb(13, 24, 36);
const SELECTED: Color = Color::Rgb(15, 46, 57);
const TEXT: Color = Color::Rgb(222, 232, 239);
const MUTED: Color = Color::Rgb(135, 154, 174);
const CYAN: Color = Color::Rgb(87, 224, 227);
const GREEN: Color = Color::Rgb(164, 216, 133);
const AMBER: Color = Color::Rgb(242, 191, 115);
const RED: Color = Color::Rgb(237, 139, 144);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Stay,
    Close,
}

pub(super) struct Arcade {
    game: Game,
    quiet: bool,
    animation_ms: u64,
    too_small: bool,
}

/// Own opening/closing read bursts and incomplete parser tails. A game action
/// must never turn the rest of its keyboard packet into native terminal input.
#[derive(Default)]
pub(super) struct Gate {
    discard: bool,
    partial: bool,
    mouse: bool,
    context: bool,
    utf8_remaining: u8,
}
impl Gate {
    pub fn pending(&self) -> bool {
        self.discard || self.partial
    }
    pub fn begin_burst(&mut self) {
        self.discard = self.partial;
    }
    pub fn end_burst(&mut self, input: &Input, pending_packet: bool) {
        self.partial = self.discard
            && (input.paste || !input.buf.is_empty() || pending_packet || self.utf8_remaining != 0);
        self.discard = false;
    }
    fn transition(&mut self) {
        self.discard = true;
    }
    fn blocks(&mut self, key: &Key, active: bool) -> bool {
        if !active && !self.discard {
            if matches!(key, Key::Mouse { .. }) {
                self.mouse = false;
            }
            if matches!(key, Key::Context { .. }) {
                self.context = false;
            }
        }
        let block = self.discard
            || (!active
                && ((self.mouse && matches!(key, Key::Drag { .. } | Key::Release { .. }))
                    || (self.context && matches!(key, Key::ContextRelease))
                    || ((self.mouse || self.context) && matches!(key, Key::Hover { .. }))));
        if active || block {
            if matches!(key, Key::Mouse { .. }) {
                self.mouse = true;
            }
            if matches!(key, Key::Context { .. }) {
                self.context = true;
            }
        }
        if matches!(key, Key::Release { .. }) {
            self.mouse = false;
        }
        if matches!(key, Key::ContextRelease) {
            self.context = false;
        }
        if self.discard
            && let Key::Bytes(bytes) = key
        {
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
        block
    }
}

impl Ui {
    pub(super) fn arcade_obscured(&self) -> bool {
        self.form.is_some()
            || self.confirm.is_some()
            || self.search.is_some()
            || self.update.is_some()
            || self.tasks.is_some()
            || self.remote_tools_open()
            || self.image_view()
    }
    pub(super) fn open_arcade(&mut self) {
        if self.form.is_some()
            || self.search.is_some()
            || self.update.is_some()
            || self.tasks.is_some()
            || self.confirm.is_some()
            || self.pending_close.is_some()
            || self.remote_tools_busy()
            || self.image_view()
            || self.screensaver.active
        {
            self.notice = "Finish the current action before opening Arcade".into();
            return;
        }
        self.flush_input();
        self.pet_close();
        self.card_close();
        self.context_menu = None;
        self.menu = false;
        self.git_hover = None;
        self.mouse_hint = None;
        self.drag = None;
        self.pane_drag = None;
        self.selection = None;
        self.smooth_scroll = None;
        self.paste_target = None;
        self.arcade_nav = self.nav;
        self.nav = true;
        let seed = super::os::nonce()
            .ok()
            .and_then(|value| u64::from_str_radix(&value[..16], 16).ok())
            .unwrap_or(1);
        let mut arcade = Arcade::new(seed, self.prefs.pet_kind, self.prefs.reduced_motion);
        arcade.resize(self.layout.width, self.layout.height);
        self.arcade = Some(arcade);
        self.arcade_checked = Instant::now();
        self.arcade_keyboard.sync(true, self.arcade_checked);
        self.arcade_keyboard.flush();
        self.arcade_gate.transition();
        if let Err(error) = self.remote_cancel_changed() {
            self.notice = error.to_string();
        }
        // Focus reporting is requested only for this local overlay; consume late
        // reports after dismissal too, and reset it on every frontend exit.
        let _ = super::remote::emit(self.remote.is_some(), b"\x1b[?1004h");
    }
    pub(super) fn arcade_key(&mut self, key: &Key) -> bool {
        let active = self.arcade.is_some();
        let now = Instant::now();
        self.arcade_keyboard
            .sync(active && !self.arcade_obscured(), now);
        let was_unfenced = self.arcade_keyboard.unfenced();
        if let Key::Bytes(bytes) = key
            && self.arcade_keyboard.reply(bytes, now)
        {
            if was_unfenced && !self.arcade_keyboard.unfenced() {
                self.arcade_gate.transition();
            }
            self.arcade_keyboard.flush();
            return true;
        }
        self.arcade_keyboard.flush();
        if active {
            // Game activity is still human activity for the ordinary idle timer.
            let _ = self.screensaver.consume(key, Instant::now());
        }
        if self.arcade_gate.blocks(key, active) {
            return true;
        }
        if self.arcade_keyboard.drains() {
            return true;
        }
        if active && self.arcade_obscured() {
            self.arcade.as_mut().unwrap().pause();
            if let Key::Bytes(bytes) = key
                && keyboard::event(bytes).is_some()
            {
                return true; // stale enhanced keys cannot become modal consent
            }
            return false; // The higher-priority local modal owns this event.
        }
        if let Key::Bytes(bytes) = key
            && matches!(bytes.as_slice(), b"\x1b[I" | b"\x1b[O")
        {
            if bytes == b"\x1b[O"
                && let Some(arcade) = &mut self.arcade
            {
                arcade.pause();
            }
            return true;
        }
        if !active {
            if let Key::Bytes(bytes) = key
                && let Some(event) = keyboard::event(bytes)
                && (self.arcade_keyboard.unfenced() || event.kind != game::KeyEvent::Press)
            {
                return true;
            }
            return false;
        }
        if let Key::Bytes(bytes) = key {
            let decoded = keyboard::event(bytes);
            let (bytes, event) = if let Some(decoded) = &decoded {
                if !self.arcade_keyboard.enhanced() {
                    return true;
                }
                (decoded.key.as_slice(), Some(decoded.kind))
            } else {
                // Functional-key press reports may omit their default numeric
                // fields even while corresponding repeat/release reports include them.
                let event = (self.arcade_keyboard.enhanced()
                    && matches!(
                        bytes.as_slice(),
                        b"\x1b[A" | b"\x1b[B" | b"\x1b[C" | b"\x1b[D"
                    ))
                .then_some(game::KeyEvent::Press);
                (bytes.as_slice(), event)
            };
            if matches!(bytes, b"s" | b"S")
                && event.is_none_or(|event| event == game::KeyEvent::Press)
            {
                let focus = self.focus;
                self.copy_screenshot();
                self.focus = focus;
                self.nav = true;
            } else if match event {
                Some(event) => self.arcade.as_mut().unwrap().key_event(bytes, event),
                None => self.arcade.as_mut().unwrap().key(bytes),
            } == Outcome::Close
            {
                self.arcade = None;
                self.arcade_keyboard.stop(now);
                self.arcade_keyboard.flush();
                self.nav = self.arcade_nav;
                self.arcade_gate.transition();
                let _ = super::remote::emit(self.remote.is_some(), b"\x1b[?1004l");
            }
        }
        true // Paste and all pointer events belong to the overlay, including release.
    }
    pub(super) fn tick_arcade(&mut self) -> bool {
        let now = Instant::now();
        let delta = now.saturating_duration_since(self.arcade_checked);
        self.arcade_checked = now;
        let obscured = self.arcade_obscured();
        self.arcade_keyboard
            .sync(self.arcade.is_some() && !obscured, now);
        self.arcade_keyboard.flush();
        if let Some(arcade) = &mut self.arcade {
            // These cannot normally open through game input, but a background
            // attachment-local transition must never spend the user's round time.
            if obscured {
                arcade.pause();
            }
            arcade.tick(delta)
        } else {
            false
        }
    }
}

impl Arcade {
    pub fn new(seed: u64, avatar: Kind, reduced_motion: bool) -> Self {
        Self {
            game: Game::new(seed, avatar),
            quiet: reduced_motion,
            animation_ms: 0,
            too_small: false,
        }
    }
    /// A size change pauses an active adventure. Growing never resumes it silently.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.too_small = width < 40 || height < 20;
        if self.game.phase == Phase::Playing {
            self.game.pause();
        }
    }
    pub fn pause(&mut self) {
        self.game.pause();
    }
    pub fn tick(&mut self, delta: Duration) -> bool {
        let changed = self.game.tick(delta);
        let frame = self.animation_ms / 100;
        if !self.quiet && !self.game.paused && !self.too_small {
            self.animation_ms = (self.animation_ms + delta.as_millis().min(100) as u64) % 120_000;
        }
        changed || frame != self.animation_ms / 100
    }
    /// Paste and pointer packets never reach the action game as decomposed keys.
    pub fn key(&mut self, bytes: &[u8]) -> Outcome {
        if self.too_small {
            return if matches!(bytes, b"q" | b"Q" | b"\x1b") {
                Outcome::Close
            } else {
                Outcome::Stay
            };
        }
        self.game.key(bytes)
    }
    fn key_event(&mut self, bytes: &[u8], event: game::KeyEvent) -> Outcome {
        if self.too_small {
            return if event == game::KeyEvent::Press && matches!(bytes, b"q" | b"Q" | b"\x1b") {
                Outcome::Close
            } else {
                Outcome::Stay
            };
        }
        self.game.key_event(bytes, event)
    }
    pub fn draw(&self, c: &mut Canvas) {
        render::draw(self, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(name: &str, canvas: &Canvas) {
        let Some(path) = std::env::var_os("FLERE_TEST_ARCADE_ARTIFACTS") else {
            return;
        };
        let root = std::path::PathBuf::from(path);
        std::fs::create_dir_all(&root).unwrap();
        let frame = crate::screenshot::Frame {
            width: canvas.width,
            height: canvas.height,
            cells: canvas.cells.clone(),
            layers: Vec::new(),
            cursor: None,
        };
        std::fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
    }
    fn plain(canvas: &Canvas) -> String {
        canvas.cells.iter().map(|cell| cell.text.as_str()).collect()
    }
    #[test]
    fn room_arena_picker_pause_and_results_fit_narrow_and_wide_terminals() {
        for (width, height) in [(40, 20), (60, 24), (80, 24), (160, 42)] {
            let mut arcade = Arcade::new(47, Kind::Flere, true);
            arcade.resize(width, height);
            for phase in [Phase::Picker, Phase::Playing, Phase::Results] {
                arcade.game.phase = phase;
                let mut canvas = Canvas::new(width, height);
                arcade.draw(&mut canvas);
                assert_eq!(canvas.cells.len(), width * height);
                assert!(canvas.badges.is_empty());
                let text = plain(&canvas);
                assert!(
                    text.contains(match phase {
                        Phase::Picker => "Enter: explore",
                        Phase::Playing => "Space jump",
                        Phase::Results => "Enter: choose avatar",
                    }),
                    "{text}"
                );
                artifact(
                    &format!("context-ruins-{width}x{height}-{phase:?}"),
                    &canvas,
                );
            }
            arcade.game.phase = Phase::Playing;
            for room in 0..3 {
                arcade.game.room = room;
                arcade.game.x = 18.;
                arcade.game.feet = 10.;
                let mut canvas = Canvas::new(width, height);
                arcade.draw(&mut canvas);
                artifact(
                    &format!("context-ruins-{width}x{height}-room{}", room + 1),
                    &canvas,
                );
            }
            arcade.pause();
            let mut canvas = Canvas::new(width, height);
            arcade.draw(&mut canvas);
            assert!(plain(&canvas).contains("PAUSED"));
        }
    }
    #[test]
    fn resize_and_focus_pause_clear_motion_and_never_auto_resume() {
        let mut arcade = Arcade::new(3, Kind::Duck, true);
        arcade.key(b"\r");
        arcade.key(b"l");
        arcade.resize(16, 8);
        arcade.tick(Duration::from_secs(60));
        assert_eq!(arcade.game.elapsed, 0.);
        arcade.resize(80, 24);
        assert!(arcade.game.paused);
        arcade.key(b"p");
        arcade.tick(Duration::from_millis(50));
        assert_eq!(arcade.game.x, 2.);
        arcade.pause();
        let time = arcade.game.elapsed;
        arcade.tick(Duration::from_secs(60));
        assert_eq!(arcade.game.elapsed, time);
        for (w, h) in [(0, 0), (1, 1), (16, 8), (39, 19)] {
            let mut canvas = Canvas::new(w, h);
            arcade.draw(&mut canvas);
            assert_eq!(canvas.cells.len(), w * h);
        }
    }
    #[test]
    fn reduced_motion_freezes_decoration_but_not_avatar_physics() {
        let mut quiet = Arcade::new(14, Kind::Flere, true);
        let mut moving = Arcade::new(14, Kind::Flere, false);
        let mut before = Canvas::new(80, 24);
        quiet.draw(&mut before);
        assert!(!quiet.tick(Duration::from_millis(500)));
        let mut after = Canvas::new(80, 24);
        quiet.draw(&mut after);
        assert_eq!(before.cells, after.cells);
        for game in [&mut quiet, &mut moving] {
            game.key(b"\r");
            game.key(b"l");
            game.key(b" ");
            game.tick(Duration::from_millis(80));
        }
        assert_eq!(
            (quiet.game.x, quiet.game.feet),
            (moving.game.x, moving.game.feet)
        );
        assert!(quiet.game.x > 2.);
    }

    #[test]
    fn transition_gate_owns_complete_bursts_fragmented_paste_escape_and_utf8_tails() {
        for chunks in [
            vec![b"q\rNATIVE".as_slice()],
            vec![b"q\x1b[200~PASTE".as_slice(), b"TAIL\r\x1b[201~"],
            vec![b"q\x1b[".as_slice(), b"A"],
            vec![b"q\xe2".as_slice(), b"\x82", b"\xac"],
        ] {
            let mut input = Input::default();
            let mut gate = Gate::default();
            let mut open = true;
            for chunk in chunks {
                gate.begin_burst();
                for key in input.feed(chunk) {
                    let blocked = gate.blocks(&key, open);
                    if open && matches!(&key, Key::Bytes(bytes) if bytes == b"q") {
                        open = false;
                        gate.transition();
                    } else {
                        assert!(blocked, "tail was forwarded: {key:?}");
                    }
                }
                gate.end_burst(&input, false);
            }
            assert!(!gate.pending());
            gate.begin_burst();
            assert!(!gate.blocks(&Key::Bytes(b"fresh".to_vec()), false));
        }
        let mut gate = Gate::default();
        gate.transition();
        gate.end_burst(&Input::default(), true);
        assert!(gate.pending()); // An incomplete remote packet is still this burst.
        gate.begin_burst();
        assert!(gate.blocks(&Key::Bytes(b"\r".to_vec()), false));
        gate.end_burst(&Input::default(), false);
        assert!(!gate.pending());
    }

    #[test]
    fn overlay_press_release_cannot_select_or_copy_hidden_content_after_close() {
        let mut gate = Gate::default();
        gate.begin_burst();
        assert!(!gate.blocks(&Key::Mouse { x: 8, y: 8 }, true));
        gate.transition();
        gate.end_burst(&Input::default(), false);
        gate.begin_burst();
        assert!(gate.blocks(&Key::Drag { x: 12, y: 9 }, false));
        assert!(gate.blocks(&Key::Release { x: 12, y: 9 }, false));
        assert!(!gate.blocks(&Key::Mouse { x: 2, y: 2 }, false));
    }
}
