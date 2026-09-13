//! Recognize one locally pasted absolute path without reading it or evaluating a shell.
//! A candidate is only a reason to ask; it is never authority to upload a file.
#[derive(Debug, PartialEq, Eq)]
pub struct Candidate {
    pub original: Vec<u8>,
    pub path: Result<String, String>,
}
pub const LIMIT: usize = 4096;
fn absolute(s: &str, windows: bool) -> bool {
    if windows {
        let b = s.as_bytes();
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
            || s.starts_with("\\\\")
    } else {
        s.starts_with('/')
    }
}
pub fn candidate(bytes: &[u8]) -> Option<Candidate> {
    parse(bytes, cfg!(windows))
}
fn parse(bytes: &[u8], windows: bool) -> Option<Candidate> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.chars().any(char::is_control) {
        return None;
    }
    // Terminal drops commonly add a separator space. It is not part of the
    // shell word; retain it only in `original` for the explicit text choice.
    let text = text.trim_matches(' ');
    let start = text.strip_prefix(['\'', '"']).unwrap_or(text);
    if !absolute(start, windows) {
        return None;
    }
    let path = word(text, windows).and_then(|s| {
        if s.len() > LIMIT || !absolute(&s, windows) || s.chars().any(char::is_control) {
            Err("Choose one absolute path without control characters".into())
        } else {
            Ok(s)
        }
    });
    Some(Candidate {
        original: [b"\x1b[200~", bytes, b"\x1b[201~"].concat(),
        path,
    })
}
fn word(text: &str, windows: bool) -> Result<String, String> {
    let invalid = || "Ambiguous path: paste one quoted or escaped absolute path".to_string();
    let mut chars = text.chars().peekable();
    let mut quote = None;
    let mut result = String::new();
    while let Some(c) = chars.next() {
        if c.is_control() {
            return Err(invalid());
        }
        match (quote, c) {
            (Some(q), c) if q == c => {
                if windows && q == '\'' && chars.peek() == Some(&'\'') {
                    chars.next();
                    result.push('\'');
                } else {
                    quote = None;
                }
            }
            (None, '\'' | '"') => quote = Some(c),
            (None, c) if c.is_whitespace() => return Err(invalid()),
            (q, '\\') if !windows && q != Some('\'') => {
                let next = chars.next().ok_or_else(invalid)?;
                if next.is_control() {
                    return Err(invalid());
                }
                if q == Some('"') && !matches!(next, '\\' | '"' | '$' | '`') {
                    result.push('\\');
                }
                result.push(next);
            }
            (q, '`') if windows && q != Some('\'') => {
                let next = chars.next().ok_or_else(invalid)?;
                // PowerShell escaping, without evaluating interpolation or
                // named control escapes such as `n, `r or `t.
                if !matches!(next, ' ' | '`' | '\'' | '"' | '$') {
                    return Err(invalid());
                }
                result.push(next);
            }
            (Some('"'), '$') if windows => return Err(invalid()),
            (q, '$' | '`') if !windows && q != Some('\'') => return Err(invalid()),
            (None, ';' | '|' | '&' | '<' | '>' | '*' | '?') if !windows => return Err(invalid()),
            _ => result.push(c),
        }
    }
    if quote.is_some() || result.is_empty() {
        return Err(invalid());
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_host_paths_are_parsed_without_expansion_or_filesystem_reads() {
        for (text, wanted) in [
            ("  '/tmp/a b.png'  ", "/tmp/a b.png"),
            (" /tmp/a.png ", "/tmp/a.png"),
            ("/tmp/a\\ b.png", "/tmp/a b.png"),
            ("'/tmp/a b.pdf'", "/tmp/a b.pdf"),
            ("\"/tmp/a b.jpg\"", "/tmp/a b.jpg"),
            ("'/tmp/it'\\''s.png'", "/tmp/it's.png"),
        ] {
            assert_eq!(parse(text.as_bytes(), false).unwrap().path.unwrap(), wanted);
        }
        for (text, wanted) in [
            (r#"  "C:\local\a b.png"  "#, r"C:\local\a b.png"),
            (r#""C:\Users\A B\report.pdf""#, r"C:\Users\A B\report.pdf"),
            ("'C:\\it's.txt'", ""),
            ("'C:\\it''s.txt'", "C:\\it's.txt"),
            (r"\\server\share\report.pdf", r"\\server\share\report.pdf"),
            (r"C:\local\a` b.pdf", r"C:\local\a b.pdf"),
            (r#""C:\local\`$literal.txt""#, r"C:\local\$literal.txt"),
        ] {
            let actual = parse(text.as_bytes(), true).unwrap().path;
            if wanted.is_empty() {
                assert!(actual.is_err());
            } else {
                assert_eq!(actual.unwrap(), wanted);
            }
        }
        for text in [
            "/a /b",
            "'/a' '/b'",
            "/a;echo",
            "/a/$HOME",
            "/a/`id`",
            "'/unterminated",
        ] {
            assert!(
                parse(text.as_bytes(), false).unwrap().path.is_err(),
                "{text}"
            );
        }
        for text in ["ordinary draft", "relative.png", "look /absolute", ""] {
            assert!(parse(text.as_bytes(), false).is_none());
        }
        assert!(parse(b"/a\n/b", false).is_none());
        let original = b"  '/tmp/a b.png'  ";
        assert_eq!(
            parse(original, false).unwrap().original,
            [b"\x1b[200~", original.as_slice(), b"\x1b[201~"].concat()
        );
    }
}
