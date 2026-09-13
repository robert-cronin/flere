//! Bounded, cancellable workspace search. Results are data, never shell input.
use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Files,
    Contents,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub text: String,
}
#[derive(Default)]
pub struct Results {
    pub hits: Vec<Hit>,
    pub limited: bool,
    pub skipped: usize,
}
fn fuzzy(query: &str, value: &str) -> bool {
    let value = value.to_lowercase();
    let mut chars = value.chars();
    query.to_lowercase().chars().all(|q| chars.any(|c| c == q))
}
fn find_column(text: &str, needle: &str) -> Option<usize> {
    let mut lowered = String::new();
    let mut offsets = Vec::new();
    for (offset, c) in text.char_indices() {
        for lower in c.to_lowercase() {
            lowered.push(lower);
            offsets.extend(std::iter::repeat_n(offset + 1, lower.len_utf8()));
        }
    }
    lowered
        .find(needle)
        .and_then(|index| offsets.get(index).copied())
}
fn paths(root: &Path, cancel: &AtomicBool, until: Instant) -> io::Result<(Vec<PathBuf>, bool)> {
    if let Ok(bytes) = crate::git::output(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            ".",
        ],
    ) {
        let mut paths = Vec::new();
        for name in bytes
            .split(|b| *b == 0)
            .filter(|b| !b.is_empty())
            .take(10_001)
        {
            if let Ok(name) = std::str::from_utf8(name) {
                paths.push(root.join(name));
            }
        }
        paths.sort();
        paths.dedup();
        let limited = paths.len() > 10_000;
        paths.truncate(10_000);
        return Ok((paths, limited));
    }
    let mut pending = vec![(root.to_owned(), 0usize)];
    let mut paths = Vec::new();
    let mut seen = 0;
    let mut limited = false;
    while let Some((dir, depth)) = pending.pop() {
        if cancel.load(Ordering::Relaxed) || Instant::now() >= until {
            limited = true;
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            seen += 1;
            if seen > 20_000 || paths.len() >= 10_000 || Instant::now() >= until {
                limited = true;
                break;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                if depth < 32
                    && !matches!(
                        entry.file_name().to_str(),
                        Some(".git" | "target" | "node_modules" | ".venv" | ".cache")
                    )
                {
                    pending.push((entry.path(), depth + 1));
                }
            } else if kind.is_file() {
                paths.push(entry.path());
            }
        }
        if limited {
            break;
        }
    }
    paths.sort();
    Ok((paths, limited))
}
pub fn workspace(root: &Path, kind: Kind, query: &str, cancel: &AtomicBool) -> io::Result<Results> {
    if query.len() > 1024 {
        return Err(crate::wire::invalid("search query exceeds 1024 bytes"));
    }
    let root = fs::canonicalize(root)?;
    let until = Instant::now() + Duration::from_secs(3);
    let (paths, limited) = paths(&root, cancel, until)?;
    let mut result = Results {
        limited,
        ..Results::default()
    };
    let needle = query.to_lowercase();
    let mut total = 0usize;
    for path in paths {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if Instant::now() >= until || total >= 64 * 1024 * 1024 || result.hits.len() >= 200 {
            result.limited = true;
            break;
        }
        // A checked path must remain inside the selected root; never follow a
        // repository symlink into another project or read devices/FIFOs.
        let Ok(canonical) = fs::canonicalize(&path) else {
            continue;
        };
        if !canonical.starts_with(&root) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let label = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy();
        if kind == Kind::Files {
            if fuzzy(query, &label) {
                result.hits.push(Hit {
                    path,
                    line: 1,
                    column: 1,
                    text: String::new(),
                });
            }
            continue;
        }
        if needle.is_empty() {
            break;
        }
        if meta.len() > 1024 * 1024 {
            result.skipped += 1;
            continue;
        }
        let Ok(file) = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
        else {
            result.skipped += 1;
            continue;
        };
        if !file.metadata()?.is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err()
            || bytes.len() > 1024 * 1024
            || bytes.contains(&0)
        {
            result.skipped += 1;
            continue;
        }
        total += bytes.len();
        let Ok(text) = std::str::from_utf8(&bytes) else {
            result.skipped += 1;
            continue;
        };
        for (line, text) in text.lines().enumerate() {
            if let Some(column) = find_column(text, &needle) {
                result.hits.push(Hit {
                    path: path.clone(),
                    line: line + 1,
                    column,
                    text: crate::wire::passive(&text.chars().take(400).collect::<String>()),
                });
                if result.hits.len() == 200 {
                    result.limited = true;
                    break;
                }
            }
        }
    }
    Ok(result)
}
