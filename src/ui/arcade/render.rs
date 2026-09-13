//! The arena is software-painted pixels packed into ordinary terminal half blocks.
//! It owns no graphics placements; screenshots use exactly these canvas cells.
use super::*;
use game::{HEIGHT, WIDTH};
type Rgb = [u8; 3];

pub(super) fn draw(a: &Arcade, c: &mut Canvas) {
    c.fill(0, 0, c.width, c.height, style(TEXT, BG, false));
    if c.width < 40 || c.height < 20 {
        c.text(
            1,
            1,
            c.width.saturating_sub(2),
            "CONTEXT RUINS",
            style(CYAN, BG, true),
        );
        c.text(
            1,
            3,
            c.width.saturating_sub(2),
            "Paused. Needs 40 x 20.",
            style(TEXT, BG, false),
        );
        c.text(
            1,
            5,
            c.width.saturating_sub(2),
            "Q / Esc: return",
            style(MUTED, BG, false),
        );
        return;
    }
    match a.game.phase {
        Phase::Picker => picker(a, c),
        Phase::Playing => {
            arena(a, c);
            if a.game.paused {
                panel(
                    c,
                    "PAUSED",
                    &[
                        "P / Enter: resume",
                        "R: checkpoint",
                        "S: screenshot   Q: return",
                    ],
                    CYAN,
                );
            }
        }
        Phase::Results => {
            arena(a, c);
            let score = format!("{} / 9 relics recovered", a.game.treasure());
            panel(
                c,
                if a.game.won {
                    "CONTEXT RESTORED"
                } else {
                    "OUT OF LIVES"
                },
                &[
                    &score,
                    if a.game.won {
                        "The agent would like some credit."
                    } else {
                        "The ruins accept another attempt."
                    },
                    "Enter: choose avatar",
                    "Q / Esc: return",
                ],
                if a.game.won { GREEN } else { AMBER },
            );
        }
    }
}
fn centered(c: &mut Canvas, y: usize, text: &str, color: Color, bold: bool) {
    let len = text.chars().count();
    let x = c.width.saturating_sub(len) / 2;
    c.text(
        x,
        y,
        c.width.saturating_sub(x),
        text,
        style(color, BG, bold),
    );
}
fn picker(a: &Arcade, c: &mut Canvas) {
    centered(c, 2, "C O N T E X T   R U I N S", CYAN, true);
    centered(c, 4, "Choose your unlikely archaeologist", TEXT, false);
    let wide = c.width >= 90;
    if !wide {
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            let selected = a.game.avatar == kind;
            c.text(
                3,
                7 + index * 2,
                c.width / 2 - 3,
                &format!(
                    "{} {} {}",
                    if selected { ">" } else { " " },
                    index + 1,
                    kind.label()
                ),
                style(if selected { CYAN } else { MUTED }, BG, selected),
            );
        }
        portrait(
            a,
            c,
            a.game.avatar,
            (c.width / 2 + 1, 6),
            (c.width / 2 - 4, 8),
            BG,
        );
        centered(c, c.height - 4, "1-4 / arrows: choose", MUTED, false);
        centered(c, c.height - 2, "Enter: explore   Q: return", CYAN, true);
        return;
    }
    let columns = 4;
    let rows = 1;
    let pitch = (c.width.saturating_sub(4) / columns).min(28);
    let height = (c.height - 12).min(16);
    let left = (c.width - pitch * columns) / 2;
    let top = 6;
    for (index, kind) in Kind::ALL.into_iter().enumerate() {
        let x = left + index * pitch;
        let y = top;
        let selected = a.game.avatar == kind;
        let color = if selected { CYAN } else { MUTED };
        let bg = if selected { SELECTED } else { PANEL };
        c.fill(x, y, pitch - 1, height - 1, style(TEXT, bg, false));
        c.border(x, y, pitch - 1, height - 1, color);
        portrait(
            a,
            c,
            kind,
            (x + 2, y + 1),
            (pitch - 5, height.saturating_sub(4)),
            bg,
        );
        c.text(
            x + 2,
            y + height - 3,
            pitch - 5,
            &format!(
                "{} {}{}",
                index + 1,
                kind.label(),
                if selected { " <" } else { "" }
            ),
            style(color, bg, selected),
        );
    }
    let footer = (top + rows * height + 1).min(c.height - 3);
    centered(c, footer, "1-4 / arrows: choose", MUTED, false);
    centered(c, c.height - 2, "Enter: explore   Q: return", CYAN, true);
}
fn portrait(
    a: &Arcade,
    c: &mut Canvas,
    kind: Kind,
    at: (usize, usize),
    size: (usize, usize),
    bg: Color,
) {
    let t = if a.quiet {
        0.
    } else {
        a.animation_ms as f32 / 1000.
    };
    let sprite = if kind == Kind::Flere {
        pet::flere::sprite(t, pet::flere::Mood::Approachable, 1., a.quiet)
    } else {
        pet::sprite_kind(kind, pet::Action::Look, t / 3., false)
    };
    let Color::Rgb(r, g, b) = bg else {
        return;
    };
    let (w, h) = size;
    if w == 0 || h == 0 {
        return;
    }
    let sample = |x: usize, y: usize| {
        let p = sprite.pixels[(y * pet::HEIGHT / (h * 2)).min(pet::HEIGHT - 1) * pet::WIDTH
            + (x * pet::WIDTH / w).min(pet::WIDTH - 1)];
        let alpha = u32::from(p[3]);
        let mix = |v: u8, base: u8| {
            ((u32::from(v) * alpha + u32::from(base) * (255 - alpha)) / 255) as u8
        };
        Color::Rgb(mix(p[0], r), mix(p[1], g), mix(p[2], b))
    };
    for y in 0..h {
        for x in 0..w {
            c.text(
                at.0 + x,
                at.1 + y,
                1,
                "▀",
                style(sample(x, y * 2), sample(x, y * 2 + 1), false),
            );
        }
    }
}
fn panel(c: &mut Canvas, title: &str, lines: &[&str], accent: Color) {
    let w = 38.min(c.width - 2);
    let h = lines.len() + 5;
    let x = (c.width - w) / 2;
    let y = (c.height - h) / 2;
    c.fill(x, y, w, h, style(TEXT, PANEL, false));
    c.border(x, y, w, h, accent);
    c.text(x + 2, y + 1, w - 4, title, style(accent, PANEL, true));
    for (n, line) in lines.iter().enumerate() {
        c.text(x + 2, y + 3 + n, w - 4, line, style(TEXT, PANEL, false));
    }
}

struct Raster {
    pixels: Vec<Rgb>,
    w: usize,
    h: usize,
    scale: f32,
    camera: f32,
}
impl Raster {
    fn new(w: usize, h: usize, scale: f32, camera: f32) -> Self {
        let mut pixels = Vec::with_capacity(w * h);
        for y in 0..h {
            for _ in 0..w {
                pixels.push([
                    8 + (y * 4 / h) as u8,
                    15 + (y * 6 / h) as u8,
                    26 + (y * 6 / h) as u8,
                ]);
            }
        }
        Self {
            pixels,
            w,
            h,
            scale,
            camera,
        }
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Rgb) {
        let x0 = ((x - self.camera) * self.scale).floor().max(0.) as usize;
        let y0 = (y * self.scale).floor().max(0.) as usize;
        let x1 = ((x + w - self.camera) * self.scale).ceil().max(0.) as usize;
        let y1 = ((y + h) * self.scale).ceil().max(0.) as usize;
        for row in y0.min(self.h)..y1.min(self.h) {
            for col in x0.min(self.w)..x1.min(self.w) {
                self.pixels[row * self.w + col] = color;
            }
        }
    }
    fn dot(&mut self, x: f32, y: f32, size: f32, color: Rgb) {
        self.rect(x - size / 2., y - size / 2., size, size, color);
    }
    fn diamond(&mut self, x: f32, y: f32, r: f32, color: Rgb) {
        for i in 0..5 {
            let yy = (i as f32 - 2.) * r / 3.;
            let span = r - yy.abs();
            self.rect(x - span, y + yy, span * 2., r / 3., color);
        }
    }
    fn key(&mut self, x: f32, y: f32, color: Rgb) {
        self.rect(x - 0.5, y - 0.4, 0.75, 0.75, color);
        self.rect(x - 0.3, y - 0.2, 0.35, 0.35, [20, 29, 37]);
        self.rect(x + 0.1, y - 0.1, 0.8, 0.2, color);
        self.rect(x + 0.65, y, 0.25, 0.4, color);
    }
    /// Flere's casing is a horizontal white disc, with fields outside it. Sample
    /// the ellipse at pixel centres so a tiny downscale cannot turn it into a face.
    fn flere(&mut self, cx: f32, cy: f32, field: Rgb, time: f32) {
        let left = ((cx - 1.6 - self.camera) * self.scale).floor().max(0.) as usize;
        let right = ((cx + 1.6 - self.camera) * self.scale).ceil().max(0.) as usize;
        let top = ((cy - 0.95) * self.scale).floor().max(0.) as usize;
        let bottom = ((cy + 0.95) * self.scale).ceil().max(0.) as usize;
        for py in top.min(self.h)..bottom.min(self.h) {
            for px in left.min(self.w)..right.min(self.w) {
                let dx = (px as f32 + 0.5) / self.scale + self.camera - cx;
                let dy = (py as f32 + 0.5) / self.scale - cy + dx * 0.10;
                let radius = ((dx / 1.1).powi(2) + (dy / 0.48).powi(2)).sqrt();
                let index = py * self.w + px;
                let ripple = 0.03 * (time * 2. + dx * 4.).sin();
                if (radius - 1.27 - ripple).abs() < 0.20 {
                    self.pixels[index] = field;
                }
                let underside = (dx / 1.1).powi(2) + ((dy - 0.14) / 0.48).powi(2);
                if underside <= 1.0 {
                    self.pixels[index] = [159, 178, 173];
                }
                if radius <= 1.0 {
                    let shade = (dy * 20.).clamp(-10., 10.);
                    let rim = if radius > 0.78 { 8. } else { 0. };
                    self.pixels[index] = [
                        (248. - shade - rim) as u8,
                        (249. - shade - rim) as u8,
                        (239. - shade - rim) as u8,
                    ];
                }
            }
        }
    }
    fn sprite(
        &mut self,
        x: f32,
        y: f32,
        pattern: &[&str],
        palette: &[Rgb],
        flip: bool,
        scale: f32,
    ) {
        for (row, line) in pattern.iter().enumerate() {
            let width = line.len();
            for (col, b) in line.bytes().enumerate() {
                if b == b'.' {
                    continue;
                }
                let col = if flip { width - 1 - col } else { col };
                self.rect(
                    x + col as f32 * scale,
                    y + row as f32 * scale,
                    scale,
                    scale,
                    palette[(b - b'0') as usize],
                );
            }
        }
    }
}
fn arena(a: &Arcade, c: &mut Canvas) {
    let g = &a.game;
    let room = g.room();
    let title = if c.width >= 65 {
        format!("CONTEXT RUINS  /  {}", room.name)
    } else {
        format!("{} / 3  {}", g.room + 1, room.name)
    };
    c.text(2, 0, c.width - 4, &title, style(CYAN, BG, true));
    let keys = g.keys.iter().filter(|&&k| k).count();
    c.text(
        2,
        1,
        c.width - 4,
        &if c.width >= 60 {
            format!(
                "{}   {} lives   {} keys   {}/9 relics",
                g.avatar.label(),
                g.lives,
                keys,
                g.treasure()
            )
        } else {
            format!(
                "{}  {} lives  {} keys  {}/9",
                g.avatar.label(),
                g.lives,
                keys,
                g.treasure()
            )
        },
        style(TEXT, BG, false),
    );
    let available_w = c.width - 4;
    let available_h = (c.height - 6) * 2;
    let scale = (available_h as f32 / HEIGHT).min(available_w as f32 / 24.);
    let w = available_w.min((WIDTH * scale).ceil() as usize);
    let h = available_h;
    let view = w as f32 / scale;
    let camera = (g.x - view / 2.).clamp(0., (WIDTH - view).max(0.));
    let mut r = Raster::new(w, h, scale, camera);
    let time = if a.quiet || g.paused {
        0.
    } else {
        a.animation_ms as f32 / 1000.
    };
    // Receding columns, masonry and tiny motes sit behind the readable play field.
    for column in [3., 12., 21., 30., 39.] {
        r.rect(column, 2., 2.2, 16., [17, 30, 43]);
        r.rect(column - 0.5, 2., 3.2, 0.7, [26, 43, 54]);
        for y in [5., 8., 11., 14., 17.] {
            r.rect(column, y, 2.2, 0.12, [10, 22, 35]);
        }
    }
    for n in 0..25 {
        let x = ((n * 17 + 7) % 40) as f32;
        let y = ((n * 11 + 3) % 17) as f32 + (time * 0.18 + n as f32).sin() * 0.12;
        r.dot(x, y, 0.13, [34, 57, 69]);
    }
    for &(start, end) in room.floors {
        r.rect(start, 18., end - start, 2., [37, 53, 66]);
        r.rect(start, 18., end - start, 0.20, room.color);
        for y in [18.8, 19.8] {
            r.rect(start, y, end - start, 0.08, [17, 28, 39]);
        }
        for x in (start as usize..end as usize).step_by(3) {
            r.rect(x as f32, 18.2, 0.1, 0.6, [16, 29, 37]);
            r.rect(x as f32 + 1.5, 18.9, 0.1, 0.8, [16, 29, 37]);
        }
    }
    for x in 0..40 {
        if !room
            .floors
            .iter()
            .any(|&(left, right)| x as f32 >= left && (x as f32) < right)
        {
            r.rect(x as f32, 19.2, 1., 0.8, [99, 40, 47]);
            r.dot(
                x as f32 + 0.4,
                19.15 + (time * 3. + x as f32).sin() * 0.15,
                0.45,
                [234, 103, 79],
            );
        }
    }
    for p in room.platforms {
        r.rect(p.x, p.y, p.w, p.h, [58, 76, 85]);
        r.rect(p.x, p.y, p.w, 0.18, room.color);
        for n in 0..p.w as usize {
            if n % 3 == 0 {
                r.rect(p.x + n as f32, p.y + 0.2, 0.12, p.h - 0.2, [22, 39, 49]);
            }
        }
    }
    for l in room.ladders {
        let width = l.w.max(3. / scale);
        let left = l.x + (l.w - width) / 2.;
        r.rect(left, l.y, 0.10, l.h, [179, 141, 81]);
        r.rect(left + width, l.y, 0.10, l.h, [179, 141, 81]);
        let mut y = l.y + 0.4;
        while y < l.y + l.h {
            r.rect(left, y, width, 0.10, [217, 177, 106]);
            y += 0.85f32.max(2.8 / scale);
        }
    }
    let door = if g.keys[g.room] {
        [111, 235, 190]
    } else {
        [243, 191, 88]
    };
    r.rect(36.9, 14.7, 2.9, 3.3, [47, 66, 75]);
    r.rect(37.3, 14.1, 2.1, 0.6, [71, 91, 96]);
    r.rect(37.4, 14.8, 1.8, 3.2, [8, 18, 28]);
    if !g.keys[g.room] {
        for x in [37.6, 38.2, 38.8] {
            r.rect(x, 15., 0.14, 3., door);
        }
        r.key(38.1, 16.2, door);
    } else {
        r.rect(37.4, 14.8, 0.18, 3.2, door);
        r.rect(39.1, 14.8, 0.18, 3.2, door);
        r.diamond(38.3, 16.1, 0.35, door);
    }
    if !g.keys[g.room] {
        let (x, y) = room.key;
        r.key(x, y + (time * 3.).sin() * 0.08, [255, 211, 104]);
    }
    for (n, &(x, y)) in room.gems.iter().enumerate() {
        if g.gems[g.room] & (1 << n) == 0 {
            r.diamond(
                x,
                y + (time * 2. + n as f32).sin() * 0.12,
                0.35,
                [98, 214, 255],
            );
            r.dot(x - 0.1, y - 0.15, 0.15, [220, 250, 255]);
        }
    }
    // Lanterns are a visible checkpoint, not a hidden respawn rule.
    let cp = if g.room == 0 {
        (2., 18.)
    } else {
        room.checkpoint
    };
    r.rect(cp.0 - 0.6, cp.1 - 2.2, 0.1, 2.2, [117, 127, 125]);
    r.rect(
        cp.0 - 0.6,
        cp.1 - 2.2,
        0.9,
        0.55,
        if g.checkpoint.1 == cp.0 {
            [106, 231, 195]
        } else {
            [82, 114, 119]
        },
    );
    for (x, y) in [(3., 7.), (20., 4.), (36., 11.)] {
        r.rect(x, y, 0.5, 0.9, [113, 87, 58]);
        r.diamond(
            x + 0.25,
            y - 0.2,
            0.3 + (time * 8.).sin() * 0.06,
            [255, 171, 91],
        );
    }
    let hazard = g.hazard();
    r.sprite(
        hazard.x - 0.25,
        hazard.y - 0.1,
        &["..11..", ".1221.", "132231", "..11..", "1....1"],
        &[[0, 0, 0], [174, 62, 96], [246, 130, 148], [255, 221, 173]],
        ((g.elapsed * 8.) as usize).is_multiple_of(2),
        0.25,
    );
    // Compact hand-pixelled avatars remain distinct at the following camera's
    // smallest scale. The collision body is shared by all four mascots.
    let bob = if g.grounded && g.vx != 0. {
        (g.elapsed * 18.).sin() * 0.06
    } else {
        0.
    };
    let x = g.x - 0.8;
    let y = g.feet - 1.95 + bob;
    if a.quiet || g.dying <= 0. || ((g.elapsed * 12.) as usize).is_multiple_of(2) {
        if g.invulnerable > 0. {
            r.rect(x - 0.15, y - 0.15, 1.9, 0.12, [186, 226, 231]);
        }
        let moving = g.vx != 0. || g.climbing;
        let step = moving && ((g.elapsed * 10.) as usize).is_multiple_of(2);
        let feet = if step { ".11..11." } else { "..1..1.." };
        match g.avatar {
            Kind::Duck => r.sprite(
                x,
                y,
                &[
                    "...111..", "..1221..", "..121133", "..1111..", ".111111.", "1111111.",
                    ".11111..", feet,
                ],
                &[[0, 0, 0], [247, 211, 94], [58, 48, 41], [247, 142, 66]],
                g.facing < 0,
                0.24,
            ),
            Kind::Robot => r.sprite(
                x,
                y,
                &[
                    "...11...", ".111111.", ".122221.", ".121121.", "..1111..", ".133331.",
                    ".133331.", feet,
                ],
                &[[0, 0, 0], [122, 201, 231], [31, 57, 82], [216, 224, 222]],
                g.facing < 0,
                0.24,
            ),
            Kind::Cat => r.sprite(
                x,
                y,
                &[
                    ".1....1.", ".11..11.", ".111111.", ".121121.", "..1331..", "..11111.",
                    "..111111", feet,
                ],
                &[[0, 0, 0], [198, 165, 240], [245, 233, 169], [108, 72, 139]],
                g.facing < 0,
                0.24,
            ),
            Kind::Flere => {
                let mood = if g.dying > 0. {
                    [246, 141, 156]
                } else if g.feedback_left > 0. && g.keys[g.room] {
                    [239, 167, 84]
                } else {
                    [190, 215, 106]
                };
                r.flere(g.x, g.feet - 1.0 + bob, mood, time);
            }
        }
    }
    if g.transition > 0. && !a.quiet {
        r.rect(camera, 0., w as f32 / scale, 0.1, room.color);
        r.rect(camera, HEIGHT - 0.1, w as f32 / scale, 0.1, room.color);
    }
    let left = (c.width - w) / 2;
    let top = 3;
    for y in 0..h / 2 {
        for x in 0..w {
            let [r1, g1, b1] = r.pixels[(y * 2) * w + x];
            let [r2, g2, b2] = r.pixels[(y * 2 + 1) * w + x];
            c.text(
                left + x,
                top + y,
                1,
                "▀",
                style(Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2), false),
            );
        }
    }
    let hud = if c.width >= 75 {
        "← → / h l move   ↑ ↓ / k j climb   Space jump   P pause   R retry   S shot"
    } else {
        "←→ move ↑↓ climb Space jump P pause"
    };
    c.text(2, c.height - 2, c.width - 4, hud, style(CYAN, BG, false));
    let notice = if g.feedback_left > 0. {
        g.feedback
    } else if c.width >= 60 {
        "Find the key. Reach the lit door. Keep the relics."
    } else {
        "R retry   S screenshot"
    };
    c.text(
        2,
        c.height - 1,
        c.width - 4,
        notice,
        style(if g.dying > 0. { RED } else { MUTED }, BG, false),
    );
}
