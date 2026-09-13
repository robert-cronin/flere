//! Screensaver-local motion and emotion; no session commands or input forwarding.
use super::*;
use crate::pet::flere::{self as art, Mood};
use std::{collections::hash_map::RandomState, hash::BuildHasher};

pub(in crate::ui) struct Flere {
    motion: Duck,
    started: Instant,
    reaction: Option<(Mood, Instant)>,
    random: u64,
    painted: Vec<(usize, usize)>,
}

impl Flere {
    pub fn new(now: Instant) -> Self {
        let seed = os::nonce()
            .ok()
            .and_then(|n| u64::from_str_radix(&n[..16], 16).ok())
            .unwrap_or_else(|| RandomState::new().hash_one("Flere Flere"));
        Self::seeded(now, seed)
    }
    fn seeded(now: Instant, seed: u64) -> Self {
        Self {
            motion: Duck::airborne(now),
            started: now,
            reaction: None,
            random: seed.max(1),
            painted: Vec::new(),
        }
    }
    pub fn reset(&mut self, now: Instant) {
        self.motion = Duck::airborne(now);
        self.started = now;
        self.reaction = None;
        self.painted.clear();
        // Preserve the once-seeded sequence across manual and automatic entries.
    }
    pub fn pointer(&mut self, key: &Key, now: Instant) {
        if let Key::Mouse { x, y } = key
            && !self.painted.contains(&(*x, *y))
        {
            self.motion.cancel_grab();
            return;
        }
        if self.motion.pointer(key, now) {
            self.random ^= self.random >> 12;
            self.random ^= self.random << 25;
            self.random ^= self.random >> 27;
            let sample = self.random.wrapping_mul(0x2545_f491_4f6c_dd1d);
            self.reaction = Some((
                if sample >> 63 == 0 {
                    Mood::Satisfied
                } else {
                    Mood::Irritated
                },
                now,
            ));
        }
    }
    fn duration(mood: Mood) -> f32 {
        if mood == Mood::Irritated { 3.6 } else { 5.5 }
    }
    fn state(&self, now: Instant, quiet: bool) -> (Mood, f32) {
        let Some((mood, started)) = self.reaction else {
            return (Mood::Approachable, 0.0);
        };
        let age = now.saturating_duration_since(started).as_secs_f32();
        let remaining = Self::duration(mood) - age;
        if remaining <= 0.0 {
            return (Mood::Approachable, 0.0);
        }
        let strength = if quiet {
            1.0
        } else {
            let fade = (remaining / 1.4).clamp(0.0, 1.0);
            fade * fade * (3.0 - 2.0 * fade)
        };
        (mood, strength)
    }
    pub fn tick(&mut self, now: Instant) -> bool {
        let moved = self.motion.tick(now);
        if self.reaction.is_some() && self.state(now, false).0 == Mood::Approachable {
            self.reaction = None;
            return true;
        }
        moved
    }
    pub(super) fn hint(&self, now: Instant) -> &'static str {
        match self.state(now, false).0 {
            Mood::Approachable => "Pet Flere · drag to throw",
            mood => mood.label(),
        }
    }
    pub fn canvas(&mut self, width: usize, height: usize, now: Instant, quiet: bool) -> Canvas {
        let step =
            (now.saturating_duration_since(self.started).as_millis() / TICK.as_millis()) as usize;
        canvas_with_mascot(width, height, step, quiet, true, Mascot::Flere(self, now))
    }
    pub(super) fn draw(
        &mut self,
        c: &mut Canvas,
        top: usize,
        step: usize,
        now: Instant,
        quiet: bool,
    ) {
        self.painted.clear();
        let Some((x, y, columns, rows, left)) = mascot_position(c.width, c.height, top, step, true)
        else {
            self.motion.hidden();
            return;
        };
        let pose = Pose {
            x: x as f32,
            y: y as f32,
            columns,
            rows,
            left,
        };
        let pose = self.motion.arrange(pose, (c.width, c.height), quiet);
        let time = if quiet {
            0.0
        } else {
            now.saturating_duration_since(self.started).as_secs_f32()
        };
        let (mood, strength) = self.state(now, quiet);
        let raster = art::frame(columns, rows * 2, time, mood, strength, quiet);
        for row in 0..rows {
            for col in 0..columns {
                let colour = |py: usize| {
                    let i = (py * columns + col) * 3;
                    Color::Rgb(raster.rgb[i], raster.rgb[i + 1], raster.rgb[i + 2])
                };
                let fg = colour(row * 2);
                let bg = colour(row * 2 + 1);
                let background = Color::Rgb(6, 14, 22);
                if fg == background && bg == background {
                    continue;
                }
                let at = (pose.x.round() as usize + col, pose.y.round() as usize + row);
                self.painted.push(at);
                c.text(at.0, at.1, 1, "▀", style(fg, bg, false));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(flere: &Flere) -> (usize, usize) {
        let pose = flere.motion.pose().unwrap();
        (
            (pose.x + pose.columns as f32 / 2.0) as usize,
            (pose.y + pose.rows as f32 / 2.0) as usize,
        )
    }
    fn artifact(name: &str, canvas: Canvas) {
        let Some(root) = std::env::var_os("FLERE_TEST_FLERE_ARTIFACTS") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let frame = crate::screenshot::Frame {
            width: canvas.width,
            height: canvas.height,
            cells: canvas.cells,
            layers: Vec::new(),
            cursor: None,
        };
        fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
    }
    #[test]
    fn seeded_petting_has_both_moods_while_hover_drag_and_time_obey_ownership() {
        let now = Instant::now();
        for (seed, expected) in [(1, Mood::Satisfied), (2, Mood::Irritated)] {
            let mut flere = Flere::seeded(now - Duration::from_secs(4), seed);
            let normal = flere.canvas(180, 42, now, false);
            artifact("flere-180x42-approachable", normal);
            let (x, y) = point(&flere);
            let p = flere.motion.pose().unwrap();
            let corner = (p.x as usize, p.y as usize);
            assert!(!flere.painted.contains(&corner));
            flere.pointer(
                &Key::Mouse {
                    x: corner.0,
                    y: corner.1,
                },
                now,
            );
            flere.pointer(
                &Key::Release {
                    x: corner.0,
                    y: corner.1,
                },
                now,
            );
            assert_eq!(flere.state(now, false).0, Mood::Approachable);
            flere.pointer(&Key::Hover { x, y }, now);
            assert_eq!(flere.state(now, false).0, Mood::Approachable);
            flere.pointer(&Key::Mouse { x, y }, now);
            assert_eq!(flere.state(now, false).0, Mood::Approachable);
            flere.pointer(&Key::Release { x, y }, now + Duration::from_millis(50));
            assert_eq!(
                flere.state(now + Duration::from_millis(50), false).0,
                expected
            );
            artifact(
                if expected == Mood::Satisfied {
                    "flere-satisfied"
                } else {
                    "flere-irritated"
                },
                flere.canvas(180, 42, now + Duration::from_millis(500), false),
            );
            assert!(flere.state(now + Duration::from_secs(3), false).1 > 0.0);
            assert!(flere.tick(now + Duration::from_secs(7)));
            assert_eq!(
                flere.state(now + Duration::from_secs(7), false).0,
                Mood::Approachable
            );
            let random = flere.random;
            flere.reset(now);
            assert_eq!(
                flere.random, random,
                "screensaver entries never reseed reactions"
            );
            flere.canvas(180, 42, now, false);
            let (x, y) = point(&flere);
            flere.pointer(&Key::Mouse { x, y }, now);
            flere.pointer(
                &Key::Drag {
                    x: x + 25,
                    y: y - 8,
                },
                now + Duration::from_millis(50),
            );
            flere.pointer(
                &Key::Release {
                    x: x + 25,
                    y: y - 8,
                },
                now + Duration::from_millis(60),
            );
            assert_eq!(flere.random, random, "a throw must not masquerade as a pet");
            let before = flere.motion.pose().unwrap();
            for step in 1..=600 {
                flere.tick(now + Duration::from_millis(60 + step * 16));
                let pose = flere.motion.pose().unwrap();
                assert!(pose.x >= 2.0 && pose.x + pose.columns as f32 <= 178.0);
                assert!(pose.y >= 3.0 && pose.y + pose.rows as f32 <= 39.0);
            }
            let after = flere.motion.pose().unwrap();
            assert!((before.x - after.x).abs() > 1.0 || (before.y - after.y).abs() > 1.0);
            for step in 601..=2400 {
                flere.tick(now + Duration::from_millis(60 + step * 16));
            }
            let settled = flere.motion.pose().unwrap();
            for step in 2401..=2460 {
                flere.tick(now + Duration::from_millis(60 + step * 16));
            }
            assert!(
                (settled.x - flere.motion.pose().unwrap().x).abs() > 1.0,
                "Flere resumes wandering after throw inertia settles"
            );
        }
    }
    #[test]
    fn fields_and_rings_animate_but_reduced_motion_and_narrow_frames_stay_bounded() {
        let now = Instant::now();
        let mut flere = Flere::seeded(now, 1);
        for (w, h) in [
            (236, 55),
            (180, 42),
            (100, 30),
            (60, 24),
            (32, 10),
            (10, 8),
            (1, 1),
        ] {
            flere.reset(now);
            let first = flere.canvas(w, h, now, true);
            let later = flere.canvas(w, h, now + Duration::from_secs(2), true);
            assert_eq!(first.cells, later.cells);
            assert_eq!(first.cells.len(), w * h);
            if w >= 34 {
                let text = first
                    .cells
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>();
                assert!(text.contains("Press any key to return"));
                assert!(text.contains("Pet Flere"));
            }
            artifact(&format!("flere-{w}x{h}-quiet"), first);
        }
        flere.reset(now);
        let before = flere.canvas(180, 42, now, false);
        let later = flere.canvas(180, 42, now + Duration::from_secs(4), false);
        assert_ne!(before.cells, later.cells);
        artifact("flere-180x42-later", later);
        flere.canvas(180, 42, now, true);
        let (x, y) = point(&flere);
        flere.pointer(&Key::Mouse { x, y }, now);
        flere.pointer(&Key::Release { x, y }, now);
        let first = flere.canvas(180, 42, now, true);
        assert_eq!(
            first.cells,
            flere
                .canvas(180, 42, now + Duration::from_secs(2), true)
                .cells
        );
        assert!(flere.tick(now + Duration::from_secs(7)));
        assert_ne!(
            first.cells,
            flere
                .canvas(180, 42, now + Duration::from_secs(7), true)
                .cells
        );
    }
    #[test]
    fn small_flere_preserves_original_wordmark_and_wanders_below_the_prompt() {
        let now = Instant::now();
        for (w, h, mascot_top) in [(180, 42, 32), (236, 55, 38)] {
            let mut flere = Flere::seeded(now, 1);
            let canvas = flere.canvas(w, h, now + Duration::from_secs(4), false);
            let original = canvas_with_mascot(w, h, 40, false, true, Mascot::Duck(None));
            assert_eq!(
                &canvas.cells[..(mascot_top - 1) * w],
                &original.cells[..(mascot_top - 1) * w],
                "the original title, wordmark and prompt must remain identical"
            );
            let pose = flere.motion.pose().unwrap();
            assert!(pose.columns <= 50 && pose.rows <= 10);
            assert!(pose.y >= mascot_top as f32);
            artifact(&format!("flere-original-{w}x{h}"), canvas);
            flere.canvas(w, h, now + Duration::from_secs(5), false);
            assert!((pose.x - flere.motion.pose().unwrap().x).abs() >= 4.0);
        }
    }
}
