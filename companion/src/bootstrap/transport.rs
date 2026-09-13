use super::{Request, invalid};
use std::{
    io::{self, Read},
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

/// Only the embedded script is executable shell text. Every variable is one
/// quoted positional argument, including aliases, paths and package identities.
pub(super) fn script(script: &str, arguments: &[&str]) -> io::Result<String> {
    let mut command = format!("exec sh -c '{}' sh", script.replace('\'', "'\\''"));
    for argument in arguments {
        command.push(' ');
        if argument.is_empty() {
            command.push_str("''");
        } else {
            command.push_str(&crate::quote(argument)?);
        }
    }
    Ok(command)
}

pub(super) fn command(executable: &str, arguments: &[&str]) -> io::Result<String> {
    let mut command = format!("exec {}", crate::quote(executable)?);
    for argument in arguments {
        command.push(' ');
        command.push_str(&crate::quote(argument)?);
    }
    Ok(command)
}

/// Authentication remains OpenSSH's native terminal dialogue, before raw mode.
/// Stdout alone carries bounded machine data; stderr is never a terminal frame.
pub(super) fn run(
    request: &Request,
    phase: &str,
    command: &str,
    input: Option<Box<dyn Read + Send>>,
    maximum: usize,
) -> io::Result<Vec<u8>> {
    run_bounded(
        request,
        phase,
        command,
        input,
        maximum,
        Duration::from_secs(180),
    )
}

pub(super) fn run_bounded(
    request: &Request,
    phase: &str,
    command: &str,
    input: Option<Box<dyn Read + Send>>,
    maximum: usize,
    timeout: Duration,
) -> io::Result<Vec<u8>> {
    request.cancellation.check()?;
    let mut child = crate::Ssh(
        Command::new(&request.connection.ssh)
            .args(["-T", "--", &request.connection.host, command])
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| {
                invalid(&format!(
                    "Cannot start installed OpenSSH for {phase}: {error}"
                ))
            })?,
    );
    enum Event {
        Output(io::Result<Vec<u8>>),
        Input(io::Result<()>),
    }
    let (sender, receiver) = mpsc::sync_channel(2);
    let output_sender = sender.clone();
    let stdout = child.0.stdout.take().unwrap();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(maximum as u64 + 1)
            .read_to_end(&mut bytes)
            .and_then(|_| {
                if bytes.len() > maximum {
                    Err(invalid("SSH bootstrap response exceeds its bound"))
                } else {
                    Ok(bytes)
                }
            });
        let _ = output_sender.send(Event::Output(result));
    });
    let mut input_done = input.is_none();
    if let Some(mut input) = input {
        let mut stdin = child.0.stdin.take().unwrap();
        std::thread::spawn(move || {
            let result = io::copy(&mut input, &mut stdin).map(|_| ());
            drop(stdin);
            let _ = sender.send(Event::Input(result));
        });
    }
    let deadline = Instant::now() + timeout;
    let mut output = None;
    loop {
        request.cancellation.check()?;
        while let Ok(event) = receiver.try_recv() {
            match event {
                Event::Output(result) => output = Some(result?),
                Event::Input(result) => {
                    result?;
                    input_done = true;
                }
            }
        }
        if input_done
            && output.is_some()
            && let Some(status) = child.0.try_wait()?
        {
            return if status.success() {
                Ok(output.take().unwrap())
            } else {
                Err(invalid(&format!(
                    "SSH bootstrap {phase} failed for {} ({status}); check the OpenSSH or remote diagnostic above",
                    request.connection.host,
                )))
            };
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "SSH bootstrap {phase} timed out; inspect the selected host before retrying an installation",
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
