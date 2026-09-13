//! Small attachment-local simulation. Distances are hundredths of the dock height.
use std::time::{Duration, Instant};
pub const RADIUS: f32 = 4.0;
const GRAVITY: f32 = 180.0;
fn limit(v: &mut [f32; 2], max: f32) {
    let speed = v[0].hypot(v[1]);
    if speed > max {
        v[0] *= max / speed;
        v[1] *= max / speed;
    }
}
#[derive(Clone, Copy)]
struct Grab {
    anchor: [f32; 2],
    offset: [f32; 2],
    last: Instant,
}
pub(super) struct Ball {
    pub at: [f32; 2],
    pub velocity: [f32; 2],
    grab: Option<Grab>,
}
impl Ball {
    pub fn new(width: f32, left: bool) -> Self {
        Self {
            at: [width * if left { 0.75 } else { 0.25 }, 35.0],
            velocity: [if left { -75.0 } else { 75.0 }, -55.0],
            grab: None,
        }
    }
    pub fn grab(&mut self, pointer: [f32; 2]) {
        self.grab = Some(Grab {
            anchor: self.at,
            offset: [self.at[0] - pointer[0], self.at[1] - pointer[1]],
            last: Instant::now(),
        });
        self.velocity = [0.0; 2];
    }
    pub fn anchor(&self) -> Option<[f32; 2]> {
        self.grab.map(|g| g.anchor)
    }
    pub fn drag(&mut self, pointer: [f32; 2], width: f32) {
        let Some(g) = &mut self.grab else { return };
        let at = [
            (pointer[0] + g.offset[0]).clamp(RADIUS, width - RADIUS),
            (pointer[1] + g.offset[1]).clamp(RADIUS, 100.0 - RADIUS),
        ];
        if at == self.at {
            return;
        }
        let dt = g.last.elapsed().as_secs_f32().clamp(0.016, 0.15);
        self.velocity = [(at[0] - self.at[0]) / dt, (at[1] - self.at[1]) / dt];
        limit(&mut self.velocity, 350.0);
        self.at = at;
        g.last = Instant::now();
    }
    pub fn release(&mut self, quiet: bool) {
        if let Some(g) = self.grab.take() {
            // A recent fast movement is a throw. Pull-and-hold uses spring recoil.
            if g.last.elapsed() > Duration::from_millis(120)
                || self.velocity[0].hypot(self.velocity[1]) < 45.0
            {
                self.velocity = [
                    (g.anchor[0] - self.at[0]) * 6.0,
                    (g.anchor[1] - self.at[1]) * 6.0,
                ];
            }
            limit(&mut self.velocity, 350.0);
        }
        if quiet {
            self.velocity = [0.0; 2];
        }
    }
    pub fn moving(&self) -> bool {
        self.grab.is_some()
            || self.at[1] < 100.0 - RADIUS - 0.01
            || self.velocity[0].hypot(self.velocity[1]) > 0.1
    }
    pub fn advance(&mut self, dt: f32, width: f32) {
        self.at[0] = self.at[0].clamp(RADIUS, width - RADIUS);
        if self.grab.is_some() {
            return;
        }
        let steps = (dt / (1.0 / 240.0)).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            self.velocity[1] += GRAVITY * h;
            self.velocity[0] *= (-0.22 * h).exp();
            for axis in 0..2 {
                self.at[axis] += self.velocity[axis] * h;
                let max = if axis == 0 { width } else { 100.0 } - RADIUS;
                if self.at[axis] < RADIUS {
                    self.at[axis] = RADIUS;
                    self.velocity[axis] = self.velocity[axis].abs() * 0.78;
                } else if self.at[axis] > max {
                    self.at[axis] = max;
                    self.velocity[axis] =
                        -self.velocity[axis].abs() * if axis == 0 { 0.78 } else { 0.70 };
                    if axis == 1 && self.velocity[1].abs() < 5.0 {
                        self.velocity[1] = 0.0;
                    }
                }
            }
            if self.at[1] >= 100.0 - RADIUS - 0.01 && self.velocity[1] == 0.0 {
                self.velocity[0] *= (-4.0 * h).exp();
                if self.velocity[0].abs() < 0.1 {
                    self.velocity[0] = 0.0;
                }
            }
        }
    }
}
pub(super) struct Carry {
    pub at: [f32; 2],
    pub hook: Option<[f32; 2]>,
    pub angle: f32,
    velocity: [f32; 2],
    length: f32,
    offset: [f32; 2],
    hook_goal: [f32; 2],
    spin: f32,
    pub bounds: [f32; 4],
}
impl Carry {
    pub fn new(at: [f32; 2], pointer: [f32; 2], length: f32) -> Self {
        Self {
            at,
            hook: Some([at[0], at[1] - length]),
            angle: 0.0,
            velocity: [0.0; 2],
            length,
            offset: [at[0] - pointer[0], at[1] - length - pointer[1]],
            hook_goal: [at[0], at[1] - length],
            spin: 0.0,
            bounds: [-20.0, 20.0, -35.0, 25.0],
        }
    }
    pub fn drag(&mut self, pointer: [f32; 2], width: f32, quiet: bool) {
        let hook = [
            (pointer[0] + self.offset[0]).clamp(self.length, width - self.length),
            (pointer[1] + self.offset[1]).clamp(0.0, 100.0 - self.length * (46.0 / 18.0)),
        ];
        self.hook_goal = hook;
        if quiet {
            self.hook = Some(hook);
            self.at = [hook[0], hook[1] + self.length];
            self.angle = 0.0;
            self.advance(0.0, width, 100.0 - self.length * (28.0 / 18.0));
        }
    }
    /// A compliant suspension supports pendulum motion and dock contacts
    /// together, even when the pointer tries to pull the body through a wall.
    /// Free flight retains both linear and angular momentum.
    pub fn advance(&mut self, dt: f32, width: f32, floor: f32) -> bool {
        let steps = (dt / (1.0 / 240.0)).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            self.velocity[1] += GRAVITY * h;
            if let Some(mut hook) = self.hook {
                let follow = 1.0 - (-35.0 * h).exp();
                hook[0] += (self.hook_goal[0] - hook[0]) * follow;
                hook[1] += (self.hook_goal[1] - hook[1]) * follow;
                self.hook = Some(hook);
                let dx = self.at[0] - hook[0];
                let dy = self.at[1] - hook[1];
                let norm = dx.hypot(dy).max(0.01);
                let radial = (self.velocity[0] * dx + self.velocity[1] * dy) / norm;
                // Elastic grip: extension stores energy, damping dissipates it.
                let tension = ((norm - self.length) * 180.0 + radial * 12.0).max(0.0);
                self.velocity[0] -= dx / norm * tension * h;
                self.velocity[1] -= dy / norm * tension * h;
                let target = (-dx).atan2(dy);
                let error = (target - self.angle + std::f32::consts::PI)
                    .rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                self.spin += (error * 90.0 - self.spin * 15.0) * h;
            } else {
                self.spin *= (-0.8 * h).exp();
            }
            limit(&mut self.velocity, 250.0);
            self.spin = self.spin.clamp(-12.0, 12.0);
            self.angle = (self.angle + self.spin * h + std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)
                - std::f32::consts::PI;
            self.at[0] += self.velocity[0] * h;
            self.at[1] += self.velocity[1] * h;
            // Rotated opaque bounds supply support distances for each wall.
            let (sin, cos) = self.angle.sin_cos();
            let mut extent = [
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ];
            for x in [self.bounds[0], self.bounds[1]] {
                for y in [self.bounds[2], self.bounds[3]] {
                    let xx = x * cos - y * sin;
                    let yy = x * sin + y * cos;
                    extent[0] = extent[0].min(xx);
                    extent[1] = extent[1].max(xx);
                    extent[2] = extent[2].min(yy);
                    extent[3] = extent[3].max(yy);
                }
            }
            let limits = [
                [-extent[0], width - extent[1]],
                [-extent[2], floor.min(100.0 - extent[3])],
            ];
            let mut grounded = false;
            for (axis, range) in limits.iter().enumerate() {
                let low = range[0];
                let high = range[1].max(low);
                if self.at[axis] < low {
                    self.at[axis] = low;
                    if self.velocity[axis] < 0.0 {
                        self.velocity[axis] *= -0.25;
                    }
                    self.spin *= 0.96;
                } else if self.at[axis] > high {
                    self.at[axis] = high;
                    if self.velocity[axis] > 0.0 {
                        self.velocity[axis] *= -0.18;
                    }
                    self.spin *= 0.96;
                    grounded = axis == 1;
                }
            }
            if grounded {
                self.velocity[0] *= (-8.0 * h).exp();
                if self.hook.is_none() {
                    self.spin += (-self.angle * 100.0 - self.spin * 15.0) * h;
                    if self.velocity[1].abs() < 2.0
                        && self.velocity[0].abs() < 2.0
                        && self.angle.abs() < 0.025
                        && self.spin.abs() < 0.3
                    {
                        self.at[1] = floor;
                        return true;
                    }
                }
            }
        }
        false
    }
    pub fn release(&mut self) {
        self.hook = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gravity_bounces_bounds_and_damping_settle() {
        let mut b = Ball::new(193.0, false);
        b.at = [188.0, 30.0];
        b.velocity = [200.0, 0.0];
        b.advance(0.04, 193.0);
        assert!(b.velocity[0] < 0.0 && b.velocity[1] > 0.0);
        let mut bounced = false;
        for _ in 0..1000 {
            let old = b.velocity[1];
            b.advance(0.04, 193.0);
            bounced |= old > 0.0 && b.velocity[1] < 0.0;
            assert!((RADIUS..=193.0 - RADIUS).contains(&b.at[0]));
            assert!((RADIUS..=100.0 - RADIUS).contains(&b.at[1]));
        }
        assert!(bounced);
        assert!(!b.moving());
        assert_eq!(b.velocity, [0.0; 2]);
    }
    #[test]
    fn held_pull_recoils_and_fast_flick_throws() {
        let mut b = Ball::new(193.0, false);
        b.grab(b.at);
        b.drag([10.0, 60.0], 193.0);
        b.grab.as_mut().unwrap().last = Instant::now() - Duration::from_millis(200);
        b.release(false);
        assert!(b.velocity[0] > 0.0 && b.velocity[1] < 0.0);
        b.grab(b.at);
        b.drag([50.0, 60.0], 193.0);
        b.release(false);
        assert!(b.velocity[0] > 0.0);
        b.grab(b.at);
        b.drag([10.0, 80.0], 193.0);
        b.release(true);
        assert_eq!(b.velocity, [0.0; 2]);
    }
    #[test]
    fn carried_opaque_corners_stay_inside_dock_during_edge_drags() {
        let mut c = Carry::new([96.0, 60.0], [96.0, 35.0], 25.0);
        for pointer in [[0.0, 0.0], [193.0, 0.0], [0.0, 100.0], [193.0, 100.0]] {
            c.drag(pointer, 193.0, false);
            for _ in 0..50 {
                c.advance(0.04, 193.0, 60.0);
                let (sin, cos) = c.angle.sin_cos();
                for x in [c.bounds[0], c.bounds[1]] {
                    for y in [c.bounds[2], c.bounds[3]] {
                        let xx = c.at[0] + x * cos - y * sin;
                        let yy = c.at[1] + x * sin + y * cos;
                        assert!((-0.001..=193.001).contains(&xx));
                        assert!((-0.001..=100.001).contains(&yy));
                    }
                }
            }
        }
    }
    #[test]
    fn carried_body_swings_then_lands_without_nonfinite_state() {
        let mut c = Carry::new([96.0, 60.0], [96.0, 35.0], 25.0);
        c.drag([130.0, 20.0], 193.0, false);
        for _ in 0..10 {
            assert!(!c.advance(0.04, 193.0, 60.0));
        }
        assert!(c.angle.abs() > 0.1);
        let hook = c.hook.unwrap();
        assert!((c.at[0] - hook[0]).hypot(c.at[1] - hook[1]) < 36.0);
        let spin = c.spin;
        c.release();
        c.advance(0.01, 193.0, 60.0);
        assert!(c.spin.abs() > spin.abs() * 0.5);
        c.release();
        assert!((0..200).any(|_| c.advance(0.04, 193.0, 60.0)));
        assert_eq!(c.at[1], 60.0);
        assert!(c.angle.is_finite());
    }
}
