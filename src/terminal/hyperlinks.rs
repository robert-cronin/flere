//! Passive, bounded HTTP(S) annotations. This module never opens a URL or emits OSC.
use super::{Cell, Grid};
use crate::wire::{Decoder, Encoder, invalid};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, de};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, Weak},
};

pub const URI_BYTES: usize = 2048;
pub const LIVE_BYTES: usize = 64 * 1024;
pub const LINK_COUNT: usize = 1024;
pub const SIDECAR_BYTES: usize = 64 * 1024;
pub(super) const OSC_BYTES: usize = URI_BYTES + 256;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hyperlink(Arc<str>);
impl Hyperlink {
    pub fn new(uri: &str) -> Option<Self> {
        valid_uri(uri).then(|| Self(Arc::from(uri)))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
fn valid_uri(uri: &str) -> bool {
    if uri.len() > URI_BYTES || uri.chars().any(|c| c.is_control() || c.is_whitespace()
        || matches!(c, '\\' | '"' | '<' | '>' | '`' | '{' | '}' | '|' | '^' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return false;
    }
    let Some((scheme, rest)) = uri.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // This is an emission safety boundary, not a replacement for the browser's
    // URL parser. Preserve userinfo, internationalized hosts and unusual valid
    // authorities; only the outer terminal/browser interprets an activated URL.
    if authority.is_empty() {
        return false;
    }
    let mut bytes = uri.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%'
            && !(bytes.next().is_some_and(|b| b.is_ascii_hexdigit())
                && bytes.next().is_some_and(|b| b.is_ascii_hexdigit()))
        {
            return false;
        }
    }
    true
}
impl Serialize for Hyperlink {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
impl<'de> Deserialize<'de> for Hyperlink {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let uri = String::deserialize(deserializer)?;
        Self::new(&uri).ok_or_else(|| de::Error::custom("invalid passive HTTP(S) hyperlink"))
    }
}

/// Weak entries retain no live link ownership. Pruning happens on a new OSC8,
/// never once per character. Immutable cells and cloned grids share URI storage.
#[derive(Clone, Default)]
pub(super) struct Pool(Vec<Weak<str>>);
impl Pool {
    pub(super) fn intern(&mut self, uri: &str) -> Option<Hyperlink> {
        if !valid_uri(uri) {
            return None;
        }
        self.0.retain(|entry| entry.strong_count() != 0);
        let mut bytes = 0;
        for entry in &self.0 {
            if let Some(value) = entry.upgrade() {
                if &*value == uri {
                    return Some(Hyperlink(value));
                }
                bytes += value.len();
            }
        }
        if self.0.len() >= LINK_COUNT || bytes + uri.len() > LIVE_BYTES {
            return None;
        }
        let value: Arc<str> = Arc::from(uri);
        self.0.push(Arc::downgrade(&value));
        Some(Hyperlink(value))
    }
}

/// The sidecar is a deduplicated string table followed by big-endian u16 cell
/// indices (zero means no link). An empty table uses an empty index list.
/// Overflow drops only annotations; callers retain every ordinary cell.
pub fn encode_links(e: &mut Encoder, cells: &[Cell]) {
    encode_budget(e, cells, SIDECAR_BYTES);
}
pub(super) fn encode_budget(e: &mut Encoder, cells: &[Cell], budget: usize) {
    let mut urls = Vec::new();
    let mut lookup: HashMap<&Hyperlink, u16> = HashMap::new();
    let mut indices = Vec::with_capacity(cells.len());
    let mut used = 16usize.saturating_add(cells.len().saturating_mul(2));
    for cell in cells {
        let index = if let Some(link) = &cell.link {
            if let Some(index) = lookup.get(link) {
                *index
            } else if urls.len() < LINK_COUNT
                && used.saturating_add(8 + link.as_str().len()) <= budget
            {
                used += 8 + link.as_str().len();
                urls.push(link);
                let index = urls.len() as u16;
                lookup.insert(link, index);
                index
            } else {
                0
            }
        } else {
            0
        };
        indices.push(index);
    }
    e.u64(urls.len() as u64);
    for url in &urls {
        e.string(url.as_str());
    }
    e.u64(if urls.is_empty() {
        0
    } else {
        cells.len() as u64
    });
    if !urls.is_empty() {
        for index in indices {
            e.0.extend_from_slice(&index.to_be_bytes());
        }
    }
}
pub fn decode_links(d: &mut Decoder<'_>, cells: &mut [Cell]) -> io::Result<()> {
    decode_budget(d, cells, SIDECAR_BYTES)
}
pub(super) fn decode_budget(
    d: &mut Decoder<'_>,
    cells: &mut [Cell],
    budget: usize,
) -> io::Result<()> {
    let initial = d.0.len();
    let count = d.count(LINK_COUNT)?;
    let mut urls = Vec::with_capacity(count);
    let mut unique = HashSet::new();
    for _ in 0..count {
        let length = d.count(URI_BYTES)?;
        if length > d.0.len() || initial - d.0.len() + length > budget {
            return Err(invalid("hyperlink sidecar exceeds bound"));
        }
        let uri =
            std::str::from_utf8(&d.0[..length]).map_err(|_| invalid("invalid hyperlink UTF-8"))?;
        let link =
            Hyperlink::new(uri).ok_or_else(|| invalid("invalid passive HTTP(S) hyperlink"))?;
        if !unique.insert(link.clone()) {
            return Err(invalid("duplicate hyperlink table entry"));
        }
        urls.push(link);
        d.0 = &d.0[length..];
    }
    let n = d.count(cells.len())?;
    if n != if count == 0 { 0 } else { cells.len() } {
        return Err(invalid("invalid hyperlink index count"));
    }
    let length = n
        .checked_mul(2)
        .ok_or_else(|| invalid("hyperlink index overflow"))?;
    if length > d.0.len() || initial - d.0.len() + length > budget {
        return Err(invalid("hyperlink sidecar exceeds bound"));
    }
    let indices = &d.0[..length];
    if indices
        .as_chunks::<2>()
        .0
        .iter()
        .any(|b| usize::from(u16::from_be_bytes([b[0], b[1]])) > urls.len())
    {
        return Err(invalid("hyperlink index exceeds table"));
    }
    for cell in cells.iter_mut() {
        cell.link = None;
    }
    for (cell, bytes) in cells.iter_mut().zip(indices.as_chunks::<2>().0.iter()) {
        let index = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
        cell.link = index.checked_sub(1).map(|i| urls[i].clone());
    }
    d.0 = &d.0[length..];
    Ok(())
}

// Keep the legacy grid cell representation intact. Link strings appear once in
// a bounded packed sidecar; absent sidecars load old refresh JSON unchanged.
#[derive(Serialize, Deserialize)]
struct PlainCell {
    text: String,
    width: u8,
    style: super::Style,
}
#[derive(Serialize, Deserialize)]
struct SavedGrid {
    cols: usize,
    rows: usize,
    cells: Vec<PlainCell>,
    x: usize,
    y: usize,
    top: usize,
    bottom: usize,
    wrap: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    links: String,
}
impl Serialize for Grid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut links = Encoder::new();
        encode_links(&mut links, &self.cells);
        SavedGrid {
            cols: self.cols,
            rows: self.rows,
            cells: self
                .cells
                .iter()
                .map(|c| PlainCell {
                    text: c.text.clone(),
                    width: c.width,
                    style: c.style,
                })
                .collect(),
            x: self.x,
            y: self.y,
            top: self.top,
            bottom: self.bottom,
            wrap: self.wrap,
            links: if self.cells.iter().any(|c| c.link.is_some()) {
                STANDARD.encode(links.0)
            } else {
                String::new()
            },
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Grid {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let saved = SavedGrid::deserialize(deserializer)?;
        if saved.cols < 2
            || saved.cols > 240
            || saved.rows < 2
            || saved.rows > 100
            || saved.cells.len() != saved.cols * saved.rows
            || saved.x >= saved.cols
            || saved.y >= saved.rows
            || saved.top > saved.bottom
            || saved.bottom >= saved.rows
            || saved.cells.iter().any(|c| c.width > 2 || c.text.len() > 64)
        {
            return Err(de::Error::custom("invalid terminal grid"));
        }
        let mut cells: Vec<_> = saved
            .cells
            .into_iter()
            .map(|c| Cell {
                text: c.text,
                width: c.width,
                style: c.style,
                link: None,
            })
            .collect();
        if !saved.links.is_empty() {
            if saved.links.len() > SIDECAR_BYTES.div_ceil(3) * 4 {
                return Err(de::Error::custom("oversized hyperlink sidecar"));
            }
            let bytes = STANDARD.decode(saved.links).map_err(de::Error::custom)?;
            if bytes.len() > SIDECAR_BYTES {
                return Err(de::Error::custom("oversized hyperlink sidecar"));
            }
            let mut d = Decoder(&bytes);
            decode_links(&mut d, &mut cells).map_err(de::Error::custom)?;
            if !d.0.is_empty() {
                return Err(de::Error::custom("trailing hyperlink sidecar bytes"));
            }
        }
        Ok(Grid {
            cols: saved.cols,
            rows: saved.rows,
            cells,
            x: saved.x,
            y: saved.y,
            top: saved.top,
            bottom: saved.bottom,
            wrap: saved.wrap,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::Terminal;
    fn open(uri: &str) -> Vec<u8> {
        format!("\x1b]8;id=ignored;{uri}\x1b\\").into_bytes()
    }
    #[test]
    fn only_bounded_passive_http_targets_are_accepted() {
        for uri in [
            "https://github.com/a/b#readme",
            "HTTP://localhost:8080/a%20b?q=x&y=2",
            "https://[::1]:443/x",
            "https://example.com/世界",
            "https://例え.テスト/資料",
            "https://user:password@example.com/",
        ] {
            assert!(Hyperlink::new(uri).is_some(), "{uri}");
        }
        for uri in [
            "",
            "https://",
            "https:///x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@b.test",
            "https://example.com\\@other.test",
            "https://x.test/\x1b]52;c;BAD\x07",
            "https://x.test/\u{009c}",
            "https://x.test/\u{202e}evil",
            "https://x.test/a b",
            "https://x.test/%zz",
        ] {
            assert!(Hyperlink::new(uri).is_none(), "{uri:?}");
            assert!(serde_json::from_value::<Hyperlink>(serde_json::json!(uri)).is_err());
        }
        assert!(Hyperlink::new(&format!("https://e.test/{}", "x".repeat(URI_BYTES))).is_none());
    }
    #[test]
    fn fragmented_osc8_unicode_sgr_and_closing_link_preserve_shared_cells() {
        let bytes = [
            open("https://e.test/a"),
            "界e\u{301}".as_bytes().to_vec(),
            b"\x1b[31mR\x1b]8;;\x07plain".to_vec(),
        ]
        .concat();
        for split in 0..=bytes.len() {
            let mut t = Terminal::new(30, 3);
            t.feed(&bytes[..split]);
            let mut t: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
            t.feed(&bytes[split..]);
            assert_eq!(t.grid.line(0), "界e\u{301}Rplain");
            assert!(t.grid.cells[..4].iter().all(|c| {
                c.link
                    .as_ref()
                    .is_some_and(|l| l.as_str() == "https://e.test/a")
            }));
            assert!(Arc::ptr_eq(
                &t.grid.cells[0].link.as_ref().unwrap().0,
                &t.grid.cells[1].link.as_ref().unwrap().0
            ));
            assert!(t.grid.cells[4..].iter().all(|c| c.link.is_none()));
            assert!(t.replies.is_empty() && t.valid_state());
        }
    }
    #[test]
    fn invalid_cancelled_or_oversized_osc8_closes_link_unknown_strings_stay_inert() {
        let invalid = [
            b"\x1b]8;missing\x07".to_vec(),
            open("javascript:bad"),
            b"\x1b]8;;https://e.test/\x18".to_vec(),
            b"\x1b]8;;\xff\x07".to_vec(),
            [
                b"\x1b]8;;https://e.test/".as_slice(),
                &vec![b'x'; OSC_BYTES + 20],
                b"\x1b\\",
            ]
            .concat(),
            b"\x1b]8;;https://e.test/\x1bXjunk\x1b\\".to_vec(),
        ];
        for bytes in invalid {
            let mut t = Terminal::new(20, 3);
            t.feed(&open("https://e.test/good"));
            t.feed(b"a");
            t.feed(&bytes);
            t.feed(b"b");
            assert_eq!(t.grid.line(0), "ab");
            assert!(t.grid.cells[0].link.is_some() && t.grid.cells[1].link.is_none());
            assert!(t.replies.is_empty() && t.valid_state());
        }
        let mut t = Terminal::new(20, 3);
        t.feed(&open("https://e.test/good"));
        t.feed(b"\x1b]52;c;secret\x07\x1b]80;unknown\x07\x1bP8;;https://evil.test/\x1b\\ok");
        assert_eq!(t.grid.line(0), "ok");
        assert_eq!(
            t.grid.cells[0].link.as_ref().unwrap().as_str(),
            "https://e.test/good"
        );
        assert!(t.replies.is_empty());
    }
    #[test]
    fn erase_save_alternate_and_reset_keep_link_ownership_explicit() {
        let mut t = Terminal::new(20, 3);
        t.feed(&open("https://e.test/primary"));
        t.feed(b"a\x1b7\x1b]8;;\x07b\x1b8c");
        assert_eq!(t.grid.line(0), "ac");
        assert!(t.grid.cells[1].link.is_some());
        t.feed(b"\x1b[?1049hZ");
        assert!(t.grid.cells[0].link.is_none());
        t.feed(&open("https://e.test/alternate"));
        t.feed(b"Y\x1b[?1049lD");
        assert_eq!(
            t.grid.cells[2].link.as_ref().unwrap().as_str(),
            "https://e.test/primary"
        );
        t.feed(b"\r\x1b[2K");
        assert!(t.grid.cells.iter().all(|c| c.link.is_none()));
        t.feed(b"E"); // erase removes cell metadata, not the currently selected OSC8.
        assert!(t.grid.cells[0].link.is_some());
        t.feed(b"\x1bcF");
        assert!(t.grid.cells.iter().all(|c| c.link.is_none()));
        assert!(t.active_link.is_none() && t.saved_link.is_none() && t.primary_link.is_none());
    }
    #[test]
    fn live_link_count_and_uri_bytes_are_bounded_and_dead_entries_are_reclaimed() {
        let mut t = Terminal::new(240, 5);
        for i in 0..LINK_COUNT + 1 {
            t.feed(&open(&format!("https://e.test/{i}")));
            t.feed(b"x");
        }
        assert!(t.grid.cells[LINK_COUNT - 1].link.is_some());
        assert!(t.grid.cells[LINK_COUNT].link.is_none());
        assert!(t.valid_state());
        t.feed(b"\x1b[2J\x1b[H");
        t.feed(&open("https://e.test/reused"));
        t.feed(b"y");
        assert!(t.grid.cells[0].link.is_some());
        let mut t = Terminal::new(80, 3);
        for i in 0..33 {
            let prefix = format!("https://e.test/{i:02}/");
            t.feed(&open(&format!(
                "{prefix}{}",
                "x".repeat(URI_BYTES - prefix.len())
            )));
            t.feed(b"x");
        }
        assert!(t.grid.cells[31].link.is_some() && t.grid.cells[32].link.is_none());
        assert!(t.valid_state());
    }
    #[test]
    fn codec_deduplicates_bounds_and_rejects_malformed_tables_transactionally() {
        let link = Hyperlink::new("https://e.test/long").unwrap();
        let cells = vec![
            Cell {
                link: Some(link),
                ..Cell::default()
            };
            24_000
        ];
        let mut e = Encoder::new();
        encode_links(&mut e, &cells);
        assert!(e.0.len() <= SIDECAR_BYTES);
        assert_eq!(
            e.0.windows(b"https://e.test/long".len())
                .filter(|w| *w == b"https://e.test/long")
                .count(),
            1
        );
        let mut restored = vec![Cell::default(); cells.len()];
        decode_links(&mut Decoder(&e.0), &mut restored).unwrap();
        assert_eq!(restored, cells);
        assert!(Arc::ptr_eq(
            &restored[0].link.as_ref().unwrap().0,
            &restored[23999].link.as_ref().unwrap().0
        ));
        let mut corrupt = Encoder::new();
        corrupt.u64(1);
        corrupt.string("javascript:bad");
        corrupt.u64(1);
        corrupt.0.extend([0, 1]);
        let mut bad_index = Encoder::new();
        bad_index.u64(1);
        bad_index.string("https://e.test/");
        bad_index.u64(1);
        bad_index.0.extend([0, 2]);
        for bytes in [&corrupt.0[..], &bad_index.0, &e.0[..10]] {
            let mut cell = cells[..1].to_vec();
            assert!(decode_links(&mut Decoder(bytes), &mut cell).is_err());
            assert_eq!(cell, cells[..1]);
        }
        let many: Vec<_> = (0..24_000)
            .map(|i| Cell {
                text: "x".into(),
                link: Hyperlink::new(&format!("https://e.test/{i}/{}", "z".repeat(1500))),
                ..Cell::default()
            })
            .collect();
        let mut e = Encoder::new();
        encode_links(&mut e, &many);
        assert!(e.0.len() <= SIDECAR_BYTES);
        let mut decoded = many.clone();
        decode_links(&mut Decoder(&e.0), &mut decoded).unwrap();
        assert!(
            decoded.iter().any(|c| c.link.is_some()) && decoded.iter().any(|c| c.link.is_none())
        );
        assert!(decoded.iter().all(|c| c.text == "x"));
    }
    #[test]
    fn grid_json_uses_one_sidecar_and_old_refresh_loads_without_links() {
        let mut t = Terminal::new(80, 25);
        t.feed(&open("https://e.test/shared"));
        t.feed(&vec![b'x'; 1900]);
        let saved = serde_json::to_value(&t).unwrap();
        assert!(
            saved["grid"]["cells"]
                .as_array()
                .unwrap()
                .iter()
                .all(|c| c.get("link").is_none())
        );
        assert!(saved["grid"]["links"].as_str().unwrap().len() < 6000);
        let restored: Terminal = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(restored.grid.cells, t.grid.cells);
        assert!(Arc::ptr_eq(
            &restored.active_link.as_ref().unwrap().0,
            &restored.grid.cells[0].link.as_ref().unwrap().0
        ));
        let mut old = saved;
        old["grid"].as_object_mut().unwrap().remove("links");
        for field in ["active_link", "saved_link", "primary_link"] {
            old.as_object_mut().unwrap().remove(field);
        }
        let restored: Terminal = serde_json::from_value(old).unwrap();
        assert!(restored.grid.cells.iter().all(|c| c.link.is_none()));
        assert_eq!(restored.grid.line(0), t.grid.line(0));
    }
    #[test]
    fn restored_primary_and_alternate_share_one_live_metadata_budget() {
        let mut t = Terminal::new(80, 25);
        t.primary = Some(Grid::new(80, 25));
        for (prefix, grid) in [("a", &mut t.grid), ("b", t.primary.as_mut().unwrap())] {
            for i in 0..30 {
                let stem = format!("https://e.test/{prefix}/{i}/");
                grid.cells[i].text = "x".into();
                grid.cells[i].link =
                    Hyperlink::new(&format!("{stem}{}", "z".repeat(2000 - stem.len())));
            }
        }
        assert!(!t.links_valid());
        let restored: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert!(restored.valid_state());
        assert_eq!(restored.grid.line(0), t.grid.line(0));
        assert_eq!(
            restored.primary.as_ref().unwrap().line(0),
            t.primary.as_ref().unwrap().line(0)
        );
        assert!(
            restored.primary.as_ref().unwrap().cells[..30]
                .iter()
                .any(|c| c.link.is_none())
        );
    }
}
