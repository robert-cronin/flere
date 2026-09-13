//! Attachment-local mascot interactions. No native-input or task-state writes.
use super::*;
use crate::pet::{self as art, Action, Kind, Sprite};
use std::f32::consts::{PI, TAU};
mod physics;
use art::flere::Mood;
use physics::{Ball, Carry};
use std::{collections::hash_map::RandomState, hash::BuildHasher};
const ROWS: usize = 8;
const FRAME: Duration = Duration::from_millis(40);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interaction {
    Pet,
    Toy,
    Trick,
    Nap,
}
impl Interaction {
    fn label(self) -> &'static str {
        match self {
            Self::Pet => "Happy",
            Self::Toy => "Chasing toy",
            Self::Trick => "Showing off",
            Self::Nap => "Napping",
        }
    }
    fn duration(self) -> f32 {
        match self {
            Self::Pet => 2.8,
            Self::Toy => 4.0,
            Self::Trick => 3.6,
            Self::Nap => 10.0,
        }
    }
    fn action(self) -> Action {
        match self {
            Self::Pet => Action::Groom,
            Self::Toy => Action::Walk,
            Self::Trick => Action::Pounce,
            Self::Nap => Action::Sleep,
        }
    }
}
#[derive(Clone, Copy)]
struct Play {
    kind: Interaction,
    age: f32,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Geometry {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    cell_w: usize,
    cell_h: usize,
}
impl Geometry {
    fn world_width(self) -> f32 {
        self.width as f32 / self.height as f32 * 100.0
    }
    fn point(self, x: usize, y: usize) -> [f32; 2] {
        [
            ((x as f32 - self.x as f32 + 0.5) * self.cell_w as f32) / self.height as f32 * 100.0,
            ((y as f32 - self.y as f32 + 0.5) * self.cell_h as f32) / self.height as f32 * 100.0,
        ]
    }
    fn contains(self, x: usize, y: usize) -> bool {
        x >= self.x
            && y >= self.y
            && (x - self.x) * self.cell_w < self.width
            && (y - self.y) * self.cell_h < self.height
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Sprite,
    Ball,
}
#[derive(Clone, Copy)]
struct Pointer {
    target: Target,
    geometry: Geometry,
    workspace: u64,
    pressed: Instant,
    point: [f32; 2],
}
fn sizing(width: usize, height: usize) -> (f32, f32) {
    let scale =
        (height as f32 / art::HEIGHT as f32 * 0.9).min(width as f32 / (art::WIDTH as f32 * 1.8));
    (scale, (width as f32 - art::WIDTH as f32 * scale).max(0.0))
}
pub(super) struct Pet {
    pub controls: Option<bool>,
    kind: Kind,
    play: Option<Play>,
    ball: Option<Ball>,
    carry: Option<Carry>,
    pointer: Option<Pointer>,
    geometry: Option<Geometry>,
    workspace: Option<u64>,
    follow_velocity: f32,
    position: f32,
    checked: Instant,
    clock: f32,
    age: f32,
    blend: f32,
    travel: f32,
    action: Action,
    routine: usize,
    mode: u8,
    context: Option<(u64, Focus, Inspector, bool, bool)>,
    sprite: Sprite,
    previous: Option<Sprite>,
    left: bool,
    quiet: bool,
    reaction: Option<(Mood, Instant)>,
    random: u64,
}
impl Pet {
    pub fn new() -> Self {
        Self {
            controls: None,
            kind: Kind::default(),
            play: None,
            ball: None,
            carry: None,
            pointer: None,
            geometry: None,
            workspace: None,
            follow_velocity: 0.0,
            position: 0.5,
            checked: Instant::now(),
            clock: 0.0,
            age: 0.0,
            blend: 1.0,
            travel: PI / 2.0,
            action: Action::Walk,
            routine: 0,
            mode: 0,
            context: None,
            sprite: art::sprite_kind(Kind::default(), Action::Walk, 0.0, false),
            previous: None,
            left: false,
            quiet: false,
            reaction: None,
            random: RandomState::new().hash_one("Flere dock Flere").max(1),
        }
    }
    fn change(&mut self, action: Action) {
        if action != self.action {
            self.previous = Some(self.sprite.clone());
            self.action = action;
            self.age = 0.0;
            self.blend = 0.0;
        }
    }
    pub fn tick(&mut self, animate: bool) -> bool {
        let lifted = self.lift();
        let expired = self.reaction.is_some() && self.mood().0 == Mood::Approachable;
        if expired {
            self.reaction = None;
        }
        let elapsed = self.checked.elapsed();
        if !animate {
            self.checked = Instant::now();
            return lifted || expired;
        }
        if elapsed < FRAME {
            return lifted || expired;
        }
        self.checked = Instant::now();
        self.advance(elapsed.as_secs_f32().min(0.1));
        true
    }
    fn interact(&mut self, kind: Interaction) {
        self.cancel_pointer();
        if self.kind == Kind::Flere && kind == Interaction::Pet {
            self.random ^= self.random >> 12;
            self.random ^= self.random << 25;
            self.random ^= self.random >> 27;
            self.reaction = Some((
                if self.random.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 63 == 0 {
                    Mood::Satisfied
                } else {
                    Mood::Irritated
                },
                Instant::now(),
            ));
        } else {
            self.reaction = None;
        }
        if kind == Interaction::Toy {
            self.ball = Some(Ball::new(self.world_width(), self.position < 0.5));
        }
        self.play = Some(Play { kind, age: 0.0 });
        self.change(kind.action());
        self.previous = Some(self.sprite.clone());
        self.blend = 0.0;
    }
    fn world_width(&self) -> f32 {
        self.geometry.map_or(193.33, Geometry::world_width)
    }
    fn mood(&self) -> (Mood, f32) {
        let Some((mood, started)) = self.reaction else {
            return (Mood::Approachable, 0.0);
        };
        let remaining =
            if mood == Mood::Irritated { 3.6 } else { 5.5 } - started.elapsed().as_secs_f32();
        if remaining <= 0.0 {
            return (Mood::Approachable, 0.0);
        }
        let strength = if self.quiet {
            1.0
        } else {
            let fade = (remaining / 1.4).clamp(0.0, 1.0);
            fade * fade * (3.0 - 2.0 * fade)
        };
        (mood, strength)
    }
    fn label(&self, idle: &'static str) -> &'static str {
        if self.kind == Kind::Flere {
            match self.mood().0 {
                Mood::Satisfied => return "Satisfied",
                Mood::Irritated => return "Annoyed",
                _ => {}
            }
            if self.play.is_some_and(|p| p.kind == Interaction::Pet) {
                return idle;
            }
        }
        self.play.map_or(idle, |p| p.kind.label())
    }
    fn float_offset(&self, height: usize) -> f32 {
        if self.kind == Kind::Flere {
            height as f32
                * (-0.14
                    + if self.quiet {
                        0.0
                    } else {
                        (self.clock * 0.8).sin() * 0.035
                    })
        } else {
            0.0
        }
    }
    fn body(&self) -> ([f32; 2], f32) {
        let g = self.geometry.unwrap_or(Geometry {
            x: 0,
            y: 0,
            width: 290,
            height: 150,
            cell_w: 10,
            cell_h: 25,
        });
        let (scale, spare) = sizing(g.width, g.height);
        let unit = 100.0 / g.height as f32;
        (
            [
                (self.position * spare + 32.0 * scale) * unit,
                (g.height as f32 - 28.0 * scale + self.float_offset(g.height)) * unit,
            ],
            18.0 * scale * unit,
        )
    }
    fn reconnect(&mut self) {
        let angle = (1.0 - 2.0 * self.position).clamp(-1.0, 1.0).acos();
        self.travel = if self.left { TAU - angle } else { angle };
        self.context = None;
        self.age = 0.0;
        self.follow_velocity = 0.0;
    }
    fn land(&mut self) {
        if let Some(c) = self.carry.take() {
            let g = self.geometry.unwrap();
            let (scale, spare) = sizing(g.width, g.height);
            self.position = ((c.at[0] * g.height as f32 / 100.0 - 32.0 * scale) / spare.max(1.0))
                .clamp(0.0, 1.0);
            self.reconnect();
            self.previous = Some(self.sprite.clone());
            self.blend = 0.0;
        }
    }
    fn lift(&mut self) -> bool {
        if let Some(p) = self.pointer
            && p.target == Target::Sprite
            && self.carry.is_none()
            && p.pressed.elapsed() >= Duration::from_millis(240)
        {
            let (at, length) = self.body();
            self.carry = Some(Carry::new(at, p.point, length));
            self.play = None;
            self.change(Action::Alert);
            return true;
        }
        false
    }
    fn cancel_pointer(&mut self) {
        if let Some(p) = self.pointer.take() {
            if p.target == Target::Ball
                && let Some(b) = &mut self.ball
            {
                b.release(true);
            }
            if p.target == Target::Sprite {
                self.land();
            }
        }
    }
    fn facing(&mut self, left: bool) {
        if left != self.left {
            self.previous = Some(self.sprite.clone());
            self.left = left;
            self.blend = 0.0;
        }
    }
    fn advance(&mut self, dt: f32) {
        self.clock += dt;
        self.age += dt;
        self.blend += dt;
        let width = self.world_width();
        if let Some(ball) = &mut self.ball {
            ball.advance(dt, width);
        }
        let floor = self.body().0[1];
        if let Some(carry) = &mut self.carry {
            if carry.advance(dt, width, floor) {
                self.land();
            }
            return;
        }
        if self.pointer.is_some_and(|p| p.target == Target::Sprite) {
            return;
        }
        if let Some(mut play) = self.play.take() {
            play.age += dt;
            let chasing =
                play.kind == Interaction::Toy && self.ball.as_ref().is_some_and(Ball::moving);
            if chasing || play.age < play.kind.duration() {
                if play.kind == Interaction::Toy
                    && let Some(ball) = &self.ball
                {
                    let target = ((ball.at[0] / width - 0.23) / 0.54).clamp(0.0, 1.0);
                    let error = target - self.position;
                    // Critically damped pursuit, with bounded acceleration and speed.
                    self.follow_velocity +=
                        (error * 12.0 - self.follow_velocity * 7.0).clamp(-2.0, 2.0) * dt;
                    self.follow_velocity = self.follow_velocity.clamp(-0.42, 0.42);
                    self.position = (self.position + self.follow_velocity * dt).clamp(0.0, 1.0);
                    if error.abs() > 0.03 {
                        self.facing(error < 0.0);
                    }
                    self.change(if error.abs() < 0.06 {
                        Action::Look
                    } else {
                        Action::Walk
                    });
                }
                self.play = Some(play);
                return;
            }
            self.reconnect();
        }
        if self.mode == 0 {
            let routine = [
                (Action::Walk, 8.0),
                (Action::Stretch, 2.4),
                (Action::Sit, 4.0),
                (Action::Groom, 2.4),
                (Action::Sleep, 9.0),
                (Action::Look, 3.0),
                (Action::Pounce, 1.8),
            ];
            if self.age >= routine[self.routine].1 {
                self.routine = (self.routine + 1) % routine.len();
                self.change(routine[self.routine].0);
            }
        }
        if matches!(self.action, Action::Walk | Action::Pounce) {
            self.travel += dt * 0.6;
            self.position = 0.5 - 0.5 * self.travel.cos();
        }
        self.facing(self.travel.sin() < 0.0);
    }
    fn position(&self) -> f32 {
        self.position
    }
    fn choose(&mut self, kind: Kind) {
        if self.kind != kind {
            self.cancel_pointer();
            self.land();
            self.ball = None;
            self.kind = kind;
            self.play = None;
            self.reaction = None;
            self.context = None;
            self.previous = Some(self.sprite.clone());
            self.blend = 0.0;
        }
    }
    fn prepare(&mut self, context: (u64, Focus, Inspector, bool, bool), quiet: bool, kind: Kind) {
        self.choose(kind);
        self.quiet = quiet;
        if self.workspace.is_some_and(|old| old != context.0) {
            self.cancel_pointer();
            self.land();
            self.ball = None;
            self.play = None;
            self.reaction = None;
        }
        self.workspace = Some(context.0);
        if self.context != Some(context) {
            self.context = Some(context);
            self.routine = 0;
            self.mode = if context.3 {
                1
            } else if context.4 {
                2
            } else {
                0
            };
            if self.play.is_none() && self.carry.is_none() {
                self.change(if context.3 {
                    Action::Walk
                } else if context.4 {
                    Action::Alert
                } else if context.1 == Focus::Files {
                    Action::Look
                } else {
                    Action::Walk
                });
            }
        }
        let mut sprite = if kind == Kind::Flere {
            let (mood, strength) = self.mood();
            art::flere::sprite(self.clock, mood, strength, quiet)
        } else {
            art::sprite_kind(
                kind,
                self.action,
                if quiet {
                    0.0
                } else {
                    self.play.map_or(self.clock, |p| p.age) / self.action.period()
                },
                self.left,
            )
        };
        if !quiet && let Some(old) = &self.previous {
            let t = (self.blend / 0.24).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            sprite.blend(old, t);
        }
        if quiet || self.blend >= 0.24 {
            self.previous = None;
        }
        self.sprite = sprite;
        if let Some(c) = &mut self.carry
            && let Some(g) = self.geometry
        {
            let (scale, _) = sizing(g.width, g.height);
            let unit = scale * 100.0 / g.height as f32;
            let mut bounds = [
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ];
            for (i, pixel) in self.sprite.pixels.iter().enumerate() {
                if pixel[3] > 0 {
                    let x = (i % art::WIDTH) as f32 - 32.0;
                    let y = (i / art::WIDTH) as f32 - 36.0;
                    bounds[0] = bounds[0].min(x * unit);
                    bounds[1] = bounds[1].max((x + 1.0) * unit);
                    bounds[2] = bounds[2].min(y * unit);
                    bounds[3] = bounds[3].max((y + 1.0) * unit);
                }
            }
            if bounds[0].is_finite() {
                c.bounds = bounds;
            }
        }
    }
    fn sprite_raster(&self, width: usize, height: usize) -> crate::sixel::Raster {
        let (scale, spare) = sizing(width, height);
        let (x, pose) = if let Some(c) = &self.carry {
            (
                c.at[0] * height as f32 / 100.0 - 32.0 * scale,
                [
                    c.at[1] * height as f32 / 100.0 - height as f32 + 28.0 * scale,
                    c.angle,
                ],
            )
        } else {
            (self.position() * spare, [self.float_offset(height), 0.0])
        };
        art::scene_pose(&self.sprite, width, height, x, scale, pose)
    }
    fn raster(&self, width: usize, height: usize) -> crate::sixel::Raster {
        let (scale, spare) = sizing(width, height);
        let x = self.position() * spare;
        let mut raster = self.sprite_raster(width, height);
        if let Some(ball) = &self.ball {
            let unit = height as f32 / 100.0;
            let centre = [ball.at[0] * unit, ball.at[1] * unit];
            let radius = (physics::RADIUS * unit).max(1.0);
            if let Some(anchor) = ball.anchor() {
                // Dotted tension line; every dot is clipped to the private raster.
                let end = [anchor[0] * unit, anchor[1] * unit];
                let steps = (centre[0] - end[0]).hypot(centre[1] - end[1]).ceil() as usize;
                for step in (0..steps).step_by(4) {
                    let t = step as f32 / steps.max(1) as f32;
                    pixel(
                        &mut raster,
                        (end[0] + (centre[0] - end[0]) * t) as i32,
                        (end[1] + (centre[1] - end[1]) * t) as i32,
                        [59, 134, 156],
                    );
                }
            }
            for dy in -(radius.ceil() as i32)..=radius.ceil() as i32 {
                for dx in -(radius.ceil() as i32)..=radius.ceil() as i32 {
                    if (dx * dx + dy * dy) as f32 <= radius * radius {
                        pixel(
                            &mut raster,
                            centre[0] as i32 + dx,
                            centre[1] as i32 + dy,
                            if dx + dy < -(radius as i32) / 2 {
                                [211, 255, 255]
                            } else if dx + dy > radius as i32 / 2 {
                                [35, 156, 188]
                            } else {
                                [83, 238, 240]
                            },
                        );
                    }
                }
            }
        }
        if let Some(play) = self.play {
            let size = (height as i32 / 60).max(1);
            let cx = (x + art::WIDTH as f32 * scale * 0.55) as i32;
            match play.kind {
                Interaction::Pet if self.kind != Kind::Flere => art::token(
                    &mut raster,
                    cx,
                    (height as f32 * 0.16 - play.age.sin() * 4.0) as i32,
                    0,
                    size,
                ),
                Interaction::Pet | Interaction::Toy => {}
                Interaction::Trick => {
                    art::token(&mut raster, cx - 20, (height as f32 * 0.25) as i32, 2, size);
                    art::token(&mut raster, cx + 22, (height as f32 * 0.14) as i32, 2, size);
                }
                Interaction::Nap => {}
            }
        }
        raster
    }
}
fn pixel(r: &mut crate::sixel::Raster, x: i32, y: i32, colour: [u8; 3]) {
    if x >= 0 && y >= 0 && (x as usize) < r.width && (y as usize) < r.height {
        let i = (y as usize * r.width + x as usize) * 3;
        r.rgb[i..i + 3].copy_from_slice(&colour);
    }
}
impl Ui {
    pub(super) fn pet_rows(&self) -> usize {
        if self.pet.controls.is_some() { ROWS } else { 6 }
    }
    pub(super) fn pet_open(&mut self) {
        self.menu = false;
        self.git_hover = None;
        self.pet.choose(self.prefs.pet_kind);
        if !self.prefs.pet {
            self.prefs.pet = true;
            self.save_preferences();
        }
        if !self.pet_visible() {
            self.notice = "Open the inspector to play with pets".into();
            return;
        }
        if self.pet.controls.is_none() {
            self.pet.controls = Some(self.nav);
        }
        self.nav = true;
        self.paste_target = None;
        self.selection = None;
        self.smooth_scroll = None;
    }
    pub(super) fn pet_close(&mut self) {
        self.pet.cancel_pointer();
        if let Some(nav) = self.pet.controls.take() {
            self.nav = nav;
        }
        if self.prefs.reduced_motion {
            self.pet.play = None;
            self.pet.context = None;
        }
    }
    fn pet_choose(&mut self, kind: Kind) {
        self.pet.choose(kind);
        self.prefs.pet_kind = kind;
        self.save_preferences();
    }
    pub(super) fn pet_key(&mut self, key: &Key) -> bool {
        self.pet_sync();
        if let Some(mut p) = self.pet.pointer {
            self.pet.lift();
            match key {
                Key::Drag { x, y } | Key::Release { x, y } => {
                    p.point = p.geometry.point(*x, *y);
                    self.pet.pointer = Some(p);
                    let width = p.geometry.world_width();
                    match p.target {
                        Target::Ball => {
                            if let Some(b) = &mut self.pet.ball {
                                b.drag(p.point, width);
                            }
                        }
                        Target::Sprite => {
                            if let Some(c) = &mut self.pet.carry {
                                c.drag(p.point, width, self.prefs.reduced_motion);
                            }
                        }
                    }
                    if matches!(key, Key::Release { .. }) {
                        self.pet.pointer = None;
                        match p.target {
                            Target::Ball => {
                                if let Some(b) = &mut self.pet.ball {
                                    b.release(self.prefs.reduced_motion);
                                }
                            }
                            Target::Sprite => {
                                if let Some(c) = &mut self.pet.carry {
                                    c.release();
                                    if self.prefs.reduced_motion {
                                        self.pet.land();
                                    }
                                } else {
                                    self.pet.interact(Interaction::Pet);
                                }
                            }
                        }
                    }
                    return true;
                }
                Key::Mouse { .. } => {
                    self.pet.cancel_pointer();
                }
                Key::Bytes(b) if matches!(b.as_slice(), b"\x1b" | b"\0" | b"q" | b"\r") => {
                    self.pet_close();
                    return true;
                }
                // Captured gestures cannot leak a paste, wheel or shortcut to a child.
                _ => return true,
            }
        }
        if self.pet.controls.is_none() {
            return false;
        }
        if !self.pet_visible() {
            self.pet_close();
            return false;
        }
        match key {
            Key::Bytes(b) => match b.as_slice() {
                b"\x1b" | b"\0" | b"q" | b"\r" => self.pet_close(),
                b"h" | b"\x1b[D" => self.pet_choose(self.prefs.pet_kind.cycle(-1)),
                b"l" | b"\x1b[C" => self.pet_choose(self.prefs.pet_kind.cycle(1)),
                b"1" => self.pet_choose(Kind::Duck),
                b"2" => self.pet_choose(Kind::Robot),
                b"3" => self.pet_choose(Kind::Cat),
                b"4" => self.pet_choose(Kind::Flere),
                b"p" | b" " => self.pet.interact(Interaction::Pet),
                b"t" => self.pet.interact(Interaction::Toy),
                b"k" => self.pet.interact(Interaction::Trick),
                b"n" => self.pet.interact(Interaction::Nap),
                _ => {}
            },
            Key::Mouse { x, y } if self.pet_hit(*x, *y) => self.pet_click(*x, *y),
            Key::Mouse { .. } => {
                self.pet_close();
                return false;
            }
            // Paste, wheel, drag and other input in this focused UI never reach a PTY.
            _ => {}
        }
        true
    }
    pub(super) fn pet_click(&mut self, x: usize, y: usize) {
        if !self.pet_visible() {
            return;
        }
        self.pet_sync();
        let (start, width) = self.inspector_size();
        let g = self.pet_geometry();
        if y == self.layout.height - self.pet_rows() - 2 {
            self.pet_open();
            if x >= start + width - 6 && x < start + width - 3 {
                self.pet_choose(self.prefs.pet_kind.cycle(if x < start + width - 5 {
                    -1
                } else {
                    1
                }));
            }
        } else if y == self.layout.height - 3
            && x >= start + 2
            && x < start + 26
            && self.pet.controls.is_some()
        {
            let offset = x - (start + 2);
            self.pet.interact(if offset < 6 {
                Interaction::Pet
            } else if offset < 12 {
                Interaction::Toy
            } else if offset < 20 {
                Interaction::Trick
            } else {
                Interaction::Nap
            });
        } else if g.contains(x, y) {
            let point = g.point(x, y);
            // Mouse reports identify cells. Test their painted content, not the
            // sprite's transparent bounding box or the dock's empty background.
            let px = (x - g.x) * g.cell_w;
            let py = (y - g.y) * g.cell_h;
            let target = if self.pet.ball.as_ref().is_some_and(|b| {
                let unit = g.height as f32 / 100.0;
                let cx = b.at[0] * unit;
                let cy = b.at[1] * unit;
                let near_x = cx.clamp(px as f32, (px + g.cell_w) as f32);
                let near_y = cy.clamp(py as f32, (py + g.cell_h) as f32);
                (cx - near_x).hypot(cy - near_y) <= (physics::RADIUS * unit).max(1.0)
            }) {
                Some(Target::Ball)
            } else {
                let r = self.pet.sprite_raster(g.width, g.height);
                (py..(py + g.cell_h).min(g.height))
                    .any(|yy| {
                        (px..(px + g.cell_w).min(g.width)).any(|xx| {
                            let i = (yy * g.width + xx) * 3;
                            r.rgb[i..i + 3] != art::BACKGROUND
                        })
                    })
                    .then_some(Target::Sprite)
            };
            if let Some(target) = target {
                if self.pet.controls.is_none() {
                    self.pet_open();
                    self.pet_sync();
                    return; // The compact dock opens first; the next press uses expanded geometry.
                }
                self.pet_open();
                self.pet.pointer = Some(Pointer {
                    target,
                    geometry: g,
                    workspace: self.snapshot.active,
                    pressed: Instant::now(),
                    point,
                });
                if target == Target::Ball {
                    if let Some(b) = &mut self.pet.ball {
                        b.grab(point);
                    }
                    self.pet.play = Some(Play {
                        kind: Interaction::Toy,
                        age: 0.0,
                    });
                }
            }
        }
    }
    fn pet_geometry(&self) -> Geometry {
        let (x, width) = self.inspector_size();
        let (cell_w, cell_h) = if self.pet_pixels_capable() {
            self.graphics_cell().unwrap()
        } else {
            (1, 2)
        };
        Geometry {
            x: x + 2,
            y: self.layout.height.saturating_sub(self.pet_rows() + 1),
            width: ((width.saturating_sub(4)) * cell_w).clamp(1, 640),
            height: ((self.pet_rows() - 2) * cell_h).min(160),
            cell_w,
            cell_h,
        }
    }
    pub(super) fn pet_sync(&mut self) {
        let g = self.pet_geometry();
        if !self.pet_visible()
            || self
                .pet
                .pointer
                .is_some_and(|p| p.geometry != g || p.workspace != self.snapshot.active)
        {
            self.pet.cancel_pointer();
        }
        if self.pet.geometry != Some(g) {
            self.pet.land();
            // Keep the old scene's normalized location; never retain a stale grab.
            if let Some(old) = self.pet.geometry
                && let Some(b) = &mut self.pet.ball
            {
                b.at[0] *= g.world_width() / old.world_width();
            }
            self.pet.geometry = Some(g);
        }
    }
    /// Stable reservation even while a menu temporarily hides its contents.
    pub(super) fn pet_room(&self) -> bool {
        self.prefs.pet
            && self.layout.height >= 24
            && self.inspector_size().1 >= 20
            && (self.layout.right > 0 || self.focus == Focus::Files)
    }
    pub(super) fn inspector_height(&self) -> usize {
        self.layout.height - if self.pet_room() { self.pet_rows() } else { 0 }
    }
    pub(super) fn pet_visible(&self) -> bool {
        let (x, width) = self.inspector_size();
        let y = self.layout.height.saturating_sub(self.pet_rows() + 2);
        self.arcade.is_none()
            && !self.screensaver.active
            && self.pet_room()
            && self.context_menu.is_none()
            && !self.card_popup_pinned()
            && !self.card_popup_overlaps(x, y, width, self.layout.height.saturating_sub(y + 1))
            && !self.menu
            && self.form.is_none()
            && self.confirm.is_none()
            && self.search.is_none()
            && self.update.is_none()
            && self.tasks.is_none()
            && !self.remote_tools_busy()
            && !self.image_view()
            && !self
                .previews
                .contains_key(&(self.snapshot.active, self.snapshot.tab))
            && self.drag.is_none()
            && self.git_hover.is_none()
    }
    pub(super) fn pet_hit(&self, x: usize, y: usize) -> bool {
        let (start, width) = self.inspector_size();
        self.pet_room()
            && x >= start
            && x < start + width
            && y >= self.layout.height - self.pet_rows() - 2
            && y < self.layout.height - 1
    }
    pub(super) fn pet_interacting(&self) -> bool {
        self.pet.pointer.is_some() || self.pet.carry.is_some()
    }
    fn pet_pixels_capable(&self) -> bool {
        self.graphics_cell().is_some()
    }
    pub(super) fn paint_pet(&mut self, c: &mut Canvas) {
        if !self.pet_visible() {
            return;
        }
        self.pet_sync();
        let (x, width) = self.inspector_size();
        let top = self.layout.height - self.pet_rows() - 2;
        let working = self
            .snapshot
            .workspace()
            .is_some_and(|w| w.tabs.iter().any(|t| t.working));
        let needs_me = self
            .snapshot
            .workspace()
            .is_some_and(|w| w.meta.status == crate::workspace::Workflow::NeedsMe);
        self.pet.prepare(
            (
                self.snapshot.active,
                self.focus,
                self.prefs.inspector,
                working,
                needs_me,
            ),
            self.prefs.reduced_motion,
            self.prefs.pet_kind,
        );
        c.fill(
            x + 1,
            top,
            width - 2,
            self.pet_rows(),
            style(TEXT, BG, false),
        );
        c.text(
            x + 1,
            top,
            width - 2,
            &"─".repeat(width - 2),
            style(BORDER, BG, false),
        );
        c.text(
            x + 2,
            top,
            width - 4,
            &format!(" PET / {} ", self.prefs.pet_kind.label()),
            style(
                if self.pet.controls.is_some() {
                    CYAN
                } else {
                    MUTED
                },
                BG,
                true,
            ),
        );
        if self.pet.controls.is_some() {
            c.text(x + width - 6, top, 3, "< >", style(CYAN, BG, true));
        }
        c.text(
            x + 2,
            self.layout.height - 3,
            width - 4,
            &if self.pet.controls.is_some() {
                "p Pet t Toy k Trick n Nap".into()
            } else {
                format!(
                    "{} · O play",
                    self.pet.label(if self.pet.kind == Kind::Flere {
                        "Hovering"
                    } else {
                        self.pet.action.label()
                    })
                )
            },
            style(MUTED, BG, false),
        );
        if self.pet.controls.is_some() {
            c.fill(
                0,
                self.layout.height - 1,
                self.layout.width,
                1,
                style(TEXT, BG, false),
            );
            c.text(
                0,
                self.layout.height - 1,
                self.layout.width,
                &format!(
                    "PET {} · 1 Duck 2 Robot 3 Cat 4 Flere · h/l choose · p pet · t ball · hold sprite to lift · Esc back",
                    if self.pet.carry.is_some() { "Dangling" } else if self.pet.pointer.is_some_and(|p| p.target==Target::Ball) { "Ball held" } else { self.pet.label("Ready") }
                ),
                style(CYAN, BG, true),
            );
        }
        if !self.pet_pixels_capable() {
            let columns = width - 4;
            let raster = self.pet.raster(columns, (self.pet_rows() - 2) * 2);
            for row in 0..self.pet_rows() - 2 {
                for col in 0..columns {
                    let a = ((row * 2) * columns + col) * 3;
                    let b = ((row * 2 + 1) * columns + col) * 3;
                    let fg = Color::Rgb(raster.rgb[a], raster.rgb[a + 1], raster.rgb[a + 2]);
                    let bg = Color::Rgb(raster.rgb[b], raster.rgb[b + 1], raster.rgb[b + 2]);
                    c.text(x + 2 + col, top + 1 + row, 1, "▀", style(fg, bg, false));
                }
            }
        }
    }
    pub(super) fn pet_screenshot(&self) -> Option<crate::screenshot::Layer> {
        if !self.pet_visible() || !self.pet_pixels_capable() {
            return None;
        }
        let g = self.pet_geometry();
        Some(crate::screenshot::Layer {
            x: g.x,
            y: g.y,
            columns: g.width as f64 / g.cell_w as f64,
            rows: g.height as f64 / g.cell_h as f64,
            pixels: crate::screenshot::Pixels::Rgb(self.pet.raster(g.width, g.height)),
        })
    }
    /// Only trusted procedural graphics use this path, after terminal negotiation.
    /// Every frame paints its entire private rectangle; child escapes never reach it.
    pub(super) fn pet_pixels(&mut self) -> io::Result<String> {
        if !self.pet_visible() || !self.pet_pixels_capable() {
            return Ok(String::new());
        }
        let (x, width) = self.inspector_size();
        let (cell_w, cell_h) = self.graphics_cell().unwrap();
        let raster = self.pet.raster(
            ((width - 4) * cell_w).min(640),
            ((self.pet_rows() - 2) * cell_h).min(160),
        );
        let row = self.layout.height - self.pet_rows() - 1;
        if let Some(g) = &mut self.local_graphics {
            return g.pet(&raster, x + 2, row);
        }
        let data = art::encode(&raster)?;
        // Leave room for the surrounding frame in the existing bounded OUTPUT packet.
        if data.len() > 24 * 1024 {
            return Ok(String::new());
        }
        Ok(format!(
            "\x1b7\x1b[{};{}H{}\x1b8",
            self.layout.height - self.pet_rows(),
            x + 3,
            data
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dock_flere_floats_and_reacts_with_bounded_art_and_quiet_expiry() {
        let context = (7, Focus::Terminal, Inspector::Files, false, false);
        for (seed, expected) in [(1, Mood::Satisfied), (2, Mood::Irritated)] {
            let mut pet = Pet::new();
            pet.random = seed;
            pet.prepare(context, false, Kind::Flere);
            let first = pet.raster(29, 12);
            let body = pet.body().0;
            pet.advance(0.1);
            pet.prepare(context, false, Kind::Flere);
            assert_ne!(pet.body().0, body, "floating should remain in motion");
            assert_eq!(
                pet.mood().0,
                Mood::Approachable,
                "animation alone is not petting"
            );
            pet.interact(Interaction::Pet);
            assert_eq!(pet.mood().0, expected);
            pet.prepare(context, true, Kind::Flere);
            let quiet = pet.raster(29, 12);
            assert_ne!(quiet.rgb, first.rgb);
            assert!(!pet.tick(false));
            pet.prepare(context, true, Kind::Flere);
            assert_eq!(quiet.rgb, pet.raster(29, 12).rgb);
            for (width, height) in [(1, 1), (16, 8), (29, 12), (640, 160)] {
                let frame = pet.raster(width, height);
                assert_eq!(frame.rgb.len(), width * height * 3);
                assert!(art::encode(&frame).unwrap().len() < 24 * 1024);
            }
            if let Some(root) = std::env::var_os("FLERE_TEST_DOCK_FLERE_ARTIFACTS") {
                let root = PathBuf::from(root);
                fs::create_dir_all(&root).unwrap();
                let mut c = Canvas::new(33, 9);
                c.fill(0, 0, 33, 9, style(TEXT, BG, false));
                c.text(2, 0, 29, "PET / Flere", style(CYAN, BG, true));
                for row in 0..6 {
                    for col in 0..29 {
                        let colour = |y| {
                            let p = (y * 29 + col) * 3;
                            Color::Rgb(quiet.rgb[p], quiet.rgb[p + 1], quiet.rgb[p + 2])
                        };
                        c.text(
                            2 + col,
                            1 + row,
                            1,
                            "▀",
                            style(colour(row * 2), colour(row * 2 + 1), false),
                        );
                    }
                }
                c.text(2, 8, 29, pet.label("Hovering"), style(MUTED, BG, false));
                let mut frame = crate::screenshot::Frame {
                    width: c.width,
                    height: c.height,
                    cells: c.cells,
                    layers: Vec::new(),
                    cursor: None,
                };
                fs::write(
                    root.join(format!("dock-half-block-{seed}.png")),
                    frame.png().unwrap(),
                )
                .unwrap();
                frame.layers.push(crate::screenshot::Layer {
                    x: 2,
                    y: 1,
                    columns: 29.0,
                    rows: 6.0,
                    pixels: crate::screenshot::Pixels::Rgb(pet.raster(290, 120)),
                });
                fs::write(
                    root.join(format!("dock-pixels-{seed}.png")),
                    frame.png().unwrap(),
                )
                .unwrap();
            }
            pet.reaction = Some((expected, Instant::now() - Duration::from_secs(6)));
            assert!(
                pet.tick(false),
                "quiet reactions must expire without starting animation"
            );
            assert_eq!(pet.label("Hovering"), "Hovering");
            pet.choose(Kind::Duck);
            pet.interact(Interaction::Pet);
            assert_eq!(
                pet.label("Ready"),
                "Happy",
                "Duck keeps its original response"
            );
        }
    }
    #[test]
    fn toy_chase_finishes_smoothly_and_explicit_play_overrides_attention() {
        let mut pet = Pet::new();
        let context = (7, Focus::Terminal, Inspector::Files, false, true);
        pet.prepare(context, false, Kind::Duck);
        pet.interact(Interaction::Toy);
        let mut last = pet.position();
        for _ in 0..1000 {
            pet.advance(0.04);
            pet.prepare(context, false, Kind::Duck);
            assert!((pet.position() - last).abs() < 0.018);
            assert!((0.0..=1.0).contains(&pet.position()));
            last = pet.position();
        }
        assert!(pet.play.is_none());
        assert_eq!(pet.action, Action::Alert);
        assert!(!pet.ball.as_ref().unwrap().moving());
        pet.interact(Interaction::Pet);
        pet.prepare(
            (8, Focus::Terminal, Inspector::Files, false, false),
            false,
            Kind::Duck,
        );
        assert!(pet.play.is_none());
        assert!(pet.ball.is_none());
    }
    #[test]
    fn travel_is_continuous_and_idle_actions_complete_through_turns() {
        let mut pet = Pet::new();
        let mut old = pet.position();
        let mut actions = Vec::new();
        for _ in 0..1100 {
            pet.advance(0.04);
            assert!((pet.position() - old).abs() < 0.013);
            assert!((0.0..=1.0).contains(&pet.position()));
            old = pet.position();
            if !actions.contains(&pet.action) {
                actions.push(pet.action);
            }
        }
        for action in [
            Action::Walk,
            Action::Stretch,
            Action::Sit,
            Action::Groom,
            Action::Sleep,
            Action::Look,
            Action::Pounce,
        ] {
            assert!(actions.contains(&action), "{action:?}");
        }
        let before = (pet.clock, pet.travel, pet.age);
        assert!(!pet.tick(false));
        assert_eq!((pet.clock, pet.travel, pet.age), before);
    }
    #[test]
    fn focus_and_activity_change_pose_without_relocating_or_inspecting_native_text() {
        let mut pet = Pet::new();
        pet.advance(0.1);
        let at = pet.position();
        pet.prepare(
            (7, Focus::Files, Inspector::Git, false, false),
            false,
            Kind::Duck,
        );
        assert_eq!(pet.action, Action::Look);
        assert_eq!(pet.position(), at);
        pet.prepare(
            (8, Focus::Cards, Inspector::Git, false, true),
            false,
            Kind::Duck,
        );
        assert_eq!(pet.action, Action::Alert);
        assert_eq!(pet.position(), at);
        pet.prepare(
            (8, Focus::Terminal, Inspector::Files, true, false),
            false,
            Kind::Duck,
        );
        assert_eq!(pet.action, Action::Walk);
        assert_eq!(pet.position(), at);
    }
}
