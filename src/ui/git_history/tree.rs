//! Stable, scoped tree rows shared by working, branch and per-commit changes.
use super::{Kind, Row};
use crate::git::Change;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Group {
    Staged,
    Unstaged,
    Untracked,
    Conflicts,
}
impl Group {
    pub const ALL: [Self; 4] = [
        Self::Conflicts,
        Self::Staged,
        Self::Unstaged,
        Self::Untracked,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Staged => "Staged",
            Self::Unstaged => "Unstaged",
            Self::Untracked => "Untracked",
            Self::Conflicts => "Conflicts",
        }
    }
    pub fn change(self, change: &Change) -> Option<Change> {
        let bytes = change.status.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        let conflict = matches!(bytes, b"DD" | b"AU" | b"UD" | b"UA" | b"DU" | b"AA" | b"UU");
        let mut value = change.clone();
        match self {
            Self::Conflicts if conflict => {}
            Self::Untracked if bytes == b"??" => {}
            Self::Staged if !conflict && bytes[0] != b' ' && bytes[0] != b'?' => {
                value.status = format!("{} ", bytes[0] as char)
            }
            Self::Unstaged if !conflict && bytes[1] != b' ' && bytes[1] != b'?' => {
                value.status = format!(" {}", bytes[1] as char);
                // A staged rename has already changed the index path.
                if !matches!(bytes[1], b'R' | b'C') {
                    value.old_path = None;
                }
            }
            _ => return None,
        }
        Some(value)
    }
}

pub(super) struct Entry {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub kind: Kind,
}
#[derive(Default)]
struct Node {
    directories: BTreeMap<String, Node>,
    files: BTreeMap<String, Entry>,
}
impl Node {
    fn insert(&mut self, entry: Entry) {
        // Git paths use '/' on every supported platform. Preserve literal names
        // for identity/actions; only presentation passes through passive().
        let path = entry.path.trim_end_matches('/').to_owned();
        // Bound nesting even for adversarial Git trees. Deeper suffixes remain
        // literal file labels, with the full original action identity.
        let parts: Vec<_> = path.splitn(65, '/').collect();
        let mut node = self;
        for component in &parts[..parts.len().saturating_sub(1)] {
            node = node.directories.entry((*component).into()).or_default();
        }
        if let Some(name) = parts.last() {
            node.files.insert((*name).into(), entry);
        }
    }
    fn rows(
        &self,
        scope: &str,
        path: &str,
        prefix: &str,
        closed: &HashSet<String>,
        out: &mut Vec<Row>,
    ) {
        let total = self.directories.len() + self.files.len();
        for (index, (name, node)) in self.directories.iter().enumerate() {
            let last = index + 1 == total;
            let path = format!("{path}{name}/");
            let key = format!("dir:{scope}:{path}");
            let folded = closed.contains(&key);
            out.push(Row::new(
                &key,
                format!(
                    "{prefix}{} {}/",
                    if folded { "▸" } else { "▾" },
                    crate::wire::passive(name)
                ),
                Kind::Folder(key.clone()),
            ));
            if !folded {
                let next = format!("{prefix}{}", if last { "  " } else { "│ " });
                node.rows(scope, &path, &next, closed, out);
            }
        }
        for (index, (name, entry)) in self.files.iter().enumerate() {
            let last = self.directories.len() + index + 1 == total;
            let lead = format!("{prefix}{} ", if last { "└" } else { "├" });
            let status = entry.status.trim();
            let status = if status == "??" { "?" } else { status };
            let label = if let Some(old) = &entry.old_path {
                format!(
                    "{} ← {}",
                    crate::wire::passive(name),
                    crate::wire::passive(old)
                )
            } else {
                crate::wire::passive(name)
            };
            let mut row = Row::new(
                format!("file:{scope}:{}", entry.path),
                format!("{lead}{status} {label}"),
                entry.kind.clone(),
            );
            row.status = Some((lead.chars().count(), status.to_owned()));
            out.push(row);
        }
    }
}
pub(super) fn rows(scope: &str, entries: Vec<Entry>, closed: &HashSet<String>) -> Vec<Row> {
    let mut node = Node::default();
    for entry in entries {
        node.insert(entry);
    }
    let mut out = Vec::new();
    node.rows(scope, "", "  ", closed, &mut out);
    out
}
