//! Local terminal capability/graphics owner. Never writes remote/native bytes as graphics.
use crate::{
    protocol::{self, Packet},
    sixel,
};
use std::io::{self, Write};
pub const QUERY: &[u8] = b"\x1b[c\x1b[16t";
type Decoder = fn(&[u8], (usize, usize)) -> io::Result<sixel::Raster>;
#[derive(Default)]
pub struct Viewer {
    sixel: bool,
    cell: Option<(usize, usize)>,
    pub advertised: bool,
    last: u64,
    image: Option<Image>,
}
impl Drop for Viewer {
    fn drop(&mut self) {
        let _ = self.clear(&mut io::stdout());
    }
}
struct Image {
    id: u64,
    expected: usize,
    bytes: Vec<u8>,
    complete: bool,
    encoded: Option<(String, usize, usize, (usize, usize))>,
}
impl Viewer {
    pub fn restart(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.clear(out)?;
        self.last = 0;
        Ok(())
    }
    pub fn active(&self) -> bool {
        self.image.is_some()
    }
    pub fn cell(&self) -> Option<(usize, usize)> {
        self.cell
    }
    pub fn capable(&self) -> bool {
        self.sixel && self.cell.is_some()
    }
    pub fn terminal(&mut self, bytes: &[u8]) {
        if bytes.len() > 128 {
            return;
        }
        if let Some(parameters) = bytes
            .strip_prefix(b"\x1b[?")
            .and_then(|b| b.strip_suffix(b"c"))
        {
            if let Ok(s) = std::str::from_utf8(parameters) {
                self.sixel = s.split(';').skip(1).any(|n| n == "4");
            }
        } else if let Some(parameters) = bytes
            .strip_prefix(b"\x1b[6;")
            .and_then(|b| b.strip_suffix(b"t"))
            && let Ok(s) = std::str::from_utf8(parameters)
        {
            let Ok(values) = s
                .split(';')
                .map(str::parse::<usize>)
                .collect::<Result<Vec<_>, _>>()
            else {
                return;
            };
            if values.len() == 2 && values.iter().all(|v| (1..=128).contains(v)) {
                let cell = Some((values[1], values[0]));
                if self.cell != cell {
                    self.cell = cell;
                    self.invalidate();
                }
            }
        }
    }
    pub fn invalidate(&mut self) {
        if let Some(image) = &mut self.image {
            image.encoded = None;
        }
    }
    pub fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.image.take().is_some() {
            out.write_all(b"\x1b[?2026l\x1b[2J\x1b[H")?;
            out.flush()?;
        }
        Ok(())
    }
    pub fn packet(&mut self, packet: Packet, out: &mut impl Write) -> io::Result<()> {
        match packet.tag {
            protocol::PREVIEW_BEGIN => {
                if !self.advertised
                    || self.image.is_some()
                    || packet.id <= self.last
                    || packet.data.len() != 8
                {
                    return Err(protocol::invalid("invalid preview offer"));
                }
                let total = u64::from_be_bytes(packet.data.as_slice().try_into().unwrap());
                if !(1..=protocol::IMAGE_LIMIT).contains(&total) {
                    return Err(protocol::invalid("preview exceeds 20 MiB"));
                }
                self.last = packet.id;
                self.image = Some(Image {
                    id: packet.id,
                    expected: total as usize,
                    bytes: Vec::new(),
                    complete: false,
                    encoded: None,
                });
            }
            protocol::PREVIEW_CLEAR => {
                if packet.id != self.last || !packet.data.is_empty() {
                    return Err(protocol::invalid("invalid preview clear"));
                }
                self.clear(out)?;
            }
            protocol::PREVIEW_DATA | protocol::PREVIEW_END => {
                let image = self
                    .image
                    .as_mut()
                    .filter(|p| p.id == packet.id && !p.complete)
                    .ok_or_else(|| protocol::invalid("preview data has no matching offer"))?;
                if packet.tag == protocol::PREVIEW_DATA {
                    if packet.data.len() <= 8 || packet.data.len() > protocol::CHUNK + 8 {
                        return Err(protocol::invalid("invalid preview chunk"));
                    }
                    let offset = u64::from_be_bytes(packet.data[..8].try_into().unwrap());
                    if offset != image.bytes.len() as u64
                        || image.bytes.len() + packet.data.len() - 8 > image.expected
                    {
                        return Err(protocol::invalid("preview chunk outside bounds/order"));
                    }
                    image.bytes.extend_from_slice(&packet.data[8..]);
                } else {
                    if !packet.data.is_empty() || image.bytes.len() != image.expected {
                        return Err(protocol::invalid("incomplete preview"));
                    }
                    image.complete = true;
                }
            }
            _ => return Err(protocol::invalid("unsupported preview packet")),
        }
        Ok(())
    }
    pub fn paint(
        &mut self,
        dimensions: (u16, u16),
        out: &mut impl Write,
        decode: Decoder,
    ) -> io::Result<Option<(u64, String)>> {
        let Some(image) = &mut self.image else {
            return Ok(None);
        };
        if !image.complete {
            return Ok(None);
        }
        let Some(cell) = self.cell else {
            return Ok(None);
        };
        let area = (
            (dimensions.0 as usize).saturating_sub(2) * cell.0,
            (dimensions.1 as usize).saturating_sub(5) * cell.1,
        );
        if image.encoded.as_ref().is_none_or(|e| e.3 != area) {
            let result = crate::image_preview::dimensions(&image.bytes)
                .and_then(|_| decode(&image.bytes, area))
                .and_then(|r| sixel::encode(&r).map(|s| (s, r.width, r.height, area)));
            match result {
                Ok(encoded) => image.encoded = Some(encoded),
                Err(e) => {
                    let id = image.id;
                    self.clear(out)?;
                    return Ok(Some((id, e.to_string())));
                }
            }
        }
        let (s, width, height, _) = image.encoded.as_ref().unwrap();
        let col = (dimensions.0 as usize).saturating_sub(width.div_ceil(cell.0)) / 2 + 1;
        let row = (dimensions.1 as usize).saturating_sub(5 + height.div_ceil(cell.1)) / 2 + 3;
        write!(
            out,
            "\x1b[?2026h\x1b[?25l\x1b7\x1b[{row};{col}H{s}\x1b8\x1b[?2026l"
        )?;
        out.flush()?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(12u32.to_be_bytes());
        b.extend(7u32.to_be_bytes());
        b.resize(33, 0);
        b
    }
    fn decoder(_: &[u8], area: (usize, usize)) -> io::Result<sixel::Raster> {
        let (width, height) = sixel::fit((12, 7), area);
        Ok(sixel::Raster {
            width,
            height,
            rgb: vec![255; width * height * 3],
        })
    }
    fn viewer() -> Viewer {
        let mut v = Viewer::default();
        v.terminal(b"\x1b[?65;1;4;6c");
        v.terminal(b"\x1b[6;20;10t");
        assert!(v.sixel);
        assert_eq!(v.cell, Some((10, 20)));
        assert!(v.capable());
        v.advertised = true;
        v
    }
    fn receive(v: &mut Viewer, id: u64, out: &mut Vec<u8>) {
        v.packet(
            Packet::new(
                protocol::PREVIEW_BEGIN,
                id,
                (png().len() as u64).to_be_bytes(),
            ),
            out,
        )
        .unwrap();
        for (i, byte) in png().iter().enumerate() {
            let mut b = (i as u64).to_be_bytes().to_vec();
            b.push(*byte);
            v.packet(Packet::new(protocol::PREVIEW_DATA, id, b), out)
                .unwrap();
        }
        v.packet(Packet::new(protocol::PREVIEW_END, id, Vec::new()), out)
            .unwrap();
    }
    #[test]
    fn transfer_paint_resize_close_and_refresh_have_single_ownership() {
        let mut v = viewer();
        let mut out = Vec::new();
        receive(&mut v, 1, &mut out);
        assert!(out.is_empty()); // No graphics before a complete validated image.
        assert!(v.paint((236, 54), &mut out, decoder).unwrap().is_none());
        assert!(out.windows(2).any(|b| b == b"\x1bP"));
        out.clear();
        v.invalidate();
        assert!(v.paint((60, 24), &mut out, decoder).unwrap().is_none());
        assert!(out.windows(2).any(|b| b == b"\x1bP"));
        out.clear();
        v.packet(
            Packet::new(protocol::PREVIEW_CLEAR, 1, Vec::new()),
            &mut out,
        )
        .unwrap();
        assert!(out.windows(4).any(|b| b == b"\x1b[2J"));
        out.clear();
        v.paint((60, 24), &mut out, decoder).unwrap();
        assert!(out.is_empty());
        receive(&mut v, 2, &mut out);
        v.restart(&mut out).unwrap();
        receive(&mut v, 1, &mut out); // A new challenge resets connection-owned IDs.
        let failure = |_: &[u8], _: (usize, usize)| Err(io::Error::other("broken decoder"));
        assert_eq!(
            v.paint((60, 24), &mut out, failure).unwrap(),
            Some((1, "broken decoder".into()))
        );
        assert!(!v.active());
    }
    #[test]
    fn capability_and_transfer_bounds_fail_closed() {
        let mut v = Viewer::default();
        let mut out = Vec::new();
        assert!(
            v.packet(
                Packet::new(protocol::PREVIEW_BEGIN, 1, 33u64.to_be_bytes()),
                &mut out
            )
            .is_err()
        );
        v = viewer();
        assert!(
            v.packet(
                Packet::new(
                    protocol::PREVIEW_BEGIN,
                    1,
                    (protocol::IMAGE_LIMIT + 1).to_be_bytes()
                ),
                &mut out
            )
            .is_err()
        );
        v.packet(
            Packet::new(protocol::PREVIEW_BEGIN, 1, 33u64.to_be_bytes()),
            &mut out,
        )
        .unwrap();
        assert!(
            v.packet(Packet::new(protocol::PREVIEW_END, 1, Vec::new()), &mut out)
                .is_err()
        );
        assert!(
            v.packet(Packet::new(protocol::PREVIEW_DATA, 2, vec![0; 9]), &mut out)
                .is_err()
        );
        let mut bad = 1u64.to_be_bytes().to_vec();
        bad.push(0);
        assert!(
            v.packet(Packet::new(protocol::PREVIEW_DATA, 1, bad), &mut out)
                .is_err()
        );
        assert!(
            v.packet(
                Packet::new(protocol::PREVIEW_CLEAR, 2, Vec::new()),
                &mut out
            )
            .is_err()
        );
        assert!(out.is_empty());
        v.clear(&mut out).unwrap();
    }
}
