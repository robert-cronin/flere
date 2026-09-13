//! Embedded pixel mascots. No native input, runtime asset loading or external processes.
use serde::{Deserialize, Serialize};
pub const WIDTH: usize = 64;
pub const HEIGHT: usize = 64;
pub const BACKGROUND: [u8; 3] = [8, 12, 20];
const ASSET: &[u8] = include_bytes!("assets/mascots.rle");
const FRAMES: usize = 24;
mod duck;
pub mod flere;
/// Cosmetic screensaver choice, independent of the optional dock pet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScreensaverMascot {
    Duck,
    #[default]
    #[serde(other)]
    Flere,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Robot,
    Cat,
    Duck,
    #[default]
    #[serde(other)]
    Flere,
}
impl Kind {
    pub const ALL: [Self; 4] = [Self::Duck, Self::Robot, Self::Cat, Self::Flere];
    pub fn label(self) -> &'static str {
        match self {
            Self::Duck => "Duck",
            Self::Robot => "Robot",
            Self::Cat => "Cat",
            Self::Flere => "Flere",
        }
    }
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap()
    }
    pub fn cycle(self, delta: i32) -> Self {
        Self::ALL[(self.index() as i32 + delta).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Walk,
    Sit,
    Sleep,
    Look,
    Alert,
    Stretch,
    Groom,
    Pounce,
}
impl Action {
    pub const ALL: [Self; 8] = [
        Self::Walk,
        Self::Sit,
        Self::Sleep,
        Self::Look,
        Self::Alert,
        Self::Stretch,
        Self::Groom,
        Self::Pounce,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Walk => "Walking",
            Self::Sit => "Sitting",
            Self::Sleep => "Sleeping",
            Self::Look => "Looking around",
            Self::Alert => "Alert",
            Self::Stretch => "Stretching",
            Self::Groom => "Grooming",
            Self::Pounce => "Pouncing",
        }
    }
    pub fn period(self) -> f32 {
        match self {
            Self::Walk => 0.9,
            Self::Sleep => 3.0,
            Self::Sit => 4.0,
            Self::Look => 3.0,
            Self::Alert => 1.6,
            Self::Stretch => 2.4,
            Self::Groom => 1.2,
            Self::Pounce => 1.8,
        }
    }
}

#[derive(Clone)]
pub struct Sprite {
    pub pixels: Vec<[u8; 4]>,
}
fn palette() -> Vec<[u8; 3]> {
    let mut colours = vec![BACKGROUND];
    for c in ASSET[12..12 + ASSET[10] as usize * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .skip(1)
    {
        colours.push([c[0], c[1], c[2]]);
    }
    colours.extend(duck::COLOURS);
    // Procedural fields and white casing share the existing bounded sixel path.
    colours.extend([
        [238, 240, 231],
        [216, 223, 217],
        [158, 173, 169],
        [89, 105, 103],
        [190, 215, 96],
        [107, 123, 56],
        [57, 69, 38],
        [238, 140, 64],
        [151, 80, 43],
        [80, 43, 30],
    ]);
    colours
}
pub fn sprite(action: Action, phase: f32, left: bool) -> Sprite {
    sprite_kind(Kind::Duck, action, phase, left)
}
pub fn sprite_kind(kind: Kind, action: Action, phase: f32, left: bool) -> Sprite {
    if matches!(kind, Kind::Duck | Kind::Flere) {
        let mut sprite = if kind == Kind::Duck {
            duck::sprite(action, phase)
        } else {
            flere::sprite(
                phase * action.period(),
                flere::Mood::Approachable,
                0.0,
                false,
            )
        };
        if left {
            for row in sprite.pixels.as_chunks_mut::<WIDTH>().0 {
                row.reverse();
            }
        }
        return sprite;
    }
    let action = Action::ALL.iter().position(|a| *a == action).unwrap();
    let frame = ((phase.rem_euclid(1.0) * FRAMES as f32) as usize).min(FRAMES - 1);
    // Embedded ordering is immutable; selector order may gain procedural pets.
    let asset = match kind {
        Kind::Robot => 1,
        Kind::Cat => 2,
        Kind::Duck | Kind::Flere => unreachable!(),
    };
    let index = (asset * Action::ALL.len() + action) * FRAMES + frame;
    let colours = ASSET[10] as usize;
    let table = 12 + colours * 4;
    let payload = table + (3 * Action::ALL.len() * FRAMES + 1) * 4;
    let offset = |n: usize| {
        u32::from_le_bytes(ASSET[table + n * 4..table + n * 4 + 4].try_into().unwrap()) as usize
    };
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT);
    for pair in ASSET[payload + offset(index)..payload + offset(index + 1)]
        .as_chunks::<2>()
        .0
    {
        let p = 12 + pair[1] as usize * 4;
        let c: [u8; 4] = ASSET[p..p + 4].try_into().unwrap();
        pixels.extend(std::iter::repeat_n(c, pair[0] as usize));
    }
    if left {
        for row in pixels.as_chunks_mut::<WIDTH>().0 {
            row.reverse();
        }
    }
    Sprite { pixels }
}
impl Sprite {
    pub fn blend(&mut self, old: &Self, t: f32) {
        let t = t.clamp(0.0, 1.0);
        for (new, old) in self.pixels.iter_mut().zip(&old.pixels) {
            let alpha = old[3] as f32 * (1.0 - t) + new[3] as f32 * t;
            if alpha > 0.0 {
                for c in 0..3 {
                    new[c] = ((old[c] as f32 * old[3] as f32 * (1.0 - t)
                        + new[c] as f32 * new[3] as f32 * t)
                        / alpha)
                        .round() as u8;
                }
            }
            new[3] = alpha.round() as u8;
        }
    }
    fn sample(&self, x: f32, y: f32) -> [u8; 4] {
        if x < 0.0 || y < 0.0 || x >= WIDTH as f32 || y >= HEIGHT as f32 {
            return [0; 4];
        }
        self.pixels[y as usize * WIDTH + x as usize]
    }
}
/// Paint the entire private background to erase the previous position.
pub fn scene(
    sprite: &Sprite,
    width: usize,
    height: usize,
    x: f32,
    scale: f32,
) -> crate::sixel::Raster {
    scene_pose(sprite, width, height, x, scale, [0.0, 0.0])
}
/// Vertical offset and rotation about the character's body centre for carrying.
pub fn scene_pose(
    sprite: &Sprite,
    width: usize,
    height: usize,
    x: f32,
    scale: f32,
    pose: [f32; 2],
) -> crate::sixel::Raster {
    let mut rgb = BACKGROUND.repeat(width * height);
    let top = height as f32 - HEIGHT as f32 * scale + pose[0];
    let (sin, cos) = pose[1].sin_cos();
    for y in 0..height {
        for px in 0..width {
            // Area samples retain small motions when many source pixels share one
            // terminal half-cell. Pixel-capable terminals keep crisp nearest sampling.
            let steps = if scale < 1.0 { 4 } else { 1 };
            let mut sum = [0.0; 3];
            for sy in 0..steps {
                for sx in 0..steps {
                    let dx = (px as f32 - x + (sx as f32 + 0.5) / steps as f32) / scale - 32.0;
                    let dy = (y as f32 - top + (sy as f32 + 0.5) / steps as f32) / scale - 36.0;
                    let pixel =
                        sprite.sample(dx * cos + dy * sin + 32.0, -dx * sin + dy * cos + 36.0);
                    let alpha = pixel[3] as f32 / 255.0;
                    for c in 0..3 {
                        sum[c] += BACKGROUND[c] as f32 * (1.0 - alpha) + pixel[c] as f32 * alpha;
                    }
                }
            }
            for c in 0..3 {
                rgb[(y * width + px) * 3 + c] = (sum[c] / (steps * steps) as f32).round() as u8;
            }
        }
    }
    crate::sixel::Raster { width, height, rgb }
}
/// Small UI-owned effect symbols, clipped to the mascot canvas.
pub fn token(r: &mut crate::sixel::Raster, x: i32, y: i32, symbol: u8, scale: i32) {
    let (rows, colour): (&[u8], [u8; 3]) = match symbol {
        0 => (
            &[0b01010, 0b11111, 0b11111, 0b01110, 0b00100],
            [247, 120, 205],
        ),
        1 => (
            &[0b01110, 0b11111, 0b11111, 0b11111, 0b01110],
            [83, 238, 240],
        ),
        _ => (
            &[0b00100, 0b00100, 0b11111, 0b00100, 0b00100],
            [255, 237, 170],
        ),
    };
    for (dy, row) in rows.iter().enumerate() {
        for dx in 0..5 {
            if row & (1 << (4 - dx)) != 0 {
                for yy in 0..scale {
                    for xx in 0..scale {
                        let (px, py) = (x + dx * scale + xx, y + dy as i32 * scale + yy);
                        if px >= 0 && py >= 0 && (px as usize) < r.width && (py as usize) < r.height
                        {
                            let i = (py as usize * r.width + px as usize) * 3;
                            r.rgb[i..i + 3].copy_from_slice(&colour);
                        }
                    }
                }
            }
        }
    }
}
/// A fixed small palette preserves crisp colours and a solid undithered floor.
pub fn encode(r: &crate::sixel::Raster) -> std::io::Result<String> {
    use std::{collections::HashMap, fmt::Write};
    if r.width == 0
        || r.height == 0
        || r.width > 640
        || r.height > 160
        || r.rgb.len() != r.width * r.height * 3
    {
        return Err(std::io::Error::other("mascot raster outside bounds"));
    }
    let colours = palette();
    let mut out = format!("\x1bP7;1q\"1;1;{};{}", r.width, r.height);
    for (n, c) in colours.iter().enumerate() {
        let _ = write!(
            out,
            "#{n};2;{};{};{}",
            (c[0] as u32 * 100 + 127) / 255,
            (c[1] as u32 * 100 + 127) / 255,
            (c[2] as u32 * 100 + 127) / 255
        );
    }
    let mut lookup: HashMap<[u8; 3], usize> = colours
        .iter()
        .copied()
        .enumerate()
        .map(|(n, c)| (c, n))
        .collect();
    let indices: Vec<usize> = r
        .rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let rgb = *p;
            *lookup.entry(rgb).or_insert_with(|| {
                colours
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, c)| {
                        (0..3)
                            .map(|i| (rgb[i] as i32 - c[i] as i32).pow(2))
                            .sum::<i32>()
                    })
                    .unwrap()
                    .0
            })
        })
        .collect();
    let mut bands = vec![0u8; colours.len() * r.width];
    for y in (0..r.height).step_by(6) {
        bands.fill(0);
        for dy in 0..6.min(r.height - y) {
            for x in 0..r.width {
                bands[indices[(y + dy) * r.width + x] * r.width + x] |= 1 << dy;
            }
        }
        let mut first = true;
        for n in 0..colours.len() {
            let values = &bands[n * r.width..(n + 1) * r.width];
            if let Some(end) = values.iter().rposition(|v| *v != 0) {
                if !first {
                    out.push('$');
                }
                first = false;
                let _ = write!(out, "#{n}");
                let mut x = 0;
                while x <= end {
                    let mut next = x + 1;
                    while next <= end && values[next] == values[x] {
                        next += 1;
                    }
                    let ch = (values[x] + 63) as char;
                    if next - x > 3 {
                        let _ = write!(out, "!{}{ch}", next - x);
                    } else {
                        for _ in x..next {
                            out.push(ch);
                        }
                    }
                    x = next;
                }
            }
        }
        if y + 6 < r.height {
            out.push('-');
        }
    }
    out.push_str("\x1b\\");
    if out.len() > 24 * 1024 {
        return Err(std::io::Error::other("mascot frame exceeds 24 KiB"));
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_preferences_and_unknown_characters_preserve_layout() {
        for value in [
            r#"{"left":38,"pet":true}"#,
            r#"{"left":38,"pet":true,"pet_kind":"future-pet"}"#,
            r#"{"left":38,"pet":true,"screensaver_mascot":"future-mascot"}"#,
        ] {
            let p: crate::workspace::Preferences = serde_json::from_str(value).unwrap();
            assert_eq!(p.left, 38);
            assert!(p.pet);
            assert_eq!(p.pet_kind, Kind::Flere);
            assert_eq!(p.screensaver_mascot, ScreensaverMascot::Flere);
        }
        for (kind, value) in [
            (Kind::Duck, "duck"),
            (Kind::Robot, "robot"),
            (Kind::Cat, "cat"),
            (Kind::Flere, "flere"),
        ] {
            let p: crate::workspace::Preferences = serde_json::from_value(
                serde_json::json!({"pet_kind":value,"screensaver_mascot":"duck"}),
            )
            .unwrap();
            assert_eq!(p.pet_kind, kind);
            assert_eq!(p.screensaver_mascot, ScreensaverMascot::Duck);
            assert_eq!(kind.cycle(4), kind);
            assert_eq!(kind.cycle(1).cycle(-1), kind);
        }
    }
    #[test]
    fn embedded_actions_are_complete_bounded_and_mirrored() {
        assert_eq!(&ASSET[..8], b"RHSPR001");
        assert_eq!(ASSET[11], 64);
        assert_eq!(u16::from_le_bytes(ASSET[8..10].try_into().unwrap()), 576);
        for kind in Kind::ALL {
            for action in Action::ALL {
                for frame in 0..FRAMES {
                    let a = sprite_kind(kind, action, frame as f32 / FRAMES as f32, false);
                    let b = sprite_kind(kind, action, frame as f32 / FRAMES as f32, true);
                    assert_eq!(a.pixels.len(), WIDTH * HEIGHT);
                    assert!(a.pixels.iter().filter(|p| p[3] > 0).count() > 300);
                    for y in 0..HEIGHT {
                        assert_eq!(a.pixels[y * WIDTH][3], 0);
                        assert_eq!(a.pixels[(y + 1) * WIDTH - 1][3], 0);
                        for x in 0..WIDTH {
                            assert_eq!(
                                a.pixels[y * WIDTH + x],
                                b.pixels[y * WIDTH + WIDTH - 1 - x]
                            );
                        }
                    }
                    assert!(a.pixels[..WIDTH].iter().all(|p| p[3] == 0));
                    assert!(a.pixels[(HEIGHT - 1) * WIDTH..].iter().all(|p| p[3] == 0));
                }
            }
        }
    }
    #[test]
    fn embedded_robot_and_cat_keep_original_asset_frames() {
        for (kind, expected) in [
            (Kind::Robot, 0xd06d_f6fd_0023_2def),
            (Kind::Cat, 0x23f9_6f86_a480_9d3f),
        ] {
            let frame = sprite_kind(kind, Action::Walk, 4.0 / FRAMES as f32, false);
            let digest = frame
                .pixels
                .iter()
                .flatten()
                .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
                    (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
                });
            assert_eq!(digest, expected, "{kind:?} changed embedded frames");
        }
    }
    #[test]
    fn coloured_scene_replaces_old_position_with_bounded_encoding() {
        for kind in Kind::ALL {
            let s = sprite_kind(kind, Action::Walk, 0.2, false);
            let a = scene(&s, 320, 150, 0.0, 2.0);
            let b = scene(&s, 320, 150, 180.0, 2.0);
            assert_ne!(a.rgb, b.rgb);
            for y in 0..150 {
                for x in 0..128 {
                    assert_eq!(
                        &b.rgb[(y * 320 + x) * 3..(y * 320 + x) * 3 + 3],
                        &BACKGROUND
                    );
                }
            }
            for (w, h) in [(1, 1), (290, 150), (640, 160)] {
                let r = scene(&s, w, h, 0.0, 2.0);
                let encoded = encode(&r).unwrap();
                assert!(encoded.len() < 24 * 1024);
            }
        }
    }
}
