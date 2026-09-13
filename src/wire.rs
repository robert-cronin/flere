//! Versioned, length-delimited local protocol. No shell parsing or third-party codec.
use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};
// A v5 snapshot can contain two retained 240×100 grids, including 64-byte
// combining cells, plus the existing 8 MiB persisted metadata budget.
pub const MAX: usize = 12 * 1024 * 1024;
pub const MAX_STRING: usize = 2 * 1024 * 1024;
pub const MAX_REQUEST: usize = 128 * 1024;
pub fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
pub fn unhex(s: &str) -> io::Result<Vec<u8>> {
    if s.len() > 131072 || !s.len().is_multiple_of(2) {
        return Err(invalid("invalid hex length"));
    }
    s.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| {
            let d = |b: u8| match b {
                b'0'..=b'9' => Some(b - b'0'),
                b'a'..=b'f' => Some(b - b'a' + 10),
                _ => None,
            };
            Ok(d(p[0]).ok_or_else(|| invalid("invalid hex"))? * 16
                + d(p[1]).ok_or_else(|| invalid("invalid hex"))?)
        })
        .collect()
}
pub fn text(s: &str) -> io::Result<String> {
    String::from_utf8(unhex(s)?).map_err(io::Error::other)
}
pub fn frame(b: &[u8]) -> Vec<u8> {
    let mut v = (b.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(b);
    v
}
pub fn read_frame(s: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut head = [0; 4];
    s.read_exact(&mut head)?;
    let len = u32::from_be_bytes(head) as usize;
    if len > MAX {
        return Err(invalid("frame exceeds bound"));
    }
    let mut b = vec![0; len];
    s.read_exact(&mut b)?;
    Ok(b)
}
pub fn take_frame(buf: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    take_bounded_frame(buf, MAX)
}
pub fn take_request_frame(buf: &mut Vec<u8>) -> io::Result<Option<Vec<u8>>> {
    take_bounded_frame(buf, MAX_REQUEST)
}
fn take_bounded_frame(buf: &mut Vec<u8>, maximum: usize) -> io::Result<Option<Vec<u8>>> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let n = u32::from_be_bytes(buf[..4].try_into().unwrap()) as usize;
    if n > maximum {
        return Err(invalid("frame exceeds bound"));
    }
    if buf.len() < n + 4 {
        return Ok(None);
    }
    let rest = buf.split_off(n + 4);
    let mut b = std::mem::replace(buf, rest);
    b.drain(..4);
    Ok(Some(b))
}
pub fn connect(state: &Path) -> io::Result<UnixStream> {
    let s = UnixStream::connect(state.join("control.sock"))?;
    if !crate::os::same_user(std::os::fd::AsRawFd::as_raw_fd(&s))? {
        return Err(invalid("socket owner differs"));
    }
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(s)
}
pub fn request(state: &Path, fields: &[&str]) -> io::Result<Vec<u8>> {
    let mut s = connect(state)?;
    s.write_all(&frame(fields.join("\t").as_bytes()))?;
    let b = read_frame(&mut s)?;
    if b.first() == Some(&b'!') {
        return Err(io::Error::other(
            String::from_utf8_lossy(&b[1..]).into_owned(),
        ));
    }
    Ok(b)
}
#[derive(Default)]
pub struct Encoder(pub Vec<u8>);
impl Encoder {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn u8(&mut self, n: u8) {
        self.0.push(n)
    }
    pub fn u64(&mut self, n: u64) {
        self.0.extend_from_slice(&n.to_be_bytes())
    }
    pub fn string(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.0.extend_from_slice(s.as_bytes())
    }
}
pub struct Decoder<'a>(pub &'a [u8]);
impl<'a> Decoder<'a> {
    pub fn u8(&mut self) -> io::Result<u8> {
        if self.0.is_empty() {
            return Err(invalid("short message"));
        }
        let n = self.0[0];
        self.0 = &self.0[1..];
        Ok(n)
    }
    pub fn u64(&mut self) -> io::Result<u64> {
        if self.0.len() < 8 {
            return Err(invalid("short message"));
        }
        let n = u64::from_be_bytes(self.0[..8].try_into().unwrap());
        self.0 = &self.0[8..];
        Ok(n)
    }
    pub fn count(&mut self, max: usize) -> io::Result<usize> {
        let n = self.u64()?;
        if n > max as u64 {
            return Err(invalid("count exceeds bound"));
        }
        Ok(n as usize)
    }
    pub fn string(&mut self) -> io::Result<String> {
        let n = self.count(MAX_STRING)?;
        if self.0.len() < n {
            return Err(invalid("short string"));
        }
        let s = std::str::from_utf8(&self.0[..n])
            .map_err(io::Error::other)?
            .into();
        self.0 = &self.0[n..];
        Ok(s)
    }
}
pub fn json(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
pub fn passive(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {
                '�'
            } else {
                c
            }
        })
        .collect()
}
