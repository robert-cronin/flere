//! Procedural interpretation of Flere-Imsaho's white disc and communicative fields.
//! Ring count, finish and cadence are visual choices; fields are not casing paint.
use crate::sixel::Raster;
use std::f32::consts::{PI, TAU};

pub const BACKGROUND: [u8; 3] = [6, 14, 22];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Approachable,
    Satisfied,
    Irritated,
    /// A colour study, not an application do-not-disturb state.
    Contrite,
}
impl Mood {
    pub fn label(self) -> &'static str {
        match self {
            Self::Approachable => "Mellow approachability",
            Self::Satisfied => "Quite satisfactory.",
            Self::Irritated => "A little dignity, please.",
            Self::Contrite => "Contrition / silver withdrawal",
        }
    }
}
fn mix(a: [f32; 3], b: [f32; 3], part: f32) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * part.clamp(0.0, 1.0))
}
fn over(rgb: &mut [f32; 3], colour: [f32; 3], alpha: f32) {
    *rgb = mix(*rgb, colour, alpha);
}
struct Scene {
    cx: f32,
    cy: f32,
    radius: f32,
    depth: f32,
    tilt: [f32; 2],
    time: f32,
    mood: Mood,
    strength: f32,
    field: [f32; 3],
    background: [u8; 3],
    aspect: f32,
}
impl Scene {
    fn new(width: usize, height: usize, time: f32, mood: Mood, strength: f32, quiet: bool) -> Self {
        let time = if quiet { 0.0 } else { time };
        let strength = strength.clamp(0.0, 1.0);
        let mut angle = -0.13;
        if mood == Mood::Irritated {
            angle += strength
                * (0.16
                    + if quiet {
                        0.0
                    } else {
                        (time * 3.1).sin() * 0.035
                    });
        } else if mood == Mood::Satisfied {
            angle -= strength * 0.045;
        }
        let (sin, cos) = angle.sin_cos();
        let field = match mood {
            Mood::Satisfied => mix([190.0, 215.0, 96.0], [238.0, 140.0, 64.0], strength),
            Mood::Contrite => mix([190.0, 215.0, 96.0], [166.0, 143.0, 208.0], strength),
            _ => [190.0, 215.0, 96.0],
        };
        let radius = (width as f32 * 0.255).min(height as f32 * 0.62).max(0.4);
        Self {
            cx: width as f32 * 0.5,
            cy: height as f32 * 0.50
                + if quiet {
                    0.0
                } else {
                    (time * 0.85).sin() * radius * 0.025
                },
            radius,
            depth: radius * 0.072,
            tilt: [sin, cos],
            time,
            mood,
            strength,
            field,
            background: BACKGROUND,
            aspect: 0.52,
        }
    }
    fn sample(&self, px: f32, py: f32) -> [f32; 3] {
        let mut rgb = self.background.map(f32::from);
        let (dx, dy) = (px - self.cx, py - self.cy);
        let x = (dx * self.tilt[1] + dy * self.tilt[0]) / self.radius;
        let y = (-dx * self.tilt[0] + dy * self.tilt[1]) / self.radius;
        let ry = self.aspect;
        let warmth = if self.mood == Mood::Satisfied {
            self.strength
        } else {
            0.0
        };
        let tight = if self.mood == Mood::Irritated {
            self.strength * 0.12
        } else {
            0.0
        };
        // A close, breathing emotion field leaves the solid white disc dominant
        // at both dock and intro sizes. No detached lobes or wing-like sheets.
        let radius = (x * x + (y / ry).powi(2)).sqrt();
        let angle = (y / ry).atan2(x);
        let ripple = (angle * 3.0 - self.time * 0.55).sin() * 0.025;
        if radius < 1.32 {
            let boundary = 1.12 + warmth * 0.035 - tight * 0.3 + ripple;
            let glow = (-((radius - boundary) / 0.11).powi(2)).exp() * 0.17;
            let contour = (-((radius - boundary) / 0.024).powi(2)).exp()
                * (0.12 + 0.10 * (angle * 2.0 + self.time * 0.43).sin().powi(2));
            let edge = ((1.32 - radius) / 0.08).clamp(0.0, 1.0);
            over(&mut rgb, self.field, (glow + contour) * edge);
        }
        if self.mood == Mood::Irritated {
            for i in 0..7 {
                let angle = i as f32 * TAU / 7.0 + self.time * 0.12;
                let (fx, fy) = (angle.cos() * 1.16, angle.sin() * ry * 1.16);
                let fleck = (-(((x - fx) / 0.037).powi(2) + ((y - fy) / 0.023).powi(2))).exp();
                over(&mut rgb, [249.0, 251.0, 239.0], fleck * self.strength);
            }
        }
        let edge_distance = (x * x + ((y - self.depth / self.radius) / ry).powi(2)).sqrt();
        if edge_distance <= 1.0 {
            // Physical white casing remains white in every emotion.
            rgb = mix(
                [158.0, 173.0, 169.0],
                [222.0, 226.0, 217.0],
                (x + 1.0) * 0.35,
            );
            if edge_distance > 0.98 {
                rgb = [196.0, 205.0, 192.0];
            }
        }
        let r = (x * x + (y / ry).powi(2)).sqrt();
        if r <= 1.0 {
            let shade = (0.57 - y * 0.7 - x * 0.09).clamp(0.0, 1.0);
            rgb = mix([216.0, 223.0, 217.0], [251.0, 251.0, 242.0], shade);
            for radius in [0.47, 0.70, 0.88] {
                let groove = (-((r - radius) / 0.022).powi(2)).exp();
                let lip = (-((r - radius + 0.015) / 0.008).powi(2)).exp();
                over(&mut rgb, [89.0, 105.0, 103.0], groove * 0.87);
                over(&mut rgb, [255.0, 255.0, 246.0], lip * 0.65);
            }
            // Independently revolving outer ring seams around an unmarked core.
            let ring = if r >= 0.88 {
                Some((0.21, 0.9))
            } else if r >= 0.70 {
                Some((-0.32, 3.1))
            } else if r >= 0.47 {
                Some((0.13, 4.6))
            } else {
                None
            };
            if let Some((speed, offset)) = ring {
                let angle = (y / ry).atan2(x);
                let distance = (angle - self.time * speed - offset + PI).rem_euclid(TAU) - PI;
                let seam = (-(distance / 0.025).powi(2)).exp();
                over(&mut rgb, [107.0, 122.0, 119.0], seam * 0.85);
            }
            if r > 0.977 {
                over(&mut rgb, [255.0, 255.0, 246.0], 0.55);
            }
        }
        rgb
    }
}
/// A bounded actual rendering shared by half-block terminal cells and QA exports.
pub fn frame(
    width: usize,
    height: usize,
    time: f32,
    mood: Mood,
    strength: f32,
    quiet: bool,
) -> Raster {
    let width = width.clamp(1, 320);
    let height = height.clamp(1, 160);
    let scene = Scene::new(width, height, time, mood, strength, quiet);
    render(width, height, &scene)
}

fn render(width: usize, height: usize, scene: &Scene) -> Raster {
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0; 3];
            for sy in 0..3 {
                for sx in 0..3 {
                    let pixel = scene.sample(
                        x as f32 + (sx as f32 + 0.5) / 3.0,
                        y as f32 + (sy as f32 + 0.5) / 3.0,
                    );
                    for i in 0..3 {
                        sum[i] += pixel[i];
                    }
                }
            }
            rgb.extend(sum.map(|v| (v / 9.0).round() as u8));
        }
    }
    Raster { width, height, rgb }
}

/// Small transparent dock sprite, using the same casing and field renderer.
/// The body centre matches the existing carry physics at (32, 36).
pub(crate) fn sprite(time: f32, mood: Mood, strength: f32, quiet: bool) -> super::Sprite {
    let mut scene = Scene::new(64, 40, time, mood, strength, quiet);
    scene.radius *= 1.27;
    scene.depth = scene.radius * 0.09;
    scene.background = super::BACKGROUND;
    let raster = render(64, 40, &scene);
    let mut pixels = vec![[0; 4]; super::WIDTH * super::HEIGHT];
    for (i, colour) in raster.rgb.as_chunks::<3>().0.iter().enumerate() {
        if *colour != super::BACKGROUND {
            pixels[16 * super::WIDTH + i] = [colour[0], colour[1], colour[2], 255];
        }
    }
    super::Sprite { pixels }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ring_seams_fields_and_casing_have_distinct_temporal_and_emotional_behaviour() {
        let first = frame(120, 48, 0.0, Mood::Approachable, 0.0, false);
        let later = frame(120, 48, 4.0, Mood::Approachable, 0.0, false);
        let mut body_changes = 0;
        let mut field_changes = 0;
        for (a, b) in first
            .rgb
            .as_chunks::<3>()
            .0
            .iter()
            .zip(later.rgb.as_chunks::<3>().0)
        {
            if a != b {
                if a[0] > 180 && a[1] > 180 && a[2] > 180 {
                    body_changes += 1;
                } else if a[1] > a[2] + 20 {
                    field_changes += 1;
                }
            }
        }
        assert!(body_changes > 80, "independent ring seams must revolve");
        assert!(field_changes > 100, "the compact emotion halo must breathe");
        for mood in [
            Mood::Approachable,
            Mood::Satisfied,
            Mood::Irritated,
            Mood::Contrite,
        ] {
            let raster = frame(120, 48, 0.0, mood, 1.0, true);
            let centre = (24 * 120 + 60) * 3;
            assert!(
                raster.rgb[centre..centre + 3].iter().all(|v| *v > 220),
                "casing stays white"
            );
        }
    }
    #[test]
    fn intro_disc_keeps_a_solid_white_body_and_compact_halo_in_every_mood() {
        for mood in [
            Mood::Approachable,
            Mood::Satisfied,
            Mood::Irritated,
            Mood::Contrite,
        ] {
            for time in [0.0, 4.0] {
                let raster = frame(120, 48, time, mood, 1.0, false);
                let mut white = 0;
                for (i, pixel) in raster.rgb.as_chunks::<3>().0.iter().enumerate() {
                    let (x, y) = (i % 120, i / 120);
                    white += usize::from(pixel.iter().all(|&c| c > 180));
                    if !(16..104).contains(&x) || !(2..46).contains(&y) {
                        assert_eq!(
                            *pixel, BACKGROUND,
                            "the emotion field must not grow detached side wings or touch frame edges"
                        );
                    }
                }
                assert!(
                    white > 700,
                    "the white casing must remain visually dominant"
                );
            }
        }
    }
    #[test]
    fn actual_raster_release_timing() {
        let started = std::time::Instant::now();
        for i in 0..30 {
            std::hint::black_box(frame(
                120,
                48,
                i as f32 * 0.1,
                Mood::Approachable,
                0.0,
                false,
            ));
        }
        eprintln!(
            "Flere 120x48, 9 area samples, 30 frames: {:.3} ms/frame ({})",
            started.elapsed().as_secs_f64() * 1000.0 / 30.0,
            if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
        );
    }
}
