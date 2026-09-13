//! Exact file snapshots for a native editor comparison; never check out files.
use super::{Change, history::Details, history::FileChange, output};
use crate::wire::invalid;
use std::{
    io::{self, Read},
    path::{Component, Path},
};

pub struct Versions {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
    pub before_label: String,
    pub after_label: String,
    pub name: String,
}

fn blob(cwd: &Path, sha: &str, mode: &str) -> io::Result<Vec<u8>> {
    super::history::oid(sha)?;
    if !matches!(mode, "100644" | "100755" | "120000") {
        return Err(invalid(
            "submodule/type changes use the unified preview (u)",
        ));
    }
    output(cwd, &["cat-file", "blob", sha])
}

fn tree(cwd: &Path, sha: &str, path: &str) -> io::Result<Vec<u8>> {
    super::history::oid(sha)?;
    let listing = output(cwd, &["ls-tree", "-z", sha, "--", path])?;
    if listing.is_empty() {
        return Ok(Vec::new());
    }
    let record = listing
        .strip_suffix(&[0])
        .ok_or_else(|| invalid("malformed Git tree entry"))?;
    let separator = record
        .iter()
        .position(|b| *b == b'\t')
        .ok_or_else(|| invalid("malformed Git tree entry"))?;
    let (header, listed) = (&record[..separator], &record[separator + 1..]);
    if listed != path.as_bytes() {
        return Err(invalid("Git tree path differs from selected file"));
    }
    let fields: Vec<_> = std::str::from_utf8(header)
        .map_err(io::Error::other)?
        .split_whitespace()
        .collect();
    if fields.len() != 3 {
        return Err(invalid("malformed Git tree entry"));
    }
    blob(cwd, fields[2], fields[0])
}

fn index(cwd: &Path, path: &str) -> io::Result<Vec<u8>> {
    let listing = output(cwd, &["ls-files", "--stage", "-z", "--", path])?;
    if listing.is_empty() {
        return Ok(Vec::new());
    }
    let records: Vec<_> = listing
        .split(|b| *b == 0)
        .filter(|b| !b.is_empty())
        .collect();
    if records.len() != 1 {
        return Err(invalid("unmerged entries use the unified preview (u)"));
    }
    let separator = records[0]
        .iter()
        .position(|b| *b == b'\t')
        .ok_or_else(|| invalid("malformed Git index entry"))?;
    let (header, listed) = (&records[0][..separator], &records[0][separator + 1..]);
    if listed != path.as_bytes() {
        return Err(invalid("Git index path differs from selected file"));
    }
    let fields: Vec<_> = std::str::from_utf8(header)
        .map_err(io::Error::other)?
        .split_whitespace()
        .collect();
    if fields.len() != 3 || fields[2] != "0" {
        return Err(invalid("unmerged entries use the unified preview (u)"));
    }
    blob(cwd, fields[1], fields[0])
}

fn working_file(cwd: &Path, name: &str) -> io::Result<Vec<u8>> {
    let relative = Path::new(name);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(invalid("working diff requires a repository-relative file"));
    }
    let path = cwd.join(relative);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let root = std::fs::canonicalize(cwd)?;
    let parent = std::fs::canonicalize(path.parent().unwrap())?;
    if !parent.starts_with(root) {
        return Err(invalid("working diff path leaves the repository"));
    }
    if meta.file_type().is_symlink() {
        use std::os::unix::ffi::OsStrExt;
        return Ok(std::fs::read_link(path)?.as_os_str().as_bytes().to_vec());
    }
    if !meta.is_file() {
        return Err(invalid(
            "submodule/type changes use the unified preview (u)",
        ));
    }
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("working diff requires a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(512 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 512 * 1024 {
        return Err(invalid("working file exceeds the 512 KiB diff limit"));
    }
    Ok(bytes)
}

pub fn commit(cwd: &Path, details: &Details, file: &FileChange) -> io::Result<Versions> {
    let before = if let Some(parent) = &details.parent {
        tree(cwd, parent, file.old_path.as_deref().unwrap_or(&file.path))?
    } else {
        Vec::new()
    };
    let after = tree(cwd, &details.sha, &file.path)?;
    Ok(Versions {
        before,
        after,
        before_label: details
            .parent
            .as_ref()
            .map_or_else(|| "empty".into(), |p| p[..8].into()),
        after_label: details.sha[..8].into(),
        name: file.path.clone(),
    })
}

/// Match the selected status row: unstaged index/worktree first, then HEAD/index.
pub fn working(cwd: &Path, change: &Change) -> io::Result<Versions> {
    let unstaged = change.status.as_bytes().get(1).is_some_and(|b| *b != b' ');
    let indexed = index(cwd, &change.path)?;
    let (before, after, before_label, after_label) = if unstaged {
        (
            indexed,
            working_file(cwd, &change.path)?,
            "index".into(),
            "working".into(),
        )
    } else {
        let head = output(cwd, &["rev-parse", "--revs-only", "HEAD"])?;
        let head = std::str::from_utf8(&head).map_err(io::Error::other)?.trim();
        let before = if head.is_empty() {
            Vec::new()
        } else {
            tree(
                cwd,
                head,
                change.old_path.as_deref().unwrap_or(&change.path),
            )?
        };
        (
            before,
            indexed,
            if head.is_empty() {
                "empty".into()
            } else {
                format!("HEAD-{}", &head[..8])
            },
            "index".into(),
        )
    };
    Ok(Versions {
        before,
        after,
        before_label,
        after_label,
        name: change.path.clone(),
    })
}
