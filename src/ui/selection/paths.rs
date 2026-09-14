//! Bounded candidates from painted text only. No shell expansion or OSC targets.
use super::*;
const CANDIDATES: usize = 64;
fn target(path: &str, cwd: &Path) -> Option<PathBuf> {
    if path.is_empty() || path.contains("://") || path.chars().any(char::is_control) {
        return None;
    }
    if let Some(rest) = path.strip_prefix("~/") {
        Some(PathBuf::from(std::env::var_os("HOME")?).join(rest))
    } else if path.starts_with('/') {
        Some(PathBuf::from(path))
    } else {
        Some(cwd.join(path))
    }
}
pub(super) fn candidates(cells: &[Cell], cols: usize, anchor: usize, cwd: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for trim in [false, true] {
        let mut text = String::new();
        let mut positions = Vec::new();
        let clicked_row = anchor / cols;
        for (y, row) in cells
            .chunks(cols)
            .enumerate()
            .skip(clicked_row.saturating_sub(8))
            .take(17)
        {
            let first = if trim {
                row.iter().position(|c| c.text != " ").unwrap_or(row.len())
            } else {
                0
            };
            let last = if trim {
                row.iter()
                    .rposition(|c| c.text != " ")
                    .map_or(first, |x| x + 1)
            } else {
                row.len()
            };
            for (x, cell) in row.iter().enumerate().take(last).skip(first) {
                if cell.width == 0 {
                    continue;
                }
                for ch in cell.text.chars() {
                    text.push(ch);
                    positions.extend(std::iter::repeat_n(y * cols + x, ch.len_utf8()));
                }
            }
            text.push('\n');
            positions.push(usize::MAX);
        }
        // Token candidates first; explicit paths with unquoted spaces are a
        // fallback for displayed captions (including the existing image links).
        for spaces in [false, true] {
            for (start, ch) in text.char_indices() {
                if ch.is_whitespace() || ";&|<>)]}:".contains(ch) {
                    continue;
                }
                if start > 0
                    && !text[..start]
                        .ends_with(|c: char| c.is_whitespace() || "([<`\"'=".contains(c))
                {
                    continue;
                }
                let quote = if "\"'`".contains(ch) { Some(ch) } else { None };
                let begin = start + if quote.is_some() { ch.len_utf8() } else { 0 };
                if spaces
                    && quote.is_none()
                    && !text[begin..].starts_with('/')
                    && !text[begin..].starts_with("./")
                    && !text[begin..].starts_with("../")
                    && !text[begin..].starts_with("~/")
                {
                    continue;
                }
                let mut path = String::new();
                let mut hit = false;
                let mut lines = 0;
                let mut after_break = false;
                let mut full_row_end = false;
                let mut chars = text[begin..].char_indices().peekable();
                while let Some((offset, c)) = chars.next() {
                    if c == '\n' {
                        // A closing delimiter may wrap after the path fills a
                        // row. A short ordinary line is not that evidence.
                        full_row_end = (begin + offset)
                            .checked_sub(1)
                            .and_then(|i| positions.get(i))
                            .and_then(|&at| cells.get(at).map(|cell| (at, cell)))
                            .is_some_and(|(at, cell)| at % cols + usize::from(cell.width) == cols)
                            && path.chars().last().is_some_and(|c| !c.is_whitespace());
                        after_break = true;
                        lines += 1;
                        if lines > 8 {
                            break;
                        }
                        continue;
                    }
                    let boundary = quote == Some(c)
                        || quote.is_none() && (c.is_whitespace() || ";&|<>)]}:\"'`".contains(c));
                    if boundary {
                        let closing = quote == Some(c) || quote.is_none() && ")]}>".contains(c);
                        if hit
                            && (!after_break || full_row_end && closing)
                            && (quote.is_none() || quote == Some(c))
                        {
                            push(&mut result, &path, cwd);
                        }
                        if quote.is_some() || !spaces || !c.is_whitespace() {
                            break;
                        }
                    }
                    if c.is_control()
                        || path.len() + c.len_utf8() > 4096
                        || text[begin + offset..].starts_with("://")
                    {
                        break;
                    }
                    let (offset, c) = if c == '\\' && quote != Some('\'') {
                        if let Some((next_offset, next)) = chars.next() {
                            (next_offset, next)
                        } else {
                            break;
                        }
                    } else {
                        (offset, c)
                    };
                    path.push(c);
                    if !c.is_whitespace() {
                        after_break = false;
                    }
                    let at = positions[begin + offset];
                    hit |= at == anchor
                        || cells.get(anchor).is_some_and(|c| c.width == 0)
                            && at.checked_add(1) == Some(anchor);
                    if chars.peek().is_none() && hit {
                        push(&mut result, &path, cwd);
                    }
                    if result.len() >= CANDIDATES {
                        return result;
                    }
                }
            }
        }
    }
    result
}
fn push(result: &mut Vec<PathBuf>, path: &str, cwd: &Path) {
    for text in [path, path.trim_end_matches(['.', ',', ';'])] {
        if let Some(p) = target(text, cwd)
            && !result.contains(&p)
            && result.len() < CANDIDATES
        {
            result.push(p);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_delimiter_after_a_full_row_keeps_the_clicked_path() {
        let path = "/work/a comparison image.png";
        for (open, close) in [("(", ")"), ("\"", "\""), ("<", ">")] {
            // The closing delimiter alone wraps to the next row.
            let cols = path.len() + open.len();
            let mut term = crate::terminal::Terminal::new(cols, 5);
            term.feed(format!("{open}{path}{close}\r\nDO_NOT_SUBMIT").as_bytes());
            assert_eq!(term.grid.cells[cols].text, close);
            assert!(
                candidates(&term.grid.cells, cols, 4, Path::new("/work"))
                    .contains(&PathBuf::from(path)),
                "wrapped {close:?} lost the full path"
            );
            assert!(
                !candidates(&term.grid.cells, cols, cols * 2 + 2, Path::new("/work"))
                    .contains(&PathBuf::from(path)),
                "a click on unrelated text must not activate the path"
            );
            // A line break followed by ordinary text must not complete the
            // preceding path. Only the explicit closing delimiter is evidence.
            let mut term = crate::terminal::Terminal::new(cols, 4);
            term.feed(format!("{open}{path}\r\nUNRELATED_TEXT").as_bytes());
            assert!(
                !candidates(&term.grid.cells, cols, 4, Path::new("/work"))
                    .contains(&PathBuf::from(path)),
                "ordinary next-line text must not complete the preceding path"
            );
        }
    }
    #[test]
    fn command_paths_quotes_punctuation_and_relative_names_are_literal() {
        let mut c = Canvas::new(180, 2);
        let command = "cat /cache/checks.json && git diff --stat && wc -l src/ui/pet.rs './a b.txt' README.md:12 ";
        c.text(0, 0, 180, command, Style::default());
        for (word, expected) in [
            ("checks.json", "/cache/checks.json"),
            ("pet.rs", "/work/src/ui/pet.rs"),
            ("a b.txt", "/work/./a b.txt"),
            ("README", "/work/README.md"),
        ] {
            let paths = candidates(
                &c.cells,
                180,
                command.find(word).unwrap(),
                Path::new("/work"),
            );
            assert!(
                paths.contains(&PathBuf::from(expected)),
                "{word}: {paths:?}"
            );
        }
        let text = "https://example.invalid/file.rs /tmp/real.txt.bak $(touch NEVER) ./escaped\\ name.txt ";
        c = Canvas::new(180, 2);
        c.text(0, 0, 180, text, Style::default());
        assert!(
            candidates(
                &c.cells,
                180,
                text.find("file.rs").unwrap(),
                Path::new("/work")
            )
            .is_empty()
        );
        assert!(
            !candidates(
                &c.cells,
                180,
                text.find("real").unwrap(),
                Path::new("/work")
            )
            .contains(&PathBuf::from("/tmp/real.txt"))
        );
        assert!(
            candidates(
                &c.cells,
                180,
                text.find("escaped").unwrap(),
                Path::new("/work")
            )
            .contains(&PathBuf::from("/work/./escaped name.txt"))
        );
    }
}
