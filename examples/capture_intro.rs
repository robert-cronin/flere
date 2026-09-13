//! Capture actual intro PTY frames for a local animation; creates no supervisor.
use flere::{os, terminal::Terminal};
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsRawFd,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err("usage: capture_intro EXE STATE OUTPUT.jsonl COLS ROWS; use a disposable home-cache output".into());
    }
    let cols = args[3].to_string_lossy().parse::<u16>()?.clamp(8, 320);
    let rows = args[4].to_string_lossy().parse::<u16>()?.clamp(4, 106);
    let output = PathBuf::from(&args[2]);
    let mut frames = File::create(&output)?;
    let mut raw = File::create(output.with_extension("ansi"))?;
    let mut command = Command::new("/usr/bin/env");
    command.args(["-u", "FLERE"]).arg(&args[0]);
    command.arg("--state").arg(&args[1]).arg("intro");
    let (mut master, mut child) =
        os::spawn_command_pty(&std::env::current_dir()?, &mut command, cols, rows)?;
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut screen = Terminal::new(cols as usize, rows as usize);
        let start = Instant::now();
        let mut next = Duration::from_millis(80);
        let mut count = 0;
        let mut bytes = 0;
        while start.elapsed() < Duration::from_secs(6) {
            // Drain complete output bursts before sampling, so a frame is not
            // delayed by the kernel PTY's small individual read chunks.
            for _ in 0..64 {
                let mut data = [0; 65536];
                match master.read(&mut data) {
                    Ok(n) if n > 0 => {
                        screen.feed(&data[..n]);
                        raw.write_all(&data[..n])?;
                        bytes += n;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Ok(_) => break,
                    Err(e) => return Err(e.into()),
                }
            }
            if start.elapsed() >= next {
                let mut runs = Vec::new();
                for y in 0..rows as usize {
                    let row = &screen.grid.cells[y * cols as usize..(y + 1) * cols as usize];
                    let mut x = 0;
                    while x < row.len() {
                        let begin = x;
                        let style = row[x].style;
                        let mut text = String::new();
                        while x < row.len() && row[x].style == style {
                            text.push_str(&row[x].text);
                            x += 1;
                        }
                        runs.push((y, begin, text, style.fg, style.bg, style.bold));
                    }
                }
                serde_json::to_writer(
                    &mut frames,
                    &serde_json::json!({"ms":start.elapsed().as_millis(),"cols":cols,"rows":rows,"runs":runs}),
                )?;
                frames.write_all(b"\n")?;
                count += 1;
                next += Duration::from_millis(100);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let text = (0..rows as usize)
            .map(|y| screen.grid.line(y))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.contains("Press any key to start") {
            return Err("intro did not become ready".into());
        }
        std::fs::write(output.with_extension("txt"), text)?;
        println!(
            "Captured {count} intro frames at {cols}x{rows}; {bytes} terminal bytes in six seconds."
        );
        Ok(())
    })();
    os::hangup(master.as_raw_fd(), &mut child);
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        let mut data = [0; 65536];
        let _ = master.read(&mut data);
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= until {
            child.kill()?;
            child.wait()?;
            return Err("intro did not exit on hangup".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    result
}
