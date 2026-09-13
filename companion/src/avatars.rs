//! Attachment-local project images. No HTTP or persistent private-repository image cache.
use crate::{
    avatar::{self, Layout},
    sixel,
};
use std::{
    collections::HashMap,
    io::{self, Write},
};
type Decode = fn(&[u8], (usize, usize)) -> io::Result<sixel::Raster>;
struct Pending {
    key: String,
    total: usize,
    bytes: Vec<u8>,
}
struct Tile {
    cell: (usize, usize),
    bg: [u8; 3],
    columns: u16,
    encoded: String,
}
#[derive(Default)]
pub struct Avatars {
    layout: Layout,
    bytes: HashMap<String, Vec<u8>>,
    encoded: HashMap<String, Tile>,
    pending: Option<Pending>,
    dirty: bool,
    pending_frame: bool,
}
impl Drop for Avatars {
    fn drop(&mut self) {
        let _ = self.clear(&mut io::stdout());
    }
}
impl Avatars {
    pub fn receive(&mut self, data: &[u8]) -> io::Result<()> {
        let (key, total, offset, chunk) = avatar::read_chunk(data)?;
        if offset == 0 {
            if self.pending.is_some()
                || self.bytes.len() >= avatar::LIMIT
                || self.bytes.contains_key(key)
            {
                return Err(io::Error::other("duplicate or excessive project image"));
            }
            self.pending = Some(Pending {
                key: key.into(),
                total,
                bytes: Vec::new(),
            });
        }
        let pending = self
            .pending
            .as_mut()
            .ok_or_else(|| io::Error::other("project image missing start"))?;
        if pending.key != key || pending.total != total || pending.bytes.len() != offset {
            return Err(io::Error::other("out-of-order project image"));
        }
        pending.bytes.extend(chunk);
        if pending.bytes.len() == total {
            let pending = self.pending.take().unwrap();
            // Invalid image data retains initials; it cannot terminate the connection.
            if avatar::validate(&pending.bytes).is_ok() {
                self.bytes.insert(pending.key, pending.bytes);
                self.dirty = true;
            }
        }
        Ok(())
    }
    pub fn begin_output(&mut self) {
        self.pending_frame = true;
    }
    pub fn frame(&mut self, data: &[u8], dimensions: (u16, u16)) -> io::Result<()> {
        self.set_frame(Layout::decode(data)?, dimensions)
    }
    pub fn sized_frame(&mut self, data: &[u8], dimensions: (u16, u16)) -> io::Result<()> {
        self.set_frame(Layout::decode_sized(data)?, dimensions)
    }
    fn set_frame(&mut self, layout: Layout, dimensions: (u16, u16)) -> io::Result<()> {
        self.pending_frame = false;
        // Resize can race queued old frames. Wait for a matching complete frame.
        self.layout = if (layout.width, layout.height) == dimensions {
            layout
        } else {
            Layout::default()
        };
        self.dirty = true;
        Ok(())
    }
    pub fn suspend(&mut self) {
        self.layout = Layout::default();
        self.dirty = false;
        self.pending_frame = false;
    }
    pub fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        if !self.layout.badges.is_empty() {
            out.write_all(b"\x1b[2J")?;
            out.flush()?;
        }
        self.suspend();
        Ok(())
    }
    pub fn paint(
        &mut self,
        cell: Option<(usize, usize)>,
        out: &mut impl Write,
        decode: Decode,
    ) -> io::Result<()> {
        let Some(cell) = cell else {
            return Ok(());
        };
        if self.pending_frame || !self.dirty || self.layout.badges.is_empty() {
            return Ok(());
        }
        self.dirty = false;
        let mut paint = String::new();
        for b in &self.layout.badges {
            let Some(bytes) = self.bytes.get(&b.key) else {
                continue;
            };
            let key = format!("{}-{:?}", b.key, b.bg);
            if self
                .encoded
                .get(&key)
                .is_none_or(|t| t.cell != cell || t.bg != b.bg || t.columns != b.columns)
            {
                let width = (cell.0 * usize::from(b.columns)).min(1280);
                let height = cell.1;
                let encoded = (|| {
                    let image = decode(bytes, (width, height))?;
                    if image.width > width
                        || image.height > height
                        || image.rgb.len() != image.width * image.height * 3
                    {
                        return Err(io::Error::other("invalid avatar decoder output"));
                    }
                    let mut rgb = b.bg.repeat(width * height);
                    let x = (width - image.width) / 2;
                    let y = (height - image.height) / 2;
                    for row in 0..image.height {
                        let dest = ((y + row) * width + x) * 3;
                        let source = row * image.width * 3;
                        rgb[dest..dest + image.width * 3]
                            .copy_from_slice(&image.rgb[source..source + image.width * 3]);
                    }
                    sixel::encode(&sixel::Raster { width, height, rgb })
                })();
                let Ok(encoded) = encoded else {
                    continue;
                };
                if self.encoded.len() >= avatar::LIMIT * 2 {
                    self.encoded.clear();
                }
                self.encoded.insert(
                    key.clone(),
                    Tile {
                        cell,
                        bg: b.bg,
                        columns: b.columns,
                        encoded,
                    },
                );
            }
            use std::fmt::Write as _;
            let _ = write!(
                paint,
                "\x1b[{};{}H{}",
                b.y + 1,
                b.x + 1,
                self.encoded[&key].encoded
            );
        }
        if !paint.is_empty() {
            // Save/restore position without changing native cursor visibility.
            write!(out, "\x1b[?2026h\x1b7{paint}\x1b8\x1b[?2026l")?;
            out.flush()?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(12u32.to_be_bytes());
        b.extend(12u32.to_be_bytes());
        b.resize(33, 0);
        b
    }
    fn decode(_: &[u8], area: (usize, usize)) -> io::Result<sixel::Raster> {
        Ok(sixel::Raster {
            width: area.0,
            height: area.1,
            rgb: vec![255; area.0 * area.1 * 3],
        })
    }
    fn layout(x: u16) -> Layout {
        Layout {
            width: 60,
            height: 24,
            badges: vec![avatar::Badge {
                x,
                y: 5,
                columns: 2,
                key: "repo-1".into(),
                bg: [18, 26, 41],
            }],
        }
    }
    #[test]
    fn portrait_square_and_wide_icons_keep_aspect_in_height_driven_slots() {
        fn decode(bytes: &[u8], area: (usize, usize)) -> io::Result<sixel::Raster> {
            let source = crate::image_preview::dimensions(bytes)?;
            let (width, height) = sixel::fit(source, area);
            // The real decoder must fit the original aspect, not stretch to the slot.
            let expected = match source {
                (442, 491) => (22, 25),
                (275, 275) => (25, 25),
                _ => (100, 25),
            };
            assert_eq!((width, height), expected);
            Ok(sixel::Raster {
                width,
                height,
                rgb: vec![255; width * height * 3],
            })
        }
        for source in [(442u32, 491u32), (275, 275), (400, 100)] {
            let mut v = Avatars::default();
            let mut p = png();
            p[16..20].copy_from_slice(&source.0.to_be_bytes());
            p[20..24].copy_from_slice(&source.1.to_be_bytes());
            let mut l = layout(3);
            l.badges[0].columns = avatar::columns(source, (10, 25), 10);
            v.sized_frame(&l.encode_sized(), (60, 24)).unwrap();
            v.receive(&avatar::chunk("repo-1", p.len(), 0, &p)).unwrap();
            let mut out = Vec::new();
            v.paint(Some((10, 25)), &mut out, decode).unwrap();
            let text = String::from_utf8(out).unwrap();
            assert!(text.contains(&format!(
                "\x1bP7;1q\"1;1;{};25",
                usize::from(l.badges[0].columns) * 10
            )));
        }
    }
    #[test]
    fn late_images_use_current_layout_and_wait_for_complete_frames() {
        let mut v = Avatars::default();
        let mut out = Vec::new();
        v.frame(&layout(3).encode(), (60, 24)).unwrap();
        v.frame(&layout(18).encode(), (60, 24)).unwrap();
        let p = png();
        v.receive(&avatar::chunk("repo-1", p.len(), 0, &p)).unwrap();
        v.paint(Some((10, 20)), &mut out, decode).unwrap();
        let s = String::from_utf8(out.clone()).unwrap();
        assert!(s.contains("\x1b[6;19H"));
        assert!(!s.contains("\x1b[6;4H"));
        assert!(!s.contains("?25"));
        assert!(s.contains("\x1b7") && s.contains("\x1b8"));
        out.clear();
        v.begin_output();
        v.dirty = true;
        v.paint(Some((10, 20)), &mut out, decode).unwrap();
        assert!(out.is_empty());
        v.frame(&layout(3).encode(), (61, 24)).unwrap();
        v.paint(Some((10, 20)), &mut out, decode).unwrap();
        assert!(out.is_empty());
        v.frame(&layout(3).encode(), (60, 24)).unwrap();
        v.clear(&mut out).unwrap();
        assert_eq!(out, b"\x1b[2J");
        out.clear();
        v.paint(Some((10, 20)), &mut out, decode).unwrap();
        assert!(out.is_empty());
    }
    #[test]
    fn image_chunks_require_order_bounds_and_valid_data() {
        let mut v = Avatars::default();
        let p = png();
        v.receive(&avatar::chunk("repo-1", p.len(), 0, &p[..10]))
            .unwrap();
        assert!(
            v.receive(&avatar::chunk("repo-1", p.len(), 11, &p[11..]))
                .is_err()
        );
        v.receive(&avatar::chunk("repo-1", p.len(), 10, &p[10..]))
            .unwrap();
        assert_eq!(v.bytes["repo-1"], p);
        assert!(v.receive(&avatar::chunk("repo-1", p.len(), 0, &p)).is_err());
        assert!(
            v.receive(&avatar::chunk("../private", p.len(), 0, &p))
                .is_err()
        );
        assert!(
            v.receive(&avatar::chunk("repo-2", avatar::BYTE_LIMIT + 1, 0, &p))
                .is_err()
        );
        v.receive(&avatar::chunk("repo-2", 33, 0, &[0; 33]))
            .unwrap();
        assert!(!v.bytes.contains_key("repo-2"));
        for i in 2..=avatar::LIMIT {
            v.receive(&avatar::chunk(&format!("repo-{i}"), p.len(), 0, &p))
                .unwrap();
        }
        assert!(
            v.receive(&avatar::chunk("repo-65", p.len(), 0, &p))
                .is_err()
        );
    }
}
