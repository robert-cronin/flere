//! Bounded VT subset for shells and native TUI experiments. Never forward raw child escapes.
mod commands;
mod history;
pub mod hyperlinks;
use hyperlinks::{Hyperlink, Pool};
pub mod search;
pub use history::{HISTORY_BYTES, HISTORY_ROWS, History, HistoryRow};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Color {
    #[default]
    Default,
    Index(u8),
    Rgb(u8, u8, u8),
}
// The virtual terminal owns a neutral palette, independent of the workbench chrome.
// Rendered defaults and OSC query replies must resolve to the exact same colors.
pub const DEFAULT_FG: Color = Color::Rgb(204, 204, 204);
pub const DEFAULT_BG: Color = Color::Rgb(12, 12, 12);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub strikethrough: bool,
    pub underline: bool,
    pub inverse: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Cell {
    pub text: String,
    pub width: u8,
    pub style: Style,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Hyperlink>,
}
impl Default for Cell {
    fn default() -> Self {
        Self {
            text: " ".into(),
            width: 1,
            style: Style::default(),
            link: None,
        }
    }
}
#[derive(Clone)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    pub x: usize,
    pub y: usize,
    top: usize,
    bottom: usize,
    wrap: bool,
}
impl Grid {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            cells: vec![Cell::default(); cols * rows],
            x: 0,
            y: 0,
            top: 0,
            bottom: rows - 1,
            wrap: false,
        }
    }
    fn resize(&mut self, cols: usize, rows: usize) {
        let mut cells = vec![Cell::default(); cols * rows];
        for y in 0..rows.min(self.rows) {
            for x in 0..cols.min(self.cols) {
                cells[y * cols + x] = self.cells[y * self.cols + x].clone()
            }
        }
        self.cells = cells;
        self.cols = cols;
        self.rows = rows;
        self.x = self.x.min(cols - 1);
        self.y = self.y.min(rows - 1);
        self.top = 0;
        self.bottom = rows - 1;
        self.wrap = false;
        self.repair();
    }
    fn repair(&mut self) {
        for y in 0..self.rows {
            for x in 0..self.cols {
                let i = y * self.cols + x;
                if (self.cells[i].width == 2
                    && (x + 1 == self.cols || self.cells[i + 1].width != 0))
                    || (self.cells[i].width == 0 && (x == 0 || self.cells[i - 1].width != 2))
                {
                    self.cells[i] = Cell::default()
                }
            }
        }
    }
    pub fn line(&self, y: usize) -> String {
        self.cells[y * self.cols..(y + 1) * self.cols]
            .iter()
            .filter(|c| c.width > 0)
            .map(|c| c.text.as_str())
            .collect::<String>()
            .trim_end()
            .into()
    }
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
enum Parse {
    #[default]
    Ground,
    Escape,
    Charset,
    Csi(Vec<u8>),
    String {
        osc: bool,
        escaped: bool,
        #[serde(default)]
        data: Vec<u8>,
        #[serde(default)]
        overflow: bool,
    },
}
#[derive(Clone, serde::Serialize)]
pub struct Terminal {
    pub grid: Grid,
    primary: Option<Grid>,
    primary_style: Style,
    parse: Parse,
    utf8: Vec<u8>,
    style: Style,
    saved: (usize, usize, Style),
    #[serde(default)]
    active_link: Option<Hyperlink>,
    #[serde(default)]
    saved_link: Option<Hyperlink>,
    #[serde(default)]
    primary_link: Option<Hyperlink>,
    #[serde(skip)]
    link_pool: Pool,
    pub cursor: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    #[serde(default)]
    alternate_scroll: bool,
    autowrap: bool,
    origin: bool,
    pub history: History,
    #[serde(default)]
    history_base: u64,
    #[serde(default)]
    command_marks: commands::Marks,
    pub replies: Vec<u8>,
}
#[derive(serde::Deserialize)]
#[serde(remote = "Terminal")]
struct SavedTerminal {
    pub grid: Grid,
    primary: Option<Grid>,
    primary_style: Style,
    parse: Parse,
    utf8: Vec<u8>,
    style: Style,
    saved: (usize, usize, Style),
    #[serde(default)]
    active_link: Option<Hyperlink>,
    #[serde(default)]
    saved_link: Option<Hyperlink>,
    #[serde(default)]
    primary_link: Option<Hyperlink>,
    #[serde(skip)]
    link_pool: Pool,
    pub cursor: bool,
    pub bracketed_paste: bool,
    pub app_cursor: bool,
    #[serde(default)]
    alternate_scroll: bool,
    autowrap: bool,
    origin: bool,
    pub history: History,
    #[serde(default)]
    history_base: u64,
    #[serde(default)]
    command_marks: commands::Marks,
    pub replies: Vec<u8>,
}
impl<'de> serde::Deserialize<'de> for Terminal {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut terminal = SavedTerminal::deserialize(deserializer)?;
        terminal.normalize_links();
        Ok(terminal)
    }
}

impl Terminal {
    fn normalize_links(&mut self) {
        self.link_pool = Pool::default();
        for link in [
            &mut self.active_link,
            &mut self.saved_link,
            &mut self.primary_link,
        ]
        .into_iter()
        .chain(self.grid.cells.iter_mut().map(|c| &mut c.link))
        .chain(
            self.primary
                .iter_mut()
                .flat_map(|g| g.cells.iter_mut().map(|c| &mut c.link)),
        ) {
            *link = link
                .as_ref()
                .and_then(|value| self.link_pool.intern(value.as_str()));
        }
    }
    fn links_valid(&self) -> bool {
        let links: std::collections::HashSet<_> =
            [&self.active_link, &self.saved_link, &self.primary_link]
                .into_iter()
                .chain(self.grid.cells.iter().map(|c| &c.link))
                .chain(
                    self.primary
                        .iter()
                        .flat_map(|g| g.cells.iter().map(|c| &c.link)),
                )
                .filter_map(Option::as_ref)
                .collect();
        links.len() <= hyperlinks::LINK_COUNT
            && links.iter().map(|link| link.as_str().len()).sum::<usize>() <= hyperlinks::LIVE_BYTES
    }
    pub fn valid_state(&self) -> bool {
        let valid = |g: &Grid| {
            g.cols >= 2
                && g.cols <= 240
                && g.rows >= 2
                && g.rows <= 100
                && g.cells.len() == g.cols * g.rows
                && g.x < g.cols
                && g.y < g.rows
                && g.top <= g.bottom
                && g.bottom < g.rows
                && g.cells.iter().all(|c| c.width <= 2 && c.text.len() <= 64)
        };
        valid(&self.grid)
            && self.primary.as_ref().is_none_or(valid)
            && match &self.parse {
                Parse::String { data, .. } => data.len() <= hyperlinks::OSC_BYTES,
                Parse::Csi(data) => data.len() <= 256,
                _ => true,
            }
            && self.links_valid()
            && self.utf8.len() <= 4
            && self.history.valid()
            && self.command_marks.0.len() <= 2048
            && self
                .command_marks
                .0
                .iter()
                .all(|(_, kind)| matches!(*kind, b'A' | b'C' | b'D'))
            && self.replies.len() <= 131072
    }

    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            grid: Grid::new(cols, rows),
            primary: None,
            primary_style: Style::default(),
            parse: Parse::Ground,
            utf8: Vec::new(),
            style: Style::default(),
            saved: (0, 0, Style::default()),
            active_link: None,
            saved_link: None,
            primary_link: None,
            link_pool: Pool::default(),
            cursor: true,
            bracketed_paste: false,
            app_cursor: false,
            alternate_scroll: false,
            autowrap: true,
            origin: false,
            history: History::default(),
            history_base: 0,
            command_marks: Default::default(),
            replies: Vec::new(),
        }
    }
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.grid.resize(cols, rows);
        if let Some(p) = &mut self.primary {
            p.resize(cols, rows)
        }
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.byte(b)
        }
    }
    fn byte(&mut self, b: u8) {
        let state = std::mem::take(&mut self.parse);
        match state {
            Parse::Ground => match b {
                0x1b => {
                    self.flush_utf8();
                    self.parse = Parse::Escape
                }
                b'\r' => {
                    self.flush_utf8();
                    self.grid.x = 0;
                    self.grid.wrap = false
                }
                b'\n' | 0x0b | 0x0c => {
                    self.flush_utf8();
                    self.newline()
                }
                b'\x08' => {
                    self.flush_utf8();
                    self.grid.x = self.grid.x.saturating_sub(1);
                    self.grid.wrap = false
                }
                b'\t' => {
                    self.flush_utf8();
                    self.grid.x = ((self.grid.x / 8 + 1) * 8).min(self.grid.cols - 1);
                    self.grid.wrap = false
                }
                0..=31 | 127 => {}
                _ => self.character_byte(b),
            },
            Parse::Escape => match b {
                b'[' => self.parse = Parse::Csi(Vec::new()),
                b']' => {
                    self.parse = Parse::String {
                        osc: true,
                        escaped: false,
                        data: Vec::new(),
                        overflow: false,
                    }
                }
                b'P' | b'_' | b'^' | b'X' => {
                    self.parse = Parse::String {
                        osc: false,
                        escaped: false,
                        data: Vec::new(),
                        overflow: false,
                    }
                }
                b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.parse = Parse::Charset,
                b'7' => self.save(),
                b'8' => self.restore(),
                b'D' => self.newline(),
                b'E' => {
                    self.grid.x = 0;
                    self.newline()
                }
                b'M' => {
                    if self.grid.y == self.grid.top {
                        self.scroll_down(1)
                    } else {
                        self.grid.y = self.grid.y.saturating_sub(1)
                    }
                }
                b'c' => {
                    let (c, r) = (self.grid.cols, self.grid.rows);
                    *self = Self::new(c, r)
                }
                0x1b => self.parse = Parse::Escape,
                _ => {}
            },
            Parse::Charset => {}
            Parse::Csi(mut v) => {
                if (0x40..=0x7e).contains(&b) {
                    self.csi(&v, b)
                } else if b == 0x1b {
                    self.parse = Parse::Escape
                } else if b == 0x18 || b == 0x1a {
                } else if v.len() < 256 {
                    v.push(b);
                    self.parse = Parse::Csi(v)
                } else {
                    self.parse = Parse::String {
                        osc: false,
                        escaped: false,
                        data: Vec::new(),
                        overflow: false,
                    }
                }
            }
            Parse::String {
                osc,
                escaped,
                mut data,
                mut overflow,
            } => {
                if escaped && b == b'\\' || osc && b == 7 {
                    if osc && !overflow {
                        self.osc(&data);
                    } else if osc && (data == b"8" || data.starts_with(b"8;")) {
                        self.active_link = None;
                    }
                } else if b != 0x18 && b != 0x1a {
                    if osc {
                        // No arbitrary strings are forwarded or retained. A malformed/oversized
                        // sequence stays discarded until its terminator, even across refresh.
                        let limit = if data.starts_with(b"8;") {
                            hyperlinks::OSC_BYTES
                        } else {
                            64
                        };
                        overflow |= escaped || data.len() >= limit;
                        if !overflow && b != 0x1b {
                            data.push(b);
                        }
                    }
                    self.parse = Parse::String {
                        osc,
                        escaped: b == 0x1b,
                        data,
                        overflow,
                    }
                } else if osc && (data == b"8" || data.starts_with(b"8;")) {
                    self.active_link = None;
                }
            }
        }
    }
    fn osc(&mut self, data: &[u8]) {
        // Links are validated passive metadata only. Their original OSC bytes
        // never leave the emulator. Clipboard/title/palette mutations stay inert.
        if data == b"8" || data.starts_with(b"8;") {
            self.active_link = None;
            if let Some(fields) = data.strip_prefix(b"8;")
                && let Some(separator) = fields.iter().position(|b| *b == b';')
                && separator <= 253
                && fields[..separator].iter().all(|b| b.is_ascii_graphic())
                && let Ok(uri) = std::str::from_utf8(&fields[separator + 1..])
            {
                self.active_link = self.link_pool.intern(uri);
            }
            return;
        }
        let Ok(text) = std::str::from_utf8(data) else {
            return;
        };
        if let Some(fields) = text.strip_prefix("133;") {
            self.command_mark(fields);
            return;
        }
        let mut parts = text.split(';');
        let first_slot = match parts.next() {
            Some("10") => 10,
            Some("11") => 11,
            _ => return,
        };
        for (slot, query) in (first_slot..).zip(parts) {
            if query != "?" || slot > 11 {
                return;
            }
            let Color::Rgb(r, g, b) = (if slot == 10 { DEFAULT_FG } else { DEFAULT_BG }) else {
                unreachable!()
            };
            self.reply(
                format!(
                    "\x1b]{slot};rgb:{:04x}/{:04x}/{:04x}\x1b\\",
                    u16::from(r) * 257,
                    u16::from(g) * 257,
                    u16::from(b) * 257
                )
                .as_bytes(),
            );
        }
    }
    fn reply(&mut self, bytes: &[u8]) {
        if self.replies.len() + bytes.len() <= 8192 {
            self.replies.extend_from_slice(bytes);
        }
    }
    fn flush_utf8(&mut self) {
        if !self.utf8.is_empty() {
            self.utf8.clear();
            self.put('�')
        }
    }
    fn character_byte(&mut self, b: u8) {
        if self.utf8.is_empty() && b < 128 {
            self.put(b as char);
            return;
        }
        self.utf8.push(b);
        match std::str::from_utf8(&self.utf8) {
            Ok(s) => {
                let c = s.chars().next().unwrap();
                self.utf8.clear();
                self.put(c)
            }
            Err(e) if e.error_len().is_some() => {
                let tail = self.utf8.pop().unwrap();
                let had_prefix = !self.utf8.is_empty();
                self.utf8.clear();
                self.put('�');
                if had_prefix {
                    self.character_byte(tail)
                }
            }
            _ => {}
        }
    }
    fn blank(&self) -> Cell {
        Cell {
            style: self.style,
            ..Cell::default()
        }
    }
    fn clear_cell(&mut self, x: usize, y: usize) {
        let i = y * self.grid.cols + x;
        let blank = self.blank();
        if self.grid.cells[i].width == 0 && x > 0 {
            self.grid.cells[i - 1] = blank.clone()
        }
        if self.grid.cells[i].width == 2 && x + 1 < self.grid.cols {
            self.grid.cells[i + 1] = blank.clone()
        }
        self.grid.cells[i] = blank;
    }
    fn put(&mut self, mut c: char) {
        if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {
            c = '�'
        }
        let mut width = char_width(c);
        if width == 0 {
            let x = if self.grid.wrap {
                self.grid.x
            } else {
                self.grid.x.saturating_sub(1)
            };
            let mut i = self.grid.y * self.grid.cols + x;
            if self.grid.cells[i].width == 0 && x > 0 {
                i -= 1
            }
            if self.grid.cells[i].text.len() + c.len_utf8() <= 64 {
                self.grid.cells[i].text.push(c)
            }
            return;
        }
        if self.grid.wrap && self.autowrap
            || width == 2 && self.grid.x + 1 >= self.grid.cols && self.autowrap
        {
            self.grid.x = 0;
            self.newline()
        }
        if width == 2 && self.grid.x + 1 >= self.grid.cols {
            c = '�';
            width = 1
        }
        let (x, y) = (self.grid.x, self.grid.y);
        self.clear_cell(x, y);
        if width == 2 {
            self.clear_cell(x + 1, y)
        }
        let i = y * self.grid.cols + x;
        self.grid.cells[i] = Cell {
            text: c.to_string(),
            width,
            style: self.style,
            link: self.active_link.clone(),
        };
        if width == 2 {
            self.grid.cells[i + 1] = Cell {
                text: String::new(),
                width: 0,
                style: self.style,
                link: self.active_link.clone(),
            }
        }
        if x + width as usize >= self.grid.cols {
            self.grid.x = self.grid.cols - 1;
            self.grid.wrap = true
        } else {
            self.grid.x += width as usize;
            self.grid.wrap = false
        }
    }
    fn newline(&mut self) {
        self.grid.wrap = false;
        if self.grid.y == self.grid.bottom {
            self.scroll_up(1)
        } else {
            self.grid.y = (self.grid.y + 1).min(self.grid.rows - 1)
        }
    }
    fn scroll_up(&mut self, n: usize) {
        let g = &mut self.grid;
        let n = n.min(g.bottom - g.top + 1);
        let c = g.cols;
        if g.top == 0 && self.primary.is_none() {
            for y in 0..n {
                let row = &g.cells[y * c..(y + 1) * c];
                let end = row
                    .iter()
                    .rposition(|cell| *cell != Cell::default())
                    .map_or(0, |i| i + 1);
                let evicted = self.history.push(HistoryRow::from_cells(&row[..end]));
                self.history_base = self.history_base.saturating_add(evicted as u64);
            }
        }
        g.cells[g.top * c..(g.bottom + 1) * c].rotate_left(n * c);
        let blank = Cell {
            style: self.style,
            ..Cell::default()
        };
        g.cells[(g.bottom + 1 - n) * c..(g.bottom + 1) * c].fill(blank);
    }
    fn scroll_down(&mut self, n: usize) {
        let g = &mut self.grid;
        let n = n.min(g.bottom - g.top + 1);
        let c = g.cols;
        g.cells[g.top * c..(g.bottom + 1) * c].rotate_right(n * c);
        g.cells[g.top * c..(g.top + n) * c].fill(Cell {
            style: self.style,
            ..Cell::default()
        });
    }
    fn save(&mut self) {
        self.saved = (self.grid.x, self.grid.y, self.style);
        self.saved_link = self.active_link.clone();
    }
    fn restore(&mut self) {
        self.grid.x = self.saved.0.min(self.grid.cols - 1);
        self.grid.y = self.saved.1.min(self.grid.rows - 1);
        self.style = self.saved.2;
        self.active_link = self.saved_link.clone();
        self.grid.wrap = false
    }
    fn csi(&mut self, raw: &[u8], op: u8) {
        let private = raw.first() == Some(&b'?');
        let prefix = raw.first().copied();
        // Do not mistake a keyboard-protocol query (CSI ? u) for restore-cursor,
        // nor unsupported prefixed/intermediate sequences for ordinary commands.
        if private && !matches!(op, b'h' | b'l' | b'n' | b'c')
            || prefix == Some(b'>') && op != b'c'
            || prefix == Some(b'!')
        {
            return;
        }
        let start = usize::from(matches!(prefix, Some(b'?') | Some(b'>')));
        if op == b'm' {
            if !private && prefix != Some(b'>') {
                self.sgr_raw(raw);
            }
            return;
        }
        if !raw[start..]
            .iter()
            .all(|b| b.is_ascii_digit() || *b == b';')
        {
            return;
        }
        let s = String::from_utf8_lossy(&raw[start..]);
        let p: Vec<usize> = s
            .split(';')
            .take(64)
            .map(|s| s.parse::<usize>().unwrap_or(0).min(65535))
            .collect();
        let n = p.first().copied().unwrap_or(0);
        let one = n.max(1);
        let param = |i: usize| p.get(i).copied().unwrap_or(0).max(1);
        match op {
            b'A' => {
                self.grid.y =
                    self.grid
                        .y
                        .saturating_sub(one)
                        .max(if self.origin { self.grid.top } else { 0 })
            }
            b'B' | b'e' => {
                self.grid.y = (self.grid.y + one).min(if self.origin {
                    self.grid.bottom
                } else {
                    self.grid.rows - 1
                })
            }
            b'C' | b'a' => self.grid.x = (self.grid.x + one).min(self.grid.cols - 1),
            b'D' => self.grid.x = self.grid.x.saturating_sub(one),
            b'E' => {
                self.grid.x = 0;
                self.grid.y = (self.grid.y + one).min(self.grid.rows - 1)
            }
            b'F' => {
                self.grid.x = 0;
                self.grid.y = self.grid.y.saturating_sub(one)
            }
            b'G' | b'`' => self.grid.x = (one - 1).min(self.grid.cols - 1),
            b'd' => {
                self.grid.y =
                    (one - 1 + if self.origin { self.grid.top } else { 0 }).min(self.grid.rows - 1)
            }
            b'H' | b'f' => {
                self.grid.y =
                    (one - 1 + if self.origin { self.grid.top } else { 0 }).min(if self.origin {
                        self.grid.bottom
                    } else {
                        self.grid.rows - 1
                    });
                self.grid.x = (param(1) - 1).min(self.grid.cols - 1)
            }
            b'J' => {
                let i = self.grid.y * self.grid.cols + self.grid.x;
                let len = self.grid.cells.len();
                let range = match n {
                    0 => i..len,
                    1 => 0..i + 1,
                    2 | 3 => 0..len,
                    _ => 0..0,
                };
                let blank = self.blank();
                self.grid.cells[range].fill(blank);
                if n == 3 {
                    self.history_base = self
                        .history_base
                        .saturating_add((self.history.len() + self.grid.rows) as u64);
                    self.history.clear()
                }
                self.grid.repair()
            }
            b'K' => {
                let start = self.grid.y * self.grid.cols;
                let range = match n {
                    0 => start + self.grid.x..start + self.grid.cols,
                    1 => start..start + self.grid.x + 1,
                    2 => start..start + self.grid.cols,
                    _ => 0..0,
                };
                let blank = self.blank();
                self.grid.cells[range].fill(blank);
                self.grid.repair()
            }
            b'X' => {
                let i = self.grid.y * self.grid.cols + self.grid.x;
                let n = one.min(self.grid.cols - self.grid.x);
                let blank = self.blank();
                self.grid.cells[i..i + n].fill(blank);
                self.grid.repair()
            }
            b'P' | b'@' => {
                let i = self.grid.y * self.grid.cols + self.grid.x;
                let end = (self.grid.y + 1) * self.grid.cols;
                let n = one.min(end - i);
                let blank = self.blank();
                let slice = &mut self.grid.cells[i..end];
                if op == b'P' {
                    slice.rotate_left(n);
                    slice[end - i - n..].fill(blank)
                } else {
                    slice.rotate_right(n);
                    slice[..n].fill(blank)
                }
                self.grid.repair()
            }
            b'L' | b'M' => {
                if self.grid.y >= self.grid.top && self.grid.y <= self.grid.bottom {
                    let top = self.grid.top;
                    self.grid.top = self.grid.y;
                    if op == b'L' {
                        self.scroll_down(one)
                    } else {
                        self.scroll_up(one)
                    }
                    self.grid.top = top
                }
            }
            b'S' => self.scroll_up(one),
            b'T' => self.scroll_down(one),
            b'r' if !private => {
                let top = one - 1;
                let bottom = p
                    .get(1)
                    .copied()
                    .filter(|n| *n > 0)
                    .unwrap_or(self.grid.rows)
                    - 1;
                if top < bottom && bottom < self.grid.rows {
                    self.grid.top = top;
                    self.grid.bottom = bottom;
                    self.grid.x = 0;
                    self.grid.y = if self.origin { top } else { 0 }
                }
            }
            b's' => self.save(),
            b'u' => self.restore(),
            b'h' | b'l' if private => {
                let yes = op == b'h';
                for mode in p {
                    match mode {
                        1 => self.app_cursor = yes,
                        6 => {
                            self.origin = yes;
                            self.grid.x = 0;
                            self.grid.y = if yes { self.grid.top } else { 0 }
                        }
                        7 => self.autowrap = yes,
                        25 => self.cursor = yes,
                        2004 => self.bracketed_paste = yes,
                        1007 => self.alternate_scroll = yes,
                        47 | 1047 | 1049 => {
                            if yes && self.primary.is_none() {
                                self.primary_style = self.style;
                                self.primary_link = self.active_link.take();
                                let blank = Grid::new(self.grid.cols, self.grid.rows);
                                self.primary = Some(std::mem::replace(&mut self.grid, blank));
                            } else if !yes && let Some(primary) = self.primary.take() {
                                self.grid = primary;
                                self.style = self.primary_style;
                                self.active_link = self.primary_link.take();
                            }
                        }
                        _ => {}
                    }
                }
            }
            b'n' => {
                if n == 5 {
                    self.reply(b"\x1b[0n")
                } else if n == 6 {
                    self.reply(format!("\x1b[{};{}R", self.grid.y + 1, self.grid.x + 1).as_bytes())
                }
            }
            b'c' => {
                if prefix == Some(b'>') {
                    self.reply(b"\x1b[>0;1;0c")
                } else {
                    self.reply(b"\x1b[?1;2c")
                }
            }
            _ => {}
        }
        if op != b'm' {
            self.grid.wrap = false
        }
        if self.replies.len() > 8192 {
            self.replies.clear()
        }
    }
    fn sgr_raw(&mut self, raw: &[u8]) {
        let Ok(text) = std::str::from_utf8(raw) else {
            return;
        };
        let mut plain = Vec::new();
        for item in text.split(';').take(64) {
            if item.contains(':') {
                self.sgr(&plain);
                plain.clear();
                let values: Option<Vec<u16>> = item
                    .split(':')
                    .map(|p| {
                        if p.is_empty() {
                            Some(0)
                        } else {
                            p.parse().ok()
                        }
                    })
                    .collect();
                let Some(v) = values else { continue };
                // ISO-style RGB accepts an omitted/zero color-space field.
                let color = match v.as_slice() {
                    [38 | 48, 5, n] if *n <= 255 => Some(Color::Index(*n as u8)),
                    [38 | 48, 2, r, g, b] | [38 | 48, 2, 0, r, g, b]
                        if *r <= 255 && *g <= 255 && *b <= 255 =>
                    {
                        Some(Color::Rgb(*r as u8, *g as u8, *b as u8))
                    }
                    _ => None,
                };
                if let Some(c) = color {
                    if v[0] == 38 {
                        self.style.fg = c
                    } else {
                        self.style.bg = c
                    }
                }
            } else if item.is_empty() {
                plain.push(0);
            } else if let Ok(n) = item.parse::<usize>() {
                plain.push(n.min(65535));
            } else {
                return;
            }
        }
        self.sgr(&plain);
    }
    fn sgr(&mut self, p: &[usize]) {
        let mut i = 0;
        while i < p.len() {
            match p[i] {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                2 => self.style.dim = true,
                3 => self.style.italic = true,
                9 => self.style.strikethrough = true,
                4 => self.style.underline = true,
                7 => self.style.inverse = true,
                22 => {
                    self.style.bold = false;
                    self.style.dim = false;
                }
                23 => self.style.italic = false,
                29 => self.style.strikethrough = false,
                24 => self.style.underline = false,
                27 => self.style.inverse = false,
                30..=37 => self.style.fg = Color::Index((p[i] - 30) as u8),
                40..=47 => self.style.bg = Color::Index((p[i] - 40) as u8),
                90..=97 => self.style.fg = Color::Index((p[i] - 90 + 8) as u8),
                100..=107 => self.style.bg = Color::Index((p[i] - 100 + 8) as u8),
                39 => self.style.fg = Color::Default,
                49 => self.style.bg = Color::Default,
                38 | 48 => {
                    let fg = p[i] == 38;
                    let color = if p.get(i + 1) == Some(&5) && i + 2 < p.len() {
                        i += 2;
                        Some(Color::Index(p[i].min(255) as u8))
                    } else if p.get(i + 1) == Some(&2) && i + 4 < p.len() {
                        let c = Color::Rgb(
                            p[i + 2].min(255) as u8,
                            p[i + 3].min(255) as u8,
                            p[i + 4].min(255) as u8,
                        );
                        i += 4;
                        Some(c)
                    } else {
                        None
                    };
                    if let Some(c) = color {
                        if fg {
                            self.style.fg = c
                        } else {
                            self.style.bg = c
                        }
                    }
                }
                _ => {}
            }
            i += 1
        }
    }
    /// DEC 1007 explicitly opts an alternate-screen app into wheel-to-arrow
    /// translation. Never synthesize prompt keys on the primary screen.
    pub fn is_alternate(&self) -> bool {
        self.primary.is_some()
    }
    pub fn alternate_scroll_input(&self, delta: i64) -> Vec<u8> {
        if self.primary.is_none() || !self.alternate_scroll {
            return Vec::new();
        }
        let key: &[u8] = match (delta < 0, self.app_cursor) {
            (true, false) => b"\x1b[A",
            (false, false) => b"\x1b[B",
            (true, true) => b"\x1bOA",
            (false, true) => b"\x1bOB",
        };
        key.repeat(delta.unsigned_abs().min(500) as usize)
    }
    /// One bounded page, anchored to a logical row rather than an offset from
    /// the moving bottom. Primary-screen regions starting at row zero enter history.
    pub fn scrollback(&self, anchor: Option<u64>, delta: i64) -> crate::model::ScrollbackPage {
        let end = self.history_base.saturating_add(self.history.len() as u64);
        let position = anchor
            .unwrap_or(end)
            .saturating_add_signed(delta)
            .clamp(self.history_base, end);
        let available = self.primary.is_none();
        let mut grid = Grid::new(self.grid.cols, self.grid.rows);
        if available {
            let start = (position - self.history_base) as usize;
            for y in 0..grid.rows {
                let row = start + y;
                if let Some(history) = self.history.get(row) {
                    let cells = history.cells();
                    let n = cells.len().min(grid.cols);
                    grid.cells[y * grid.cols..y * grid.cols + n].clone_from_slice(&cells[..n]);
                } else {
                    let source = row - self.history.len();
                    if source < self.grid.rows {
                        grid.cells[y * grid.cols..(y + 1) * grid.cols].clone_from_slice(
                            &self.grid.cells[source * grid.cols..(source + 1) * grid.cols],
                        );
                    }
                }
            }
            grid.repair();
        }
        crate::model::ScrollbackPage {
            available,
            position,
            end,
            cols: grid.cols,
            rows: grid.rows,
            cells: grid.cells,
        }
    }
    pub fn capture(&self, lines: usize) -> String {
        // Read only the bounded recent tail, regardless of total retained history.
        let live = (0..self.grid.rows).rev().map(|y| self.grid.line(y));
        let history = self
            .history
            .iter()
            .rev()
            .take(if self.primary.is_none() {
                lines.min(200)
            } else {
                0
            })
            .map(HistoryRow::text);
        let mut tail: Vec<_> = live
            .chain(history)
            .skip_while(String::is_empty)
            .take(lines.min(200))
            .collect();
        tail.reverse();
        tail.join("\n")
    }
}
// Compact width ranges covering common shell/source text. Full grapheme shaping is a later milestone.
pub fn char_width(c: char) -> u8 {
    unicode_width::UnicodeWidthChar::width(c)
        .unwrap_or(0)
        .min(2) as u8
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alternate_scrolling_requires_both_screen_and_explicit_mode() {
        let mut t = Terminal::new(10, 4);
        assert!(t.alternate_scroll_input(-3).is_empty());
        t.feed(b"\x1b[?1007h");
        assert!(t.alternate_scroll_input(-3).is_empty()); // a primary prompt is never arrow-scrolled
        t.feed(b"\x1b[?1049h");
        assert_eq!(t.alternate_scroll_input(-1), b"\x1b[A");
        t.feed(b"\x1b[?1h");
        let mut t: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(t.alternate_scroll_input(2), b"\x1bOB\x1bOB");
        assert_eq!(t.alternate_scroll_input(i64::MIN).len(), 1500);
        t.feed(b"\x1b[?1007l");
        assert!(t.alternate_scroll_input(-3).is_empty());
        t.feed(b"\x1b[?1049l");
        assert!(t.alternate_scroll_input(-3).is_empty());
    }
    #[test]
    fn styled_scrollback_anchors_survive_output_eviction_refresh_and_resize() {
        use crate::model::ScrollbackPage;
        let mut t = Terminal::new(18, 4);
        for i in 0..20 {
            t.feed(format!("\x1b[32mline-{i:03} 界\x1b[0m\r\n").as_bytes());
        }
        let page = t.scrollback(None, -5);
        assert!(page.available && page.position < page.end);
        assert_eq!(page.cells[0].style.fg, Color::Index(2));
        let encoded = page.encode();
        assert_eq!(ScrollbackPage::decode(&encoded).unwrap().cells, page.cells);
        assert!(ScrollbackPage::decode(&encoded[..encoded.len() - 1]).is_err());
        t.feed(b"next\r\n");
        assert_eq!(t.scrollback(Some(page.position), 0).cells, page.cells);
        let mut t: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(t.scrollback(Some(page.position), 0).cells, page.cells);
        for _ in 0..HISTORY_ROWS + 5 {
            t.feed(b"new\r\n");
        }
        assert_eq!(t.history.len(), HISTORY_ROWS);
        assert!(t.scrollback(Some(page.position), 0).position > page.position);
        let latest = t.scrollback(None, 0);
        assert_eq!(latest.position, latest.end);
        assert_eq!(latest.cells, t.grid.cells);
        t.resize(10, 3);
        let narrow = t.scrollback(None, -4);
        assert_eq!(narrow.cells.len(), 30);
        assert!(narrow.cells.chunks(10).all(|row| row[9].width != 2));
        t.feed(b"\x1b[?1049hhidden\r\n");
        assert!(!t.scrollback(None, -5).available);
        t.feed(b"\x1b[?1049l\x1b[3J");
        assert!(t.history.is_empty());
        assert_eq!(t.scrollback(None, -5).position, t.scrollback(None, 0).end);
    }
    #[test]
    fn top_scroll_region_keeps_transcript_and_legacy_history_is_passive() {
        let mut t = Terminal::new(16, 5);
        t.feed(b"answer A\r\nanswer B\r\nanswer C\r\ncomposer\r\ndraft");
        t.feed(b"\x1b[1;3r\x1b[3;1H\n");
        assert_eq!(t.history.front().unwrap().text(), "answer A");
        assert!(t.grid.line(4).contains("draft"));
        let page = t.scrollback(None, -1);
        assert_eq!(page.position + 1, page.end);
        t.feed(b"\x1b[2;3r\x1b[3;1H\n");
        assert_eq!(t.history.len(), 1); // subregions cannot contaminate global history
        let mut image = serde_json::to_value(&t).unwrap();
        image.as_object_mut().unwrap().remove("history_base");
        image["history"] = serde_json::json!(["old 界e\u{301}", "\x1b]52;c;BAD\x07"]);
        let t: Terminal = serde_json::from_value(image).unwrap();
        assert!(t.valid_state());
        let page = t.scrollback(None, -2);
        assert!(
            page.cells
                .iter()
                .all(|c| !c.text.chars().any(char::is_control))
        );
        assert!(page.cells.iter().any(|c| c.text == "e\u{301}"));
        assert!(t.capture(200).contains("old 界"));
    }
    #[test]
    fn sgr_attributes_colors_resets_and_erased_background() {
        let mut t = Terminal::new(20, 4);
        t.feed(b"\x1b[1;2;3;4;9mA\x1b[22mB\x1b[23;24;29mC\x1b[0mD");
        let a = t.grid.cells[0].style;
        assert!(a.bold && a.dim && a.italic && a.underline && a.strikethrough);
        let b = t.grid.cells[1].style;
        assert!(!b.bold && !b.dim && b.italic && b.underline && b.strikethrough);
        assert_eq!(t.grid.cells[2].style, Style::default());
        assert_eq!(t.grid.cells[3].style, Style::default());
        t.feed(b"\x1b[2;1H\x1b[48;2;41;41;41m\x1b[2Kprompt\x1b[0m");
        assert!(
            t.grid.cells[20..40]
                .iter()
                .all(|c| c.style.bg == Color::Rgb(41, 41, 41))
        );
        t.feed(b"\x1b[3;1H\x1b[38:2::10:20:30;48:5:235;2mX\x1b[22;39;49mY");
        assert_eq!(t.grid.cells[40].style.fg, Color::Rgb(10, 20, 30));
        assert_eq!(t.grid.cells[40].style.bg, Color::Index(235));
        assert!(t.grid.cells[40].style.dim);
        assert_eq!(t.grid.cells[41].style, Style::default());
        // Unsupported color spaces/underline styles must not reset other attributes.
        t.feed(b"\x1b[1;38:2:1:255:255:255;4:3mZ");
        assert!(t.grid.cells[42].style.bold);
        assert_eq!(t.grid.cells[42].style.fg, Color::Default);
    }
    #[test]
    fn startup_probe_fragmentation_and_color_queries_do_not_move_cursor() {
        let bytes = b"\x1b[6n\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b[?u\x1b[c";
        for split in 0..=bytes.len() {
            let mut t = Terminal::new(20, 4);
            t.feed(b"\x1b[3;5H");
            t.feed(&bytes[..split]);
            // Same serialization used during a supported supervisor refresh.
            let mut t: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
            t.feed(&bytes[split..]);
            assert_eq!((t.grid.x, t.grid.y), (4, 2));
            assert_eq!(t.replies, b"\x1b[3;5R\x1b]10;rgb:cccc/cccc/cccc\x1b\\\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\\x1b[?1;2c");
            assert!(t.valid_state());
        }
        let mut t = Terminal::new(20, 4);
        t.feed(b"\x1b]10;?;?\x07");
        assert_eq!(
            t.replies,
            b"\x1b]10;rgb:cccc/cccc/cccc\x1b\\\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\"
        );
    }
    #[test]
    fn osc_is_bounded_and_never_replays_mutations_or_cancelled_strings() {
        let mut t = Terminal::new(20, 4);
        for _ in 0..5000 {
            t.feed(b"\x1b]10;?\x07\x1b[6n");
        }
        assert!(t.replies.len() <= 8192);
        t.replies.clear();
        t.feed(b"safe\x1b]52;c;secret\x07\x1b]11;#ffffff\x07\x1b]10;?\x18");
        t.feed(b"\x1b]11;?");
        t.feed(&[b'x'; 100_000]);
        assert!(t.valid_state());
        t.feed(b"\x1b\\done");
        assert_eq!(t.capture(10), "safedone");
        assert!(t.replies.is_empty());
        t.feed(b"\x1b]11;?\x07");
        assert_eq!(t.replies, b"\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\");
    }
    #[test]
    fn old_refresh_attributes_default_and_new_styles_survive_refresh() {
        let old: Style = serde_json::from_str(
            r#"{"fg":"Default","bg":"Default","bold":true,"underline":false,"inverse":false}"#,
        )
        .unwrap();
        assert!(old.bold);
        assert!(!old.dim && !old.italic && !old.strikethrough);
        let old_parse: Parse =
            serde_json::from_str(r#"{"String":{"osc":true,"escaped":false}}"#).unwrap();
        let mut t = Terminal::new(20, 4);
        t.parse = old_parse;
        t.feed(b"\x1b\\\x1b[2;3;9mstyled");
        let restored: Terminal = serde_json::from_slice(&serde_json::to_vec(&t).unwrap()).unwrap();
        assert_eq!(restored.grid.cells, t.grid.cells);
        assert!(restored.grid.cells[0].style.dim);
        assert!(restored.valid_state());
    }
    #[test]
    fn incremental_unicode_and_wrap() {
        let mut t = Terminal::new(6, 3);
        for b in "ab界e\u{301}z!".as_bytes() {
            t.feed(&[*b])
        }
        assert_eq!(t.grid.line(0), "ab界e\u{301}z");
        assert_eq!(t.grid.line(1), "!");
    }
    #[test]
    fn alternate_restores_shell() {
        let mut t = Terminal::new(10, 3);
        t.feed(b"draft\x1b[?1049h\x1b[2Jeditor\x1b[3;2H\x1b7\x1b[?1049l");
        assert_eq!(t.grid.line(0), "draft");
        assert_eq!(t.grid.x, 5);
    }
    #[test]
    fn cursor_erase_and_color() {
        let mut t = Terminal::new(10, 3);
        t.feed(b"abcd\x1b[2D\x1b[K\x1b[31mZ");
        assert_eq!(t.grid.line(0), "abZ");
        assert_eq!(t.grid.cells[2].style.fg, Color::Index(1));
    }
    #[test]
    fn untrusted_osc_dcs_are_never_replayed() {
        let mut t = Terminal::new(20, 3);
        t.feed(b"safe\x1b]52;c;SECRET\x07\x1bPdanger\x1b\\done");
        assert_eq!(t.capture(10), "safedone");
    }
    #[test]
    fn bounded_history_and_queries() {
        let mut t = Terminal::new(10, 3);
        for _ in 0..HISTORY_ROWS + 20 {
            t.feed(b"x\r\n")
        }
        assert_eq!(t.history.len(), HISTORY_ROWS);
        t.feed(b"\x1b[6n");
        assert_eq!(t.replies, b"\x1b[3;1R");
    }
    #[test]
    fn scroll_region_preserves_header() {
        let mut t = Terminal::new(10, 4);
        t.feed(b"header\x1b[2;4r\x1b[4;1Hlast\n");
        assert_eq!(t.grid.line(0), "header");
        assert_eq!(t.grid.line(2), "last");
    }
    #[test]
    fn resize_cannot_leave_half_wide_char() {
        let mut t = Terminal::new(4, 2);
        t.feed("ab界".as_bytes());
        t.resize(3, 2);
        assert_eq!(t.grid.line(0), "ab");
    }
}

#[cfg(test)]
mod adversarial_tests {
    use super::*;
    #[test]
    fn bounded_random_stream_never_emits_controls_or_leaks_cells() {
        let mut t = Terminal::new(80, 24);
        let mut n = 1u32;
        for round in 0..1000 {
            let mut bytes = [0; 128];
            for b in &mut bytes {
                n = n.wrapping_mul(1664525).wrapping_add(1013904223);
                *b = (n >> 24) as u8;
            }
            t.feed(&bytes);
            if round % 20 == 0 {
                t.resize(2 + (round % 118), 2 + (round % 38));
            }
            assert_eq!(t.grid.cells.len(), t.grid.cols * t.grid.rows);
            assert!(
                t.grid
                    .cells
                    .iter()
                    .all(|c| c.text.len() <= 64 && !c.text.chars().any(char::is_control))
            );
            assert!(t.grid.x < t.grid.cols && t.grid.y < t.grid.rows);
        }
    }
}
