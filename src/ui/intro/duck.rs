//! Bounded attachment-local grabbing and throwing, in terminal cell coordinates.
//! Like the dock pet, throws use a capped velocity and short physics substeps.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Pose {
    pub x: f32,
    pub y: f32,
    pub columns: usize,
    pub rows: usize,
    pub left: bool,
}
#[derive(Clone, Copy)]
struct Grab {
    offset: [f32; 2],
    moved: Instant,
    origin: [usize; 2],
    travelled: bool,
    free_before: bool,
}
pub(in crate::ui) struct Duck {
    pose: Option<Pose>,
    viewport: (usize, usize),
    bounds: [f32; 4],
    free: bool,
    walking: bool,
    quiet: bool,
    grab: Option<Grab>,
    velocity: [f32; 2],
    checked: Instant,
    airborne: bool,
}
impl Duck {
    pub fn new(now: Instant) -> Self {
        Self {
            pose: None,
            viewport: (0, 0),
            bounds: [0.0; 4],
            free: false,
            walking: false,
            quiet: false,
            grab: None,
            velocity: [0.0; 2],
            checked: now,
            airborne: false,
        }
    }
    pub(super) fn airborne(now: Instant) -> Self {
        Self {
            airborne: true,
            ..Self::new(now)
        }
    }
    #[cfg(test)]
    pub(super) fn pose(&self) -> Option<Pose> {
        self.pose
    }
    pub(super) fn arrange(&mut self, default: Pose, viewport: (usize, usize), quiet: bool) -> Pose {
        let framed = viewport.0 > 36 && viewport.1 > 12;
        let gutter = if framed { 2 } else { 0 };
        let top = if framed { 3 } else { 0 };
        let bottom = viewport.1.saturating_sub(if framed { 3 } else { 1 });
        self.bounds = [
            gutter as f32,
            viewport
                .0
                .saturating_sub(gutter + default.columns)
                .max(gutter) as f32,
            top as f32,
            bottom.saturating_sub(default.rows).max(top) as f32,
        ];
        self.quiet = quiet;
        let mut pose = if self.free {
            self.pose.unwrap_or(default)
        } else {
            default
        };
        pose.columns = default.columns;
        pose.rows = default.rows;
        if self.viewport != viewport {
            self.grab = None;
            self.velocity = [0.0; 2];
            self.viewport = viewport;
        }
        pose.x = pose.x.clamp(self.bounds[0], self.bounds[1]);
        pose.y = pose.y.clamp(self.bounds[2], self.bounds[3]);
        self.pose = Some(pose);
        pose
    }
    pub(super) fn hidden(&mut self) {
        self.pose = None;
        self.grab = None;
        self.free = false;
    }
    pub(super) fn cancel_grab(&mut self) {
        self.grab = None;
    }
    /// Returns true only for a release on a grabbed mascot without a real drag.
    pub fn pointer(&mut self, key: &Key, now: Instant) -> bool {
        match key {
            Key::Mouse { x, y } => {
                self.grab = None;
                let Some(p) = self.pose else { return false };
                if (*x as f32) >= p.x
                    && (*x as f32) < p.x + p.columns as f32
                    && (*y as f32) >= p.y
                    && (*y as f32) < p.y + p.rows as f32
                {
                    self.grab = Some(Grab {
                        offset: [p.x - *x as f32, p.y - *y as f32],
                        moved: now,
                        origin: [*x, *y],
                        travelled: false,
                        free_before: self.free,
                    });
                    self.free = true;
                    self.walking = false;
                    self.velocity = [0.0; 2];
                }
            }
            Key::Drag { x, y } => self.drag(*x, *y, now),
            Key::Release { x, y } => {
                self.drag(*x, *y, now);
                if let Some(grab) = self.grab.take() {
                    if self.airborne && !grab.travelled {
                        self.free = grab.free_before;
                        self.velocity = [0.0; 2];
                        self.checked = now;
                        return true;
                    }
                    if self.quiet
                        || now.saturating_duration_since(grab.moved) > Duration::from_millis(150)
                    {
                        self.velocity = [0.0; 2];
                    }
                    self.checked = now;
                }
            }
            _ => {}
        }
        false
    }
    fn drag(&mut self, x: usize, y: usize, now: Instant) {
        let (Some(grab), Some(p)) = (&mut self.grab, &mut self.pose) else {
            return;
        };
        grab.travelled |= x.abs_diff(grab.origin[0]) >= 2 || y.abs_diff(grab.origin[1]) >= 2;
        let at = [
            (x as f32 + grab.offset[0]).clamp(self.bounds[0], self.bounds[1]),
            (y as f32 + grab.offset[1]).clamp(self.bounds[2], self.bounds[3]),
        ];
        if at == [p.x, p.y] {
            return;
        }
        let dt = now
            .saturating_duration_since(grab.moved)
            .as_secs_f32()
            .clamp(0.016, 0.15);
        self.velocity = [
            ((at[0] - p.x) / dt).clamp(-120.0, 120.0),
            ((at[1] - p.y) / dt).clamp(-45.0, 45.0),
        ];
        if self.velocity[0].abs() > 0.1 {
            p.left = self.velocity[0] < 0.0;
        }
        [p.x, p.y] = at;
        grab.moved = now;
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        let dt = now
            .saturating_duration_since(self.checked)
            .as_secs_f32()
            .min(0.05);
        self.checked = now;
        let Some(p) = &mut self.pose else {
            return false;
        };
        if !self.free || self.grab.is_some() || self.quiet || dt <= 0.0 {
            return false;
        }
        let before = (p.x.round() as usize, p.y.round() as usize, p.left);
        let steps = (dt * 120.0).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            if self.airborne {
                if self.walking {
                    self.velocity = [if p.left { -5.0 } else { 5.0 }, 0.0];
                } else {
                    self.velocity[0] *= (-0.65 * h).exp();
                    self.velocity[1] *= (-0.65 * h).exp();
                    if self.velocity[0].abs() < 1.0 && self.velocity[1].abs() < 1.0 {
                        self.walking = true;
                    }
                }
            } else if self.walking {
                self.velocity = [if p.left { -5.0 } else { 5.0 }, 0.0];
            } else {
                self.velocity[1] += 45.0 * h;
                self.velocity[0] *= (-0.7 * h).exp();
            }
            p.x += self.velocity[0] * h;
            p.y += self.velocity[1] * h;
            for (axis, value) in [(0, &mut p.x), (1, &mut p.y)] {
                let (min, max) = (self.bounds[axis * 2], self.bounds[axis * 2 + 1]);
                if *value < min {
                    *value = min;
                    self.velocity[axis] = self.velocity[axis].abs() * 0.65;
                    if axis == 0 {
                        p.left = false;
                    }
                } else if *value > max {
                    *value = max;
                    self.velocity[axis] = -self.velocity[axis].abs() * 0.55;
                    if axis == 0 {
                        p.left = true;
                    }
                    if axis == 1 && self.velocity[1].abs() < 3.0 {
                        self.velocity[1] = 0.0;
                    }
                }
            }
            if !self.airborne && p.y >= self.bounds[3] && self.velocity[1] == 0.0 {
                self.velocity[0] *= (-5.0 * h).exp();
                if self.velocity[0].abs() < 1.0 {
                    self.walking = true;
                }
            }
        }
        before != (p.x.round() as usize, p.y.round() as usize, p.left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drag_throw_bounces_within_bounds_then_wanders_and_quiet_release_stays_put() {
        let now = Instant::now();
        for quiet in [false, true] {
            let mut duck = Duck::new(now);
            let spawn = Pose {
                x: 20.0,
                y: 30.0,
                columns: 16,
                rows: 8,
                left: false,
            };
            duck.arrange(spawn, (180, 42), quiet);
            duck.pointer(&Key::Mouse { x: 24, y: 33 }, now);
            duck.pointer(&Key::Drag { x: 80, y: 10 }, now + Duration::from_millis(80));
            let held = duck.pose.unwrap();
            assert_eq!((held.x, held.y), (76.0, 7.0));
            duck.pointer(
                &Key::Release { x: 80, y: 10 },
                now + Duration::from_millis(90),
            );
            let mut moved = false;
            for i in 1..=1200 {
                moved |= duck.tick(now + Duration::from_millis(90 + i * 16));
                let p = duck.pose.unwrap();
                assert!((2.0..=162.0).contains(&p.x));
                assert!((3.0..=31.0).contains(&p.y));
            }
            assert_eq!(moved, !quiet);
            assert_eq!(duck.walking, !quiet);
            if quiet {
                assert_eq!(
                    (duck.pose.unwrap().x, duck.pose.unwrap().y),
                    (held.x, held.y)
                );
            }
            duck.arrange(
                Pose {
                    columns: 4,
                    rows: 2,
                    ..spawn
                },
                (10, 8),
                quiet,
            );
            let p = duck.pose.unwrap();
            assert!(p.x + p.columns as f32 <= 10.0 && p.y + p.rows as f32 <= 7.0);
        }
    }
}
