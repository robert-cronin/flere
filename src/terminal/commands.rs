//! OSC133 markers are untrusted annotations, never authority or input.
//! Semantics: https://iterm2.com/documentation-escape-codes.html
use super::*;
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct Marks(pub Vec<(u64, u8)>);
impl Terminal {
    pub(super) fn command_mark(&mut self, fields: &str) {
        if self.primary.is_some() {
            return;
        }
        let mut fields = fields.split(';');
        let kind = match fields.next() {
            Some("A") => b'A',
            Some("C") => b'C',
            Some("D") => b'D',
            _ => return,
        };
        if let Some(status) = fields.next()
            && (kind != b'D' || status.parse::<u8>().is_err())
        {
            return;
        }
        if fields.next().is_some() {
            return;
        }
        let row = self.history_base + self.history.len() as u64 + self.grid.y as u64;
        self.command_marks
            .0
            .retain(|(row, _)| *row >= self.history_base);
        if self.command_marks.0.last() != Some(&(row, kind)) {
            self.command_marks.0.push((row, kind));
        }
        if self.command_marks.0.len() > 2048 {
            self.command_marks
                .0
                .drain(..self.command_marks.0.len() - 2048);
        }
    }
    pub fn command_jump(
        &self,
        anchor: Option<u64>,
        previous: bool,
    ) -> std::io::Result<crate::model::ScrollbackPage> {
        if self.primary.is_some() {
            return Err(crate::wire::invalid(
                "command navigation is available on the primary shell screen",
            ));
        }
        let from =
            anchor.unwrap_or(self.history_base + self.history.len() as u64 + self.grid.y as u64);
        let mut marks = self
            .command_marks
            .0
            .iter()
            .filter(|(row, kind)| *kind == b'A' && *row >= self.history_base);
        let row = if previous {
            marks.rfind(|(row, _)| *row < from)
        } else {
            marks.find(|(row, _)| *row > from)
        };
        row.map(|(row, _)| self.scrollback(Some(*row), 0))
            .ok_or_else(|| {
                crate::wire::invalid(
                    "No further command marker; new Flere shells include prompt integration",
                )
            })
    }
    pub fn command_output(&self, anchor: Option<u64>) -> std::io::Result<String> {
        let from = anchor.unwrap_or(u64::MAX);
        let end_index = self
            .command_marks
            .0
            .iter()
            .enumerate()
            .rev()
            .find(|(_, (row, kind))| *kind == b'D' && *row <= from)
            .map(|(i, _)| i)
            .ok_or_else(|| crate::wire::invalid("No completed command output marker"))?;
        let end = self.command_marks.0[end_index].0;
        let start = self.command_marks.0[..end_index]
            .iter()
            .rev()
            .find(|(_, kind)| matches!(*kind, b'A' | b'C'))
            .filter(|(_, kind)| *kind == b'C')
            .map(|(row, _)| *row)
            .ok_or_else(|| crate::wire::invalid("This shell did not mark command output"))?;
        if self.primary.is_some() || start < self.history_base || end.saturating_sub(start) > 4096 {
            return Err(crate::wire::invalid(
                "Command output is unavailable or exceeds the copy limit",
            ));
        }
        let mut text = String::new();
        for row in start..end {
            let index = (row - self.history_base) as usize;
            let line = if index < self.history.len() {
                self.history.get(index).unwrap().text()
            } else if index - self.history.len() < self.grid.rows {
                self.grid.line(index - self.history.len())
            } else {
                break;
            };
            if text.len() + line.len() + 1 > 65536 {
                return Err(crate::wire::invalid(
                    "Command output exceeds 64 KiB; select a smaller region",
                ));
            }
            text.push_str(&line);
            text.push('\n');
        }
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markers_bound_navigation_and_copy_to_interpreted_output() {
        let mut t = Terminal::new(40, 5);
        t.feed(b"\x1b]133;A\x07$ echo one\r\n\x1b]133;C\x07one\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ echo two\r\n\x1b]133;C\x07two\r\n\x1b]133;D;0\x07\x1b]133;A\x07$ ");
        assert_eq!(t.command_output(None).unwrap(), "two\n");
        assert!(t.command_jump(None, true).is_ok());
        let restored: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(restored.command_output(None).unwrap(), "two\n");
        t.feed(b"\x1b[?1049h\x1b]133;A\x07");
        assert!(t.command_jump(None, true).is_err());
        t.feed(b"\x1b[?1049l");
        for _ in 0..3000 {
            t.feed(b"\x1b]133;A\x07\x1b]133;C\x07");
        }
        assert!(t.command_marks.0.len() <= 2048);
        assert!(t.replies.is_empty());
    }
}
