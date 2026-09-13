//! A small original action adventure. No I/O, native input, or persisted state.
use super::Outcome;
use crate::pet::Kind;
use std::time::Duration;

pub(super) const WIDTH: f32 = 40.0;
pub(super) const HEIGHT: f32 = 20.0;
const STEP: f32 = 1.0 / 120.0;
const BODY_W: f32 = 0.75;
const BODY_H: f32 = 1.55;
const SPEED: f32 = 7.5;
const GRAVITY: f32 = 23.0;
const JUMP: f32 = 13.6;
const HOLD: f32 = 0.18;

/// Events from a terminal that explicitly reports key-up. Legacy bytes still
/// use `key`, whose expiring intent cannot infer the user's physical key state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyEvent {
    Press,
    Repeat,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Picker,
    Playing,
    Results,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
    fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.w
            && self.x + self.w > other.x
            && self.y < other.y + other.h
            && self.y + self.h > other.y
    }
}
#[derive(Clone, Copy)]
pub(super) struct Room {
    pub name: &'static str,
    pub subtitle: &'static str,
    pub color: [u8; 3],
    pub floors: &'static [(f32, f32)],
    pub platforms: &'static [Rect],
    pub ladders: &'static [Rect],
    pub key: (f32, f32),
    pub gems: &'static [(f32, f32)],
    pub patrol: (f32, f32),
    pub checkpoint: (f32, f32),
}
pub(super) const ROOMS: [Room; 3] = [
    Room {
        name: "THE LOST PROMPT",
        subtitle: "The agent said the key was in scope.",
        color: [62, 207, 183],
        floors: &[(0., 21.), (25., 40.)],
        platforms: &[Rect::new(7., 13., 11., 0.6), Rect::new(24., 9., 8., 0.6)],
        ladders: &[Rect::new(10., 13., 1., 5.), Rect::new(27., 9., 1., 9.)],
        key: (15., 12.),
        gems: &[(5., 17.), (27., 8.), (34., 17.)],
        patrol: (28., 34.),
        checkpoint: (2., 18.),
    },
    Room {
        name: "THE CONTEXT VAULT",
        subtitle: "Somebody compacted the floor.",
        color: [188, 139, 242],
        floors: &[(0., 17.), (22., 40.)],
        platforms: &[
            Rect::new(5., 12., 12., 0.6),
            Rect::new(14., 13., 11., 0.6),
            Rect::new(22., 8., 14., 0.6),
        ],
        ladders: &[Rect::new(8., 12., 1., 6.), Rect::new(27., 8., 1., 10.)],
        key: (31., 7.),
        gems: &[(13., 11.), (19., 12.), (34., 17.)],
        patrol: (10., 16.),
        checkpoint: (26., 18.),
    },
    Room {
        name: "THE MERGE TEMPLE",
        subtitle: "One more room. Probably.",
        color: [246, 170, 86],
        floors: &[(0., 9.), (13., 25.), (29., 40.)],
        platforms: &[
            Rect::new(3., 14., 6., 0.6),
            Rect::new(12., 10., 9., 0.6),
            Rect::new(24., 6., 13., 0.6),
        ],
        ladders: &[
            Rect::new(6., 14., 1., 4.),
            Rect::new(16., 10., 1., 8.),
            Rect::new(31., 6., 1., 12.),
        ],
        key: (34., 5.),
        gems: &[(7., 13.), (19., 9.), (26., 5.)],
        patrol: (18., 24.),
        checkpoint: (30., 18.),
    },
];

pub(super) struct Game {
    pub phase: Phase,
    pub paused: bool,
    pub avatar: Kind,
    pub room: usize,
    pub x: f32,
    pub feet: f32,
    pub vx: f32,
    pub vy: f32,
    pub grounded: bool,
    pub climbing: bool,
    pub facing: i8,
    pub keys: [bool; 3],
    pub gems: [u8; 3],
    pub lives: u8,
    pub elapsed: f32,
    pub invulnerable: f32,
    pub dying: f32,
    pub transition: f32,
    pub checkpoint: (usize, f32, f32),
    pub won: bool,
    pub feedback: &'static str,
    pub feedback_left: f32,
    horizontal: i8,
    vertical: i8,
    held_x: f32,
    held_y: f32,
    pressed: [u64; 8],
    press_order: u64,
    jump_down: bool,
    air_horizontal: i8,
    jump_buffer: f32,
    coyote: f32,
    drop_left: f32,
    ladder_regrab: f32,
    accumulator: f32,
}
impl Game {
    pub fn new(_seed: u64, avatar: Kind) -> Self {
        Self {
            phase: Phase::Picker,
            paused: false,
            avatar,
            room: 0,
            x: 2.,
            feet: 18.,
            vx: 0.,
            vy: 0.,
            grounded: true,
            climbing: false,
            facing: 1,
            keys: [false; 3],
            gems: [0; 3],
            lives: 3,
            elapsed: 0.,
            invulnerable: 0.,
            dying: 0.,
            transition: 0.,
            checkpoint: (0, 2., 18.),
            won: false,
            feedback: ROOMS[0].subtitle,
            feedback_left: 4.,
            horizontal: 0,
            vertical: 0,
            held_x: 0.,
            held_y: 0.,
            pressed: [0; 8],
            press_order: 0,
            jump_down: false,
            air_horizontal: 0,
            jump_buffer: 0.,
            coyote: 0.10,
            drop_left: 0.,
            ladder_regrab: 0.,
            accumulator: 0.,
        }
    }
    pub fn room(&self) -> Room {
        ROOMS[self.room]
    }
    pub fn body(&self) -> Rect {
        Rect::new(self.x - BODY_W / 2., self.feet - BODY_H, BODY_W, BODY_H)
    }
    pub fn treasure(&self) -> u32 {
        self.gems.iter().map(|bits| bits.count_ones()).sum()
    }
    pub fn hazard(&self) -> Rect {
        let (left, right) = self.room().patrol;
        let span = right - left;
        let distance = (self.elapsed * 2.1 + self.room as f32 * 1.3) % (span * 2.);
        Rect::new(
            left + if distance < span {
                distance
            } else {
                2. * span - distance
            },
            17.1,
            1.0,
            0.9,
        )
    }
    fn clear_motion(&mut self) {
        self.horizontal = 0;
        self.vertical = 0;
        self.held_x = 0.;
        self.held_y = 0.;
        self.pressed = [0; 8];
        self.press_order = 0;
        self.jump_down = false;
        self.air_horizontal = 0;
        self.jump_buffer = 0.;
        self.vx = 0.;
        self.accumulator = 0.;
    }
    fn held_axis(&self, horizontal: bool) -> Option<i8> {
        let first = if horizontal { 0 } else { 4 };
        (first..first + 4)
            .filter(|&index| self.pressed[index] != 0)
            .max_by_key(|&index| self.pressed[index])
            .map(|index| if index % 4 < 2 { -1 } else { 1 })
    }
    fn direction_binding(key: &[u8]) -> Option<usize> {
        match key {
            b"h" => Some(0),
            b"\x1b[D" | b"\x1bOD" => Some(1),
            b"l" => Some(2),
            b"\x1b[C" | b"\x1bOC" => Some(3),
            b"k" => Some(4),
            b"\x1b[A" | b"\x1bOA" => Some(5),
            b"j" => Some(6),
            b"\x1b[B" | b"\x1bOB" => Some(7),
            _ => None,
        }
    }
    /// Direction repeats refresh an existing physical hold but never recreate a
    /// hold cleared by pause, retry, a death or a room transition. Space is an edge.
    pub fn key_event(&mut self, key: &[u8], event: KeyEvent) -> Outcome {
        let binding = Self::direction_binding(key);
        if event == KeyEvent::Release {
            if let Some(index) = binding {
                self.pressed[index] = 0;
                if index < 4 {
                    self.held_x = 0.;
                    self.air_horizontal = 0;
                } else {
                    self.held_y = 0.;
                }
            }
            if key == b" " {
                self.jump_down = false;
            }
            return Outcome::Stay;
        }
        if self.phase != Phase::Playing || self.paused {
            return if event == KeyEvent::Press
                || (event == KeyEvent::Repeat && self.phase == Phase::Picker && binding.is_some())
            {
                self.key(key)
            } else {
                Outcome::Stay
            };
        }
        if let Some(index) = binding {
            if event == KeyEvent::Press && self.pressed[index] == 0 {
                self.press_order = self.press_order.saturating_add(1);
                self.pressed[index] = self.press_order;
                if index < 4 {
                    self.held_x = 0.;
                    self.air_horizontal = 0;
                } else {
                    self.held_y = 0.;
                    if index >= 6 {
                        self.drop_through();
                    }
                }
            }
            return Outcome::Stay;
        }
        if key == b" " {
            if event == KeyEvent::Press && !self.jump_down {
                self.jump_down = true;
                self.jump_buffer = 0.14;
            }
            return Outcome::Stay;
        }
        if event == KeyEvent::Press {
            self.key(key)
        } else {
            Outcome::Stay
        }
    }
    fn drop_through(&mut self) {
        if self.grounded
            && self.feet < 17.9
            && !self
                .room()
                .ladders
                .iter()
                .any(|l| l.contains(self.x, self.feet + 0.1))
        {
            self.drop_left = 0.3;
            self.feet += 0.08;
            self.grounded = false;
        }
    }
    pub fn pause(&mut self) {
        if self.phase == Phase::Playing {
            self.paused = true;
            self.clear_motion();
        }
    }
    fn say(&mut self, message: &'static str) {
        self.feedback = message;
        self.feedback_left = 3.5;
    }
    fn respawn(&mut self) {
        (self.room, self.x, self.feet) = self.checkpoint;
        self.vy = 0.;
        self.grounded = true;
        self.climbing = false;
        self.dying = 0.;
        self.ladder_regrab = 0.;
        self.invulnerable = 1.5;
        self.clear_motion();
    }
    fn hurt(&mut self) {
        if self.invulnerable > 0. || self.dying > 0. {
            return;
        }
        self.lives = self.lives.saturating_sub(1);
        self.dying = 0.6;
        self.clear_motion();
        self.say("Minor setback. Very dramatic incident report.");
    }
    pub fn tick(&mut self, delta: Duration) -> bool {
        if self.paused || self.phase != Phase::Playing {
            return false;
        }
        // A stalled UI never catches up by running seconds of unseen hazards.
        self.accumulator += delta.as_secs_f32().min(0.10);
        let mut changed = false;
        while self.accumulator >= STEP {
            self.accumulator -= STEP;
            self.step();
            changed = true;
        }
        changed
    }
    fn step(&mut self) {
        self.elapsed += STEP;
        self.feedback_left = (self.feedback_left - STEP).max(0.);
        self.invulnerable = (self.invulnerable - STEP).max(0.);
        self.transition = (self.transition - STEP).max(0.);
        if self.dying > 0. {
            self.dying -= STEP;
            if self.dying <= 0. {
                if self.lives == 0 {
                    self.phase = Phase::Results;
                } else {
                    self.respawn();
                }
            }
            return;
        }
        self.held_x = (self.held_x - STEP).max(0.);
        self.held_y = (self.held_y - STEP).max(0.);
        self.jump_buffer = (self.jump_buffer - STEP).max(0.);
        self.drop_left = (self.drop_left - STEP).max(0.);
        self.ladder_regrab = (self.ladder_regrab - STEP).max(0.);
        if self.grounded {
            self.coyote = 0.10;
        } else {
            self.coyote = (self.coyote - STEP).max(0.);
        }
        let explicit_x = self.held_axis(true);
        let dx = explicit_x.unwrap_or({
            if self.held_x > 0. {
                self.horizontal
            } else if !self.grounded && !self.climbing {
                self.air_horizontal
            } else {
                0
            }
        }) as f32;
        let dy = self
            .held_axis(false)
            .unwrap_or(if self.held_y > 0. { self.vertical } else { 0 }) as f32;
        self.vx = dx * SPEED;
        if dx != 0. {
            self.facing = dx as i8;
        }
        let room = self.room();
        let ladder = room.ladders.iter().find(|ladder| {
            self.ladder_regrab == 0.
                && self.x >= ladder.x - 0.35
                && self.x <= ladder.x + ladder.w + 0.35
                && self.feet >= ladder.y - 0.05
                && self.feet - BODY_H < ladder.y + ladder.h
        });
        if let Some(ladder) = ladder {
            if dy != 0. {
                self.climbing = true;
                self.x = ladder.x + ladder.w / 2.;
            }
        } else {
            self.climbing = false;
        }
        // Capture ladder support before horizontal intent releases the grip:
        // Space + Left/Right in one input burst is a jump, not an accidental fall.
        let ladder_jump = self.climbing;
        if dx != 0. {
            self.climbing = false;
        }
        if self.jump_buffer > 0. && (self.coyote > 0. || ladder_jump) {
            // Legacy terminals stop repeating Left/Right when Space becomes the
            // repeated key. Carry the launch direction through this jump only.
            self.air_horizontal = if explicit_x.is_none() { dx as i8 } else { 0 };
            if ladder_jump {
                // Held/repeated Up cannot cancel the launch on the next step.
                // Re-grabbing becomes possible just after the normal jump apex.
                self.ladder_regrab = 0.65;
            }
            self.vy = -JUMP;
            self.grounded = false;
            self.climbing = false;
            self.coyote = 0.;
            self.jump_buffer = 0.;
        }
        let old_feet = self.feet;
        if self.climbing {
            self.vy = dy * 5.;
            self.feet += self.vy * STEP;
            if let Some(ladder) = ladder {
                self.feet = self.feet.clamp(ladder.y, ladder.y + ladder.h);
                self.grounded =
                    self.feet <= ladder.y + 0.01 || self.feet >= ladder.y + ladder.h - 0.01;
            }
        } else {
            self.vy = (self.vy + GRAVITY * STEP).min(17.);
            self.feet += self.vy * STEP;
            self.grounded = false;
        }
        self.x = (self.x + self.vx * STEP).min(39.5);
        if !self.keys[self.room] && self.x > 36.7 && self.feet > 14.5 {
            self.x = 36.7;
            if dx > 0. && self.feedback_left < 0.2 {
                self.say("A key opens this. Confidence does not.");
            }
        }
        if self.room == 0 {
            self.x = self.x.max(0.6);
        }
        // Floors and ledges are one-way: jump through their underside, land on top.
        if !self.climbing && self.vy >= 0. {
            let mut landing = None;
            for &(start, end) in room.floors {
                if self.x + BODY_W / 2. > start
                    && self.x - BODY_W / 2. < end
                    && old_feet <= 18.01
                    && self.feet >= 18.
                {
                    landing = Some(18.);
                }
            }
            if self.drop_left == 0. {
                for p in room.platforms {
                    if self.x + BODY_W / 2. > p.x
                        && self.x - BODY_W / 2. < p.x + p.w
                        && old_feet <= p.y + 0.01
                        && self.feet >= p.y
                        && landing.is_none_or(|y| p.y < y)
                    {
                        landing = Some(p.y);
                    }
                }
            }
            if let Some(y) = landing {
                self.feet = y;
                self.vy = 0.;
                self.grounded = true;
                self.air_horizontal = 0;
            }
        }
        let body = self.body();
        if !self.keys[self.room]
            && Rect::new(room.key.0 - 0.5, room.key.1 - 0.5, 1., 1.).intersects(body)
        {
            self.keys[self.room] = true;
            self.say("Key acquired. Scope expanded.");
        }
        for (index, &(x, y)) in room.gems.iter().enumerate() {
            if Rect::new(x - 0.4, y - 0.4, 0.8, 0.8).intersects(body) {
                self.gems[self.room] |= 1 << index;
            }
        }
        if self.room > 0
            && (self.x - room.checkpoint.0).abs() < 1.
            && self.grounded
            && (self.feet - room.checkpoint.1).abs() < 0.1
            && self.checkpoint.0 == self.room
            && self.checkpoint.1 != room.checkpoint.0
        {
            self.checkpoint = (self.room, room.checkpoint.0, room.checkpoint.1);
            self.say("Checkpoint. Finally, a useful summary.");
        }
        if self.feet > HEIGHT + 2. || self.hazard().intersects(body) {
            self.hurt();
        }
        if self.dying > 0. {
            return;
        }
        if self.x > 39.2 && self.feet > 14. && self.keys[self.room] {
            if self.room == 2 {
                self.won = true;
                self.phase = Phase::Results;
                self.clear_motion();
            } else {
                self.enter(self.room + 1, false);
            }
        } else if self.x < 0.2 && self.room > 0 {
            self.enter(self.room - 1, true);
        }
    }
    fn enter(&mut self, room: usize, back: bool) {
        self.room = room;
        self.x = if back { 37.5 } else { 2. };
        self.feet = 18.;
        self.vy = 0.;
        self.grounded = true;
        self.checkpoint = (room, self.x, 18.);
        self.transition = 0.65;
        self.invulnerable = 1.;
        self.climbing = false;
        self.ladder_regrab = 0.;
        self.clear_motion();
        self.say(ROOMS[room].subtitle);
    }
    pub fn key(&mut self, key: &[u8]) -> Outcome {
        if self.paused {
            match key {
                b"q" | b"Q" => return Outcome::Close,
                b"p" | b"P" | b"\r" | b"\x1b" => {
                    self.paused = false;
                    self.clear_motion();
                }
                b"r" | b"R" => {
                    self.respawn();
                    self.paused = false;
                }
                _ => {}
            }
            return Outcome::Stay;
        }
        match self.phase {
            Phase::Picker => match key {
                b"q" | b"Q" | b"\x1b" => return Outcome::Close,
                b"h" | b"k" | b"\x1b[A" | b"\x1b[D" | b"\x1bOA" | b"\x1bOD" => {
                    self.avatar = self.avatar.cycle(-1)
                }
                b"l" | b"j" | b"\x1b[B" | b"\x1b[C" | b"\x1bOB" | b"\x1bOC" => {
                    self.avatar = self.avatar.cycle(1)
                }
                [number @ b'1'..=b'4'] => self.avatar = Kind::ALL[(number - b'1') as usize],
                b"\r" | b" " => self.phase = Phase::Playing,
                _ => {}
            },
            Phase::Results => match key {
                b"q" | b"Q" | b"\x1b" => return Outcome::Close,
                b"\r" | b" " | b"r" | b"R" => *self = Self::new(1, self.avatar),
                _ => {}
            },
            Phase::Playing => match key {
                b"h" | b"\x1b[D" | b"\x1bOD" => {
                    self.horizontal = -1;
                    self.held_x = HOLD;
                    if !self.grounded && !self.climbing {
                        self.air_horizontal = -1;
                    }
                }
                b"l" | b"\x1b[C" | b"\x1bOC" => {
                    self.horizontal = 1;
                    self.held_x = HOLD;
                    if !self.grounded && !self.climbing {
                        self.air_horizontal = 1;
                    }
                }
                b"k" | b"\x1b[A" | b"\x1bOA" => {
                    self.vertical = -1;
                    self.held_y = HOLD;
                }
                b"j" | b"\x1b[B" | b"\x1bOB" => {
                    self.vertical = 1;
                    self.held_y = HOLD;
                    self.drop_through();
                }
                b" " => self.jump_buffer = 0.14,
                b"p" | b"P" | b"q" | b"Q" | b"\x1b" => self.pause(),
                b"r" | b"R" => {
                    self.respawn();
                    self.say("Back to the checkpoint. No meeting required.");
                }
                _ => {}
            },
        }
        Outcome::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn start() -> Game {
        let mut g = Game::new(1, Kind::Flere);
        g.key(b"\r");
        g
    }
    fn advance(g: &mut Game, seconds: f32) {
        for _ in 0..(seconds * 120.) as usize {
            g.tick(Duration::from_secs_f32(STEP));
        }
    }
    fn walk(g: &mut Game, target: f32) {
        for _ in 0..2000 {
            if (g.x - target).abs() < 0.08 {
                g.key_event(b"h", KeyEvent::Release);
                g.key_event(b"l", KeyEvent::Release);
                return;
            }
            g.key_event(if g.x < target { b"l" } else { b"h" }, KeyEvent::Press);
            g.tick(Duration::from_secs_f32(STEP));
        }
        panic!(
            "could not walk to {target}: room{} x{} y{} lives{}",
            g.room, g.x, g.feet, g.lives
        );
    }
    fn climb(g: &mut Game, x: f32, y: f32) {
        walk(g, x);
        for _ in 0..600 {
            if g.feet <= y + 0.04 {
                g.held_y = 0.;
                return;
            }
            g.key(b"k");
            g.tick(Duration::from_secs_f32(STEP));
        }
        panic!("climb failed {} {}", g.x, g.feet);
    }
    fn jump_to(g: &mut Game, target: f32) {
        g.key(b" ");
        walk(g, target);
        advance(g, 0.9);
    }
    #[test]
    fn explicit_move_and_jump_hold_independently_until_release_without_repeat_bounce() {
        let mut g = start();
        g.key_event(b"\x1b[C", KeyEvent::Press);
        advance(&mut g, 0.1);
        g.key_event(b" ", KeyEvent::Press);
        advance(&mut g, 0.7); // No further Right repeat: Space owns OS repeat now.
        assert!(g.x > 7.5 && g.feet < 16. && g.vx > 7.);
        g.key_event(b"\x1b[C", KeyEvent::Release);
        let x = g.x;
        for _ in 0..180 {
            g.key_event(b" ", KeyEvent::Repeat);
            g.tick(Duration::from_secs_f32(STEP));
        }
        assert_eq!(g.x, x);
        assert_eq!(g.feet, 18.);
        assert!(g.grounded); // Holding Space did not jump again on landing.
        g.key_event(b" ", KeyEvent::Release);
        g.key_event(b" ", KeyEvent::Press);
        advance(&mut g, 0.05);
        assert!(g.vy < -12. && g.feet < 18.);
    }
    #[test]
    fn opposing_and_alias_direction_releases_restore_only_still_held_keys() {
        let mut g = start();
        g.x = 10.;
        g.key_event(b"\x1b[C", KeyEvent::Press);
        advance(&mut g, 0.1);
        let right = g.x;
        g.key_event(b"h", KeyEvent::Press);
        advance(&mut g, 0.1);
        assert!(g.x < right);
        g.key_event(b"\x1b[C", KeyEvent::Repeat); // An older hold cannot steal priority.
        assert_eq!(g.held_axis(true), Some(-1));
        g.key_event(b"h", KeyEvent::Release);
        advance(&mut g, 0.1);
        assert_eq!(g.held_axis(true), Some(1));
        g.key_event(b"\x1b[D", KeyEvent::Press);
        g.key_event(b"h", KeyEvent::Press);
        g.key_event(b"\x1b[D", KeyEvent::Release);
        assert_eq!(g.held_axis(true), Some(-1)); // Releasing an alias keeps h down.
        g.key_event(b"h", KeyEvent::Release);
        assert_eq!(g.held_axis(true), Some(1));
        g.key_event(b"\x1b[C", KeyEvent::Release);
        let stopped = g.x;
        advance(&mut g, 0.3);
        assert_eq!(g.x, stopped);
        g.x = 10.5;
        g.feet = 15.;
        g.climbing = true;
        g.grounded = false;
        g.key_event(b"\x1b[A", KeyEvent::Press);
        advance(&mut g, 0.1);
        let up = g.feet;
        g.key_event(b"j", KeyEvent::Press);
        advance(&mut g, 0.1);
        assert!(g.feet > up);
        g.key_event(b"j", KeyEvent::Release);
        advance(&mut g, 0.1);
        assert_eq!(g.held_axis(false), Some(-1));
        g.key_event(b"\x1b[A", KeyEvent::Release);
        let stopped = g.feet;
        advance(&mut g, 0.3);
        assert_eq!(g.feet, stopped);
    }
    #[test]
    fn lifecycle_resets_cannot_be_undone_by_stale_repeat_events() {
        for transition in 0..3 {
            let mut g = start();
            g.key_event(b"l", KeyEvent::Press);
            g.key_event(b" ", KeyEvent::Press);
            match transition {
                0 => {
                    g.pause();
                    g.key_event(b"p", KeyEvent::Press);
                }
                1 => {
                    g.key_event(b"r", KeyEvent::Press);
                }
                _ => g.enter(1, false),
            }
            let x = g.x;
            g.key_event(b"l", KeyEvent::Repeat);
            g.key_event(b" ", KeyEvent::Repeat);
            advance(&mut g, 0.2);
            assert_eq!(g.x, x);
            assert_eq!(g.feet, 18.);
            g.key_event(b"l", KeyEvent::Release);
            g.key_event(b"l", KeyEvent::Press);
            advance(&mut g, 0.1);
            assert!(g.x > x);
        }
    }
    #[test]
    fn legacy_jump_carries_launch_direction_but_grounded_taps_and_landings_stop() {
        for direction in [b"h".as_slice(), b"l".as_slice()] {
            let mut g = start();
            g.x = if direction == b"h" { 12. } else { 2. };
            g.key(direction);
            advance(&mut g, 0.1);
            let launch = g.x;
            g.key(b" ");
            advance(&mut g, 0.7);
            assert!((g.x - launch).abs() > 4.5); // Past the old 180 ms deadline.
            assert!(g.feet < 16.);
            for _ in 0..180 {
                g.tick(Duration::from_secs_f32(STEP));
                if g.grounded {
                    break;
                }
            }
            assert!(g.grounded);
            let landed = g.x;
            advance(&mut g, 0.5);
            assert_eq!(g.x, landed);
            assert_eq!(g.air_horizontal, 0);
        }
        let mut g = start();
        g.key(b"l");
        advance(&mut g, 0.3);
        let stopped = g.x;
        advance(&mut g, 0.5);
        assert_eq!(g.x, stopped);
        g.key(b" ");
        advance(&mut g, 0.3);
        assert_eq!(g.x, stopped); // No stale tap resurrected.
    }

    #[test]
    fn walking_expires_and_jump_is_forgiving_but_lands_on_real_surfaces() {
        let mut g = start();
        g.key(b"l");
        advance(&mut g, 1.);
        assert!(g.x > 3. && g.x < 3.5);
        let x = g.x;
        advance(&mut g, 2.);
        assert_eq!(g.x, x);
        g.key(b" ");
        advance(&mut g, 0.3);
        assert!(g.feet < 15.);
        advance(&mut g, 1.);
        assert_eq!(g.feet, 18.);
        walk(&mut g, 19.5);
        jump_to(&mut g, 26.);
        assert_eq!(g.feet, 18.);
        assert_eq!(g.lives, 3);
    }
    #[test]
    fn edge_grace_landing_buffer_and_delayed_ticks_are_bounded() {
        let mut g = start();
        g.x = 21.5; // Just beyond the first ledge; the next step discovers the fall.
        advance(&mut g, 0.05);
        assert!(!g.grounded);
        g.key(b" ");
        advance(&mut g, 0.02);
        assert!(g.vy < -12.); // Coyote time accepts a late edge jump.
        g.x = 5.;
        g.feet = 17.7;
        g.vy = 6.;
        g.grounded = false;
        g.coyote = 0.;
        g.key(b" ");
        advance(&mut g, 0.1);
        assert!(g.vy < -12.); // A press just before landing becomes one jump.
        let elapsed = g.elapsed;
        g.tick(Duration::from_secs(30));
        assert!(g.elapsed - elapsed <= 0.11); // A stall cannot kill the unseen player.
        g.x = 39.4;
        g.feet = 8.;
        g.vy = 0.;
        g.key(b"l");
        advance(&mut g, 0.1);
        assert!(g.x <= 39.5); // The arena remains bounded above a locked door too.
    }

    #[test]
    fn sideways_ladder_jumps_launch_and_held_up_cannot_cancel_them() {
        for direction in [b"h".as_slice(), b"l".as_slice()] {
            let mut g = start();
            climb(&mut g, 10.5, 15.);
            assert!(g.climbing);
            let x = g.x;
            g.key(direction);
            g.key(b" ");
            advance(&mut g, 0.1);
            assert!(!g.climbing);
            assert!(g.vy < -10. && g.feet < 14.);
            assert!(if direction == b"h" { g.x < x } else { g.x > x });
        }
        let mut g = start();
        g.enter(2, false);
        g.x = 31.5;
        g.feet = 15.;
        g.grounded = false;
        g.climbing = true;
        g.key(b"k");
        g.key(b" ");
        for _ in 0..60 {
            g.key(b"k"); // A long ladder and OS key-repeat would otherwise re-grab.
            g.tick(Duration::from_secs_f32(STEP));
            assert!(!g.climbing);
        }
        assert!(g.feet < 11.5 && g.vy < 0.);
        g.key(b"r");
        assert_eq!(g.ladder_regrab, 0.);
        g.x = 31.5;
        g.key(b"k");
        advance(&mut g, 0.05);
        assert!(g.climbing); // Retry restores ordinary ladder access immediately.
    }

    #[test]
    fn ladders_keys_locked_doors_and_three_connected_rooms_have_a_playable_witness() {
        let mut g = start();
        climb(&mut g, 10.5, 13.);
        walk(&mut g, 15.);
        assert!(g.keys[0]);
        walk(&mut g, 19.5);
        advance(&mut g, 0.8);
        jump_to(&mut g, 26.);
        // Jump over the moving floor sentinel, then continue through the keyed door.
        g.key(b" ");
        walk(&mut g, 35.);
        advance(&mut g, 0.5);
        for _ in 0..180 {
            g.key(b"l");
            g.tick(Duration::from_secs_f32(STEP));
            if g.room == 1 {
                break;
            }
        }
        assert_eq!(g.room, 1);
        assert!(g.lives > 0);
        climb(&mut g, 8.5, 12.);
        walk(&mut g, 25.8);
        advance(&mut g, 0.9);
        climb(&mut g, 27.5, 8.);
        walk(&mut g, 31.);
        assert!(g.keys[1]);
        walk(&mut g, 37.5);
        advance(&mut g, 1.2);
        for _ in 0..100 {
            g.key(b"l");
            g.tick(Duration::from_secs_f32(STEP));
            if g.room == 2 {
                break;
            }
        }
        assert_eq!(g.room, 2);
        assert!(g.lives > 0);
        walk(&mut g, 7.5);
        jump_to(&mut g, 14.5);
        climb(&mut g, 16.5, 10.);
        walk(&mut g, 20.);
        jump_to(&mut g, 30.);
        climb(&mut g, 31.5, 6.);
        walk(&mut g, 34.);
        assert_eq!(g.keys, [true; 3]);
        walk(&mut g, 38.);
        advance(&mut g, 1.5);
        for _ in 0..100 {
            g.key(b"l");
            g.tick(Duration::from_secs_f32(STEP));
            if g.won {
                break;
            }
        }
        assert!(g.won);
        assert_eq!(g.phase, Phase::Results);
        assert!(g.lives > 0);
    }
    #[test]
    fn hazards_pits_checkpoint_retry_and_game_over_keep_collected_progress() {
        let mut g = start();
        g.keys[0] = true;
        g.gems[0] = 3;
        g.x = 23.;
        advance(&mut g, 2.);
        assert_eq!(g.lives, 2);
        assert_eq!(g.x, 2.);
        assert!(g.keys[0]);
        assert_eq!(g.treasure(), 2);
        g.invulnerable = 0.;
        g.x = g.hazard().x + 0.5;
        advance(&mut g, 0.8);
        assert_eq!(g.lives, 1);
        g.key(b"r");
        assert_eq!(g.lives, 1);
        assert_eq!(g.x, 2.);
        g.invulnerable = 0.;
        g.x = 23.;
        advance(&mut g, 2.);
        assert_eq!(g.phase, Phase::Results);
        assert!(!g.won);
        g.key(b"\r");
        assert_eq!(g.phase, Phase::Picker);
        assert_eq!(g.lives, 3);
    }
    #[test]
    fn pause_stalls_neither_native_time_nor_replays_movement_and_avatar_is_cosmetic() {
        for avatar in Kind::ALL {
            let mut g = Game::new(9, avatar);
            g.key(b"\r");
            g.key(b"l");
            g.pause();
            let before = (g.x, g.feet, g.elapsed);
            advance(&mut g, 2.);
            assert_eq!((g.x, g.feet, g.elapsed), before);
            g.key(b"p");
            advance(&mut g, 0.2);
            assert_eq!(g.x, 2.);
            g.key(b"l");
            advance(&mut g, 0.1);
            assert!(g.x > 2.5);
            g.key(b"l\r");
            g.pause();
            assert_eq!(g.key(b"q"), Outcome::Close);
        }
    }
    #[test]
    fn blocked_gate_and_backtracking_do_not_lose_keys_or_room_checkpoints() {
        let mut g = start();
        g.x = 36.;
        g.key(b"l");
        advance(&mut g, 0.2);
        assert!(g.x <= 36.7);
        assert_eq!(g.room, 0);
        g.keys[0] = true;
        g.enter(1, false);
        g.x = 0.1;
        advance(&mut g, 0.02);
        assert_eq!(g.room, 0);
        assert!(g.keys[0]);
        g.enter(2, false);
        g.x = 30.;
        advance(&mut g, 0.02);
        assert_eq!(g.checkpoint, (2, 30., 18.));
        g.x = 35.;
        g.key(b"r");
        assert_eq!(g.x, 30.);
    }
}
