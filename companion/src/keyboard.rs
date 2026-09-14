//! Track keyboard ownership and OSC boundaries in trusted, rendered OUTPUT.
//! The physical terminal belongs to this companion even when SSH disconnects.
use std::io::{self, Write};

const PUSH: &[u8] = b"\x1b[>11u";
const POP: &[u8] = b"\x1b[<u";
const ENTER: &[u8] = b"\x1b[?1049h";
const LEAVE: &[u8] = b"\x1b[?1049l";
// Cancel a partial parser sequence before terminating any string and closing its
// hyperlink. Local safety text must never become part of a remote OSC target.
const CLOSE_LINK: &[u8] = b"\x18\x1b\\\x1b]8;;\x1b\\";

#[derive(Default)]
struct Osc {
    field: u8,
    target: bool,
    escape: bool,
    hidden: bool,
}
impl Osc {
    /// Only track OSC 8's fields; URL validation belongs to the core renderer.
    /// Other OSCs retain their existing policy, including explicit UI copies.
    fn feed(&mut self, byte: u8) -> Option<Option<bool>> {
        if matches!(byte, 0x18 | 0x1a) {
            return Some(None);
        }
        if byte == 7 || self.escape && byte == b'\\' {
            return Some((self.field == 3).then_some(self.target));
        }
        if self.escape {
            self.field = 4;
        }
        self.escape = byte == 27;
        if !self.escape {
            match (self.field, byte) {
                (0, b'8') => self.field = 1,
                (1 | 2, b';') => self.field += 1,
                (2, _) => {}
                (3, _) => self.target = true,
                _ => self.field = 4,
            }
        }
        None
    }
}

#[derive(Default)]
pub struct Output {
    prefix: Vec<u8>,
    prefix_visible: bool,
    osc: Option<Osc>,
    link_open: bool,
    write_failed: bool,
    requested: usize,
    applied: usize,
    alternate: bool,
    modal: bool,
}
/// A no-op local paint must not interrupt a remote sequence split across packets.
/// Cancel/close only when the local renderer actually writes its first bytes.
pub struct LocalOutput<'a, W> {
    owner: &'a mut Output,
    out: &'a mut W,
}
impl<W: Write> Write for LocalOutput<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !bytes.is_empty() {
            self.owner.interrupt(self.out)?;
        }
        self.out.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}
impl Output {
    pub fn local_output<'a, W: Write>(&'a mut self, out: &'a mut W) -> LocalOutput<'a, W> {
        LocalOutput { owner: self, out }
    }
    fn interrupt(&mut self, out: &mut impl Write) -> io::Result<()> {
        // Retain the remote parser until its terminator: a later URL tail must
        // not be painted as ordinary local text after this cancellation.
        if self.write_failed || self.link_open || self.osc.as_ref().is_some_and(|osc| !osc.hidden) {
            out.write_all(CLOSE_LINK)?;
        }
        self.write_failed = false;
        self.link_open = false;
        self.prefix_visible = false;
        if let Some(osc) = &mut self.osc {
            osc.hidden = true;
        }
        Ok(())
    }
    fn paint(&mut self, bytes: &[u8], out: &mut impl Write) -> io::Result<()> {
        if let Err(error) = out.write_all(bytes) {
            self.write_failed = true;
            return Err(error);
        }
        Ok(())
    }
    fn pop(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.applied != 0 {
            out.write_all(POP)?;
            self.applied -= 1;
        }
        Ok(())
    }
    fn apply(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.alternate && !self.modal {
            while self.applied < self.requested {
                out.write_all(PUSH)?;
                self.applied += 1;
            }
        }
        Ok(())
    }
    /// Buffered prefixes never reach the terminal half-written. Hidden frames
    /// still retire remote-owned entries, so a local prompt cannot hide cleanup.
    pub fn write(&mut self, bytes: &[u8], visible: bool, out: &mut impl Write) -> io::Result<()> {
        if !visible {
            self.interrupt(out)?;
        }
        let mut plain = Vec::new();
        for &byte in bytes {
            if let Some(osc) = &mut self.osc {
                if visible && !osc.hidden {
                    plain.push(byte);
                }
                if let Some(link) = osc.feed(byte) {
                    if !osc.hidden
                        && let Some(open) = link
                    {
                        self.link_open = open;
                    }
                    self.osc = None;
                }
                continue;
            }
            if self.prefix.is_empty() && byte != 27 {
                if visible {
                    plain.push(byte);
                }
                continue;
            }
            if self.prefix.is_empty() {
                self.prefix_visible = visible;
            } else {
                self.prefix_visible &= visible;
            }
            self.prefix.push(byte);
            if self.prefix == b"\x1b]" {
                let hidden = !self.prefix_visible;
                if !hidden {
                    plain.extend_from_slice(&self.prefix);
                }
                self.prefix.clear();
                self.osc = Some(Osc {
                    hidden,
                    ..Osc::default()
                });
                continue;
            }
            loop {
                if let Some(index) = [PUSH, POP, ENTER, LEAVE]
                    .iter()
                    .position(|token| *token == self.prefix)
                {
                    self.paint(&plain, out)?;
                    plain.clear();
                    self.prefix.clear();
                    match index {
                        0 => {
                            if self.requested == 16 {
                                return Err(io::Error::other(
                                    "Arcade keyboard stack exceeds bound",
                                ));
                            }
                            self.requested += 1;
                            if visible {
                                self.apply(out)?;
                            }
                        }
                        1 => {
                            self.requested = self.requested.saturating_sub(1);
                            if self.applied > self.requested {
                                self.pop(out)?;
                            }
                        }
                        2 if visible => {
                            out.write_all(ENTER)?;
                            self.alternate = true;
                            self.apply(out)?;
                        }
                        3 => {
                            self.interrupt(out)?;
                            self.requested = 0;
                            while self.applied != 0 {
                                self.pop(out)?;
                            }
                            if visible {
                                out.write_all(LEAVE)?;
                                self.alternate = false;
                            }
                        }
                        _ => {}
                    }
                    break;
                }
                if [PUSH, POP, ENTER, LEAVE]
                    .iter()
                    .any(|token| token.starts_with(&self.prefix))
                {
                    break;
                }
                let byte = self.prefix.remove(0);
                if visible && self.prefix_visible {
                    plain.push(byte);
                }
                if self.prefix.is_empty() {
                    break;
                }
            }
        }
        self.paint(&plain, out)?;
        out.flush()
    }
    /// Local forms use ordinary input. Late encoded events remain CSI and are
    /// discarded by their existing parsers, never translated into fresh consent.
    /// A suspended game receives focus-out through the existing ordered key path.
    pub fn modal(&mut self, active: bool, out: &mut impl Write) -> io::Result<bool> {
        if active {
            self.interrupt(out)?;
        }
        if self.modal == active {
            return Ok(false);
        }
        self.modal = active;
        let pause = active && self.applied != 0;
        if active {
            while self.applied != 0 {
                self.pop(out)?;
            }
        } else {
            self.apply(out)?;
        }
        out.flush()?;
        Ok(pause)
    }
    /// Called before HELLO reinitialization and before alternate-screen teardown.
    /// Never pop an entry if its complete push did not reach this terminal.
    pub fn reset(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.interrupt(out)?;
        self.prefix.clear();
        self.osc = None;
        self.requested = 0;
        while self.applied != 0 {
            self.pop(out)?;
        }
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_frames_preserve_output_and_only_pop_owned_entries() {
        let frame = [ENTER, b"paint\x1b[31m", PUSH, b"more", POP, LEAVE].concat();
        for split in 0..=frame.len() {
            let mut keyboard = Output::default();
            let mut output = Vec::new();
            keyboard.write(&frame[..split], true, &mut output).unwrap();
            keyboard.write(&frame[split..], true, &mut output).unwrap();
            keyboard.reset(&mut output).unwrap();
            assert_eq!(output, frame, "split {split}");
        }
        let mut keyboard = Output::default();
        let mut output = Vec::new();
        keyboard.write(POP, true, &mut output).unwrap();
        keyboard.reset(&mut output).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn disconnect_and_remote_screen_exit_restore_mode_before_shell_screen() {
        let mut keyboard = Output::default();
        let mut output = Vec::new();
        for byte in [ENTER, PUSH].concat() {
            keyboard.write(&[byte], true, &mut output).unwrap();
            assert!(keyboard.prefix.len() < ENTER.len());
        }
        keyboard.reset(&mut output).unwrap();
        assert_eq!(output, [ENTER, PUSH, POP].concat());
        keyboard.reset(&mut output).unwrap();
        assert_eq!(output, [ENTER, PUSH, POP].concat());
        keyboard.write(PUSH, true, &mut output).unwrap();
        keyboard.write(LEAVE, true, &mut output).unwrap();
        keyboard.reset(&mut output).unwrap();
        assert!(output.ends_with(&[PUSH, POP, LEAVE].concat()));
        let length = output.len();
        keyboard.write(POP, true, &mut output).unwrap();
        keyboard.reset(&mut output).unwrap();
        assert_eq!(output.len(), length);
    }

    #[test]
    fn local_modal_suspends_game_and_hidden_cleanup_prevents_reactivation() {
        let mut keyboard = Output::default();
        let mut output = Vec::new();
        keyboard
            .write(&[ENTER, PUSH].concat(), true, &mut output)
            .unwrap();
        assert!(keyboard.modal(true, &mut output).unwrap());
        assert!(!keyboard.modal(true, &mut output).unwrap());
        assert!(output.ends_with(POP));
        keyboard.modal(false, &mut output).unwrap();
        assert!(output.ends_with(PUSH));
        assert!(keyboard.modal(true, &mut output).unwrap());
        let length = output.len();
        keyboard
            .write(&[b"hidden frame", POP].concat(), false, &mut output)
            .unwrap();
        keyboard.modal(false, &mut output).unwrap();
        keyboard.reset(&mut output).unwrap();
        assert_eq!(output.len(), length);
    }
    const OPEN: &[u8] = b"\x1b]8;;https://example.test/path?a=1&b=2\x1b\\";
    const CLOSE: &[u8] = b"\x1b]8;;\x1b\\";

    #[test]
    fn rendered_links_and_explicit_ui_clipboard_copy_survive_every_packet_split() {
        // Native OSC 52 is rejected by the core emulator. An explicit UI copy
        // is trusted OUTPUT and must retain that existing remote policy.
        let clipboard = b"\x1b]52;c;c2VsZWN0ZWQ=\x07";
        let target = format!("https://example.test/{}", "x".repeat(2027));
        assert_eq!(target.len(), 2048);
        let long_open = format!("\x1b]8;;{target}\x1b\\");
        for open in [OPEN, long_open.as_bytes()] {
            let frame = [ENTER, open, "linked λ".as_bytes(), CLOSE, clipboard, LEAVE].concat();
            for split in 0..=frame.len() {
                let mut keyboard = Output::default();
                let mut output = Vec::new();
                keyboard.write(&frame[..split], true, &mut output).unwrap();
                // This is the real call made after each visible OUTPUT packet.
                // A preview with nothing to paint must not interrupt a split OSC.
                crate::preview::Viewer::default()
                    .paint((80, 24), &mut keyboard.local_output(&mut output), |_, _| {
                        panic!("empty preview must not decode")
                    })
                    .unwrap();
                keyboard.write(&frame[split..], true, &mut output).unwrap();
                keyboard.reset(&mut output).unwrap();
                assert_eq!(output, frame, "split {split}");
            }
        }
    }

    #[test]
    fn disconnect_cancels_partial_osc_before_safety_text_and_reset_is_idempotent() {
        for split in 1..=OPEN.len() {
            let mut keyboard = Output::default();
            let mut output = Vec::new();
            keyboard.write(&OPEN[..split], true, &mut output).unwrap();
            let emitted = output.len();
            keyboard.reset(&mut output).unwrap();
            let reset = output.len();
            keyboard.reset(&mut output).unwrap();
            assert_eq!(output.len(), reset);
            output.extend_from_slice(b"SAFE: connection closed");
            if split == 1 {
                // The lone ESC was buffered and never reached the host parser.
                assert_eq!(output, b"SAFE: connection closed");
            } else {
                assert_eq!(
                    &output[emitted..],
                    [CLOSE_LINK, b"SAFE: connection closed"].concat()
                );
            }
        }
    }

    #[test]
    fn first_local_prompt_write_closes_links_before_modal_tick_and_discards_url_tail() {
        for split in 1..=OPEN.len() {
            let mut keyboard = Output::default();
            let mut output = Vec::new();
            keyboard.write(&OPEN[..split], true, &mut output).unwrap();
            let emitted = output.len();
            let mut gate = crate::local_tools::Gate::default();
            let request = crate::protocol::Packet::new(
                crate::remote_services::REQUEST,
                1,
                br#"{"op":"upload","destination":"remote-file"}"#,
            );
            assert!(gate.offer(request).unwrap().is_none());
            assert!(!keyboard.modal);
            // Main draws this first prompt before its next modal-state tick.
            gate.draw((80, 24), &mut keyboard.local_output(&mut output))
                .unwrap();
            let prompt = &output[emitted..];
            if split > 1 {
                assert!(prompt.starts_with(CLOSE_LINK));
            }
            assert!(String::from_utf8_lossy(prompt).contains("Upload local file"));
            let prompt_end = output.len();
            keyboard.write(&OPEN[split..], true, &mut output).unwrap();
            assert_eq!(output.len(), prompt_end, "interrupted URL tail at {split}");
            keyboard
                .write(&[b"plain", CLOSE].concat(), true, &mut output)
                .unwrap();
            assert_eq!(&output[prompt_end..], [b"plain", CLOSE].concat());
            assert!(!keyboard.link_open);
        }
    }

    #[test]
    fn hidden_osc_tails_never_reactivate_links_or_keyboard_modes() {
        let sequence = [OPEN, b"hidden", CLOSE].concat();
        for split in 1..sequence.len() {
            let mut keyboard = Output::default();
            let mut output = Vec::new();
            keyboard
                .write(&sequence[..split], true, &mut output)
                .unwrap();
            keyboard.modal(true, &mut output).unwrap();
            let boundary = output.len();
            keyboard
                .write(&sequence[split..], false, &mut output)
                .unwrap();
            keyboard.modal(false, &mut output).unwrap();
            keyboard.reset(&mut output).unwrap();
            assert_eq!(output.len(), boundary, "hidden split {split}");
            assert!(!keyboard.link_open);
        }
        let mut keyboard = Output::default();
        let mut output = Vec::new();
        let osc = [b"\x1b]52;c;", PUSH, b"\x07"].concat();
        keyboard.write(&osc, true, &mut output).unwrap();
        assert_eq!(output, osc);
        assert_eq!(keyboard.requested, 0);
    }

    #[test]
    fn partial_output_write_failure_still_forces_parser_cleanup() {
        struct FailsAfter {
            bytes: Vec<u8>,
            remaining: usize,
        }
        impl Write for FailsAfter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.remaining == 0 {
                    return Err(io::Error::other("fixture output failure"));
                }
                let n = bytes.len().min(self.remaining);
                self.bytes.extend_from_slice(&bytes[..n]);
                self.remaining -= n;
                Ok(n)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut keyboard = Output::default();
        let mut output = FailsAfter {
            bytes: Vec::new(),
            remaining: OPEN.len() - 1,
        };
        assert!(
            keyboard
                .write(&[OPEN, b"label", CLOSE].concat(), true, &mut output)
                .is_err()
        );
        let emitted = output.bytes.len();
        output.remaining = usize::MAX;
        keyboard.reset(&mut output).unwrap();
        output.write_all(b"SAFE").unwrap();
        assert_eq!(&output.bytes[emitted..], [CLOSE_LINK, b"SAFE"].concat());
    }
}
