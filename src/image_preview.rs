//! Header bounds shared by the remote reader and the local OS decoder.
//! This is not an image codec: platform preview decoders or the portable screenshot
//! renderer validate/decode the complete file.
use std::io;
pub const PIXELS: u64 = 20_000_000;
pub fn dimensions(bytes: &[u8]) -> io::Result<(u32, u32)> {
    let size = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.len() < 33 || &bytes[8..16] != b"\0\0\0\rIHDR" {
            return Err(io::Error::other("invalid PNG header"));
        }
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    } else if bytes.starts_with(b"\xff\xd8") {
        let mut i = 2;
        let mut size = None;
        while i < bytes.len() {
            if bytes[i] != 0xff {
                break;
            }
            while i < bytes.len() && bytes[i] == 0xff {
                i += 1;
            }
            let Some(&marker) = bytes.get(i) else { break };
            i += 1;
            if marker == 0xda || marker == 0xd9 || marker == 0 {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            let Some(length) = bytes.get(i..i + 2) else {
                break;
            };
            let n = u16::from_be_bytes(length.try_into().unwrap()) as usize;
            if n < 2 || i + n > bytes.len() {
                break;
            }
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                if n < 8 {
                    break;
                }
                size = Some((
                    u16::from_be_bytes(bytes[i + 5..i + 7].try_into().unwrap()) as u32,
                    u16::from_be_bytes(bytes[i + 3..i + 5].try_into().unwrap()) as u32,
                ));
                break;
            }
            i += n;
        }
        size.ok_or_else(|| io::Error::other("invalid JPEG header"))?
    } else {
        return Err(io::Error::other("Preview supports PNG and JPEG images"));
    };
    if size.0 == 0 || size.1 == 0 || size.0 as u64 * size.1 as u64 > PIXELS {
        return Err(io::Error::other("image exceeds 20 million pixels"));
    }
    Ok(size)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_headers_reject_truncation_zero_and_pixel_bombs() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(100u32.to_be_bytes());
        png.extend(50u32.to_be_bytes());
        png.resize(33, 0);
        assert_eq!(dimensions(&png).unwrap(), (100, 50));
        for n in 0..33 {
            assert!(dimensions(&png[..n]).is_err());
        }
        png[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(dimensions(&png).is_err());
        png[16..20].fill(0);
        assert!(dimensions(&png).is_err());
        let jpg = b"\xff\xd8\xff\xe0\0\x04ab\xff\xc0\0\x0b\x08\0\x32\0\x64\x01\x01\x11\0";
        assert_eq!(dimensions(jpg).unwrap(), (100, 50));
        for n in 0..jpg.len() {
            assert!(dimensions(&jpg[..n]).is_err());
        }
        assert!(dimensions(b"\xff\xd8\xff\xe0\0\0").is_err());
    }
}
