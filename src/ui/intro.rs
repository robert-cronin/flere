//! Shared first-use and idle presentation; no socket commands or child launches.
use super::*;
mod duck;
mod flere;
pub(super) use duck::Duck;
use duck::Pose;
pub(super) use flere::Flere;

const TICK: Duration = Duration::from_millis(100);
const REVEAL: usize = 26;
const INK: Color = Color::Rgb(25, 53, 77);
const GHOST: Color = Color::Rgb(56, 27, 66);
const FONT: [[u8; 7]; 5] = [
    [31, 16, 16, 30, 16, 16, 16], // F
    [16, 16, 16, 16, 16, 16, 31], // L
    [31, 16, 16, 30, 16, 16, 31], // E
    [30, 17, 17, 30, 20, 18, 17], // R
    [31, 16, 16, 30, 16, 16, 31], // E
];

fn blend(a: Color, b: Color, part: usize) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return a;
    };
    let p = part.min(100) as u16;
    let channel = |a: u8, b: u8| ((a as u16 * (100 - p) + b as u16 * p) / 100) as u8;
    Color::Rgb(channel(ar, br), channel(ag, bg), channel(ab, bb))
}
fn center(c: &mut Canvas, y: usize, text: &str, color: Color, bold: bool) {
    let width = text
        .chars()
        .map(|ch| char_width(ch) as usize)
        .sum::<usize>();
    c.text(
        c.width.saturating_sub(width) / 2,
        y,
        c.width,
        text,
        style(color, BG, bold),
    );
}
fn noise(x: usize, y: usize) -> usize {
    x.wrapping_mul(7919).wrapping_add(y.wrapping_mul(104729)) ^ (x + y).wrapping_mul(31)
}
enum Mascot<'a> {
    Duck(Option<&'a mut Duck>),
    Flere(&'a mut Flere, Instant),
}
pub(super) fn interactive_canvas(
    width: usize,
    height: usize,
    step: usize,
    quiet: bool,
    duck: &mut Duck,
) -> Canvas {
    canvas_with_mascot(width, height, step, quiet, true, Mascot::Duck(Some(duck)))
}
fn canvas_with_mascot(
    width: usize,
    height: usize,
    step: usize,
    quiet: bool,
    saver: bool,
    mascot: Mascot<'_>,
) -> Canvas {
    let w = width.clamp(1, 320);
    let h = height.clamp(1, 106);
    let mut c = Canvas::new(w, h);
    let step = if quiet { REVEAL + 8 } else { step };
    let ready = step >= REVEAL;

    // Sparse falling data stays behind the clean wordmark; no terminal output is reused.
    let digits = ["0", "1", ":", "·", "╎", "+", "░", "⋮"];
    for x in (3..w.saturating_sub(2)).step_by(if w >= 100 { 4 } else { 7 }) {
        let head = (step * (1 + x % 3) + noise(x, 0)) % h;
        for tail in 0..9.min(h) {
            let y = (head + h - tail) % h;
            let color = if tail == 0 {
                blend(INK, CYAN, 48)
            } else {
                blend(BG, INK, 90 - tail * 8)
            };
            c.text(
                x,
                y,
                1,
                digits[noise(x, y + step / 3) % digits.len()],
                style(color, BG, false),
            );
        }
    }
    if w > 36 && h > 12 {
        c.border(1, 2, w - 2, h - 4, INK);
        let scanner = 3 + step * 3 % w.saturating_sub(6).max(1);
        c.text(scanner, 2, 3, "━━╸", style(CYAN, BG, true));
        c.text(
            w.saturating_sub(scanner + 3),
            h - 3,
            3,
            "╺━━",
            style(MAGENTA, BG, true),
        );
        c.text(
            3,
            0,
            w - 6,
            "▰ FLERE   /   TERMINAL WORKBENCH",
            style(MUTED, BG, false),
        );
    }

    let large = w >= 35 && h >= 19;
    let sx = if w >= 170 && h >= 38 {
        3
    } else if w >= 110 && h >= 28 {
        2
    } else {
        1
    };
    let sy = if w >= 170 && h >= 38 { 2 } else { 1 };
    let logo_w = if large {
        (FONT.len() * 6 - 1) * sx
    } else {
        9.min(w)
    };
    let logo_h = if large { 7 * sy } else { 1 };
    let left = w.saturating_sub(logo_w) / 2;
    let compact = h < 28;
    let top = (h.saturating_sub(logo_h) / 2)
        .saturating_sub(if compact { 4 } else { 2 })
        .max(if large && w > 36 { 3 } else { 0 });
    let floor = (top + logo_h + 6).min(h);

    // Perspective rails and traveling pulses create depth below the title.
    let depth = h.saturating_sub(floor + 3);
    if depth > 2 {
        for y in floor..h - 3 {
            let d = y - floor + 1;
            if (d * d + step / 2) % (depth + 2) < 2 {
                c.fill(3, y, w.saturating_sub(6), 1, style(INK, BG, false));
                c.text(
                    3,
                    y,
                    w.saturating_sub(6),
                    &"─".repeat(w.saturating_sub(6)),
                    style(INK, BG, false),
                );
            }
            for lane in -5_i64..=5 {
                let x = w as i64 / 2 + lane * (d * d * w / (depth * depth * 10).max(1)) as i64;
                if x > 2 && x < w as i64 - 3 {
                    let pulse = (d + step / 2 + lane.unsigned_abs() as usize) % 11 < 2;
                    let color = if pulse {
                        blend(INK, if lane < 0 { CYAN } else { MAGENTA }, 55)
                    } else {
                        INK
                    };
                    c.text(
                        x as usize,
                        y,
                        1,
                        if lane < 0 {
                            "╱"
                        } else if lane > 0 {
                            "╲"
                        } else {
                            "│"
                        },
                        style(color, BG, false),
                    );
                }
            }
        }
    }
    // A quiet central field keeps the animated block font readable.
    let quiet_top = top
        .saturating_sub(3)
        .max(if w > 36 && h > 12 { 3 } else { 0 });
    c.fill(
        left.saturating_sub(3),
        quiet_top,
        logo_w + 6,
        (top + logo_h + 5)
            .min(if w > 36 && h > 12 { h - 3 } else { h })
            .saturating_sub(quiet_top),
        style(TEXT, BG, false),
    );
    if large {
        if top >= 6 {
            center(&mut c, top - 3, "[ F L E R E  / /  N E O N ]", MUTED, false);
        }
        for (letter, rows) in FONT.iter().enumerate() {
            for (row, bits) in rows.iter().enumerate() {
                for col in 0..5 {
                    if bits & (1 << (4 - col)) == 0 {
                        continue;
                    }
                    let phase = letter * 2 + row / 2;
                    let age = step.saturating_sub(phase);
                    if step < phase {
                        continue;
                    }
                    let glyph = if !ready && age < 2 {
                        "░"
                    } else if !ready && age < 4 {
                        "▓"
                    } else {
                        "█"
                    };
                    let x = left + (letter * 6 + col) * sx;
                    let y = top + row * sy;
                    let wave = (letter * 10 + col * 3 + step * 2) % 160;
                    let tint = if wave < 80 { wave } else { 160 - wave };
                    let mut color = blend(CYAN, MAGENTA, tint);
                    if (step * 3 + 170 - (letter * 6 + col) * 3) % 170 < 7 {
                        color = blend(color, TEXT, 75);
                    }
                    c.text(x + 1, y + sy, sx, &"░".repeat(sx), style(GHOST, BG, false));
                    for yy in y..y + sy {
                        c.text(x, yy, sx, &glyph.repeat(sx), style(color, BG, true));
                    }
                }
            }
        }
    } else {
        center(
            &mut c,
            top,
            if w >= 9 { "F L E R E" } else { "FLERE" },
            CYAN,
            true,
        );
    }
    let caption = top + logo_h + 2;
    if h >= 28 {
        center(&mut c, caption, "YOUR WORK. YOUR TERMINAL.", MUTED, false);
    }
    let prompt = if h >= 12 {
        caption + if compact { 0 } else { 2 }
    } else {
        (top + 2).min(h - 1)
    };
    let prompt_color = if ready {
        blend(CYAN, TEXT, (step % 20).abs_diff(10) * 6)
    } else {
        MUTED
    };
    center(
        &mut c,
        prompt,
        if saver && w >= 28 {
            "Press any key to return"
        } else if saver {
            "Any key to return"
        } else if w >= 28 {
            "Press any key to start"
        } else {
            "Any key to start"
        },
        prompt_color,
        true,
    );
    if w >= 80 && h >= 25 {
        center(
            &mut c,
            top.saturating_sub(5),
            if ready {
                "━━━━━━━━━━━━  ◆  ━━━━━━━━━━━━"
            } else {
                "┄┄┄┄┄┄┄┄┄┄┄┄  ◇  ┄┄┄┄┄┄┄┄┄┄┄┄"
            },
            if ready { MAGENTA } else { INK },
            false,
        );
        if !saver {
            c.text(4, h - 1, 30, "Ctrl+C exit", style(MUTED, BG, false));
        }
        let version = concat!("FLERE / ", env!("CARGO_PKG_VERSION"));
        c.text(
            w.saturating_sub(version.len() + 4),
            h - 1,
            version.len(),
            version,
            style(MUTED, BG, false),
        );
    }
    if w >= 200 && h >= 38 {
        for (x, label, color) in [
            (5, "CHROMA / CYAN", CYAN),
            (w - 28, "CHROMA / MAGENTA", MAGENTA),
        ] {
            c.text(
                x,
                h / 2 - 5,
                23,
                label,
                style(blend(INK, color, 55), BG, false),
            );
            for row in 0..7 {
                let count = 3 + noise(row, step / 2) % 17;
                let line = format!("{:02X} {}", (step + row * 31) % 256, "▰".repeat(count));
                c.text(
                    x,
                    h / 2 - 3 + row,
                    23,
                    &line,
                    style(blend(INK, color, 25 + row * 4), BG, false),
                );
            }
        }
    }
    let content_bottom = h.saturating_sub(if w > 36 && h > 12 { 3 } else { 1 });
    if saver && w >= 28 && prompt + 1 < content_bottom {
        let hint = match &mascot {
            Mascot::Duck(_) => "Drag the duck to throw",
            Mascot::Flere(flere, now) => flere.hint(*now),
        };
        center(&mut c, prompt + 1, hint, MUTED, false);
    }
    match mascot {
        Mascot::Duck(duck) => draw_duck(&mut c, prompt.saturating_add(2), step, quiet, duck),
        Mascot::Flere(flere, now) => flere.draw(&mut c, prompt.saturating_add(2), step, now, quiet),
    }
    if saver {
        center(
            &mut c,
            h - 1,
            if w >= 40 {
                "SCREENSAVER · sessions still running"
            } else {
                "SCREENSAVER"
            },
            MUTED,
            false,
        );
    }
    c
}

// Walk below the prompt, keeping the wordmark, instructions and footer readable.
// The shared alpha sprite becomes normal half-block cells: no graphics protocol
// or external asset is necessary, including in remote terminals and screenshots.
fn duck_position(
    w: usize,
    h: usize,
    top: usize,
    step: usize,
) -> Option<(usize, usize, usize, usize, bool)> {
    mascot_position(w, h, top, step, false)
}
fn mascot_position(
    w: usize,
    h: usize,
    top: usize,
    step: usize,
    wide: bool,
) -> Option<(usize, usize, usize, usize, bool)> {
    let framed = w > 36 && h > 12;
    let gutter = if framed { 2 } else { 1 };
    let bottom = h.saturating_sub(if framed { 3 } else { 1 });
    let room = bottom.saturating_sub(top);
    let rows = room.saturating_sub(usize::from(room > 7)).min(10);
    let columns = (rows * 5 / if wide { 1 } else { 2 }).min(w.saturating_sub(gutter * 2));
    if rows == 0 || columns < 2 {
        return None;
    }
    let travel = w.saturating_sub(columns + gutter * 2);
    let across = (step / 2) % (travel * 2).max(1);
    let left = across > travel;
    let x = gutter + if left { travel * 2 - across } else { across };
    let bob = room - rows;
    let down = (step / 17) % (bob * 2).max(1);
    let y = top + if down > bob { bob * 2 - down } else { down };
    Some((x, y, columns, rows, left))
}
fn draw_duck(c: &mut Canvas, top: usize, step: usize, quiet: bool, duck: Option<&mut Duck>) {
    let Some((x, y, columns, rows, left)) = duck_position(c.width, c.height, top, step) else {
        if let Some(duck) = duck {
            duck.hidden();
        }
        return;
    };
    let pose = Pose {
        x: x as f32,
        y: y as f32,
        columns,
        rows,
        left,
    };
    let pose = duck.map_or(pose, |duck| duck.arrange(pose, (c.width, c.height), quiet));
    let (x, y, left) = (pose.x.round() as usize, pose.y.round() as usize, pose.left);
    let action = if quiet {
        crate::pet::Action::Sit
    } else {
        crate::pet::Action::Walk
    };
    let sprite = crate::pet::sprite(action, step as f32 * 0.1 / action.period(), left);
    let sample = |col: usize, py: usize| {
        // Area sampling keeps the beak and feet legible at narrow terminal sizes.
        let mut rgb = [0_u32; 3];
        let mut visible = false;
        for sy in 0..4 {
            for sx in 0..4 {
                // Walk/Sit occupy this fixed inset in the shared 64px canvas.
                // Discarding its transparent padding keeps the small duck readable.
                let px = 12 + (col * 4 + sx) * 40 / (columns * 4);
                let py = 16 + (py * 4 + sy) * 40 / (rows * 8);
                let pixel = sprite.pixels[py * crate::pet::WIDTH + px];
                visible |= pixel[3] != 0;
                for (channel, background) in [6_u32, 14, 22].into_iter().enumerate() {
                    rgb[channel] += (u32::from(pixel[channel]) * u32::from(pixel[3])
                        + background * (255 - u32::from(pixel[3])))
                        / 255;
                }
            }
        }
        (
            Color::Rgb(
                (rgb[0] / 16) as u8,
                (rgb[1] / 16) as u8,
                (rgb[2] / 16) as u8,
            ),
            visible,
        )
    };
    for row in 0..rows {
        for col in 0..columns {
            let (fg, a) = sample(col, row * 2);
            let (bg, b) = sample(col, row * 2 + 1);
            if !a && !b {
                continue;
            }
            c.text(x + col, y + row, 1, "▀", style(fg, bg, false));
        }
    }
}

// Swallow paste and pointer/focus reports, including split input sequences.
fn start_key(key: Key) -> Option<bool> {
    match key {
        Key::Bytes(bytes) if bytes == b"\x03" => Some(false),
        Key::Bytes(bytes) if bytes != b"\x1b[I" && bytes != b"\x1b[O" => Some(true),
        _ => None,
    }
}

pub(super) fn run(state: &Path) -> io::Result<bool> {
    let prefs = crate::workspace::load_preferences(state);
    let quiet = prefs.reduced_motion;
    let _raw = os::RawTerminal::enter(0)?;
    os::signals();
    let _display = DisplayGuard { remote: false };
    io::stdout().write_all(b"\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?2004h\x1b[2J")?;
    let start = Instant::now();
    let mut flere = Flere::new(start);
    let mut previous = Vec::new();
    let mut cursor = None;
    let mut input = Input::default();
    let mut last = None;
    while !os::stopping() {
        let (w, h) = os::dimensions(0);
        let step = if quiet {
            0
        } else {
            (start.elapsed().as_millis() / TICK.as_millis()) as usize
        };
        if last != Some((w, h, step)) {
            let mascot = match prefs.screensaver_mascot {
                crate::pet::ScreensaverMascot::Duck => Mascot::Duck(None),
                crate::pet::ScreensaverMascot::Flere => Mascot::Flere(&mut flere, Instant::now()),
            };
            let canvas = canvas_with_mascot(w as usize, h as usize, step, quiet, false, mascot);
            if last.is_some_and(|(old_w, old_h, _)| (old_w, old_h) != (w, h)) {
                previous.clear();
            }
            let output = display_frame(render(&canvas, &mut previous), None, &mut cursor, None);
            io::stdout().write_all(output.as_bytes())?;
            io::stdout().flush()?;
            last = Some((w, h, step));
        }
        let mut fds = [os::PollFd {
            fd: 0,
            events: os::READ,
            revents: 0,
        }];
        os::wait(&mut fds, 25)?;
        if fds[0].revents & (os::READ | os::HUP) != 0 {
            let mut bytes = [0; 8192];
            let count = io::stdin().read(&mut bytes)?;
            if count == 0 {
                return Ok(false);
            }
            for key in input.feed(&bytes[..count]) {
                if let Some(start) = start_key(key) {
                    return Ok(start);
                }
            }
        }
        for key in input.timeout() {
            if let Some(start) = start_key(key) {
                return Ok(start);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn canvas(width: usize, height: usize, step: usize, quiet: bool, saver: bool) -> Canvas {
        canvas_with_mascot(width, height, step, quiet, saver, Mascot::Duck(None))
    }
    fn draw(width: usize, height: usize, step: usize, quiet: bool) -> Canvas {
        let start = Instant::now();
        let mut flere = Flere::new(start);
        canvas_with_mascot(
            width,
            height,
            step,
            quiet,
            false,
            Mascot::Flere(&mut flere, start + TICK * step as u32),
        )
    }
    #[test]
    fn screensaver_duck_keeps_frame_and_return_prompt_readable_at_every_size() {
        for (w, h) in [
            (180, 42),
            (100, 30),
            (60, 24),
            (40, 20),
            (60, 19),
            (32, 10),
            (10, 8),
            (1, 1),
        ] {
            for step in [0, 34, 45, 99, 350, 999] {
                let frame = canvas(w, h, step, false, true);
                assert_eq!(frame.cells.len(), w * h);
                if w > 36 && h > 12 {
                    for x in 1..w - 1 {
                        assert!(
                            !frame.cells[2 * w + x].text.trim().is_empty(),
                            "intro erased top border at {w}x{h}, step {step}"
                        );
                        assert!(
                            !frame.cells[(h - 3) * w + x].text.trim().is_empty(),
                            "duck erased bottom border at {w}x{h}, step {step}"
                        );
                    }
                    for y in 2..h - 2 {
                        for x in [1, w - 2] {
                            assert!(!frame.cells[y * w + x].text.trim().is_empty());
                        }
                    }
                }
                if w >= 34 {
                    let text: String = frame.cells.iter().map(|c| c.text.as_str()).collect();
                    assert!(text.contains("Press any key to return"));
                }
            }
            assert_eq!(
                canvas(w, h, 0, true, true).cells,
                canvas(w, h, 999, true, true).cells
            );
        }
        let narrow = canvas(60, 24, 34, false, true);
        let duck_cells = narrow
            .cells
            .iter()
            .filter(|c| matches!(c.style.fg, Color::Rgb(r,g,b) if r > 180 && g > 100 && b < 130))
            .count();
        assert!(
            duck_cells >= 20,
            "narrow duck became too small to recognize"
        );
        assert_ne!(
            duck_position(180, 42, 32, 34),
            duck_position(180, 42, 32, 44)
        );
    }
    #[test]
    fn intro_resizes_animates_then_settles_and_reduced_motion_is_static() {
        for (w, h) in [(236, 54), (100, 30), (60, 24), (32, 10), (8, 4)] {
            let first = draw(w, h, 0, false);
            let ready = draw(w, h, 40, false);
            assert_eq!(ready.cells.len(), w * h);
            assert_ne!(first.cells, ready.cells);
            assert_eq!(draw(w, h, 0, true).cells, draw(w, h, 999, true).cells);
            assert_ne!(ready.cells, draw(w, h, 45, false).cells);
            if w >= 28 {
                let text: String = ready.cells.iter().map(|c| c.text.as_str()).collect();
                assert!(text.contains("Press any key to start"));
            }
        }
    }
    #[test]
    fn intro_consumes_split_paste_and_pointer_reports_without_starting() {
        let mut input = Input::default();
        for data in [
            b"\x1b[20".as_slice(),
            b"0~rm -rf not-a-command\r",
            b"\x1b[201~",
            b"\x1b[<0;12;4M",
            b"\x1b[I",
        ] {
            assert!(
                input
                    .feed(data)
                    .into_iter()
                    .all(|key| start_key(key).is_none())
            );
        }
        assert_eq!(start_key(Key::Bytes(b"\x03".to_vec())), Some(false));
        assert_eq!(start_key(Key::Bytes(b"\x1b[A".to_vec())), Some(true));
    }
}
