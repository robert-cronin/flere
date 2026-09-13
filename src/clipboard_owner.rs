//! Linux selection ownership outlives UI detach. A small instance of the same
//! executable owns only clipboard data, and exits when that selection is replaced.
use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
use std::{
    io::{self, BufRead, Write},
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};
const MARKER: &str = "application/x-flere-screenshot-owner";

pub fn publish(path: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("_clipboard")
        .arg(path)
        .env(
            "TMPDIR",
            path.parent()
                .ok_or_else(|| io::Error::other("invalid attachment path"))?,
        )
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let output = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = io::BufReader::new(output)
            .take(2048)
            .read_line(&mut line)
            .map(|_| line);
        let _ = send.send(result);
    });
    use std::io::Read;
    match receive.recv_timeout(Duration::from_secs(3)) {
        Ok(Ok(line)) if line.trim() == "copied" => {
            // Reap while attached; on detach the selection owner can finish independently.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
        result => {
            let _ = child.kill();
            let _ = child.wait();
            Err(io::Error::other(format!(
                "Linux clipboard unavailable: {result:?}"
            )))
        }
    }
}
pub fn serve(bytes: Vec<u8>, path: &Path) -> io::Result<()> {
    let text = path
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("clipboard path must be printable UTF-8"))?;
    let context = ClipboardContext::new().map_err(io::Error::other)?;
    context
        .set(vec![
            ClipboardContent::Other("image/png".into(), bytes.clone()),
            ClipboardContent::Text(text.into()),
            ClipboardContent::Other(MARKER.into(), text.as_bytes().to_vec()),
        ])
        .map_err(io::Error::other)?;
    if context.get_text().map_err(io::Error::other)? != text
        || context.get_buffer("image/png").map_err(io::Error::other)? != bytes
    {
        return Err(io::Error::other(
            "clipboard image and text could not be verified",
        ));
    }
    println!("copied");
    io::stdout().flush()?;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if context.get_buffer(MARKER).ok().as_deref() != Some(text.as_bytes()) {
            return Ok(());
        }
    }
}
