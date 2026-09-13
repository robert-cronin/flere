//! Standalone mascot animation workshop. It never attaches to a supervisor or starts a shell.
use flere::{
    os,
    pet::{self, Action},
};
use std::{
    fs,
    io::{self, Read, Write},
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = write!(io::stdout(), "\x1b[0m\x1b[?25h\x1b[?1049l");
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--dump") {
        let root = PathBuf::from(args.get(1).ok_or("--dump needs an output directory")?);
        fs::create_dir_all(&root)?;
        for kind in pet::Kind::ALL {
            for (action_index, action) in Action::ALL.iter().enumerate() {
                for frame in 0..24 {
                    let sprite = pet::sprite_kind(kind, *action, frame as f32 / 24.0, false);
                    let raster = pet::scene(&sprite, 128, 128, 0.0, 2.0);
                    let mut bytes =
                        format!("P6\n{} {}\n255\n", raster.width, raster.height).into_bytes();
                    bytes.extend(&raster.rgb);
                    fs::write(
                        root.join(format!("{}-{action_index}-{frame:02}.ppm", kind.label())),
                        bytes,
                    )?;
                }
                println!("{} / {}: 24 frames", kind.label(), action.label());
            }
        }
        return Ok(());
    }
    let pixels = args.iter().any(|a| a == "--sixel");
    let raw = os::RawTerminal::enter(0)?;
    write!(io::stdout(), "\x1b[?1049h\x1b[2J\x1b[?25l")?;
    let screen = Screen;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut byte = [0];
        while io::stdin().read_exact(&mut byte).is_ok() {
            if tx.send(byte[0]).is_err() {
                break;
            }
        }
    });
    let mut action = Action::Walk;
    let mut kind = pet::Kind::Duck;
    let mut clock = 0.0f32;
    let mut paused = false;
    let mut slow = false;
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.1);
        last = now;
        for key in rx.try_iter() {
            match key {
                b'q' | 3 | 27 => {
                    drop(screen);
                    drop(raw);
                    return Ok(());
                }
                b' ' => paused = !paused,
                b's' => slow = !slow,
                b'h' => kind = kind.cycle(-1),
                b'l' => kind = kind.cycle(1),
                b'1'..=b'8' => {
                    action = Action::ALL[(key - b'1') as usize];
                    clock = 0.0;
                }
                _ => {}
            }
        }
        if !paused {
            clock += dt * if slow { 0.25 } else { 1.0 };
        }
        let (cols, rows) = os::dimensions(0);
        let facing_left = action == Action::Walk && (clock * 0.6).sin() < 0.0;
        let sprite = pet::sprite_kind(kind, action, clock / action.period(), facing_left);
        let mut paint = format!(
            "\x1b[?2026h\x1b[H\x1b[38;2;67;227;247mFLERE / MASCOT WORKSHOP\x1b[0m\x1b[K\r\n1 walk  2 sit  3 sleep  4 look  5 alert  6 stretch  7 groom  8 pounce\x1b[K\r\nh/l pet / Space pause / s slow / q exit    {} / {}{}\x1b[K",
            kind.label(),
            action.label(),
            if slow { " / quarter speed" } else { "" }
        );
        if pixels && cols >= 36 && rows >= 14 {
            let x = if action == Action::Walk {
                10.0 + (1.0 - (clock * 0.6).cos()) * 80.0
            } else {
                80.0
            };
            let raster = pet::scene(&sprite, 350, 150, x, 1.6);
            paint.push_str("\x1b[5;2H");
            paint.push_str(&pet::encode(&raster)?);
        } else {
            let width = usize::from(cols.saturating_sub(4)).min(64);
            let height = usize::from(rows.saturating_sub(5)).min(18) * 2;
            if width > 0 && height > 0 {
                let scale =
                    (height as f32 / pet::HEIGHT as f32).min(width as f32 / pet::WIDTH as f32);
                let raster = pet::scene(
                    &sprite,
                    width,
                    height,
                    (width as f32 - pet::WIDTH as f32 * scale) / 2.0,
                    scale,
                );
                for y in (0..height).step_by(2) {
                    paint.push_str(&format!("\x1b[{};2H", 5 + y / 2));
                    for x in 0..width {
                        let a = (y * width + x) * 3;
                        let b = ((y + 1) * width + x) * 3;
                        paint.push_str(&format!(
                            "\x1b[38;2;{};{};{}m\x1b[48;2;{};{};{}m▀",
                            raster.rgb[a],
                            raster.rgb[a + 1],
                            raster.rgb[a + 2],
                            raster.rgb[b],
                            raster.rgb[b + 1],
                            raster.rgb[b + 2]
                        ));
                    }
                }
            }
        }
        paint.push_str("\x1b[0m\x1b[?2026l");
        io::stdout().write_all(paint.as_bytes())?;
        io::stdout().flush()?;
        std::thread::sleep(Duration::from_millis(40));
    }
}
