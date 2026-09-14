//! Compact, bounded styled history. Decode only the requested rows; cloning for
//! refresh shares immutable row bytes instead of cloning every cell/string.
use super::{Cell, Color, Style, char_width};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, de, ser::SerializeSeq};
use std::{collections::VecDeque, sync::Arc};

pub const HISTORY_ROWS: usize = 100_000;
pub const HISTORY_BYTES: usize = 32 * 1024 * 1024;
const ROW_BYTES: usize = 20_000;

#[derive(Clone)]
pub struct HistoryRow(Arc<[u8]>);
impl HistoryRow {
    pub(crate) fn bytes(&self) -> usize {
        self.0.len()
    }
    pub fn from_cells(cells: &[Cell]) -> Self {
        debug_assert!(cells.len() <= 240);
        let mut data = Vec::with_capacity(2 + cells.len() * 3);
        data.extend_from_slice(&[2, cells.len() as u8]);
        let mut first = 0;
        while first < cells.len() {
            let style = cells[first].style;
            let mut end = first + 1;
            while end < cells.len() && cells[end].style == style {
                end += 1;
            }
            data.push((end - first) as u8);
            put_color(&mut data, style.fg);
            put_color(&mut data, style.bg);
            data.push(
                style.bold as u8
                    | (style.dim as u8) << 1
                    | (style.italic as u8) << 2
                    | (style.strikethrough as u8) << 3
                    | (style.underline as u8) << 4
                    | (style.inverse as u8) << 5,
            );
            for cell in &cells[first..end] {
                debug_assert!(cell.width <= 2 && cell.text.len() <= 64);
                data.extend_from_slice(&[cell.width, cell.text.len() as u8]);
                data.extend_from_slice(cell.text.as_bytes());
            }
            first = end;
        }
        let remaining = ROW_BYTES.saturating_sub(data.len());
        let mut encoded = crate::wire::Encoder(data);
        super::hyperlinks::encode_budget(&mut encoded, cells, remaining);
        debug_assert!(encoded.0.len() <= ROW_BYTES);
        Self(encoded.0.into())
    }
    pub fn cells(&self) -> Vec<Cell> {
        self.decode_cells().expect("validated history row")
    }
    fn decode_cells(&self) -> Result<Vec<Cell>, &'static str> {
        let mut cells = Vec::with_capacity(usize::from(*self.0.get(1).ok_or("short history row")?));
        let tail = self.visit(|text, width, style| {
            cells.push(Cell {
                text: text.into(),
                width,
                style,
                link: None,
            })
        })?;
        if self.0[0] == 2 {
            let mut decoder = crate::wire::Decoder(tail);
            super::hyperlinks::decode_budget(&mut decoder, &mut cells, tail.len())
                .map_err(|_| "invalid history hyperlinks")?;
            if !decoder.0.is_empty() {
                return Err("trailing history hyperlink bytes");
            }
        }
        Ok(cells)
    }
    pub fn text(&self) -> String {
        let mut line = String::new();
        self.visit(|text, width, _| {
            if width > 0 {
                line.push_str(text);
            }
        })
        .expect("validated history row");
        line.truncate(line.trim_end().len());
        line
    }
    fn visit(&self, mut cell: impl FnMut(&str, u8, Style)) -> Result<&[u8], &'static str> {
        let mut r = Reader(&self.0);
        let version = r.byte()?;
        if !matches!(version, 1 | 2) {
            return Err("unknown history row version");
        }
        let mut remaining = r.byte()? as usize;
        if remaining > 240 {
            return Err("oversized history row");
        }
        while remaining > 0 {
            let n = r.byte()? as usize;
            if n == 0 || n > remaining {
                return Err("invalid history style run");
            }
            let fg = r.color()?;
            let bg = r.color()?;
            let flags = r.byte()?;
            if flags > 63 {
                return Err("invalid history flags");
            }
            let style = Style {
                fg,
                bg,
                bold: flags & 1 != 0,
                dim: flags & 2 != 0,
                italic: flags & 4 != 0,
                strikethrough: flags & 8 != 0,
                underline: flags & 16 != 0,
                inverse: flags & 32 != 0,
            };
            for _ in 0..n {
                let width = r.byte()?;
                let len = r.byte()? as usize;
                if width > 2 || len > 64 || len > r.0.len() {
                    return Err("invalid history cell");
                }
                let text = std::str::from_utf8(&r.0[..len]).map_err(|_| "invalid history UTF-8")?;
                r.0 = &r.0[len..];
                cell(text, width, style);
            }
            remaining -= n;
        }
        if version == 1 && !r.0.is_empty() {
            return Err("trailing history bytes");
        }
        Ok(r.0)
    }
}
fn put_color(data: &mut Vec<u8>, color: Color) {
    match color {
        Color::Default => data.push(0),
        Color::Index(n) => data.extend_from_slice(&[1, n]),
        Color::Rgb(r, g, b) => data.extend_from_slice(&[2, r, g, b]),
    }
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, &'static str> {
        let (&byte, rest) = self.0.split_first().ok_or("short history row")?;
        self.0 = rest;
        Ok(byte)
    }
    fn color(&mut self) -> Result<Color, &'static str> {
        match self.byte()? {
            0 => Ok(Color::Default),
            1 => Ok(Color::Index(self.byte()?)),
            2 => Ok(Color::Rgb(self.byte()?, self.byte()?, self.byte()?)),
            _ => Err("invalid history color"),
        }
    }
}
impl Serialize for HistoryRow {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Packed {
            packed: String,
        }
        Packed {
            packed: STANDARD.encode(&self.0),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for HistoryRow {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Packed { packed: String },
            Text(String),
            Cells(Vec<Cell>),
        }
        match Saved::deserialize(deserializer)? {
            Saved::Packed { packed } => {
                if packed.len() > ROW_BYTES * 4 / 3 + 4 {
                    return Err(de::Error::custom("oversized packed history"));
                }
                let bytes = STANDARD.decode(packed).map_err(de::Error::custom)?;
                if bytes.len() > ROW_BYTES {
                    return Err(de::Error::custom("oversized history row"));
                }
                let row = Self(bytes.into());
                row.decode_cells().map_err(de::Error::custom)?;
                Ok(row)
            }
            Saved::Cells(cells) => {
                if cells.len() > 240 || cells.iter().any(|c| c.width > 2 || c.text.len() > 64) {
                    return Err(de::Error::custom("invalid legacy history cells"));
                }
                Ok(Self::from_cells(&cells))
            }
            Saved::Text(text) => {
                if text.len() > 240 * 64 {
                    return Err(de::Error::custom("oversized legacy history text"));
                }
                let mut cells: Vec<Cell> = Vec::new();
                for ch in crate::wire::passive(&text).chars() {
                    let width = char_width(ch);
                    if width == 0 {
                        if let Some(c) = cells.iter_mut().rev().find(|c| c.width > 0)
                            && c.text.len() + ch.len_utf8() <= 64
                        {
                            c.text.push(ch);
                        }
                        continue;
                    }
                    if cells.len() + width as usize > 240 {
                        break;
                    }
                    cells.push(Cell {
                        text: ch.to_string(),
                        width,
                        style: Style::default(),
                        link: None,
                    });
                    if width == 2 {
                        cells.push(Cell {
                            text: String::new(),
                            width: 0,
                            style: Style::default(),
                            link: None,
                        });
                    }
                }
                Ok(Self::from_cells(&cells))
            }
        }
    }
}
#[derive(Clone, Default)]
pub struct History {
    rows: VecDeque<HistoryRow>,
    bytes: usize,
}
impl History {
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn front(&self) -> Option<&HistoryRow> {
        self.rows.front()
    }
    pub fn get(&self, index: usize) -> Option<&HistoryRow> {
        self.rows.get(index)
    }
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &HistoryRow> {
        self.rows.iter()
    }
    pub fn clear(&mut self) {
        self.rows.clear();
        self.bytes = 0;
    }
    pub fn push(&mut self, row: HistoryRow) -> usize {
        self.bytes += row.0.len();
        self.rows.push_back(row);
        let mut evicted = 0;
        while self.rows.len() > HISTORY_ROWS || self.bytes > HISTORY_BYTES {
            self.bytes -= self.rows.pop_front().expect("nonempty history").0.len();
            evicted += 1;
        }
        evicted
    }
    pub fn valid(&self) -> bool {
        self.rows.len() <= HISTORY_ROWS && self.bytes <= HISTORY_BYTES
    }
}
impl Serialize for History {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.rows.len()))?;
        for row in &self.rows {
            seq.serialize_element(row)?;
        }
        seq.end()
    }
}
impl<'de> Deserialize<'de> for History {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = History;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bounded terminal history")
            }
            fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut history = History::default();
                while let Some(row) = seq.next_element::<HistoryRow>()? {
                    if history.rows.len() == HISTORY_ROWS
                        || history.bytes + row.0.len() > HISTORY_BYTES
                    {
                        return Err(de::Error::custom("history exceeds retention bound"));
                    }
                    history.bytes += row.0.len();
                    history.rows.push_back(row);
                }
                Ok(history)
            }
        }
        deserializer.deserialize_seq(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_history_preserves_cells_styles_and_legacy_refresh_rows() {
        let cells = vec![
            Cell {
                text: "A".into(),
                width: 1,
                style: Style {
                    fg: Color::Index(3),
                    dim: true,
                    italic: true,
                    ..Style::default()
                },
                link: None,
            },
            Cell {
                text: "界".into(),
                width: 2,
                style: Style {
                    fg: Color::Rgb(1, 2, 3),
                    bg: Color::Rgb(4, 5, 6),
                    bold: true,
                    strikethrough: true,
                    underline: true,
                    inverse: true,
                    ..Style::default()
                },
                link: None,
            },
            Cell {
                text: String::new(),
                width: 0,
                style: Style::default(),
                link: None,
            },
            Cell {
                text: "e\u{301}".into(),
                width: 1,
                style: Style::default(),
                link: None,
            },
        ];
        let row = HistoryRow::from_cells(&cells);
        assert_eq!(row.cells(), cells);
        assert_eq!(row.text(), "A界e\u{301}");
        let copy = row.clone();
        assert!(Arc::ptr_eq(&copy.0, &row.0)); // refresh shares the immutable row
        let encoded = serde_json::to_vec(&row).unwrap();
        assert_eq!(
            serde_json::from_slice::<HistoryRow>(&encoded)
                .unwrap()
                .cells(),
            cells
        );
        let legacy = serde_json::to_vec(&cells).unwrap();
        assert_eq!(
            serde_json::from_slice::<HistoryRow>(&legacy)
                .unwrap()
                .cells(),
            cells
        );
        let legacy = serde_json::to_vec("old 界e\u{301}\x1b").unwrap();
        let row = serde_json::from_slice::<HistoryRow>(&legacy).unwrap();
        assert_eq!(row.text(), "old 界e\u{301}�");
        for bytes in [
            vec![],
            vec![1],
            vec![2, 0],
            vec![1, 241],
            vec![1, 1, 0],
            vec![1, 0, 0],
            vec![1, 1, 1, 0, 0, 0, 1, 2, 255, 255],
        ] {
            let data = serde_json::json!({"packed":STANDARD.encode(bytes)});
            assert!(serde_json::from_value::<HistoryRow>(data).is_err());
        }
        let wide: Vec<_> = (0..240)
            .map(|_| Cell {
                text: "x".into(),
                ..Cell::default()
            })
            .collect();
        let compact = HistoryRow::from_cells(&wide);
        assert!(compact.0.len() < 750);
        assert!(
            serde_json::to_vec(&compact).unwrap().len() * 10
                < serde_json::to_vec(&wide).unwrap().len()
        );
    }
    #[test]
    fn compact_history_enforces_byte_budget_and_validates_restored_rows() {
        let cells: Vec<_> = (0..240)
            .map(|_| Cell {
                text: "x".repeat(64),
                ..Cell::default()
            })
            .collect();
        let row = HistoryRow::from_cells(&cells);
        let size = row.0.len();
        let mut h = History::default();
        let mut evicted = 0;
        for _ in 0..HISTORY_BYTES / size + 10 {
            evicted += h.push(row.clone());
        }
        assert!(evicted > 0 && h.bytes() <= HISTORY_BYTES && h.len() < HISTORY_ROWS);
        assert_eq!(h.bytes(), h.len() * size);
        // Counter is recomputed from validated bytes during restore, not trusted from JSON.
        let encoded = serde_json::to_vec(&History {
            rows: VecDeque::from([row]),
            bytes: size,
        })
        .unwrap();
        let restored: History = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.bytes(), size);
        assert_eq!(restored.len(), 1);
        let oversized = serde_json::json!({"packed":"A".repeat(ROW_BYTES*2)});
        assert!(serde_json::from_value::<HistoryRow>(oversized).is_err());
        h.clear();
        assert!(h.is_empty() && h.bytes() == 0);
    }
}

#[cfg(test)]
mod hyperlink_tests {
    use super::*;
    use crate::terminal::hyperlinks::{Hyperlink, URI_BYTES};
    #[test]
    fn version_two_links_round_trip_and_version_one_remains_readable() {
        let cells = vec![
            Cell {
                text: "a".into(),
                link: Hyperlink::new("https://e.test/history"),
                ..Cell::default()
            };
            20
        ];
        let row = HistoryRow::from_cells(&cells);
        assert_eq!(row.0[0], 2);
        assert_eq!(row.cells(), cells);
        assert_eq!(
            row.0
                .windows(22)
                .filter(|s| *s == b"https://e.test/history")
                .count(),
            1
        );
        let restored: HistoryRow =
            serde_json::from_slice(&serde_json::to_vec(&row).unwrap()).unwrap();
        assert_eq!(restored.cells(), cells);
        let old = serde_json::json!({"packed":STANDARD.encode([1,1,1,0,0,0,1,1,b'a'])});
        let old: HistoryRow = serde_json::from_value(old).unwrap();
        assert_eq!(old.text(), "a");
        assert!(old.cells()[0].link.is_none());
        let mut malformed = row.0.to_vec();
        malformed.pop();
        assert!(
            serde_json::from_value::<HistoryRow>(
                serde_json::json!({"packed":STANDARD.encode(malformed)})
            )
            .is_err()
        );
    }
    #[test]
    fn row_budget_drops_only_overflow_links_and_history_counts_sidecars() {
        let cells: Vec<_> = (0..240)
            .map(|i| Cell {
                text: "x".repeat(64),
                width: 1,
                style: Style {
                    fg: Color::Rgb(i as u8, 0, 0),
                    bg: Color::Rgb(0, 1, 2),
                    ..Style::default()
                },
                link: Hyperlink::new(&format!("https://e.test/{i}")),
            })
            .collect();
        let row = HistoryRow::from_cells(&cells);
        assert!(row.0.len() <= ROW_BYTES);
        let restored = row.cells();
        assert!(
            restored.iter().any(|c| c.link.is_some()) && restored.iter().any(|c| c.link.is_none())
        );
        assert!(
            restored
                .iter()
                .zip(&cells)
                .all(|(a, b)| a.text == b.text && a.width == b.width && a.style == b.style)
        );
        let mut history = History::default();
        let size = row.0.len();
        for _ in 0..HISTORY_BYTES / size + 2 {
            history.push(row.clone());
        }
        assert!(history.bytes() <= HISTORY_BYTES);
        assert_eq!(history.bytes(), history.len() * size);
        let mut cells = cells;
        let uri = format!("https://e.test/{}", "x".repeat(URI_BYTES - 15));
        assert!(uri.len() <= URI_BYTES);
        for cell in &mut cells {
            cell.link = Hyperlink::new(&uri);
        }
        let row = HistoryRow::from_cells(&cells);
        assert!(row.0.len() <= ROW_BYTES && row.cells().iter().all(|c| c.link.is_none()));
    }
}
