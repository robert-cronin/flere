pub use crate::panes::{PaneGroup, SplitAxis};
use crate::{
    terminal::{Cell, Color, Style, hyperlinks},
    wire::{Decoder, Encoder, invalid},
};
use std::io;
#[derive(Clone)]
pub struct PaneBuffer {
    pub tab: u64,
    pub cols: usize,
    pub rows: usize,
    pub x: usize,
    pub y: usize,
    pub cursor: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub cells: Vec<Cell>,
}
#[derive(Clone)]
pub struct SplitView {
    pub axis: SplitAxis,
    pub ratio: u16,
    pub zoomed: bool,
    pub focused: u8,
    pub revision: u64,
    pub groups: [PaneGroup; 2],
    pub other: PaneBuffer,
}
#[derive(Clone)]
pub struct TabView {
    pub id: u64,
    pub run: String,
    pub pid: u32,
    pub alive: bool,
    pub title: String,
    pub kind: String,
    pub path: String,
    pub working: bool,
}
#[derive(Clone)]
pub struct WorkspaceView {
    pub id: u64,
    pub name: String,
    pub cwd: String,
    pub tabs: Vec<TabView>,
    pub meta: crate::workspace::CardMeta,
}
#[derive(Clone)]
pub struct Snapshot {
    pub epoch: String,
    pub generation: u64,
    pub active: u64,
    pub tab: u64,
    pub workspaces: Vec<WorkspaceView>,
    pub cols: usize,
    pub rows: usize,
    pub x: usize,
    pub y: usize,
    pub cursor: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    pub notice: String,
    pub cells: Vec<Cell>,
    pub split: Option<SplitView>,
}
fn color(e: &mut Encoder, c: Color) {
    match c {
        Color::Default => e.u8(0),
        Color::Index(n) => {
            e.u8(1);
            e.u8(n)
        }
        Color::Rgb(r, g, b) => {
            e.u8(2);
            e.u8(r);
            e.u8(g);
            e.u8(b)
        }
    }
}
fn read_color(d: &mut Decoder<'_>) -> io::Result<Color> {
    match d.u8()? {
        0 => Ok(Color::Default),
        1 => Ok(Color::Index(d.u8()?)),
        2 => Ok(Color::Rgb(d.u8()?, d.u8()?, d.u8()?)),
        _ => Err(invalid("color tag")),
    }
}
impl Snapshot {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.u8(4);
        e.string(&self.epoch);
        e.u64(self.generation);
        e.u64(self.active);
        e.u64(self.tab);
        e.u64(self.workspaces.len() as u64);
        for w in &self.workspaces {
            e.u64(w.id);
            e.string(&w.name);
            e.string(&w.cwd);
            // Recency is supervisor-owned saved state. Keep legacy frontend metadata
            // compatible; explicit resume asks the supervisor for its current choice.
            let mut meta = w.meta.clone();
            meta.last_conversation = None;
            e.string(&serde_json::to_string(&meta).expect("serializable metadata"));
            e.u64(w.tabs.len() as u64);
            for t in &w.tabs {
                e.u64(t.id);
                e.string(&t.run);
                e.u64(t.pid as u64);
                e.u8(t.alive as u8);
                e.string(&t.title);
                e.string(&t.kind);
                e.string(&t.path);
                e.u8(t.working as u8)
            }
        }
        e.u64(self.cols as u64);
        e.u64(self.rows as u64);
        e.u64(self.x as u64);
        e.u64(self.y as u64);
        e.u8(self.cursor as u8);
        e.u8(self.bracketed_paste as u8);
        e.u8(self.app_cursor as u8);
        e.string(&self.notice);
        encode_cells(&mut e, &self.cells);
        e.0
    }
    pub fn encode_panes(&self) -> Vec<u8> {
        let mut bytes = self.encode();
        bytes[0] = 5;
        let mut e = Encoder(bytes);
        e.u8(u8::from(self.split.is_some()));
        if let Some(split) = &self.split {
            e.u8(match split.axis {
                SplitAxis::Right => 0,
                SplitAxis::Below => 1,
            });
            e.u64(u64::from(split.ratio));
            e.u8(u8::from(split.zoomed));
            e.u8(split.focused);
            e.u64(split.revision);
            for group in &split.groups {
                e.u64(group.tabs.len() as u64);
                for tab in &group.tabs {
                    e.u64(*tab);
                }
                e.u64(group.selected);
            }
            let b = &split.other;
            for n in [b.tab, b.cols as u64, b.rows as u64, b.x as u64, b.y as u64] {
                e.u64(n);
            }
            e.u8(u8::from(b.cursor));
            e.u8(u8::from(b.bracketed_paste));
            e.u8(u8::from(b.app_cursor));
            encode_cells(&mut e, &b.cells);
        }
        e.0
    }
    /// Opt-in hyperlink snapshot. Legacy v4/v5 projections retain their exact layout.
    pub fn encode_links(&self) -> Vec<u8> {
        let mut bytes = self.encode_panes();
        bytes[0] = 6;
        let mut e = Encoder(bytes);
        hyperlinks::encode_links(&mut e, &self.cells);
        if let Some(split) = &self.split {
            hyperlinks::encode_links(&mut e, &split.other.cells);
        }
        e.0
    }
    pub fn decode(b: &[u8]) -> io::Result<Self> {
        let mut d = Decoder(b);
        let version = d.u8()?;
        if !(1..=6).contains(&version) {
            return Err(invalid("unsupported snapshot version"));
        }
        let epoch = d.string()?;
        let generation = d.u64()?;
        let active = d.u64()?;
        let tab = d.u64()?;
        let mut workspaces = Vec::new();
        for _ in 0..d.count(65536)? {
            let id = d.u64()?;
            let name = d.string()?;
            let cwd = d.string()?;
            let meta = if version >= 2 {
                serde_json::from_str(&d.string()?).map_err(io::Error::other)?
            } else {
                Default::default()
            };
            let mut tabs = Vec::new();
            for _ in 0..d.count(65536)? {
                tabs.push(TabView {
                    id: d.u64()?,
                    run: d.string()?,
                    pid: d.u64()? as u32,
                    alive: d.u8()? != 0,
                    title: d.string()?,
                    kind: if version >= 2 {
                        d.string()?
                    } else {
                        "shell".into()
                    },
                    path: if version >= 2 {
                        d.string()?
                    } else {
                        String::new()
                    },
                    working: if version >= 4 { d.u8()? != 0 } else { false },
                })
            }
            workspaces.push(WorkspaceView {
                id,
                name,
                cwd,
                tabs,
                meta,
            })
        }
        let cols = d.count(240)?;
        let rows = d.count(100)?;
        let x = d.count(240)?;
        let y = d.count(100)?;
        let cursor = d.u8()? != 0;
        let bracketed_paste = d.u8()? != 0;
        let app_cursor = d.u8()? != 0;
        let notice = if version >= 3 {
            d.string()?
        } else {
            String::new()
        };
        let mut cells = decode_cells(&mut d, cols * rows)?;
        if version >= 5 && (cols < 2 || rows < 2 || x >= cols || y >= rows) {
            return Err(invalid("invalid pane snapshot viewport"));
        }
        let mut split = if version >= 5 && d.u8()? != 0 {
            let axis = match d.u8()? {
                0 => SplitAxis::Right,
                1 => SplitAxis::Below,
                _ => return Err(invalid("invalid split axis")),
            };
            let ratio = d.count(800)? as u16;
            let zoomed = d.u8()? != 0;
            let focused = d.u8()?;
            let revision = d.u64()?;
            let mut groups = [
                PaneGroup {
                    tabs: Vec::new(),
                    selected: 0,
                },
                PaneGroup {
                    tabs: Vec::new(),
                    selected: 0,
                },
            ];
            for group in &mut groups {
                for _ in 0..d.count(512)? {
                    group.tabs.push(d.u64()?);
                }
                group.selected = d.u64()?;
            }
            let other_tab = d.u64()?;
            let other_cols = d.count(240)?;
            let other_rows = d.count(100)?;
            let other_x = d.count(240)?;
            let other_y = d.count(100)?;
            let other_cursor = d.u8()? != 0;
            let other_bracketed = d.u8()? != 0;
            let other_app = d.u8()? != 0;
            let other_cells = decode_cells(&mut d, other_cols * other_rows)?;
            let ids: Vec<_> = workspaces
                .iter()
                .find(|w| w.id == active)
                .map(|w| w.tabs.iter().map(|t| t.id).collect())
                .unwrap_or_default();
            crate::panes::PaneLayout {
                axis,
                ratio,
                zoomed,
                focused,
                revision,
                groups: groups.clone(),
            }
            .validate(&ids)?;
            if cols < 2
                || rows < 2
                || other_cols < 2
                || other_rows < 2
                || other_x >= other_cols
                || other_y >= other_rows
                || groups[usize::from(focused)].selected != tab
                || groups[usize::from(1 - focused)].selected != other_tab
            {
                return Err(invalid("invalid split buffer identity"));
            }
            Some(SplitView {
                axis,
                ratio,
                zoomed,
                focused,
                revision,
                groups,
                other: PaneBuffer {
                    tab: other_tab,
                    cols: other_cols,
                    rows: other_rows,
                    x: other_x,
                    y: other_y,
                    cursor: other_cursor,
                    bracketed_paste: other_bracketed,
                    app_cursor: other_app,
                    cells: other_cells,
                },
            })
        } else {
            None
        };
        if version >= 6 {
            hyperlinks::decode_links(&mut d, &mut cells)?;
            if let Some(split) = &mut split {
                hyperlinks::decode_links(&mut d, &mut split.other.cells)?;
            }
        }
        if !d.0.is_empty() {
            return Err(invalid("snapshot trailing data"));
        }
        Ok(Self {
            epoch,
            generation,
            active,
            tab,
            workspaces,
            cols,
            rows,
            x,
            y,
            cursor,
            bracketed_paste,
            app_cursor,
            notice,
            cells,
            split,
        })
    }
    pub fn workspace(&self) -> Option<&WorkspaceView> {
        self.workspaces.iter().find(|w| w.id == self.active)
    }
    pub fn session(&self) -> Option<&TabView> {
        self.workspace()?.tabs.iter().find(|t| t.id == self.tab)
    }
}

fn encode_cells(e: &mut Encoder, cells: &[Cell]) {
    for c in cells {
        e.string(&c.text);
        e.u8(c.width);
        color(e, c.style.fg);
        color(e, c.style.bg);
        e.u8(c.style.bold as u8
            | ((c.style.underline as u8) << 1)
            | ((c.style.inverse as u8) << 2)
            | ((c.style.dim as u8) << 3)
            | ((c.style.italic as u8) << 4)
            | ((c.style.strikethrough as u8) << 5))
    }
}
fn decode_cells(d: &mut Decoder<'_>, count: usize) -> io::Result<Vec<Cell>> {
    let mut cells = Vec::new();
    for _ in 0..count {
        let text = d.string()?;
        let width = d.u8()?;
        if width > 2 || text.len() > 64 {
            return Err(invalid("invalid cell"));
        }
        let fg = read_color(d)?;
        let bg = read_color(d)?;
        let flags = d.u8()?;
        cells.push(Cell {
            text,
            width,
            link: None,
            style: Style {
                fg,
                bg,
                bold: flags & 1 != 0,
                underline: flags & 2 != 0,
                inverse: flags & 4 != 0,
                dim: flags & 8 != 0,
                italic: flags & 16 != 0,
                strikethrough: flags & 32 != 0,
            },
        })
    }
    Ok(cells)
}
/// Separate on-demand protocol; live snapshot/watch framing stays unchanged.
pub struct ScrollbackPage {
    pub available: bool,
    pub position: u64,
    pub end: u64,
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
}
impl ScrollbackPage {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Encoder::new();
        e.u8(1);
        e.u8(self.available as u8);
        e.u64(self.position);
        e.u64(self.end);
        e.u64(self.cols as u64);
        e.u64(self.rows as u64);
        encode_cells(&mut e, &self.cells);
        e.0
    }
    /// Opt-in hyperlink sidecar; the v1 page projection remains available.
    pub fn encode_links(&self) -> Vec<u8> {
        let mut bytes = self.encode();
        bytes[0] = 2;
        let mut e = Encoder(bytes);
        hyperlinks::encode_links(&mut e, &self.cells);
        e.0
    }
    pub fn decode(bytes: &[u8]) -> io::Result<Self> {
        let mut d = Decoder(bytes);
        let version = d.u8()?;
        if !(1..=2).contains(&version) {
            return Err(invalid("unsupported scrollback version"));
        }
        let available = d.u8()? != 0;
        let position = d.u64()?;
        let end = d.u64()?;
        let cols = d.count(240)?;
        let rows = d.count(100)?;
        if cols < 2 || rows < 2 || position > end {
            return Err(invalid("invalid scrollback page"));
        }
        let mut cells = decode_cells(&mut d, cols * rows)?;
        if version >= 2 {
            hyperlinks::decode_links(&mut d, &mut cells)?;
        }
        if !d.0.is_empty() {
            return Err(invalid("scrollback trailing data"));
        }
        Ok(Self {
            available,
            position,
            end,
            cols,
            rows,
            cells,
        })
    }
}

#[cfg(test)]
mod pane_tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            epoch: "a".repeat(32),
            generation: 9,
            active: 1,
            tab: 3,
            workspaces: vec![WorkspaceView {
                id: 1,
                name: "panes".into(),
                cwd: "/fixture".into(),
                meta: Default::default(),
                tabs: [2, 3]
                    .into_iter()
                    .map(|id| TabView {
                        id,
                        run: format!("{id:032x}"),
                        pid: 10,
                        alive: true,
                        title: "shell".into(),
                        kind: "shell".into(),
                        path: String::new(),
                        working: false,
                    })
                    .collect(),
            }],
            cols: 18,
            rows: 4,
            x: 1,
            y: 2,
            cursor: true,
            bracketed_paste: true,
            app_cursor: false,
            notice: String::new(),
            cells: vec![Cell::default(); 72],
            split: Some(SplitView {
                axis: SplitAxis::Right,
                ratio: 500,
                zoomed: false,
                focused: 1,
                revision: 8,
                groups: [
                    PaneGroup {
                        tabs: vec![2],
                        selected: 2,
                    },
                    PaneGroup {
                        tabs: vec![3],
                        selected: 3,
                    },
                ],
                other: PaneBuffer {
                    tab: 2,
                    cols: 18,
                    rows: 4,
                    x: 0,
                    y: 0,
                    cursor: false,
                    bracketed_paste: false,
                    app_cursor: false,
                    cells: vec![Cell::default(); 72],
                },
            }),
        }
    }
    #[test]
    fn negotiated_snapshot_preserves_both_buffers_and_legacy_projection() {
        let original = snapshot();
        let legacy = Snapshot::decode(&original.encode()).unwrap();
        assert!(legacy.split.is_none());
        assert_eq!((legacy.tab, legacy.cols, legacy.rows), (3, 18, 4));
        let panes = Snapshot::decode(&original.encode_panes()).unwrap();
        let split = panes.split.unwrap();
        assert_eq!(split.groups[1].selected, panes.tab);
        assert_eq!((split.other.tab, split.other.cells.len()), (2, 72));
        assert_eq!(split.revision, 8);
    }
    #[test]
    fn negotiated_hyperlinks_preserve_both_panes_and_leave_legacy_bytes_unchanged() {
        let mut original = snapshot();
        let legacy = original.encode();
        let panes = original.encode_panes();
        original.cells[0].link = hyperlinks::Hyperlink::new("https://github.com/example/core");
        original.split.as_mut().unwrap().other.cells[3].link =
            hyperlinks::Hyperlink::new("https://github.com/example/companion");
        assert_eq!(original.encode(), legacy);
        assert_eq!(original.encode_panes(), panes);
        assert!(
            Snapshot::decode(&panes)
                .unwrap()
                .cells
                .iter()
                .all(|c| c.link.is_none())
        );
        let bytes = original.encode_links();
        assert_eq!(bytes[0], 6);
        let decoded = Snapshot::decode(&bytes).unwrap();
        assert_eq!(decoded.cells, original.cells);
        assert_eq!(
            decoded.split.unwrap().other.cells,
            original.split.as_ref().unwrap().other.cells
        );
        for end in [panes.len(), bytes.len() - 1] {
            assert!(Snapshot::decode(&bytes[..end]).is_err());
        }
        let mut unnegotiated = bytes.clone();
        unnegotiated[0] = 5;
        assert!(Snapshot::decode(&unnegotiated).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(Snapshot::decode(&trailing).is_err());
        original.split = None;
        let decoded = Snapshot::decode(&original.encode_links()).unwrap();
        assert!(decoded.split.is_none());
        assert_eq!(decoded.cells, original.cells);
    }
    #[test]
    fn hyperlink_scrollback_is_opt_in_and_rejects_incomplete_sidecars() {
        let mut page = ScrollbackPage {
            available: true,
            position: 4,
            end: 8,
            cols: 2,
            rows: 2,
            cells: vec![Cell::default(); 4],
        };
        let legacy = page.encode();
        page.cells[1].link = hyperlinks::Hyperlink::new("https://github.com/example/history");
        assert_eq!(page.encode(), legacy);
        assert!(
            ScrollbackPage::decode(&legacy)
                .unwrap()
                .cells
                .iter()
                .all(|c| c.link.is_none())
        );
        let bytes = page.encode_links();
        assert_eq!(bytes[0], 2);
        assert_eq!(ScrollbackPage::decode(&bytes).unwrap().cells, page.cells);
        for end in [legacy.len(), bytes.len() - 1] {
            assert!(ScrollbackPage::decode(&bytes[..end]).is_err());
        }
        let mut unnegotiated = bytes.clone();
        unnegotiated[0] = 1;
        assert!(ScrollbackPage::decode(&unnegotiated).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(ScrollbackPage::decode(&trailing).is_err());
    }
    #[test]
    fn snapshot_metadata_keeps_the_baseline_writer_contract_for_legacy_bridges() {
        let metadata: serde_json::Value = serde_json::from_str(crate::build_info::json()).unwrap();
        let snapshot = &metadata["compatibility"]["snapshot"];
        assert_eq!(snapshot["current"], 5);
        assert_eq!(snapshot["read_min"], 1);
        assert_eq!(snapshot["read_max"], 6);
        // bootstrap's legacy reader.accepts(writer.current) check must still allow
        // a v5 bridge, since only explicit link requests receive v6 frames.
        let legacy_reader = crate::install::VersionRange {
            current: 5,
            read_min: 1,
            read_max: 5,
        };
        assert!(legacy_reader.accepts(snapshot["current"].as_u64().unwrap()));
    }
    #[test]
    fn negotiated_snapshot_rejects_wrong_or_duplicate_pane_targets() {
        for mutation in 0..5 {
            let mut snapshot = snapshot();
            let split = snapshot.split.as_mut().unwrap();
            match mutation {
                0 => split.groups[0].tabs.push(3),
                1 => split.other.tab = 3,
                2 => split.focused = 2,
                3 => split.ratio = 199,
                _ => split.groups[1].selected = 2,
            }
            assert!(Snapshot::decode(&snapshot.encode_panes()).is_err());
        }
    }
    #[test]
    fn maximum_retained_grids_and_large_metadata_fit_response_but_not_request_budget() {
        let mut snapshot = snapshot();
        let cell = Cell {
            text: format!("x{}", "\u{20d0}".repeat(21)),
            width: 1,
            link: None,
            style: Style {
                fg: Color::Rgb(1, 2, 3),
                bg: Color::Rgb(4, 5, 6),
                ..Style::default()
            },
        };
        assert_eq!(cell.text.len(), 64);
        snapshot.cols = 240;
        snapshot.rows = 100;
        snapshot.cells = vec![cell.clone(); 24_000];
        let split = snapshot.split.as_mut().unwrap();
        split.zoomed = true;
        split.other.cols = 240;
        split.other.rows = 100;
        split.other.cells = vec![cell; 24_000];
        for id in 10..136 {
            snapshot.workspaces.push(WorkspaceView {
                id,
                name: "metadata fixture".into(),
                cwd: "/fixture".into(),
                tabs: Vec::new(),
                meta: crate::workspace::CardMeta {
                    notes: "x".repeat(65536),
                    ..Default::default()
                },
            });
        }
        let bytes = snapshot.encode_panes();
        assert!(bytes.len() > 8 * 1024 * 1024);
        assert!(bytes.len() <= crate::wire::MAX);
        let framed = crate::wire::frame(&bytes);
        let received = crate::wire::read_frame(&mut &framed[..]).unwrap();
        let decoded = Snapshot::decode(&received).unwrap();
        assert_eq!(decoded.cells.len(), 24_000);
        assert_eq!(decoded.split.unwrap().other.cells.len(), 24_000);
        assert_eq!(decoded.workspaces.len(), 127);
        let urls: Vec<_> = (0..64)
            .map(|i| {
                hyperlinks::Hyperlink::new(&format!(
                    "https://example.test/{i}/{}",
                    "x".repeat(1950)
                ))
                .unwrap()
            })
            .collect();
        for (i, cell) in snapshot.cells.iter_mut().enumerate() {
            cell.link = Some(urls[i % urls.len()].clone());
        }
        snapshot
            .split
            .as_mut()
            .unwrap()
            .other
            .cells
            .clone_from(&snapshot.cells);
        assert_eq!(snapshot.encode_panes(), bytes);
        let linked = snapshot.encode_links();
        assert!(linked.len() <= crate::wire::MAX);
        let decoded = Snapshot::decode(
            &crate::wire::read_frame(&mut &crate::wire::frame(&linked)[..]).unwrap(),
        )
        .unwrap();
        assert_eq!(decoded.cells.len(), 24_000);
        assert!(decoded.cells.iter().all(|cell| cell.text.len() == 64));
        assert!(decoded.cells.iter().any(|cell| cell.link.is_some()));
        assert!(decoded.cells.iter().any(|cell| cell.link.is_none()));
        assert_eq!(decoded.split.unwrap().other.cells, decoded.cells);
        let mut prefix = ((crate::wire::MAX_REQUEST + 1) as u32)
            .to_be_bytes()
            .to_vec();
        assert!(crate::wire::take_request_frame(&mut prefix).is_err());
        let prefix = ((crate::wire::MAX + 1) as u32).to_be_bytes();
        assert!(crate::wire::read_frame(&mut &prefix[..]).is_err());
        let string = ((crate::wire::MAX_STRING + 1) as u64).to_be_bytes();
        assert!(Decoder(&string).string().is_err());
        assert_eq!(crate::remote_protocol::MAX, 65_536);
    }
}
