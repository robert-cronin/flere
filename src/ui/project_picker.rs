//! A path field with directory suggestions, sharing the attachment's bounded
//! explorer worker. Enter always submits the displayed path, never a suggestion.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct Picker {
    pub id: u64,
    pub explorer: Explorer,
    base: PathBuf,
    home: Option<PathBuf>,
    prefix: String,
}

pub(super) fn directory_text(path: &Path) -> String {
    format!("{}/", path.to_string_lossy().trim_end_matches('/'))
}

impl Picker {
    pub fn new(base: PathBuf) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            explorer: Explorer::new(base.clone()),
            base,
            home: std::env::var_os("HOME").map(PathBuf::from),
            prefix: String::new(),
        }
    }

    pub fn resolve(&self, value: &str) -> PathBuf {
        if let Some(home) = &self.home {
            if value == "~" {
                return home.clone();
            }
            if let Some(rest) = value.strip_prefix("~/") {
                return home.join(rest);
            }
        }
        self.base.join(value)
    }

    pub fn update(&mut self, value: &[u8]) {
        let text = String::from_utf8_lossy(value);
        let path = self.resolve(&text);
        let (root, prefix) = if text.is_empty() || text.ends_with('/') || text == "~" {
            (path, String::new())
        } else {
            (
                path.parent().unwrap_or(&self.base).to_path_buf(),
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        if self.explorer.root != root {
            self.explorer.load(root);
        }
        self.prefix = prefix;
        self.explorer.selected = 0;
    }

    fn directories(&self) -> Vec<&(String, PathBuf, bool)> {
        self.explorer
            .entries
            .iter()
            .filter(|(name, path, directory)| {
                *directory
                    && name != ".."
                    && name.starts_with(&self.prefix)
                    && path
                        .to_str()
                        .is_some_and(|p| !p.chars().any(char::is_control))
            })
            .collect()
    }

    pub fn key(&mut self, key: &[u8], value: &mut Vec<u8>) -> bool {
        match key {
            b"\x1b[A" | b"\x1b[B" => {
                let count = self.directories().len();
                self.explorer.selected = if key == b"\x1b[B" {
                    (self.explorer.selected + 1).min(count.saturating_sub(1))
                } else {
                    self.explorer.selected.saturating_sub(1)
                };
            }
            b"\t" | b"\x1b[C" => {
                if let Some((_, path, _)) = self.directories().get(self.explorer.selected) {
                    *value = directory_text(path).into_bytes();
                    self.update(value);
                }
            }
            b"\x1b[D" | b"\x1b[1;3A" => {
                let path = self.resolve(&String::from_utf8_lossy(value));
                if let Some(parent) = path.parent() {
                    *value = directory_text(parent).into_bytes();
                    self.update(value);
                }
            }
            b"\x12" => self.explorer.load(self.explorer.root.clone()),
            _ => return false,
        }
        true
    }

    pub fn draw(&self, c: &mut Canvas, l: Layout, value: &[u8]) {
        let w = 78.min(l.width);
        let h = 19.min(l.height.saturating_sub(2));
        let x = (l.width - w) / 2;
        let y = (l.height - h) / 2;
        let inner = w.saturating_sub(6);
        c.fill(x, y, w, h, style(TEXT, PANEL, false));
        c.border(x, y, w, h, BORDER);
        c.text(x + 2, y + 1, w - 4, "Add project", style(CYAN, PANEL, true));
        c.text(
            x + 3,
            y + 2,
            inner,
            "Project directory",
            style(MUTED, PANEL, false),
        );
        c.fill(x + 1, y + 3, w - 2, 1, style(TEXT, ACTIVE_BG, false));
        c.text(
            x + 3,
            y + 3,
            inner,
            &chrome::tail(&wire::passive(&String::from_utf8_lossy(value)), inner),
            style(TEXT, ACTIVE_BG, true),
        );
        if h >= 11 {
            c.text(
                x + 3,
                y + 5,
                inner,
                "Folders · Tab opens the highlighted suggestion",
                style(MUTED, PANEL, false),
            );
            let rows = h.saturating_sub(10);
            let items = self.directories();
            let start = self
                .explorer
                .selected
                .saturating_sub(rows.saturating_sub(1));
            let message = if self.explorer.loading {
                Some("Loading folders…")
            } else if let Some(error) = &self.explorer.error {
                Some(error.as_str())
            } else if items.is_empty() {
                Some("No matching folders · edit the path or go to its parent")
            } else {
                None
            };
            if let Some(message) = message {
                c.text(x + 3, y + 6, inner, message, style(MUTED, PANEL, false));
            } else {
                for (i, (name, _, _)) in items.iter().enumerate().skip(start).take(rows) {
                    let selected = i == self.explorer.selected;
                    let bg = if selected { ACTIVE_BG } else { PANEL };
                    let row = y + 6 + i - start;
                    c.fill(x + 1, row, w - 2, 1, style(TEXT, bg, false));
                    c.text(
                        x + 3,
                        row,
                        inner,
                        &chrome::elide(
                            &format!("{} {name}/", if selected { "›" } else { " " }),
                            inner,
                        ),
                        style(if selected { CYAN } else { TEXT }, bg, selected),
                    );
                }
            }
            c.text(
                x + 3,
                y + h - 4,
                inner,
                "Name from Git remote or checkout directory",
                style(MUTED, PANEL, false),
            );
        }
        if h >= 8 {
            c.text(
                x + 2,
                y + h - 3,
                w - 4,
                "↑/↓ select · Tab browse/complete · ← parent · Esc cancel",
                style(MUTED, PANEL, false),
            );
        }
        c.text(
            x + 2,
            y + h - 2,
            w - 4,
            "Enter adds shown path · Ctrl+U clear · Ctrl+R retry",
            style(MUTED, PANEL, false),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_preserves_spaces_and_literal_shell_characters() {
        let mut picker = Picker::new("/projects".into());
        picker.explorer.loading = false;
        picker.explorer.entries = vec![
            ("..".into(), "/".into(), true),
            (
                "hello $(world)".into(),
                "/projects/hello $(world)".into(),
                true,
            ),
            ("hello.rs".into(), "/projects/hello.rs".into(), false),
        ];
        let mut value = b"hell".to_vec();
        picker.update(&value);
        assert_eq!(picker.directories().len(), 1);
        assert!(!picker.key(b"h", &mut value));
        assert!(!picker.key(b"j", &mut value));
        assert!(!picker.key(b"k", &mut value));
        assert!(!picker.key(b"l", &mut value));
        assert!(!picker.key(b"\r", &mut value));
        assert_eq!(value, b"hell"); // Enter never chooses a suggestion.
        assert!(picker.key(b"\t", &mut value));
        assert_eq!(value, b"/projects/hello $(world)/");
        assert_eq!(picker.explorer.root, Path::new("/projects/hello $(world)"));
        assert!(picker.key(b"\x1b[D", &mut value));
        assert_eq!(value, b"/projects/");
    }

    #[test]
    fn home_relative_paths_and_stale_suggestions_are_handled_without_io() {
        let mut picker = Picker::new("/projects".into());
        picker.home = Some("/home/person".into());
        assert_eq!(
            picker.resolve("~/with spaces"),
            Path::new("/home/person/with spaces")
        );
        assert_eq!(picker.resolve("relative"), Path::new("/projects/relative"));
        picker.update(b"/different/pa");
        assert_eq!(picker.explorer.root, Path::new("/different"));
        assert!(picker.explorer.loading);
        assert!(picker.directories().is_empty());
        let mut value = b"/different/pa".to_vec();
        picker.key(b"\t", &mut value);
        assert_eq!(value, b"/different/pa");
    }
}
