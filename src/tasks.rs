//! User-started task reports. Locations are passive data parsed from interpreted
//! terminal rows; only an explicit editor action opens one of these paths.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_TASKS: usize = 64;
pub const MAX_PROBLEMS: usize = 128;
pub const MAX_COMMAND: usize = 4096;
pub const MAX_LABEL: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    pub workspace: u64,
    pub session: u64,
    pub run: String,
    pub label: String,
    pub command: String,
    pub cwd: PathBuf,
    pub ended: bool,
    pub exit_code: Option<i32>,
    pub problems: usize,
    pub limited: bool,
}
impl Summary {
    pub fn status(&self) -> String {
        if !self.ended {
            "running".into()
        } else {
            self.exit_code
                .map_or_else(|| "ended by signal".into(), |n| format!("exit {n}"))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    pub path: PathBuf,
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub task: Summary,
    pub problems: Vec<Problem>,
}

/// Rust's two-line diagnostics and the common file:line[:column]:message
/// convention. This is deliberately a location recognizer, not a compiler or
/// shell parser. Unsupported formats remain readable in the task's terminal.
#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Parser {
    header: String,
}
impl Parser {
    pub(crate) fn valid(&self) -> bool {
        self.header.len() <= 1024 && !self.header.chars().any(char::is_control)
    }
    pub(crate) fn row(&mut self, root: &Path, row: &str) -> Option<Problem> {
        let row = crate::wire::passive(row);
        let row = row.trim();
        if row.starts_with("error:")
            || row.starts_with("error[")
            || row.starts_with("warning:")
            || row.starts_with("warning[")
        {
            self.header = bounded(row, 1024);
            return None;
        }
        let arrow = row.starts_with("-->") || row.starts_with(":::");
        let row = if arrow { row[3..].trim() } else { row };
        if row.len() > 8192 || row.contains("://") {
            return None;
        }
        for (at, _) in row.match_indices(':') {
            let file = row[..at].trim();
            if file.is_empty() || file.starts_with('<') || file.len() > 4096 {
                continue;
            }
            let rest = &row[at + 1..];
            if !rest.as_bytes().first().is_some_and(u8::is_ascii_digit) {
                continue;
            }
            let (line, rest) = number(rest, 10_000_000)?;
            let (column, rest) = if rest
                .split(':')
                .next()
                .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            {
                number(rest, 1_000_000)?
            } else {
                (1, rest)
            };
            let path = Path::new(file);
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            if path.as_os_str().len() > 4096 {
                return None;
            }
            let message = rest.trim();
            return Some(Problem {
                path,
                line,
                column,
                message: bounded(
                    if message.is_empty() && arrow {
                        &self.header
                    } else {
                        message
                    },
                    1024,
                ),
            });
        }
        None
    }
}
fn number(text: &str, maximum: usize) -> Option<(usize, &str)> {
    let end = text.find(':').unwrap_or(text.len());
    let digits = &text[..end];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = digits.parse::<usize>().ok()?;
    (n > 0 && n <= maximum).then_some((n, text.get(end + 1..).unwrap_or("")))
}
pub(crate) fn bounded(text: &str, limit: usize) -> String {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compiler_locations_are_passive_bounded_data() {
        let root = Path::new("/work/project");
        let mut parser = Parser::default();
        assert!(parser.row(root, "error[E0308]: mismatched types").is_none());
        let rust = parser.row(root, "  --> src/main.rs:73:9").unwrap();
        assert_eq!((rust.line, rust.column), (73, 9));
        assert_eq!(rust.path, root.join("src/main.rs"));
        assert_eq!(rust.message, "error[E0308]: mismatched types");
        let c = parser
            .row(root, "/work/other dir/a.c:12:4: warning: unused value")
            .unwrap();
        assert_eq!(c.path, Path::new("/work/other dir/a.c"));
        assert_eq!(c.message, "warning: unused value");
        let go = parser.row(root, "main.go:21: undefined: value").unwrap();
        assert_eq!((go.line, go.column), (21, 1));
        for malformed in [
            "https://host:32:4",
            "file:0:3: bad",
            "file:999999999999999:1: bad",
            "<stdin>:2:3: bad",
            "file:2:0: bad",
            "file:2:1000001: bad",
        ] {
            assert!(parser.row(root, malformed).is_none(), "{malformed}");
        }
        let large = parser
            .row(root, &format!("a.rs:1:1: {}", "界".repeat(500)))
            .unwrap();
        assert!(large.message.len() <= 1024);
        assert!(parser.valid());
    }
}
