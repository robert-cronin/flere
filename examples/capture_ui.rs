//! Development diagnostic: capture an actual Flere UI PTY as SVG and text.
//! Uses the existing supervisor and creates no shell workspace or model session.
use flere::{
    os,
    terminal::{Color, Terminal},
};
use std::{
    fs,
    io::{Read, Write},
    os::fd::AsRawFd,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn color(c: Color, background: bool) -> String {
    match c {
        Color::Default => if background { "#0c0c0c" } else { "#cccccc" }.into(),
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Index(n) => {
            let base = [
                "#242a35", "#e5757e", "#91c68e", "#e3c78a", "#83a8de", "#bb9fd9", "#83c5cc",
                "#d4d9e1", "#67778a", "#fa8e95", "#ade0a8", "#f1df9d", "#a3c7ef", "#d6b9ee",
                "#a6e6ec", "#ffffff",
            ];
            if n < 16 {
                base[n as usize].into()
            } else if n < 232 {
                let k = n - 16;
                let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
                format!(
                    "#{:02x}{:02x}{:02x}",
                    level(k / 36),
                    level(k / 6 % 6),
                    level(k % 6)
                )
            } else {
                let l = 8 + (n - 232) * 10;
                format!("#{l:02x}{l:02x}{l:02x}")
            }
        }
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !matches!(args.len(), 3 | 6) {
        return Err("usage: capture_ui FLERE_EXE STATE OUTPUT.svg [COLS ROWS UI_KEYS_HEX]; use disposable states for scripted keys".into());
    }
    let cols = args
        .get(3)
        .map(|s| s.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(140)
        .clamp(20, 320);
    let rows = args
        .get(4)
        .map(|s| s.to_string_lossy().parse::<usize>())
        .transpose()?
        .unwrap_or(38)
        .clamp(8, 106);
    let keys = args
        .get(5)
        .map(|s| flere::wire::unhex(&s.to_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let mut keys_sent = false;
    let cwd = std::env::current_dir()?;
    // This diagnostic PTY represents the external user's terminal.
    let mut command = Command::new("/usr/bin/env");
    command.args(["-u", "FLERE"]).arg(&args[0]);
    command.arg("--state").arg(&args[1]).arg("attach");
    let (mut master, mut child) =
        os::spawn_command_pty(&cwd, &mut command, cols as u16, rows as u16)?;
    let mut screen = Terminal::new(cols, rows);
    let mut raw = Vec::new();
    let start = Instant::now();
    let end = start + Duration::from_secs(2);
    while Instant::now() < end {
        if !keys_sent && start.elapsed() > Duration::from_millis(500) {
            master.write_all(&keys)?;
            keys_sent = true;
        }
        let mut b = [0; 65536];
        match master.read(&mut b) {
            Ok(n) if n > 0 => {
                screen.feed(&b[..n]);
                raw.extend_from_slice(&b[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(_) => {}
            Err(e) => return Err(e.into()),
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let text = (0..screen.grid.rows)
        .map(|y| screen.grid.line(y))
        .collect::<Vec<_>>()
        .join("\n");
    if !text.contains("FLERE") {
        return Err(format!("UI did not become ready: {text}").into());
    }
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"#0f141d\"/><g font-family=\"DejaVu Sans Mono,monospace\" font-size=\"16\">",
        cols * 10,
        rows * 22,
        cols * 10,
        rows * 22
    );
    for y in 0..rows {
        for x in 0..cols {
            let c = &screen.grid.cells[y * cols + x];
            let fg = if c.style.fg == Color::Default {
                flere::terminal::DEFAULT_FG
            } else {
                c.style.fg
            };
            let bg = if c.style.bg == Color::Default {
                flere::terminal::DEFAULT_BG
            } else {
                c.style.bg
            };
            let (fg, bg) = if c.style.inverse { (bg, fg) } else { (fg, bg) };
            svg.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"10\" height=\"22\" fill=\"{}\"/>",
                x * 10,
                y * 22,
                color(bg, true)
            ));
            if c.width > 0 && !c.text.trim().is_empty() {
                let attributes = format!(
                    "{}{}{}{}",
                    if c.style.bold {
                        " font-weight=\"bold\""
                    } else {
                        ""
                    },
                    if c.style.dim { " opacity=\"0.5\"" } else { "" },
                    if c.style.italic {
                        " font-style=\"italic\""
                    } else {
                        ""
                    },
                    match (c.style.underline, c.style.strikethrough) {
                        (true, true) => " text-decoration=\"underline line-through\"",
                        (true, false) => " text-decoration=\"underline\"",
                        (false, true) => " text-decoration=\"line-through\"",
                        _ => "",
                    }
                );
                svg.push_str(&format!(
                    "<text x=\"{}\" y=\"{}\" fill=\"{}\"{}>{}</text>",
                    x * 10,
                    y * 22 + 17,
                    color(fg, false),
                    attributes,
                    escape(&c.text)
                ));
            }
        }
    }
    svg.push_str("</g></svg>");
    let output = PathBuf::from(&args[2]);
    fs::write(&output, svg)?;
    fs::write(output.with_extension("txt"), text)?;
    fs::write(output.with_extension("ansi"), raw)?;
    // Detach our own UI irrespective of which overlay is visible; no key reaches a shell.
    os::hangup(master.as_raw_fd(), &mut child);
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        let mut b = [0; 65536];
        let _ = master.read(&mut b);
        if child.try_wait()?.is_some() {
            println!(
                "Captured actual UI; detached; shells preserved: {}",
                output.display()
            );
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err("UI did not detach".into())
}
