//! Bounded read-only Git inspection. UI runs this on a worker thread, never the PTY loop.
pub mod branch;
pub mod history;
pub mod versions;
use std::{
    io::{self, Read},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Change {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct GitView {
    pub branch: String,
    pub root: std::path::PathBuf,
    pub primary: bool,
    pub changes: Vec<Change>,
    pub comparison: Option<branch::BranchDiff>,
    pub comparison_error: String,
    pub error: String,
}
pub fn output(cwd: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    output_for(cwd, args, Duration::from_secs(3))
}
fn output_for(cwd: &Path, args: &[&str], timeout: Duration) -> io::Result<Vec<u8>> {
    run_output(&mut read_command(cwd, args), timeout)
}
/// A card's checkout identity belongs to its directory, even when Flere was
/// started from a Git hook that exported another repository's selectors.
pub(crate) fn checkout_identity_output(cwd: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let mut command = read_command(cwd, args);
    for key in ["GIT_DIR", "GIT_WORK_TREE", "GIT_COMMON_DIR"] {
        command.env_remove(key);
    }
    run_output(&mut command, Duration::from_secs(3))
}
fn read_command(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--no-pager")
        .arg("--literal-pathspecs")
        .args(["-c", "color.ui=false"])
        .env("GIT_NO_LAZY_FETCH", "1")
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}
fn run_output(command: &mut Command, timeout: Duration) -> io::Result<Vec<u8>> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    crate::os::nonblock(std::os::fd::AsRawFd::as_raw_fd(&stdout))?;
    crate::os::nonblock(std::os::fd::AsRawFd::as_raw_fd(&stderr))?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let until = Instant::now() + timeout;
    let mut eof = [false; 2];
    let mut status = None;
    loop {
        for (index, (reader, data)) in [
            (&mut stdout as &mut dyn Read, &mut out),
            (&mut stderr as &mut dyn Read, &mut err),
        ]
        .into_iter()
        .enumerate()
        {
            if eof[index] {
                continue;
            }
            let mut buf = [0u8; 8192];
            for _ in 0..32 {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        eof[index] = true;
                        break;
                    }
                    Ok(n) => {
                        if data.len() + n > 512 * 1024 {
                            let _ = child.kill();
                            let _ = child.wait();
                            return Err(crate::wire::invalid(
                                "Git output exceeds 512 KiB; narrow the repository view",
                            ));
                        }
                        data.extend_from_slice(&buf[..n]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(e);
                    }
                }
            }
        }
        if status.is_none() {
            status = child.try_wait()?;
        }
        // A reaped Git process can leave a short-lived descendant holding a pipe.
        // Readiness/EAGAIN is not EOF; retain the deadline and drain both streams.
        if let Some(status) = status
            && eof.iter().all(|v| *v)
        {
            return if status.success() {
                Ok(out)
            } else {
                Err(io::Error::other(crate::wire::passive(
                    &String::from_utf8_lossy(&err),
                )))
            };
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Git inspection timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
/// The first entry in Git's worktree list is its main checkout, even from a linked worktree.
pub fn primary_root(directory: &Path) -> io::Result<std::path::PathBuf> {
    let bytes = output(directory, &["worktree", "list", "--porcelain", "-z"])?;
    let mut fields = bytes.split(|b| *b == 0);
    let first = fields
        .next()
        .and_then(|f| f.strip_prefix(b"worktree "))
        .ok_or_else(|| crate::wire::invalid("choose an existing Git checkout"))?;
    if fields.take_while(|f| !f.is_empty()).any(|f| f == b"bare") {
        return Err(crate::wire::invalid(
            "choose a checked-out Git project, not a bare repository",
        ));
    }
    let path = std::str::from_utf8(first).map_err(io::Error::other)?;
    let root = std::fs::canonicalize(path)?;
    if !root.is_dir() {
        return Err(crate::wire::invalid(
            "main project directory is unavailable",
        ));
    }
    Ok(root)
}
pub fn inspect(cwd: &Path) -> GitView {
    match output(
        cwd,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--untracked-files=all",
        ],
    ) {
        Ok(bytes) => {
            let mut view = parse(&bytes);
            match output(cwd, &["rev-parse", "--show-toplevel"])
                .and_then(|b| String::from_utf8(b).map_err(io::Error::other))
            {
                Ok(root) => {
                    view.root = std::path::PathBuf::from(root.strip_suffix('\n').unwrap_or(&root))
                }
                Err(e) => view.error = e.to_string(),
            }
            view.primary = primary_root(&view.root).is_ok_and(|root| root == view.root);
            if view.error.is_empty() {
                match branch::inspect(&view.root) {
                    Ok(comparison) => view.comparison = comparison,
                    Err(e) => view.comparison_error = crate::wire::passive(&e.to_string()),
                }
            }
            view
        }
        Err(e) => GitView {
            error: e.to_string(),
            ..GitView::default()
        },
    }
}
pub fn parse(bytes: &[u8]) -> GitView {
    let mut view = GitView::default();
    let mut records = bytes.split(|b| *b == 0);
    while let Some(raw) = records.next() {
        if raw.starts_with(b"## ") {
            view.branch = String::from_utf8_lossy(&raw[3..]).into();
        } else if raw.len() >= 4 {
            let old_path = if raw[..2].iter().any(|c| *c == b'R' || *c == b'C') {
                records
                    .next()
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            } else {
                None
            };
            view.changes.push(Change {
                status: String::from_utf8_lossy(&raw[..2]).into(),
                path: String::from_utf8_lossy(&raw[3..]).into(),
                old_path,
            });
        }
    }
    view
}
/// Preserve document layout while making control sequences and bidi overrides inert.
pub(super) fn document(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\n' || c == '\t' {
                c
            } else if c.is_control()
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                '�'
            } else {
                c
            }
        })
        .collect()
}
pub fn diff(cwd: &Path, path: &str) -> io::Result<String> {
    let mut out = output(cwd, &["diff", "--no-ext-diff", "--no-textconv", "--", path])?;
    if out.is_empty() {
        out = output(
            cwd,
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--",
                path,
            ],
        )?;
    }
    Ok(document(&String::from_utf8_lossy(&out)))
}
/// Create only the explicit local branch/worktree requested by the user. No network or checkout of the source worktree.
pub fn create_worktree(repo: &Path, path: &Path, branch: &str, base: &str) -> io::Result<String> {
    output(repo, &["check-ref-format", "--branch", branch])?;
    let revision = format!("{base}^{{commit}}");
    let bytes = output(
        repo,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?;
    let sha = String::from_utf8(bytes)
        .map_err(io::Error::other)?
        .trim()
        .to_owned();
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(crate::wire::invalid("Git did not return a commit identity"));
    }
    if path.exists() {
        return Err(crate::wire::invalid(
            "worktree target already exists; inspect it before retrying",
        ));
    }
    let path = path
        .to_str()
        .ok_or_else(|| crate::wire::invalid("worktree path must be UTF-8"))?;
    output_for(
        repo,
        &["worktree", "add", "--quiet", "-b", branch, path, &sha],
        Duration::from_secs(60),
    )?;
    Ok(sha)
}

/// Preview exactly the selected staged/unstaged group; never fall through into
/// the other group when a file has both index and working-tree changes.
pub fn diff_change(cwd: &Path, change: &Change) -> io::Result<String> {
    if change.status == "??" {
        let versions = versions::working(cwd, change)?;
        return Ok(format!(
            "Untracked file: {}\n\n{}",
            crate::wire::passive(&change.path),
            document(&String::from_utf8_lossy(&versions.after))
        ));
    }
    let staged = change.status.as_bytes().get(1) == Some(&b' ');
    let mut args = vec!["diff", "--no-ext-diff", "--no-textconv"];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    if let Some(old) = &change.old_path {
        args.push(old);
    }
    args.push(&change.path);
    Ok(document(&String::from_utf8_lossy(&output(cwd, &args)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_exit_does_not_turn_pending_pipe_output_into_wouldblock() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf before; (sleep 0.05; printf after; printf diagnostic >&2) & exit 0",
        ]);
        let bytes = run_output(&mut command, Duration::from_secs(2)).unwrap();
        assert_eq!(bytes, b"beforeafter");
    }
    #[test]
    fn git_document_keeps_lines_and_tabs_without_terminal_controls() {
        assert_eq!(
            document("first\n\tsecond\x1b[31m\r\n"),
            "first\n\tsecond�[31m�\n"
        );
    }
    #[test]
    fn nul_paths_and_renames() {
        let v = parse(b"## main\0 M spaces and\nnewline\0R  new name\0old name\0?? --option\0");
        assert_eq!(v.branch, "main");
        assert_eq!(v.changes.len(), 3);
        assert_eq!(v.changes[1].path, "new name");
        assert_eq!(v.changes[2].path, "--option");
    }
}
