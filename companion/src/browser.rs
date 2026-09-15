//! Open the exact displayed GitHub target from fresh physical-terminal input.
//! Display packets, remote requests and pasted text can never launch a browser.
use crate::{
    browser_links::{self as links, Context, Region},
    protocol::{self, Packet},
};
use std::{io, time::Instant};

struct Displayed {
    context: Context,
    at: Instant,
}
struct Press {
    identity: String,
    url: String,
    region: Region,
    x: u16,
    y: u16,
}
#[derive(Default)]
pub struct Browser {
    enabled: bool,
    sent: u64,
    last: u64,
    displayed: Option<Displayed>,
    previous: Option<Displayed>,
    press: Option<Press>,
    captured: bool,
    bytes: Vec<u8>,
    since: Option<Instant>,
    paste: bool,
    string: bool,
    tail: bool,
}
pub struct Input {
    pub bytes: Vec<u8>,
    pub open: Option<String>,
}
impl Browser {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn enable(&mut self) {
        if !self.enabled {
            self.reset();
            self.enabled = true;
        }
    }
    // Called before each remote repaint; only its following context can rearm
    // targets. Retain a held mouse press for comparison with that new context.
    pub fn repaint(&mut self) {
        if let Some(displayed) = self.displayed.take() {
            self.previous = Some(displayed);
        }
    }
    pub fn suspend(&mut self) {
        self.repaint();
        self.previous = None;
        self.press = None;
    }
    pub fn context(&mut self, packet: &Packet, size: (u16, u16), visible: bool) -> io::Result<()> {
        if !self.enabled || packet.id == 0 || packet.id <= self.last {
            return Ok(());
        }
        self.last = packet.id;
        let context = Context::decode(&packet.data)?;
        if !visible
            || self.paste
            || self.string
            || self.tail
            || context.input != self.sent
            || (context.width, context.height) != (size.0.clamp(10, 320), size.1.clamp(8, 106))
        {
            self.suspend();
            return Ok(());
        }
        if self.press.as_ref().is_some_and(|p| {
            p.identity != context.identity
                || !context
                    .targets
                    .iter()
                    .any(|t| t.url == p.url && t.region.as_ref() == Some(&p.region))
        }) {
            self.press = None;
        }
        // Background terminal repainting does not change a visible link. Keep
        // its original display time so an input already queued during that
        // repaint remains a fresh action on the same target.
        let at = self
            .displayed
            .take()
            .or_else(|| self.previous.take())
            .filter(|old| old.context == context)
            .map(|old| old.at)
            .unwrap_or_else(Instant::now);
        self.previous = None;
        self.displayed = Some(Displayed { context, at });
        Ok(())
    }
    // The core acknowledges the entire input packet after processing it. A
    // context queued before local navigation cannot rearm the previous target.
    pub fn outgoing(&mut self, packet: &mut Packet) -> io::Result<()> {
        if self.enabled && packet.tag == protocol::KEYS {
            self.sent = self
                .sent
                .checked_add(1)
                .ok_or_else(|| io::Error::other("browser input counter exhausted"))?;
            packet.tag = links::INPUT;
            packet.id = self.sent;
            self.suspend();
        }
        Ok(())
    }
    pub fn input(&mut self, bytes: &[u8], received: Instant) -> Input {
        if !self.enabled {
            return Input {
                bytes: bytes.to_vec(),
                open: None,
            };
        }
        if self.bytes.is_empty() {
            self.since = Some(received);
        }
        self.bytes.extend_from_slice(bytes);
        let mut out = Input {
            bytes: Vec::new(),
            open: None,
        };
        while !self.bytes.is_empty() {
            let Some(n) = token_length(&self.bytes) else {
                break;
            };
            let token: Vec<_> = self.bytes.drain(..n).collect();
            let at = self.since.unwrap_or(received);
            self.since = (!self.bytes.is_empty()).then_some(received);
            if matches!(
                token.as_slice(),
                b"\x1b]" | b"\x1b_" | b"\x1bP" | b"\x1b^" | b"\x1bX"
            ) {
                self.string = true;
            }
            if self.string {
                if matches!(token.as_slice(), b"\x1b\\" | b"\x07" | b"\0") {
                    self.string = false;
                }
                self.suspend();
                out.bytes.extend(token);
                continue;
            }
            if token == b"\x1b[200~" {
                self.paste = true;
            }
            if self.paste {
                if token == b"\x1b[201~" {
                    self.paste = false;
                }
                self.suspend();
                out.bytes.extend(token);
                continue;
            }
            if self.tail {
                // A timed-out escape prefix has already gone to the native
                // parser. Its eventual suffix must stay literal as well.
                if token == b"\0"
                    || token.starts_with(b"\x1b")
                    || token.last().is_some_and(|b| (0x40..=0x7e).contains(b))
                {
                    self.tail = false;
                }
                self.suspend();
                out.bytes.extend(token);
                continue;
            }
            let fresh = self.displayed.as_ref().is_some_and(|d| at > d.at);
            if let Some((button, x, y, release)) = mouse(&token) {
                if button == 0 && !release && fresh {
                    let displayed = self.displayed.as_ref().unwrap();
                    if let Some(target) = displayed
                        .context
                        .targets
                        .iter()
                        .find(|t| t.region.as_ref().is_some_and(|r| r.contains(x, y)))
                    {
                        self.press = Some(Press {
                            identity: displayed.context.identity.clone(),
                            url: target.url.clone(),
                            region: target.region.clone().unwrap(),
                            x,
                            y,
                        });
                        self.captured = true;
                        continue;
                    }
                }
                if self.captured && (release || button & 32 != 0) {
                    if release {
                        self.captured = false;
                        if let Some(p) = self.press.take()
                            && button == 0
                            && (x, y) == (p.x, p.y)
                            && fresh
                            && self.displayed.as_ref().is_some_and(|d| {
                                d.context.identity == p.identity
                                    && d.context.targets.iter().any(|t| {
                                        t.url == p.url && t.region.as_ref() == Some(&p.region)
                                    })
                            })
                        {
                            out.open = Some(p.url);
                            self.suspend();
                        }
                    } else {
                        self.press = None;
                    }
                    continue;
                }
            }
            if fresh
                && let Some(key) = keypress(&token)
                && let Some(target) = self
                    .displayed
                    .as_ref()
                    .unwrap()
                    .context
                    .targets
                    .iter()
                    .find(|t| t.key == Some(key))
            {
                out.open = Some(target.url.clone());
                self.suspend();
                continue;
            }
            self.suspend();
            out.bytes.extend(token);
        }
        out
    }
    pub fn timeout(&mut self) -> Vec<u8> {
        // Incomplete escape input retains normal native Escape behavior, but
        // cannot be reinterpreted as a fresh shortcut after a context arrives.
        let paste_marker = self.bytes.len() >= 3
            && [b"\x1b[200~".as_slice(), b"\x1b[201~"]
                .iter()
                .any(|marker| marker.starts_with(&self.bytes));
        if !paste_marker && self.since.is_some_and(|at| at.elapsed().as_millis() >= 30) {
            self.tail = self.bytes.len() > 1;
            self.suspend();
            self.since = None;
            std::mem::take(&mut self.bytes)
        } else {
            Vec::new()
        }
    }
}
fn token_length(bytes: &[u8]) -> Option<usize> {
    if bytes[0] != 27 {
        return Some(1);
    }
    if bytes.len() == 1 {
        return None;
    }
    if bytes[1] == b'[' {
        bytes[2..]
            .iter()
            .position(|b| (0x40..=0x7e).contains(b))
            .map(|n| n + 3)
            .or_else(|| (bytes.len() >= 128).then_some(bytes.len()))
    } else if bytes[1] == b'O' {
        (bytes.len() >= 3).then_some(3)
    } else {
        Some(2)
    }
}
fn mouse(bytes: &[u8]) -> Option<(u16, u16, u16, bool)> {
    let text = std::str::from_utf8(bytes).ok()?.strip_prefix("\x1b[<")?;
    let release = text.ends_with('m');
    if !release && !text.ends_with('M') {
        return None;
    }
    let mut fields = text[..text.len() - 1].split(';');
    let button = fields.next()?.parse().ok()?;
    let x = fields.next()?.parse::<u16>().ok()?.checked_sub(1)?;
    let y = fields.next()?.parse::<u16>().ok()?.checked_sub(1)?;
    if fields.next().is_some() {
        return None;
    }
    Some((button, x, y, release))
}
fn keypress(bytes: &[u8]) -> Option<u8> {
    if bytes.len() == 1 {
        return matches!(bytes[0], b'i' | b'p' | b'\r').then_some(bytes[0]);
    }
    let text = std::str::from_utf8(bytes)
        .ok()?
        .strip_prefix("\x1b[")?
        .strip_suffix('u')?;
    let mut fields = text.split(';');
    let code = fields.next()?.parse::<u8>().ok()?;
    if let Some(modifier) = fields.next()
        && !matches!(modifier, "1" | "1:1")
    {
        return None;
    }
    if fields.next().is_some() {
        return None;
    }
    matches!(code, b'i' | b'p' | b'\r').then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_links::Target;
    const ISSUE: &str = "https://github.com/demo/fixture/issues/7";
    const PR: &str = "https://github.com/demo/fixture/pull/9";
    fn context(input: u64) -> Context {
        Context {
            input,
            width: 100,
            height: 30,
            identity: "fixture:1".into(),
            targets: vec![
                Target {
                    url: ISSUE.into(),
                    key: Some(b'i'),
                    region: Some(Region {
                        x: 40,
                        y: 5,
                        width: 30,
                    }),
                },
                Target {
                    url: PR.into(),
                    key: Some(b'p'),
                    region: Some(Region {
                        x: 40,
                        y: 6,
                        width: 30,
                    }),
                },
                Target {
                    url: ISSUE.into(),
                    key: Some(b'\r'),
                    region: None,
                },
            ],
        }
    }
    fn arm(browser: &mut Browser, id: u64, context: &Context) {
        browser
            .context(
                &Packet::new(links::CONTEXT, id, serde_json::to_vec(context).unwrap()),
                (100, 30),
                true,
            )
            .unwrap();
    }
    fn ready() -> Browser {
        let mut browser = Browser::default();
        browser.enable();
        arm(&mut browser, 1, &context(0));
        browser
    }
    #[test]
    fn only_fresh_local_shortcuts_activate_displayed_targets() {
        for (key, url) in [
            (b"i".as_slice(), ISSUE),
            (b"p", PR),
            (b"\r", ISSUE),
            (b"\x1b[105;1:1u", ISSUE),
        ] {
            let mut browser = ready();
            let result = browser.input(key, Instant::now());
            assert_eq!(result.open.as_deref(), Some(url));
            assert!(result.bytes.is_empty());
        }
        for key in [
            b"\x1b[105;1:2u".as_slice(),
            b"\x1b[105;1:3u",
            b"\x1b[105;5u",
        ] {
            let mut browser = ready();
            let result = browser.input(key, Instant::now());
            assert!(result.open.is_none());
            assert_eq!(result.bytes, key);
        }
        let before_display = Instant::now();
        let mut browser = ready();
        assert!(browser.input(b"i", before_display).open.is_none());
        browser.reset();
        arm(&mut browser, 2, &context(0));
        assert!(browser.input(b"i", Instant::now()).open.is_none());
    }
    #[test]
    fn background_repaints_retain_fresh_input_only_for_the_same_displayed_target() {
        for changed in [false, true] {
            let mut browser = ready();
            let received = Instant::now();
            browser.repaint();
            let mut c = context(0);
            if changed {
                c.identity = "fixture:2".into();
            }
            arm(&mut browser, 2, &c);
            let result = browser.input(b"p", received);
            assert_eq!(
                result.open.as_deref(),
                if changed { None } else { Some(PR) }
            );
        }
    }
    #[test]
    fn paste_and_terminal_strings_remain_inert_across_every_fragment() {
        for bytes in [
            b"\x1b[200~ip\r\x1b[<0;41;6M\x1b[<0;41;6m\x1b[201~".as_slice(),
            b"\x1b]response ip\r\x07",
            b"\x1bPip\r\x1b\\",
        ] {
            for split in 1..bytes.len() {
                let mut browser = ready();
                let first = browser.input(&bytes[..split], Instant::now());
                arm(&mut browser, 2, &context(0));
                let second = browser.input(&bytes[split..], Instant::now());
                assert!(
                    first.open.is_none() && second.open.is_none(),
                    "split {split}"
                );
                assert_eq!([first.bytes, second.bytes].concat(), bytes);
            }
        }
    }
    #[test]
    fn mouse_needs_matching_press_release_and_unchanged_target() {
        let down = b"\x1b[<0;41;6M";
        let up = b"\x1b[<0;41;6m";
        let mut browser = ready();
        assert!(browser.input(up, Instant::now()).open.is_none());
        for split in 1..down.len() {
            let mut browser = ready();
            assert!(browser.input(&down[..split], Instant::now()).open.is_none());
            assert!(
                browser
                    .input(&down[split..], Instant::now())
                    .bytes
                    .is_empty()
            );
            browser.repaint();
            arm(&mut browser, 2, &context(0));
            assert_eq!(
                browser.input(up, Instant::now()).open.as_deref(),
                Some(ISSUE)
            );
        }
        for mode in 0..5 {
            let mut browser = ready();
            browser.input(down, Instant::now());
            match mode {
                0 => {
                    browser.input(b"\x1b[<32;42;6M", Instant::now());
                }
                1 => {
                    let mut c = context(0);
                    c.identity = "fixture:2".into();
                    arm(&mut browser, 2, &c);
                }
                2 => {
                    let mut c = context(0);
                    c.targets[0].url = PR.into();
                    arm(&mut browser, 2, &c);
                }
                3 => {
                    browser.suspend();
                    arm(&mut browser, 2, &context(0));
                }
                _ => browser.repaint(),
            }
            assert!(browser.input(up, Instant::now()).open.is_none());
        }
    }
    #[test]
    fn timed_out_alt_and_escape_sequences_preserve_native_bytes_without_late_activation() {
        for prefix in [b"\x1bO".as_slice(), b"\x1b["] {
            let mut browser = ready();
            assert!(browser.input(prefix, Instant::now()).bytes.is_empty());
            browser.since = Some(Instant::now() - std::time::Duration::from_millis(31));
            assert_eq!(browser.timeout(), prefix);
            arm(&mut browser, 2, &context(0));
            let continuation = browser.input(b"i", Instant::now());
            assert_eq!(continuation.bytes, b"i");
            assert!(continuation.open.is_none());
            arm(&mut browser, 3, &context(0));
            assert_eq!(
                browser.input(b"i", Instant::now()).open.as_deref(),
                Some(ISSUE)
            );
        }
    }
    #[test]
    fn navigation_acknowledgement_resize_and_hidden_views_fence_stale_contexts() {
        let mut browser = ready();
        let result = browser.input(b"\t", Instant::now());
        let mut packet = Packet::new(protocol::KEYS, 0, result.bytes);
        browser.outgoing(&mut packet).unwrap();
        assert_eq!((packet.tag, packet.id), (links::INPUT, 1));
        browser.outgoing(&mut packet).unwrap(); // A full writer queue must not resequence it.
        arm(&mut browser, 2, &context(0));
        assert!(browser.input(b"p", Instant::now()).open.is_none());
        arm(&mut browser, 3, &context(1));
        assert_eq!(
            browser.input(b"p", Instant::now()).open.as_deref(),
            Some(PR)
        );
        // The core paints the protocol's bounded viewport on larger terminals.
        let mut browser = ready();
        let mut large = context(0);
        large.width = 320;
        large.height = 106;
        browser
            .context(
                &Packet::new(links::CONTEXT, 2, serde_json::to_vec(&large).unwrap()),
                (400, 120),
                true,
            )
            .unwrap();
        assert_eq!(
            browser.input(b"i", Instant::now()).open.as_deref(),
            Some(ISSUE)
        );
        for (size, visible) in [((80, 24), true), ((100, 30), false)] {
            let mut browser = ready();
            let packet = Packet::new(links::CONTEXT, 2, serde_json::to_vec(&context(0)).unwrap());
            browser.context(&packet, size, visible).unwrap();
            assert!(browser.input(b"i", Instant::now()).open.is_none());
        }
    }
}
