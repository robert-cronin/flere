//! Optional v6 companion tools. The bridge owns request IDs and remote origins;
//! the companion requires fresh local consent for local resources and applications.
//! Terminal bytes never create requests. All transfer bodies remain bounded.
use serde_json::{Value, json};
use std::io;
pub const CAPABILITY: &[u8] = b"remote-tools-v1";
// Old v6 cores display NOTICE and reject unknown capabilities. Probe through that
// existing harmless message; only a new core acknowledges this optional feature.
pub const DROP_PROBE: &[u8] = b"[flere capability probe: chat-drop-v1]";
pub const DROP_CAPABILITY: &[u8] = b"chat-drop-v1";
pub const DROP_OFFER: u8 = 49;
pub const DROP_TEXT: u8 = 50;
pub const DROP_RESULT: u8 = 51;
pub const DROP_CONTEXT: u8 = 52;
pub const REQUEST: u8 = 32;
pub const METADATA: u8 = 33;
pub const READY: u8 = 34;
pub const DATA: u8 = 35;
pub const END: u8 = 36;
pub const CANCEL: u8 = 37;
pub const RESULT: u8 = 38;
pub const PORTS_REQUEST: u8 = 39;
pub const PORTS_RESULT: u8 = 40;
pub const FILE_LIMIT: u64 = 128 * 1024 * 1024;
pub const CHUNK: usize = 48 * 1024;
pub const IDLE_SECONDS: u64 = 120;
pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub fn printable(s: &str) -> bool {
    !s.is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control)
}
pub fn name(s: &str) -> io::Result<()> {
    if !printable(s)
        || s.len() > 255
        || s == "."
        || s == ".."
        || s.contains(['/', '\\', ':', '<', '>', '"', '|', '?', '*'])
        || s.ends_with(['.', ' '])
    {
        return Err(invalid(
            "file name must be one printable portable path component",
        ));
    }
    let stem = s.split('.').next().unwrap().to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'))
    {
        return Err(invalid("reserved local file name"));
    }
    Ok(())
}
pub fn metadata(name: &str, size: u64) -> io::Result<Vec<u8>> {
    self::name(name)?;
    if size > FILE_LIMIT {
        return Err(invalid("file exceeds 128 MiB"));
    }
    serde_json::to_vec(&json!({"name":name,"size":size})).map_err(io::Error::other)
}
pub fn decode(data: &[u8]) -> io::Result<Value> {
    if data.len() > 8192 {
        return Err(invalid("remote tool metadata exceeds 8 KiB"));
    }
    serde_json::from_slice(data).map_err(io::Error::other)
}
pub fn file_metadata(data: &[u8]) -> io::Result<(String, u64)> {
    let value = decode(data)?;
    let filename = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing file name"))?;
    name(filename)?;
    let size = value
        .get("size")
        .and_then(Value::as_u64)
        .filter(|n| *n <= FILE_LIMIT)
        .ok_or_else(|| invalid("invalid file size"))?;
    Ok((filename.into(), size))
}
pub fn chunk(offset: u64, bytes: &[u8]) -> io::Result<Vec<u8>> {
    if bytes.is_empty()
        || bytes.len() > CHUNK
        || offset.saturating_add(bytes.len() as u64) > FILE_LIMIT
    {
        return Err(invalid("invalid transfer chunk"));
    }
    let mut data = offset.to_be_bytes().to_vec();
    data.extend_from_slice(bytes);
    Ok(data)
}
pub fn unchunk(data: &[u8]) -> io::Result<(u64, &[u8])> {
    if !(9..=CHUNK + 8).contains(&data.len()) {
        return Err(invalid("invalid transfer chunk"));
    }
    let offset = u64::from_be_bytes(data[..8].try_into().unwrap());
    if offset.saturating_add((data.len() - 8) as u64) > FILE_LIMIT {
        return Err(invalid("transfer offset exceeds bound"));
    }
    Ok((offset, &data[8..]))
}
pub fn result(success: bool, message: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"success":success,"message":message.chars().filter(|c| !c.is_control()).take(512).collect::<String>()})).unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_names_and_ordered_chunks_do_not_accept_paths_or_unbounded_data() {
        for bad in [
            "../secret",
            "/absolute",
            "C:\\secret",
            "x/y",
            "NUL",
            "com1.txt",
            "tail.",
            "bad\nname",
        ] {
            assert!(name(bad).is_err(), "{bad}");
        }
        assert!(name("report ü 1.html").is_ok());
        assert!(metadata("empty.txt", 0).is_ok());
        assert!(metadata("large.txt", FILE_LIMIT + 1).is_err());
        assert_eq!(
            unchunk(&chunk(12, b"part").unwrap()).unwrap(),
            (12, b"part".as_slice())
        );
        assert!(chunk(FILE_LIMIT, b"x").is_err());
    }
}
