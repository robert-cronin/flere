//! Data-only card badge contract. No URLs, terminal escapes or credentials on the wire.
use std::io;
pub const LIMIT: usize = 64;
pub const BYTE_LIMIT: usize = 256 * 1024;
/// Attachment-local image IDs cannot name files, URLs or terminal commands.
pub fn key(value: &str) -> bool {
    value
        .strip_prefix("repo-")
        .is_some_and(|n| (1..=20).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_digit()))
}
pub fn validate(bytes: &[u8]) -> io::Result<()> {
    if bytes.len() > BYTE_LIMIT {
        return Err(io::Error::other("project icon exceeds 256 KiB"));
    }
    let (w, h) = crate::image_preview::dimensions(bytes)?;
    if w > 1024 || h > 1024 {
        return Err(io::Error::other("project icon exceeds 1024 pixels"));
    }
    Ok(())
}
/// One ordered chunk; the full asset never exceeds BYTE_LIMIT.
pub fn chunk(key: &str, total: usize, offset: usize, bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![key.len() as u8];
    out.extend(key.as_bytes());
    out.extend((total as u32).to_be_bytes());
    out.extend((offset as u32).to_be_bytes());
    out.extend(bytes);
    out
}
pub fn read_chunk(data: &[u8]) -> io::Result<(&str, usize, usize, &[u8])> {
    let bad = || io::Error::other("invalid project icon chunk");
    let len = *data.first().ok_or_else(bad)? as usize;
    if data.len() < len + 9 {
        return Err(bad());
    }
    let name = std::str::from_utf8(&data[1..1 + len]).map_err(|_| bad())?;
    let total = u32::from_be_bytes(data[1 + len..5 + len].try_into().unwrap()) as usize;
    let offset = u32::from_be_bytes(data[5 + len..9 + len].try_into().unwrap()) as usize;
    let bytes = &data[9 + len..];
    if !key(name)
        || !(33..=BYTE_LIMIT).contains(&total)
        || bytes.is_empty()
        || offset > total
        || bytes.len() > total - offset
    {
        return Err(bad());
    }
    Ok((name, total, offset, bytes))
}
/// Reserve text columns for the source aspect at one cell's pixel height.
/// Very wide logos still fit the available card; tiny sources are not upscaled.
pub fn columns(source: (u32, u32), cell: (usize, usize), max: usize) -> u16 {
    let height = cell.1.min(source.1 as usize);
    let width = (source.0 as usize * height / source.1 as usize).max(1);
    width.div_ceil(cell.0).clamp(1, max.clamp(1, 32)) as u16
}
/// Pixel metrics are not terminal grid dimensions: preserve widths below ten.
pub fn cell_bytes(size: (usize, usize)) -> io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(4);
    for value in [size.0, size.1] {
        data.extend(
            u16::try_from(value)
                .map_err(io::Error::other)?
                .to_be_bytes(),
        );
    }
    cell(&data)?;
    Ok(data)
}
pub fn cell(data: &[u8]) -> io::Result<(usize, usize)> {
    if data.len() != 4 {
        return Err(io::Error::other("invalid icon cell size"));
    }
    let w = u16::from_be_bytes(data[..2].try_into().unwrap()) as usize;
    let h = u16::from_be_bytes(data[2..].try_into().unwrap()) as usize;
    if !(1..=128).contains(&w) || !(1..=128).contains(&h) {
        return Err(io::Error::other("invalid icon cell size"));
    }
    Ok((w, h))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub x: u16,
    pub y: u16,
    pub columns: u16,
    pub key: String,
    pub bg: [u8; 3],
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub width: u16,
    pub height: u16,
    pub badges: Vec<Badge>,
}
impl Layout {
    pub fn encode(&self) -> Vec<u8> {
        self.encode_slots(false)
    }
    pub fn encode_sized(&self) -> Vec<u8> {
        self.encode_slots(true)
    }
    /// A complete sized layout plus whether preceding output damaged its pixels.
    pub fn encode_frame(&self, damaged: bool) -> Vec<u8> {
        let mut data = vec![u8::from(damaged)];
        data.extend(self.encode_sized());
        data
    }
    pub fn decode_frame(data: &[u8]) -> io::Result<(Self, bool)> {
        let Some((&flag @ 0..=1, layout)) = data.split_first() else {
            return Err(io::Error::other("invalid avatar damage flag"));
        };
        Ok((Self::decode_sized(layout)?, flag == 1))
    }
    fn encode_slots(&self, sized: bool) -> Vec<u8> {
        let mut data = [self.width.to_be_bytes(), self.height.to_be_bytes()].concat();
        for badge in &self.badges {
            data.extend(badge.x.to_be_bytes());
            data.extend(badge.y.to_be_bytes());
            if sized {
                data.extend(badge.columns.to_be_bytes());
            }
            data.extend(badge.bg);
            data.push(badge.key.len() as u8);
            data.extend(badge.key.as_bytes());
        }
        data
    }
    pub fn decode(data: &[u8]) -> io::Result<Self> {
        Self::decode_slots(data, false)
    }
    pub fn decode_sized(data: &[u8]) -> io::Result<Self> {
        Self::decode_slots(data, true)
    }
    fn decode_slots(data: &[u8], sized: bool) -> io::Result<Self> {
        fn bad() -> io::Error {
            io::Error::other("invalid avatar layout")
        }
        if data.len() < 4 {
            return Err(bad());
        }
        let width = u16::from_be_bytes(data[..2].try_into().unwrap());
        let height = u16::from_be_bytes(data[2..4].try_into().unwrap());
        if !(10..=320).contains(&width) || !(8..=106).contains(&height) {
            return Err(bad());
        }
        let mut layout = Self {
            width,
            height,
            badges: Vec::new(),
        };
        let mut rest = &data[4..];
        let extra = if sized { 2 } else { 0 };
        while !rest.is_empty() {
            if rest.len() < 8 + extra || layout.badges.len() >= LIMIT {
                return Err(bad());
            }
            let x = u16::from_be_bytes(rest[..2].try_into().unwrap());
            let y = u16::from_be_bytes(rest[2..4].try_into().unwrap());
            let columns = if sized {
                u16::from_be_bytes(rest[4..6].try_into().unwrap())
            } else {
                2
            };
            let bg = rest[4 + extra..7 + extra].try_into().unwrap();
            let len = rest[7 + extra] as usize;
            if !(1..=32).contains(&columns)
                || x.saturating_add(columns) > width
                || y >= height
                || rest.len() < 8 + extra + len
            {
                return Err(bad());
            }
            let name = std::str::from_utf8(&rest[8 + extra..8 + extra + len]).map_err(|_| bad())?;
            if !key(name)
                || layout
                    .badges
                    .iter()
                    .any(|b| b.y == y && b.x < x + columns && b.x + b.columns > x)
            {
                return Err(bad());
            }
            layout.badges.push(Badge {
                x,
                y,
                bg,
                columns,
                key: name.to_owned(),
            });
            rest = &rest[8 + extra + len..];
        }
        Ok(layout)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_pixel_cells_round_trip_without_grid_dimension_clamping() {
        for size in [(7, 19), (8, 16), (1, 1), (128, 128)] {
            assert_eq!(cell(&cell_bytes(size).unwrap()).unwrap(), size);
        }
        assert_eq!(
            columns((442, 491), cell(&cell_bytes((7, 19)).unwrap()).unwrap(), 12),
            3
        );
        assert_eq!(columns((275, 275), (7, 19), 12), 3);
        for size in [(0, 19), (129, 20), (usize::MAX, 20)] {
            assert!(cell_bytes(size).is_err());
        }
    }
    #[test]
    fn height_driven_slots_follow_source_and_font_with_card_bounds() {
        assert_eq!(columns((442, 491), (10, 25), 10), 3);
        assert_eq!(columns((275, 275), (10, 25), 10), 3);
        assert_eq!(columns((400, 100), (10, 25), 10), 10);
        assert_eq!(columns((400, 100), (10, 25), 5), 5);
        assert_eq!(columns((442, 491), (8, 24), 10), 3);
        assert_eq!(columns((12, 7), (10, 25), 10), 2);
        assert_eq!(cell(&[0, 10, 0, 25]).unwrap(), (10, 25));
        for invalid in [
            &[0, 0, 0, 25][..],
            &[0, 10, 0, 129],
            &[0, 10, 0],
            &[0, 10, 0, 25, 1],
        ] {
            assert!(cell(invalid).is_err());
        }
    }
    #[test]
    fn sized_layout_rejects_overlap_and_clipping() {
        let mut layout = Layout {
            width: 60,
            height: 24,
            badges: vec![Badge {
                x: 3,
                y: 5,
                columns: 4,
                key: "repo-1".into(),
                bg: [18, 26, 41],
            }],
        };
        assert_eq!(
            Layout::decode_sized(&layout.encode_sized()).unwrap(),
            layout
        );
        assert!(Layout::decode(&layout.encode_sized()).is_err());
        let mut next = layout.badges[0].clone();
        next.x = 6;
        layout.badges.push(next);
        assert!(Layout::decode_sized(&layout.encode_sized()).is_err());
        layout.badges[1].x = 7;
        assert!(Layout::decode_sized(&layout.encode_sized()).is_ok());
        for columns in [0, 33, 59, u16::MAX] {
            layout.badges[1].columns = columns;
            assert!(Layout::decode_sized(&layout.encode_sized()).is_err());
        }
    }
    #[test]
    fn badge_layout_rejects_overflow_overlap_and_control_strings() {
        let layout = Layout {
            width: 60,
            height: 24,
            badges: vec![Badge {
                x: 3,
                y: 5,
                columns: 2,
                key: "repo-1".into(),
                bg: [18, 26, 41],
            }],
        };
        assert_eq!(Layout::decode(&layout.encode()).unwrap(), layout);
        let mut bad = layout.clone();
        bad.badges[0].x = 59;
        assert!(Layout::decode(&bad.encode()).is_err());
        bad = layout.clone();
        bad.badges.push(bad.badges[0].clone());
        assert!(Layout::decode(&bad.encode()).is_err());
        bad = layout;
        bad.badges[0].key = "a\x1b[2J".into();
        assert!(Layout::decode(&bad.encode()).is_err());
    }
}
