//! Read-only, bounded commit pages and exact-commit diffs using installed Git.
use super::output;
use crate::wire::{invalid, passive};
use std::{io, path::Path};

pub const PAGE_SIZE: usize = 50;
#[derive(Clone, Debug)]
pub struct Commit {
    pub sha: String,
    pub parents: Vec<String>,
    pub refs: String,
    pub subject: String,
    pub graph: String,
    pub edges: Vec<String>,
}
#[derive(Clone, Debug, Default)]
pub struct History {
    pub tips: Vec<String>,
    pub page: usize,
    pub commits: Vec<Commit>,
    pub more: bool,
}
#[derive(Clone, Debug)]
pub struct FileChange {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
}
#[derive(Clone, Debug)]
pub struct Details {
    pub sha: String,
    pub parent: Option<String>,
    pub message: String,
    pub files: Vec<FileChange>,
}
pub(super) fn oid(value: &str) -> io::Result<()> {
    if matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(invalid("expected an exact Git object ID"))
    }
}
/// Page traversal uses the captured tips, so branch movement does not shift later pages.
pub fn history(cwd: &Path, tips: Option<&[String]>, page: usize) -> io::Result<History> {
    let tips = if let Some(tips) = tips {
        tips.to_vec()
    } else {
        let bytes = output(cwd, &["rev-parse", "--revs-only", "--all", "HEAD"])?;
        let mut tips: Vec<_> = String::from_utf8(bytes)
            .map_err(io::Error::other)?
            .lines()
            .map(String::from)
            .collect();
        tips.sort();
        tips.dedup();
        tips
    };
    if tips.len() > 2048 {
        return Err(invalid("history supports at most 2048 local ref tips"));
    }
    for tip in &tips {
        oid(tip)?;
    }
    if tips.is_empty() {
        return Ok(History {
            tips,
            page: 0,
            ..Default::default()
        });
    }
    let skip = page
        .checked_mul(PAGE_SIZE)
        .ok_or_else(|| invalid("history page overflow"))?;
    let skip = format!("--skip={skip}");
    let count = format!("--max-count={}", PAGE_SIZE + 1);
    let mut args = vec![
        "log",
        "--graph",
        "--topo-order",
        "--decorate=short",
        "--no-color",
        "--format=%x00%H%x00%P%x00%D%x00%s%x00",
        &skip,
        &count,
    ];
    args.extend(tips.iter().map(String::as_str));
    args.push("--");
    let bytes = output(cwd, &args)?;
    let mut commits = parse_history(&bytes)?;
    let more = commits.len() > PAGE_SIZE;
    commits.truncate(PAGE_SIZE);
    Ok(History {
        tips,
        page,
        commits,
        more,
    })
}
fn parse_history(bytes: &[u8]) -> io::Result<Vec<Commit>> {
    let mut commits: Vec<Commit> = Vec::new();
    for line in bytes.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let fields: Vec<_> = line.split(|b| *b == 0).collect();
        if fields.len() == 1 {
            let edge = graph(fields[0])?;
            if let Some(commit) = commits.last_mut() {
                commit.edges.push(edge);
            }
            continue;
        }
        if fields.len() != 6 || !fields[5].is_empty() {
            return Err(invalid("malformed Git history record"));
        }
        let sha = std::str::from_utf8(fields[1])
            .map_err(io::Error::other)?
            .to_owned();
        oid(&sha)?;
        let parents: Vec<_> = std::str::from_utf8(fields[2])
            .map_err(io::Error::other)?
            .split_whitespace()
            .map(String::from)
            .collect();
        for p in &parents {
            oid(p)?;
        }
        commits.push(Commit {
            sha,
            parents,
            refs: passive(&String::from_utf8_lossy(fields[3])),
            subject: passive(&String::from_utf8_lossy(fields[4])),
            graph: graph(fields[0])?,
            edges: Vec::new(),
        });
    }
    Ok(commits)
}
fn graph(bytes: &[u8]) -> io::Result<String> {
    if !bytes.iter().all(|b| b" */\\|_.-".contains(b)) {
        return Err(invalid("malformed Git graph"));
    }
    Ok(String::from_utf8_lossy(bytes).into())
}
pub fn details(cwd: &Path, sha: &str) -> io::Result<Details> {
    oid(sha)?;
    let parents = output(cwd, &["show", "--no-patch", "--format=%P", sha, "--"])?;
    let parents = std::str::from_utf8(&parents).map_err(io::Error::other)?;
    let parent = parents.split_whitespace().next().map(String::from);
    if let Some(p) = &parent {
        oid(p)?;
    }
    let message = output(
        cwd,
        &[
            "show",
            "--no-patch",
            "--format=Commit: %H%nParents: %P%nRefs: %D%nAuthor: %an <%ae>%nDate: %aI%n%n%B",
            sha,
            "--",
        ],
    )?;
    let bytes = if let Some(p) = &parent {
        output(
            cwd,
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--name-status",
                "-z",
                "-M",
                p,
                sha,
                "--",
            ],
        )?
    } else {
        output(
            cwd,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "-r",
                "--name-status",
                "-z",
                "-M",
                sha,
                "--",
            ],
        )?
    };
    let mut message = super::document(&String::from_utf8_lossy(&message));
    message.push_str(&format!(
        "\nDiff base: {}\n",
        parent
            .as_ref()
            .map(|p| format!("first parent {p}"))
            .unwrap_or_else(|| "empty tree (root commit)".into())
    ));
    Ok(Details {
        sha: sha.into(),
        parent,
        message,
        files: parse_files(&bytes)?,
    })
}
pub(super) fn parse_files(bytes: &[u8]) -> io::Result<Vec<FileChange>> {
    let mut fields = bytes.split(|b| *b == 0);
    let mut files = Vec::new();
    while let Some(status) = fields.next() {
        if status.is_empty() {
            continue;
        }
        let status = std::str::from_utf8(status).map_err(io::Error::other)?;
        if !matches!(
            status.as_bytes().first(),
            Some(b'A' | b'D' | b'M' | b'T' | b'U' | b'R' | b'C')
        ) || !status.as_bytes()[1..].iter().all(u8::is_ascii_digit)
        {
            return Err(invalid("malformed Git change status"));
        }
        let path = std::str::from_utf8(fields.next().ok_or_else(|| invalid("missing Git path"))?)
            .map_err(|_| invalid("Git history paths must be UTF-8"))?
            .to_owned();
        let (old_path, path) = if status.starts_with(['R', 'C']) {
            let destination = std::str::from_utf8(
                fields
                    .next()
                    .ok_or_else(|| invalid("missing rename destination"))?,
            )
            .map_err(|_| invalid("Git history paths must be UTF-8"))?
            .to_owned();
            (Some(path), destination)
        } else {
            (None, path)
        };
        if path.is_empty() {
            return Err(invalid("empty Git path"));
        }
        files.push(FileChange {
            status: status.into(),
            path,
            old_path,
        });
    }
    Ok(files)
}
/// A merge is compared with its first parent; a root commit with the empty tree.
pub fn diff(cwd: &Path, details: &Details, file: Option<&FileChange>) -> io::Result<String> {
    oid(&details.sha)?;
    let mut args = if let Some(parent) = &details.parent {
        oid(parent)?;
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "-M",
            parent,
            &details.sha,
            "--",
        ]
    } else {
        vec![
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "-p",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "-M",
            &details.sha,
            "--",
        ]
    };
    if let Some(file) = file {
        if let Some(old) = &file.old_path {
            args.push(old);
        }
        args.push(&file.path);
    }
    let bytes = output(cwd, &args)?;
    Ok(format!(
        "{}\n{}\n",
        details.message,
        super::document(&String::from_utf8_lossy(&bytes))
    ))
}
