//! Launch the user's editor as an argv, never as interpolated shell input.
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

pub mod cache;

/// A managed document keeps its creation reservation through the UI open reply.
pub struct DiffDocument {
    pub path: PathBuf,
    _lease: cache::Lease,
}
pub struct ManagedComparison {
    pub before: PathBuf,
    pub after: PathBuf,
    _lease: cache::Lease,
}
fn snapshot_name(value: &str, limit: usize) -> String {
    let name: String = value
        .chars()
        .take(limit)
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() || name.starts_with('.') {
        format!("file-{name}")
    } else {
        name
    }
}
pub fn managed_diff_file(state: &Path, text: &str, title: &str) -> io::Result<DiffDocument> {
    use std::hash::{Hash, Hasher};
    if text.len() > 1024 * 1024 {
        return Err(crate::wire::invalid("diff document exceeds 1 MiB"));
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (text, title).hash(&mut hash);
    let name = format!("{}.diff", snapshot_name(title, 64));
    let (mut paths, lease) = cache::create(
        state,
        &format!("patch-{:016x}", hash.finish()),
        &[(&name, text.as_bytes())],
    )?;
    Ok(DiffDocument {
        path: paths.remove(0),
        _lease: lease,
    })
}
pub fn managed_comparison(
    state: &Path,
    versions: &crate::git::versions::Versions,
) -> io::Result<ManagedComparison> {
    use std::hash::{Hash, Hasher};
    for bytes in [&versions.before, &versions.after] {
        if bytes.len() > 512 * 1024 || bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
            return Err(crate::wire::invalid(
                "binary/non-UTF-8 or oversized files use the unified preview (u)",
            ));
        }
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (
        &versions.before,
        &versions.after,
        &versions.before_label,
        &versions.after_label,
        &versions.name,
    )
        .hash(&mut hash);
    let name = snapshot_name(
        Path::new(&versions.name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file"),
        100,
    );
    let before = format!(
        "before-{}-{name}",
        snapshot_name(&versions.before_label, 48)
    );
    let after = format!("after-{}-{name}", snapshot_name(&versions.after_label, 48));
    let (mut paths, lease) = cache::create(
        state,
        &format!("compare-{:016x}", hash.finish()),
        &[(&before, &versions.before), (&after, &versions.after)],
    )?;
    Ok(ManagedComparison {
        before: paths.remove(0),
        after: paths.remove(0),
        _lease: lease,
    })
}

pub fn argv() -> io::Result<Vec<String>> {
    let value = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
    if let Some(value) = value {
        let args = shell_words::split(&value).map_err(io::Error::other)?;
        if args.is_empty() {
            return Err(crate::wire::invalid("configured editor is empty"));
        }
        return Ok(args);
    }
    for editor in ["nvim", "vim", "vi"] {
        if executable(editor) {
            return Ok(vec![editor.into()]);
        }
    }
    Err(crate::wire::invalid(
        "set VISUAL or EDITOR to your installed editor",
    ))
}
pub fn executable(name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).is_file();
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|p| p.join(name).is_file())
}
pub fn file(cwd: &Path, value: &str) -> io::Result<PathBuf> {
    let input = Path::new(value);
    let path = std::fs::canonicalize(if input.is_absolute() {
        input.to_owned()
    } else {
        cwd.join(input)
    })?;
    if !path.is_file() {
        return Err(crate::wire::invalid("editor requires a regular file"));
    }
    if path.to_str().is_none() {
        return Err(crate::wire::invalid("editor path must be UTF-8"));
    }
    Ok(path)
}
pub fn command(path: &Path) -> io::Result<Command> {
    let args = argv()?;
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]).arg("--").arg(path);
    Ok(cmd)
}

/// An explicit file/line jump is argv data; never synthesize Vim keystrokes.
pub fn command_at(path: &Path, line: usize, column: usize) -> io::Result<Command> {
    let args = argv()?;
    if !matches!(
        Path::new(&args[0]).file_name().and_then(|s| s.to_str()),
        Some("nvim" | "vim" | "vi" | "nvimdiff" | "vimdiff")
    ) {
        return Err(crate::wire::invalid(
            "file/line jumps require Vim or Neovim",
        ));
    }
    let mut command = Command::new(&args[0]);
    command
        .args(&args[1..])
        .arg(format!("+call cursor({line},{column})"))
        .arg("--")
        .arg(path);
    Ok(command)
}

/// Retain a bounded diff document outside the checkout and reuse its editor tab.
/// Existing files must match exactly; symlinks and hash collisions are rejected.
pub fn diff_file(text: &str, title: &str) -> io::Result<PathBuf> {
    use std::{
        fs::OpenOptions,
        hash::{Hash, Hasher},
        io::{Read, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    if text.len() > 1024 * 1024 {
        return Err(crate::wire::invalid("diff document exceeds 1 MiB"));
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .ok_or_else(|| {
            crate::wire::invalid("HOME or XDG_CACHE_HOME is required for diff documents")
        })?
        .join("flere")
        .join("git-diffs");
    crate::server::private_state(&cache)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    let name: String = title
        .chars()
        .take(64)
        .map(|c| {
            if c.is_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let path = cache.join(format!("{name}-{:016x}.diff", hash.finish()));
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(e) = file
                .write_all(text.as_bytes())
                .and_then(|_| file.sync_all())
            {
                let _ = std::fs::remove_file(&path);
                return Err(e);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)?;
            let meta = file.metadata()?;
            if !meta.is_file() || meta.uid() != crate::os::uid() || meta.mode() & 0o077 != 0 {
                return Err(crate::wire::invalid(
                    "diff document must be a private owned file",
                ));
            }
            let mut existing = String::new();
            file.take(1024 * 1024 + 1).read_to_string(&mut existing)?;
            if existing != text {
                return Err(crate::wire::invalid(
                    "cached diff changed; retain it and inspect before retrying",
                ));
            }
        }
        Err(e) => return Err(e),
    }
    Ok(path)
}

#[derive(Clone)]
pub struct Comparison {
    pub before: PathBuf,
    pub after: PathBuf,
}

/// Preserve both file versions in private immutable files with their original extension.
pub fn comparison(versions: &crate::git::versions::Versions) -> io::Result<Comparison> {
    use std::{
        fs::{DirBuilder, OpenOptions},
        hash::{Hash, Hasher},
        io::{Read, Write},
        os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    };
    for bytes in [&versions.before, &versions.after] {
        if bytes.len() > 512 * 1024 || bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
            return Err(crate::wire::invalid(
                "binary/non-UTF-8 or oversized files use the unified preview (u)",
            ));
        }
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")))
        .ok_or_else(|| crate::wire::invalid("HOME or XDG_CACHE_HOME is required"))?
        .join("flere")
        .join("git-diffs");
    crate::server::private_state(&cache)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    (
        &versions.before,
        &versions.after,
        &versions.before_label,
        &versions.after_label,
        &versions.name,
    )
        .hash(&mut hash);
    let root = cache.join(format!("compare-{:016x}", hash.finish()));
    let safe = |text: &str| -> String {
        text.chars()
            .take(120)
            .map(|c| {
                if c.is_alphanumeric() || "-_.".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let name = safe(
        Path::new(&versions.name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file"),
    );
    let private_dir = |path: &Path| -> io::Result<()> {
        match DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_dir() || meta.uid() != crate::os::uid() || meta.mode() & 0o077 != 0 {
            return Err(crate::wire::invalid(
                "comparison directory must be private and owned",
            ));
        }
        Ok(())
    };
    private_dir(&root)?;
    let mut paths = Vec::new();
    for (prefix, label, bytes) in [
        ("before", &versions.before_label, &versions.before),
        ("after", &versions.after_label, &versions.after),
    ] {
        let dir = root.join(format!("{prefix}-{}", safe(label)));
        private_dir(&dir)?;
        let path = dir.join(&name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(e);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW)
                    .open(&path)?;
                let meta = file.metadata()?;
                if !meta.is_file() || meta.uid() != crate::os::uid() || meta.mode() & 0o277 != 0 {
                    return Err(crate::wire::invalid(
                        "comparison snapshot must be private and read-only",
                    ));
                }
                let mut existing = Vec::new();
                file.take(512 * 1024 + 1).read_to_end(&mut existing)?;
                if &existing != bytes {
                    return Err(crate::wire::invalid(
                        "comparison snapshot changed; inspect before retrying",
                    ));
                }
            }
            Err(e) => return Err(e),
        }
        paths.push(path);
    }
    Ok(Comparison {
        before: paths.remove(0),
        after: paths.remove(0),
    })
}

/// Vim/Neovim own highlighting, alignment and synchronized scrolling.
pub fn comparison_command(before: &Path, after: &Path) -> io::Result<Command> {
    use std::os::unix::fs::MetadataExt;
    for path in [before, after] {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.uid() != crate::os::uid() || meta.mode() & 0o277 != 0 {
            return Err(crate::wire::invalid(
                "comparison requires private read-only snapshots",
            ));
        }
    }
    let args = argv()?;
    let editor = Path::new(&args[0])
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !matches!(editor, "nvim" | "vim" | "vi" | "nvimdiff" | "vimdiff") {
        return Err(crate::wire::invalid(
            "side-by-side diffs require Vim/Neovim; u previews the patch",
        ));
    }
    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..])
        .args(["-d", "-O", "-R", "-M", "-n", "--cmd", "set nomodeline"])
        .args(["-c", "windo setlocal number nowrap nofoldenable"])
        .arg("--")
        .arg(before)
        .arg(after);
    Ok(cmd)
}
