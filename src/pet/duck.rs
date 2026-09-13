//! Compact yellow duck authored on a fixed pixel grid from the approved mockup.
//! The surrounding pet controller supplies timing, transitions, mirroring and physics.
use super::{Action, HEIGHT, Sprite, WIDTH};
pub(super) const COLOURS: [[u8; 3]; 4] = [
    [255, 220, 55],
    [255, 237, 139],
    [231, 173, 43],
    [245, 143, 38],
];
// Lower the head onto one short neck row while retaining the body's ground line.
const HEAD_DROP: i32 = 4;
const BODY: &[&str] = &[
    "                       ",
    "                       ",
    "                       ",
    "                       ",
    "           HHHHH       ",
    "          HYYYYYY      ",
    "          HYYYYYY      ",
    "          HYYYYYY      ",
    "          HYYYYYY      ",
    "          HYYYYY       ",
    "    H     HYYYY        ",
    "    HH   HYYYYYYY      ",
    "    HYHHHYYYYYYYYY     ",
    "     YYYYYYYYYYYYYY    ",
    "     YYYYYYYYYYYYYY    ",
    "      YYYYYYYYYYYY     ",
    "       YYYYYYYYYY      ",
    "        SYYYYYSS       ",
    "         SSSSSS        ",
];
pub(super) fn sprite(action: Action, phase: f32) -> Sprite {
    let phase = phase.rem_euclid(1.0);
    let bob = if action == Action::Walk {
        ((phase * std::f32::consts::TAU * 2.0).sin() > 0.0) as i32
    } else {
        0
    };
    let hop = if action == Action::Pounce {
        (5.0 * (phase * std::f32::consts::PI).sin().powi(2)).round() as i32
    } else {
        0
    };
    let mut sprite = Sprite {
        pixels: vec![[0; 4]; WIDTH * HEIGHT],
    };
    let mut block = |x: i32, y: i32, colour: [u8; 3]| {
        for dy in 0..2 {
            for dx in 0..2 {
                let px = 8 + x * 2 + dx;
                let py = 12 + y * 2 + dy - hop;
                if px > 0 && px < WIDTH as i32 - 1 && py > 0 && py < HEIGHT as i32 - 1 {
                    sprite.pixels[py as usize * WIDTH + px as usize] =
                        [colour[0], colour[1], colour[2], 255];
                }
            }
        }
    };
    for (y, row) in BODY.iter().enumerate() {
        for (x, ch) in row.bytes().enumerate() {
            let colour = match ch {
                b'Y' => Some(COLOURS[0]),
                b'H' => Some(COLOURS[1]),
                b'S' => Some(COLOURS[2]),
                _ => None,
            };
            if let Some(colour) = colour {
                block(x as i32, y as i32 + bob, colour);
            }
        }
    }
    // A single dark eye and a short orange bill; no outfit or visor.
    let eye_y = if action == Action::Sleep { 5 } else { 3 } + HEAD_DROP + bob;
    block(15, eye_y, [6, 11, 22]);
    if action != Action::Sleep && !(0.84..0.91).contains(&phase) {
        block(15, eye_y + 1, [6, 11, 22]);
    }
    for x in 17..20 {
        block(x, 5 + HEAD_DROP + bob, COLOURS[3]);
    }
    for x in 17..19 {
        block(x, 6 + HEAD_DROP + bob, COLOURS[3]);
    }
    // Subtle wing and distinct gestures stay readable at native pixel scale.
    for x in 9..13 {
        block(x, 14 + bob, COLOURS[2]);
    }
    if action == Action::Stretch || action == Action::Groom {
        for y in 10..14 {
            block(14, y + bob, COLOURS[1]);
        }
    }
    let step = if action == Action::Walk {
        if phase < 0.5 { -1 } else { 1 }
    } else {
        0
    };
    for (x, y) in [
        (10 - step, 19),
        (10 - step, 20),
        (11 - step, 20),
        (14 + step, 19),
        (14 + step, 20),
        (15 + step, 20),
    ] {
        block(x, y, COLOURS[3]);
    }
    sprite
}
