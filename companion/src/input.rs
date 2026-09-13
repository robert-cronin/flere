//! Preserve native bytes, intercepting only a deliberate clipboard paste gesture.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Bytes(Vec<u8>),
    Clipboard(Vec<u8>),
    Screenshot(Vec<u8>),
    Terminal(Vec<u8>),
    Drop(crate::drop_path::Candidate),
}
#[derive(Default)]
pub struct Input {
    buffer: Vec<u8>,
    paste: bool,
    content: bool,
    screenshot_path: Vec<u8>,
    screenshot_paste: Option<Vec<u8>>,
    drop_enabled: bool,
    local_paste: Option<Vec<u8>>,
}
const START: &[u8] = b"\x1b[200~";
const END: &[u8] = b"\x1b[201~";
impl Input {
    pub fn enable_drop(&mut self) {
        self.drop_enabled = true;
    }
    pub fn disable_drop(&mut self) {
        self.drop_enabled = false;
    }
    pub fn image_path(&mut self, path: &[u8]) {
        self.screenshot_path = path.to_vec();
    }
    pub fn reset(&mut self) {
        let path = std::mem::take(&mut self.screenshot_path);
        let enabled = self.drop_enabled;
        *self = Self::default();
        self.screenshot_path = path;
        self.drop_enabled = enabled;
    }
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Action> {
        self.buffer.extend_from_slice(bytes);
        let mut actions = Vec::new();
        let mut native = Vec::new();
        loop {
            if self.buffer.is_empty() {
                break;
            }
            let marker = if self.paste { END } else { START };
            if self.buffer.starts_with(marker) {
                self.buffer.drain(..marker.len());
                if self.paste {
                    if let Some(bytes) = self.local_paste.take() {
                        if !native.is_empty() {
                            actions.push(Action::Bytes(std::mem::take(&mut native)));
                        }
                        actions.push(if bytes.is_empty() {
                            Action::Clipboard([START, END].concat())
                        } else if bytes == self.screenshot_path {
                            Action::Screenshot(bytes)
                        } else if let Some(candidate) = crate::drop_path::candidate(&bytes) {
                            Action::Drop(candidate)
                        } else {
                            Action::Bytes([START, &bytes, END].concat())
                        });
                    } else if let Some(path) = self.screenshot_paste.take() {
                        if path.is_empty() || path == self.screenshot_path {
                            if !native.is_empty() {
                                actions.push(Action::Bytes(std::mem::take(&mut native)));
                            }
                            actions.push(if path.is_empty() {
                                Action::Clipboard([START, END].concat())
                            } else {
                                Action::Screenshot(path)
                            });
                        } else {
                            native.extend([START, &path, END].concat());
                        }
                    } else if self.content {
                        native.extend_from_slice(END);
                    } else {
                        if !native.is_empty() {
                            actions.push(Action::Bytes(std::mem::take(&mut native)));
                        }
                        actions.push(Action::Clipboard([START, END].concat()));
                    }
                    self.paste = false;
                } else {
                    self.paste = true;
                    self.content = false;
                    self.local_paste = self.drop_enabled.then(Vec::new);
                    self.screenshot_paste =
                        (!self.drop_enabled && !self.screenshot_path.is_empty()).then(Vec::new);
                }
                continue;
            }
            if marker.starts_with(&self.buffer) {
                break;
            }
            if !self.paste && self.buffer.starts_with(b"\x1b[") {
                let tail = &self.buffer[2..];
                let query =
                    tail.starts_with(b"?") || tail.starts_with(b"6;") || b"6;".starts_with(tail);
                if query {
                    if let Some(n) = tail.iter().position(|b| (0x40..=0x7e).contains(b)) {
                        let n = n + 3;
                        let reply = &self.buffer[..n];
                        if (reply.starts_with(b"\x1b[?") && reply.ends_with(b"c"))
                            || (reply.starts_with(b"\x1b[6;") && reply.ends_with(b"t"))
                        {
                            if !native.is_empty() {
                                actions.push(Action::Bytes(std::mem::take(&mut native)));
                            }
                            actions.push(Action::Terminal(self.buffer.drain(..n).collect()));
                            continue;
                        }
                    } else if self.buffer.len() <= 128 {
                        break;
                    }
                }
            }
            let byte = self.buffer.remove(0);
            if let Some(bytes) = &mut self.local_paste {
                bytes.push(byte);
                if bytes.len() > crate::drop_path::LIMIT {
                    native.extend_from_slice(START);
                    native.extend(self.local_paste.take().unwrap());
                    self.content = true;
                }
                continue;
            }
            if let Some(candidate) = &mut self.screenshot_paste {
                candidate.push(byte);
                if !self.screenshot_path.starts_with(candidate) {
                    native.extend_from_slice(START);
                    native.extend(self.screenshot_paste.take().unwrap());
                    self.content = true;
                }
                continue;
            }
            if !self.paste && byte == 0x16 {
                if !native.is_empty() {
                    actions.push(Action::Bytes(std::mem::take(&mut native)));
                }
                actions.push(Action::Clipboard(vec![byte]));
            } else {
                if self.paste && !self.content {
                    native.extend_from_slice(START);
                    self.content = true;
                }
                native.push(byte);
            }
        }
        if !native.is_empty() {
            actions.push(Action::Bytes(native));
        }
        actions
    }
    pub fn timeout(&mut self) -> Vec<Action> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        // A lone Escape outside paste must not wait indefinitely for an escape sequence.
        if !self.paste && !self.buffer.starts_with(b"\x1b[?") && !self.buffer.starts_with(b"\x1b[6")
        {
            return vec![Action::Bytes(std::mem::take(&mut self.buffer))];
        }
        Vec::new()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_events_and_flags_reply_reach_remote_unchanged_when_fragmented() {
        let bytes = b"\x1b[?11u\x1b[108;1:1u\x1b[32;1:1u\x1b[108;1:2u\x1b[108;1:3u\x1b[1;1:3D";
        for split in 0..=bytes.len() {
            let mut input = Input::default();
            let actions = input
                .feed(&bytes[..split])
                .into_iter()
                .chain(input.feed(&bytes[split..]));
            let actual: Vec<u8> = actions
                .flat_map(|action| match action {
                    Action::Bytes(bytes) => bytes,
                    other => panic!("keyboard bytes were intercepted: {other:?}"),
                })
                .collect();
            assert_eq!(actual, bytes, "split {split}");
        }
    }
    #[test]
    fn negotiated_path_candidates_keep_unbracketed_input_and_all_text_bytes_literal() {
        let path = if cfg!(windows) {
            br#""C:\local\report 1.pdf""#.as_slice()
        } else {
            b"'/local/report 1.pdf'".as_slice()
        };
        let original = [START, path, END].concat();
        let mut parser = Input::default();
        assert_eq!(parser.feed(&original), [Action::Bytes(original.clone())]);
        parser.enable_drop();
        assert_eq!(parser.feed(path), [Action::Bytes(path.to_vec())]);
        let actions: Vec<_> = original.iter().flat_map(|b| parser.feed(&[*b])).collect();
        assert!(
            matches!(actions.as_slice(), [Action::Drop(c)] if c.original == original && c.path.is_ok())
        );
        let text = [START, b"ordinary multiline\ntext\x16", END].concat();
        assert_eq!(parser.feed(&text), [Action::Bytes(text)]);
        let long = [START, &vec![b'x'; crate::drop_path::LIMIT + 1], END].concat();
        let actual: Vec<_> = parser
            .feed(&long)
            .into_iter()
            .flat_map(|a| match a {
                Action::Bytes(b) => b,
                _ => panic!("long paste must stay text"),
            })
            .collect();
        assert_eq!(actual, long);
        parser.disable_drop();
        parser.reset();
        assert_eq!(parser.feed(&original), [Action::Bytes(original)]);
    }
    #[test]
    fn screenshot_path_pastes_upload_the_image_across_fragmentation_and_refresh() {
        let path = b"/local/cache/a screenshot.png";
        let paste = [START, path, END].concat();
        let mut input = Input::default();
        input.image_path(path);
        input.reset();
        let out: Vec<_> = paste.iter().flat_map(|b| input.feed(&[*b])).collect();
        assert_eq!(out, [Action::Screenshot(path.to_vec())]);
        assert_eq!(
            input.feed(&[START, END].concat()),
            [Action::Clipboard([START, END].concat())]
        );
        let other = [START, b"ordinary draft", END].concat();
        assert_eq!(input.feed(&other), [Action::Bytes(other)]);
        let prefix = [START, b"/local", END].concat();
        assert_eq!(input.feed(&prefix), [Action::Bytes(prefix)]);
    }
    #[test]
    fn fragmented_clipboard_gestures_preserve_text_and_controls() {
        let mut input = Input::default();
        let mut out = Vec::new();
        let bytes = b"a\x1b[200~text\x16inside\x1b[201~b\x16\x1b[200~\x1b[201~";
        for byte in bytes {
            out.extend(input.feed(&[*byte]));
        }
        let native: Vec<u8> = out
            .iter()
            .filter_map(|a| {
                if let Action::Bytes(b) = a {
                    Some(b.as_slice())
                } else {
                    None
                }
            })
            .flatten()
            .copied()
            .collect();
        assert_eq!(native, b"a\x1b[200~text\x16inside\x1b[201~b");
        assert_eq!(
            out.iter()
                .filter(|a| matches!(a, Action::Clipboard(_)))
                .count(),
            2
        );
        assert!(input.feed(b"\x1b").is_empty());
        assert_eq!(input.timeout(), vec![Action::Bytes(vec![27])]);
    }
    #[test]
    fn terminal_replies_are_consumed_fragmented_but_literal_paste_and_keys_survive() {
        let mut parser = Input::default();
        let mut out = Vec::new();
        for byte in b"draft\x1b[?65;1;4;6c\x1b[6;20;10t\x1b[6;5~x" {
            out.extend(parser.feed(&[*byte]));
        }
        let native: Vec<u8> = out
            .iter()
            .filter_map(|a| {
                if let Action::Bytes(b) = a {
                    Some(b.as_slice())
                } else {
                    None
                }
            })
            .flatten()
            .copied()
            .collect();
        assert_eq!(native, b"draft\x1b[6;5~x");
        assert_eq!(
            out.iter()
                .filter(|a| matches!(a, Action::Terminal(_)))
                .count(),
            2
        );
        let bytes = b"\x1b[200~literal\x1b[?65;4c\x1b[6;20;10t\x1b[201~";
        let out = parser.feed(bytes);
        assert_eq!(out, vec![Action::Bytes(bytes.to_vec())]);
        assert!(parser.feed(b"\x1b[6;").is_empty());
        assert!(parser.timeout().is_empty());
        assert_eq!(
            parser.feed(b"20;10t"),
            vec![Action::Terminal(b"\x1b[6;20;10t".to_vec())]
        );
        assert!(parser.feed(b"\x1b").is_empty());
        assert_eq!(parser.timeout(), vec![Action::Bytes(vec![27])]);
    }
}
