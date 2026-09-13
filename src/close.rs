//! Private, expiring close requests consumed by cooperating shells/editors.
//! A clean probe never authorizes a hangup: the program rechecks and exits itself.
use crate::{os, wire::invalid, workspace::atomic_write};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn directory(state: &Path, run: &str) -> PathBuf {
    state.join("close").join(run)
}

fn private_directory(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != os::uid() || m.mode() & 0o077 != 0 {
        return Err(invalid("close integration directory is not private"));
    }
    Ok(())
}

fn prepare(state: &Path, run: &str) -> io::Result<PathBuf> {
    private_directory(&state.join("close"))?;
    let dir = directory(state, run);
    private_directory(&dir)?;
    atomic_write(&dir.join("ready"), b"")?;
    atomic_write(&dir.join("reply"), b"")?;
    Ok(dir)
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub(crate) fn shell_command(state: &Path, run: &str, shell: &str) -> io::Result<Command> {
    let mut cmd = Command::new(shell);
    cmd.arg("-i");
    if Path::new(shell).file_name().and_then(|s| s.to_str()) == Some("bash") {
        let dir = prepare(state, run)?;
        atomic_write(&dir.join("shell.bash"), include_bytes!("close/shell.bash"))?;
        let mut command = Command::new(shell);
        command
            .arg("--rcfile")
            .arg(dir.join("shell.bash"))
            .arg("-i");
        return Ok(command);
    }
    if Path::new(shell).file_name().and_then(|s| s.to_str()) != Some("zsh") {
        return Ok(cmd);
    }
    let dir = prepare(state, run)?;
    let original = std::env::var_os("ZDOTDIR");
    let original_dir = original.clone().or_else(|| std::env::var_os("HOME"));
    let Some(original_dir) = original_dir else {
        return Ok(cmd);
    };
    let original_dir = quote(&original_dir.to_string_lossy());
    let private = quote(&dir.to_string_lossy());
    // Source each user startup file once; preserve a ZDOTDIR selected by .zshenv.
    atomic_write(&dir.join(".zshenv"), format!(
        "typeset -g _flere_close_dir={private}\nZDOTDIR={original_dir}\n[[ -r $ZDOTDIR/.zshenv ]] && source $ZDOTDIR/.zshenv\ntypeset -g _flere_user_zdotdir=${{ZDOTDIR:-$HOME}}\nZDOTDIR={private}\n"
    ).as_bytes())?;
    atomic_write(&dir.join(".zshrc"), format!(
        "ZDOTDIR=$_flere_user_zdotdir\n[[ -r $ZDOTDIR/.zshrc ]] && source $ZDOTDIR/.zshrc\n{}\nsource {private}/shell.zsh\n",
        if original.is_none() { "[[ $ZDOTDIR == $HOME ]] && unset ZDOTDIR" } else { ":" }
    ).as_bytes())?;
    atomic_write(&dir.join("shell.zsh"), include_bytes!("close/shell.zsh"))?;
    os::private_fifo(&dir.join("wake"))?;
    cmd.env("ZDOTDIR", &dir);
    Ok(cmd)
}

pub(crate) fn editor_command(state: &Path, run: &str, original: Command) -> io::Result<Command> {
    let name = Path::new(original.get_program())
        .file_name()
        .and_then(|s| s.to_str());
    if !matches!(name, Some("nvim" | "vim" | "nvimdiff" | "vimdiff")) {
        return Ok(original);
    }
    let dir = prepare(state, run)?;
    atomic_write(&dir.join("editor.vim"), include_bytes!("close/editor.vim"))?;
    atomic_write(&dir.join("editor.lua"), include_bytes!("close/editor.lua"))?;
    let path = dir.join("editor.vim").to_string_lossy().replace('\'', "''");
    let mut cmd = Command::new(original.get_program());
    cmd.args([
        "--cmd",
        &format!("execute 'source ' . fnameescape('{path}')"),
    ]);
    cmd.args(original.get_args());
    for (key, value) in original.get_envs() {
        if let Some(value) = value {
            cmd.env(key, value);
        } else {
            cmd.env_remove(key);
        }
    }
    if let Some(cwd) = original.get_current_dir() {
        cmd.current_dir(cwd);
    }
    Ok(cmd)
}

pub(crate) fn read(path: &Path) -> io::Result<Vec<u8>> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || m.uid() != os::uid() || m.mode() & 0o077 != 0 || m.len() > 16384 {
        return Err(invalid("invalid close integration response"));
    }
    let mut bytes = Vec::new();
    f.take(16385).read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(invalid("close response exceeds bound"));
    }
    Ok(bytes)
}

pub(crate) fn jump(
    state: &Path,
    run: &str,
    pid: u32,
    path: &Path,
    line: usize,
    column: usize,
) -> io::Result<()> {
    let dir = directory(state, run);
    let ready = read(&dir.join("jump-ready")).map_err(|_| {
        invalid("this editor predates file/line jumps; reopen it to enable integration")
    })?;
    if String::from_utf8_lossy(&ready).trim().parse::<u32>().ok() != Some(pid) {
        return Err(invalid(
            "this editor does not support safe file/line jumps; reopen it to enable integration",
        ));
    }
    let expires = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs()
        + 2;
    atomic_write(&dir.join("jump.json"), &serde_json::to_vec(&serde_json::json!({
        "token": os::nonce()?, "pid": pid, "path": path, "line": line, "column": column, "expires": expires,
    })).map_err(io::Error::other)?)
}

pub(crate) fn request(dir: &Path, token: &str, phase: &str, processes: &[u32]) -> io::Result<()> {
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_secs()
        + 2;
    let pids = processes
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    atomic_write(
        &dir.join("request"),
        format!("{token}\n{phase}\n{expiry}\n{pids}\n").as_bytes(),
    )?;
    if dir.join("wake").exists() {
        use std::io::Write;
        let mut f = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(dir.join("wake"))?;
        f.write_all(b"x")?;
    }
    Ok(())
}

#[derive(serde::Deserialize)]
pub(crate) struct Reply {
    pub token: String,
    pub phase: String,
    pub pid: u32,
    pub reason: String,
    pub processes: Vec<u32>,
    pub services: Vec<Vec<String>>,
}
