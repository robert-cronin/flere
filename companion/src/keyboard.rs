//! Track only Arcade's keyboard stack entries in trusted, rendered OUTPUT.
//! The physical terminal belongs to this companion even when SSH disconnects.
use std::io::{self, Write};

const PUSH: &[u8] = b"\x1b[>11u";
const POP: &[u8] = b"\x1b[<u";
const ENTER: &[u8] = b"\x1b[?1049h";
const LEAVE: &[u8] = b"\x1b[?1049l";

#[derive(Default)]
pub struct Output {
    prefix: Vec<u8>,
    requested: usize,
    applied: usize,
    alternate: bool,
    modal: bool,
}
impl Output {
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
        let mut plain = Vec::new();
        for &byte in bytes {
            if self.prefix.is_empty() && byte != 27 {
                if visible {
                    plain.push(byte);
                }
                continue;
            }
            self.prefix.push(byte);
            loop {
                if let Some(index) = [PUSH, POP, ENTER, LEAVE]
                    .iter()
                    .position(|token| *token == self.prefix)
                {
                    out.write_all(&plain)?;
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
                if visible {
                    plain.push(byte);
                }
                if self.prefix.is_empty() {
                    break;
                }
            }
        }
        out.write_all(&plain)?;
        out.flush()
    }
    /// Local forms use ordinary input. Late encoded events remain CSI and are
    /// discarded by their existing parsers, never translated into fresh consent.
    /// A suspended game receives focus-out through the existing ordered key path.
    pub fn modal(&mut self, active: bool, out: &mut impl Write) -> io::Result<bool> {
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
        self.prefix.clear();
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
}
