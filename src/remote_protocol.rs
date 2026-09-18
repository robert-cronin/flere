//! Small SSH stdio protocol shared with the optional local companion.
//! This carries Flere-rendered UI, never raw child-program output.
use std::io::{self, Read, Write};
pub const VERSION: &[u8] = b"flere-remote-v6";
pub const SCREENSHOT_VERSION: &[u8] = b"flere-remote-v5";
pub const SIZED_VERSION: &[u8] = b"flere-remote-v4";
pub const SCREENSHOT_CAP: &[u8] = b"clipboard-screenshot-v1";
pub const SCREENSHOT_BEGIN: u8 = 27;
pub const SCREENSHOT_DATA: u8 = 28;
pub const SCREENSHOT_END: u8 = 29;
pub const SCREENSHOT_CANCEL: u8 = 30;
pub const SCREENSHOT_RESULT: u8 = 31;
pub const AVATAR_VERSION: &[u8] = b"flere-remote-v3";
pub const LEGACY_VERSION: &[u8] = b"flere-remote-v2";
pub const AVATAR_LAYOUT: u8 = 23;
pub const AVATAR_CLEAR: u8 = 24;
pub const AVATAR_IMAGE: u8 = 25;
pub const AVATAR_LAYOUT_SIZED: u8 = 26;
pub const AVATAR_FRAME: u8 = 55;
pub const AVATAR_DAMAGE_CAP: &[u8] = b"sixel-project-icons-damage-v1";
pub const AVATAR_SIZE_CAP: &[u8] = b"sixel-project-icons-v2";
pub const AVATAR_CAP: &[u8] = b"sixel-project-icons-v1";
pub const MAX: usize = 65_536;
pub const CHUNK: usize = 48 * 1024;
pub const IMAGE_LIMIT: u64 = 20 * 1024 * 1024;
pub const HELLO: u8 = 1;
pub const KEYS: u8 = 2;
pub const RESIZE: u8 = 3;
pub const IMAGE: u8 = 4;
pub const DATA: u8 = 5;
pub const END: u8 = 6;
pub const CANCEL: u8 = 7;
pub const NOTICE: u8 = 8;
pub const CAPABILITIES: u8 = 9;
pub const PREVIEW_ERROR: u8 = 10;
pub const PREVIEW_BEGIN: u8 = 19;
pub const PREVIEW_DATA: u8 = 20;
pub const PREVIEW_END: u8 = 21;
pub const PREVIEW_CLEAR: u8 = 22;
pub const PREVIEW_CAP: &[u8] = b"sixel-preview-v1";
pub const OUTPUT: u8 = 16;
pub const READY: u8 = 17;
pub const RESULT: u8 = 18;
#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub tag: u8,
    pub id: u64,
    pub data: Vec<u8>,
}
pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
impl Packet {
    pub fn new(tag: u8, id: u64, data: impl Into<Vec<u8>>) -> Self {
        Self {
            tag,
            id,
            data: data.into(),
        }
    }
    pub fn write(&self, writer: &mut impl Write) -> io::Result<()> {
        let len = self
            .data
            .len()
            .checked_add(9)
            .ok_or_else(|| invalid("remote frame too large"))?;
        if len > MAX {
            return Err(invalid("remote frame too large"));
        }
        writer.write_all(&(len as u32).to_be_bytes())?;
        writer.write_all(&[self.tag])?;
        writer.write_all(&self.id.to_be_bytes())?;
        writer.write_all(&self.data)?;
        writer.flush()
    }
    pub fn read(reader: &mut impl Read) -> io::Result<Self> {
        let mut head = [0; 4];
        reader.read_exact(&mut head)?;
        let n = u32::from_be_bytes(head) as usize;
        if !(9..=MAX).contains(&n) {
            return Err(invalid("invalid remote frame length"));
        }
        let mut data = vec![0; n];
        reader.read_exact(&mut data)?;
        Ok(Self {
            tag: data[0],
            id: u64::from_be_bytes(data[1..9].try_into().unwrap()),
            data: data[9..].to_vec(),
        })
    }
    pub fn take(buffer: &mut Vec<u8>) -> io::Result<Option<Self>> {
        if buffer.len() < 4 {
            return Ok(None);
        }
        let n = u32::from_be_bytes(buffer[..4].try_into().unwrap()) as usize;
        if !(9..=MAX).contains(&n) {
            return Err(invalid("invalid remote frame length"));
        }
        if buffer.len() < n + 4 {
            return Ok(None);
        }
        let packet = Self::read(&mut &buffer[..n + 4])?;
        buffer.drain(..n + 4);
        Ok(Some(packet))
    }
}
pub fn dimensions(data: &[u8]) -> io::Result<(u16, u16)> {
    if data.len() != 4 {
        return Err(invalid("remote size requires two dimensions"));
    }
    let w = u16::from_be_bytes(data[..2].try_into().unwrap());
    let h = u16::from_be_bytes(data[2..].try_into().unwrap());
    if !(10..=320).contains(&w) || !(8..=106).contains(&h) {
        return Err(invalid("remote size outside supported bounds"));
    }
    Ok((w, h))
}
pub fn size_bytes(width: u16, height: u16) -> Vec<u8> {
    [
        width.clamp(10, 320).to_be_bytes(),
        height.clamp(8, 106).to_be_bytes(),
    ]
    .concat()
}

/// Echo a per-attachment challenge so a refresh cannot accept an old queued handshake.
pub fn hello_reply(hello: &Packet, width: u16, height: u16) -> io::Result<Packet> {
    if hello.tag != HELLO
        || hello.id != 0
        || hello.data.len() != VERSION.len() + 32
        || !(hello.data.starts_with(VERSION)
            || hello.data.starts_with(SCREENSHOT_VERSION)
            || hello.data.starts_with(SIZED_VERSION)
            || hello.data.starts_with(AVATAR_VERSION)
            || hello.data.starts_with(LEGACY_VERSION))
        || !hello.data[VERSION.len()..]
            .iter()
            .all(u8::is_ascii_hexdigit)
    {
        return Err(invalid("incompatible Flere remote companion"));
    }
    let mut data = hello.data.clone();
    data.extend(size_bytes(width, height));
    Ok(Packet::new(HELLO, 0, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_accepts_current_and_prior_servers_but_not_unknown_protocols() {
        for version in [
            VERSION,
            SCREENSHOT_VERSION,
            SIZED_VERSION,
            AVATAR_VERSION,
            LEGACY_VERSION,
        ] {
            let mut data = version.to_vec();
            data.extend([b'a'; 32]);
            let hello = Packet::new(HELLO, 0, data.clone());
            let reply = hello_reply(&hello, 209, 49).unwrap();
            data.extend(size_bytes(209, 49));
            assert_eq!(reply.data, data);
        }
        let mut data = b"flere-remote-v7".to_vec();
        data.extend([b'a'; 32]);
        assert!(hello_reply(&Packet::new(HELLO, 0, data), 209, 49).is_err());
    }
}
