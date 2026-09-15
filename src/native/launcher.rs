//! Card-local command resolution. No user shell files or global Codex config are edited.
use super::*;
use std::{io::IsTerminal, os::unix::fs::PermissionsExt};

pub(crate) fn directory(state: &Path, shell_run: &str) -> PathBuf {
    crate::close::directory(state, shell_run).join("bin")
}

pub(crate) fn prepare(state: &Path, shell_run: &str) -> io::Result<PathBuf> {
    let dir = directory(state, shell_run);
    crate::close::private_directory(&dir)?;
    let quote = |s: &Path| format!("'{}'", s.to_string_lossy().replace('\'', "'\\''"));
    let script = format!(
        "#!/bin/sh\nexec {} --state {} _shell-codex '{}' \"$@\"\n",
        quote(&crate::os::executable_path()?),
        quote(state),
        shell_run
    );
    let path = dir.join("codex");
    crate::workspace::atomic_write(&path, script.as_bytes())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// Unknown options pass through to Codex, which owns its command-line grammar.
/// Only confidently interactive invocations register a native tab.
fn interactive(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let name = arg.split('=').next().unwrap_or(arg);
        match name {
            "--help" | "-h" | "--version" | "-V" => return false,
            "-c" | "--config" | "-m" | "--model" | "-p" | "--profile" | "-s" | "--sandbox"
            | "-a" | "--ask-for-approval" | "-C" | "--cd" | "-i" | "--image" | "--enable"
            | "--disable" | "--add-dir" | "--local-provider" => {
                if name == arg {
                    i += 1;
                }
            }
            "--oss"
            | "--search"
            | "--no-alt-screen"
            | "--full-auto"
            | "--dangerously-bypass-approvals-and-sandbox" => {}
            "--" => return true,
            "resume" | "fork" => {
                return !args[i + 1..]
                    .iter()
                    .any(|a| matches!(a.as_str(), "--help" | "-h"));
            }
            "queue" | "exec" | "e" | "review" | "login" | "logout" | "mcp" | "mcp-server"
            | "app" | "app-server" | "completion" | "sandbox" | "debug" | "apply" | "a"
            | "cloud" | "features" | "help" => return false,
            _ if arg.starts_with('-') => return false,
            _ => return true, // Positional initial prompt.
        }
        i += 1;
    }
    true
}

/// Resolve only explicit directory options before a positional prompt. Codex still
/// receives the original argv and performs its own argument validation.
pub(crate) fn working_directory(args: &[String], base: &Path) -> io::Result<PathBuf> {
    let mut directory = base.to_path_buf();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let (name, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(k, v)| (k, Some(v)));
        match name {
            "-C" | "--cd" => {
                let value = match value {
                    Some(v) => v,
                    None => {
                        i += 1;
                        args.get(i).ok_or_else(|| {
                            crate::wire::invalid("Codex directory option requires a value")
                        })?
                    }
                };
                directory = base.join(value);
            }
            "-c" | "--config" | "-m" | "--model" | "-p" | "--profile" | "-s" | "--sandbox"
            | "-a" | "--ask-for-approval" | "-i" | "--image" | "--enable" | "--disable"
            | "--add-dir" | "--local-provider" => {
                if value.is_none() {
                    i += 1;
                }
            }
            "resume" | "fork" => {}
            "--" => break,
            _ if name.starts_with('-') => {}
            _ => break,
        }
        i += 1;
    }
    directory.canonicalize()
}

pub fn run(state: &Path, shell_run: &str, args: &[String]) -> io::Result<()> {
    if shell_run.len() != 32 || !shell_run.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(crate::wire::invalid("invalid shell launcher identity"));
    }
    // Remove our private launcher before resolving/spawning the actual executable.
    // Native subprocesses cannot accidentally recursively launch through this wrapper.
    let dir = directory(state, shell_run);
    let search: Vec<_> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| p != &dir)
        .collect();
    let executable = search
        .iter()
        .map(|p| p.join("codex"))
        .find(|p| p.is_file() && fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "codex executable not found on PATH",
            )
        })?;
    let executable = std::path::absolute(executable)?;
    let path = std::env::join_paths(search).map_err(io::Error::other)?;
    let mut command = Command::new(&executable);
    command.env("PATH", &path);
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() || !interactive(args) {
        command.args(args);
        return Err(command.exec());
    }
    let pid = std::process::id();
    let start = crate::os::child_identity(pid)?.1;
    let scope: serde_json::Value = serde_json::from_slice(&crate::wire::request(
        state,
        &["shell-context", shell_run, &pid.to_string(), &start],
    )?)
    .map_err(io::Error::other)?;
    let argv: Vec<_> = std::iter::once(executable.to_string_lossy().into_owned())
        .chain(args.iter().cloned())
        .collect();
    let spec: HostSpec = serde_json::from_slice(&crate::wire::request(
        state,
        &[
            "shell-native",
            scope["epoch"]
                .as_str()
                .ok_or_else(|| crate::wire::invalid("missing shell epoch"))?,
            &scope["session"]
                .as_u64()
                .ok_or_else(|| crate::wire::invalid("missing shell session"))?
                .to_string(),
            scope["run"]
                .as_str()
                .ok_or_else(|| crate::wire::invalid("missing shell run"))?,
            shell_run,
            &pid.to_string(),
            &start,
            &crate::wire::hex(&serde_json::to_vec(&argv).map_err(io::Error::other)?),
        ],
    )?)
    .map_err(io::Error::other)?;
    command.args(&spec.argv[1..]);
    crate::os::clean_environment(&mut command);
    command
        .env("FLERE_STATE", state)
        .env("FLERE_SESSION", spec.id.to_string())
        .env("FLERE_RUN", &spec.run);
    crate::os::host_signals();
    let result = command.spawn().and_then(|mut child| child.wait());
    let _ = crate::wire::request(state, &["native-ended", &spec.id.to_string(), &spec.run]);
    match result {
        Ok(status) => {
            use std::os::unix::process::ExitStatusExt;
            std::process::exit(
                status
                    .code()
                    .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)),
            );
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_cli_grammar_is_preserved() {
        for args in [
            vec![],
            vec!["resume"],
            vec!["--model", "example", "resume", "--last"],
            vec!["-c", "key=\"exec\"", "a prompt"],
            vec!["fork"],
        ] {
            assert!(interactive(
                &args.into_iter().map(String::from).collect::<Vec<_>>()
            ));
        }
        for args in [
            vec!["--version"],
            vec!["resume", "--help"],
            vec!["exec", "resume", "--last"],
            vec!["-m", "example", "mcp", "list"],
            vec!["--unknown", "exec"],
            vec!["login", "status"],
        ] {
            assert!(!interactive(
                &args.into_iter().map(String::from).collect::<Vec<_>>()
            ));
        }
    }
}
