//! UI-owned, bounded frozen selections. Only explicit release emits OSC 52.
use super::*;
use crate::{model::ScrollbackPage, terminal::HistoryRow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::collections::BTreeMap;
mod paths;
const CACHE_BYTES: usize = 4 * 1024 * 1024;
const CACHE_ROWS: usize = 4096;
type Point = (u64, usize);
struct History {
    rows: BTreeMap<u64, HistoryRow>,
    bytes: usize,
    end: u64,
}
struct Edge {
    direction: i64,
    entered: Instant,
    checked: Instant,
}
pub(super) struct Selection {
    location: (String, u64, u64, String),
    layout: Layout,
    terminal_size: (usize, usize),
    cells: Vec<Cell>,
    anchor: Point,
    end: Point,
    top: u64,
    history: Option<History>,
    pointer: (usize, usize),
    edge: Option<Edge>,
    error: Option<&'static str>,
    moved: bool,
    pub dragging: bool,
}
fn normalized(cells: &[Cell]) -> Vec<Cell> {
    cells
        .iter()
        .cloned()
        .map(|mut c| {
            if c.style.fg == Color::Default {
                c.style.fg = crate::terminal::DEFAULT_FG;
            }
            if c.style.bg == Color::Default {
                c.style.bg = crate::terminal::DEFAULT_BG;
            }
            c
        })
        .collect()
}
impl Selection {
    pub fn new(snapshot: &Snapshot, layout: Layout, canvas: &Canvas, x: usize, y: usize) -> Self {
        let mut cells = Vec::with_capacity(layout.cols * layout.rows);
        for row in 0..layout.rows {
            let start = (layout.terminal_y + row) * canvas.width + layout.terminal_x;
            cells.extend_from_slice(&canvas.cells[start..start + layout.cols]);
        }
        let anchor = ((y - layout.terminal_y) as u64, x - layout.terminal_x);
        Self {
            location: location(snapshot),
            layout,
            terminal_size: (snapshot.cols, snapshot.rows),
            cells,
            anchor,
            end: anchor,
            top: 0,
            history: None,
            pointer: (x, y),
            edge: None,
            error: None,
            moved: false,
            dragging: true,
        }
    }
    /// The page must describe exactly the cells the user pressed. If output
    /// raced the request, retain the displayed selection without guessing IDs.
    pub fn attach_history(&mut self, page: &ScrollbackPage) {
        if !page.available
            || (page.cols, page.rows) != (self.layout.cols, self.layout.rows)
            || normalized(&page.cells) != self.cells
        {
            return;
        }
        self.top = page.position;
        self.anchor.0 += self.top;
        self.end.0 += self.top;
        let mut history = History {
            rows: BTreeMap::new(),
            bytes: 0,
            end: page.end,
        };
        for (y, row) in self.cells.chunks(self.layout.cols).enumerate() {
            let row = HistoryRow::from_cells(row);
            history.bytes += row.bytes();
            history.rows.insert(self.top + y as u64, row);
        }
        self.history = Some(history);
    }
    pub fn matches(&self, snapshot: &Snapshot, layout: Layout) -> bool {
        self.location == location(snapshot)
            && self.layout == layout
            && self.terminal_size == (snapshot.cols, snapshot.rows)
    }
    pub fn extend(&mut self, x: usize, y: usize) {
        self.pointer = (x, y);
        let column = x
            .saturating_sub(self.layout.terminal_x)
            .min(self.layout.cols - 1);
        let row = y
            .saturating_sub(self.layout.terminal_y)
            .min(self.layout.rows - 1);
        self.end = (self.top + row as u64, column);
        self.moved |= self.end != self.anchor;
        let direction = if self.moved && y <= self.layout.terminal_y {
            -1
        } else if self.moved && y >= self.layout.terminal_y + self.layout.rows - 1 {
            1
        } else {
            0
        };
        if direction == 0 {
            self.edge = None;
            self.error = None;
        } else if self.edge.as_ref().is_none_or(|e| e.direction != direction) {
            self.edge = Some(Edge {
                direction,
                entered: Instant::now(),
                checked: Instant::now(),
            });
            self.error = None;
        }
    }
    pub fn scroll_target(&mut self) -> Result<Option<u64>, &'static str> {
        let Some(edge) = &mut self.edge else {
            return Ok(None);
        };
        if !self.dragging
            || edge.entered.elapsed() < Duration::from_millis(240)
            || edge.checked.elapsed() < Duration::from_millis(40)
        {
            return Ok(None);
        }
        edge.checked = Instant::now();
        let Some(history) = &self.history else {
            return Err("Selection kept · history changed or unavailable · release to copy");
        };
        let distance = if edge.direction < 0 {
            self.layout.terminal_y.saturating_sub(self.pointer.1)
        } else {
            self.pointer
                .1
                .saturating_sub(self.layout.terminal_y + self.layout.rows - 1)
        };
        let step = (1 + distance / 2).min(4) as i64 * edge.direction;
        let target = self.top.saturating_add_signed(step).min(history.end);
        if target == self.top {
            self.edge = None;
            return Ok(None);
        }
        Ok(Some(target))
    }
    pub fn cached(&self, top: u64) -> bool {
        self.history
            .as_ref()
            .is_some_and(|h| (0..self.layout.rows).all(|y| h.rows.contains_key(&(top + y as u64))))
    }
    pub fn move_view(&mut self, top: u64) {
        let h = self.history.as_ref().unwrap();
        self.cells = (0..self.layout.rows)
            .flat_map(|y| h.rows[&(top + y as u64)].cells())
            .collect();
        self.top = top;
        self.extend(self.pointer.0, self.pointer.1);
    }
    pub fn accept_page(&mut self, page: ScrollbackPage, target: u64) -> Result<bool, &'static str> {
        if !page.available || (page.cols, page.rows) != (self.layout.cols, self.layout.rows) {
            return Err("Selection kept · terminal history changed · release to copy");
        }
        let h = self.history.as_mut().unwrap();
        let first = *h.rows.first_key_value().unwrap().0;
        let last = *h.rows.last_key_value().unwrap().0;
        if page.position > h.end
            || (page.position != target
                && !(target < self.top && page.position <= self.top && page.position > target))
            || page.position > last.saturating_add(1)
            || page.position.saturating_add(page.rows as u64) < first
        {
            return Err("Selection kept · older history was evicted · release to copy");
        }
        if page.position == self.top {
            self.edge = None;
            return Ok(false);
        }
        let mut fresh = Vec::new();
        let mut bytes = 0;
        for (y, row) in normalized(&page.cells).chunks(page.cols).enumerate() {
            let at = page.position + y as u64;
            if !h.rows.contains_key(&at) {
                let row = HistoryRow::from_cells(row);
                bytes += row.bytes();
                fresh.push((at, row));
            }
        }
        if h.rows.len() + fresh.len() > CACHE_ROWS || h.bytes + bytes > CACHE_BYTES {
            return Err(
                "Selection history limit reached · release to copy, or select a smaller range",
            );
        }
        h.rows.extend(fresh);
        h.bytes += bytes;
        self.move_view(page.position);
        Ok(true)
    }
    pub fn stop_scroll(&mut self, message: &'static str) {
        self.edge = None;
        self.error = Some(message);
    }
    pub fn status(&self) -> &'static str {
        self.error.unwrap_or(
            "Selecting text · drag at top/bottom to scroll · release to copy · Esc cancels",
        )
    }
    pub fn page(&self) -> ScrollbackPage {
        ScrollbackPage {
            available: true,
            position: self.top,
            end: self.history.as_ref().unwrap().end,
            cols: self.layout.cols,
            rows: self.layout.rows,
            cells: self.cells.clone(),
        }
    }
    fn row(&self, row: u64) -> Vec<Cell> {
        if let Some(h) = &self.history {
            h.rows[&row].cells()
        } else {
            let start = (row - self.top) as usize * self.layout.cols;
            self.cells[start..start + self.layout.cols].to_vec()
        }
    }
    fn range(&self) -> (Point, Point) {
        let mut start = self.anchor.min(self.end);
        let mut end = self.anchor.max(self.end);
        if start.1 > 0 && self.row(start.0)[start.1].width == 0 {
            start.1 -= 1;
        }
        if end.1 + 1 < self.layout.cols && self.row(end.0)[end.1].width == 2 {
            end.1 += 1;
        }
        (start, end)
    }
    pub fn text(&self) -> Result<String, &'static str> {
        if !self.moved {
            return Ok(String::new());
        }
        let (start, end) = self.range();
        let mut text = String::new();
        for row in start.0..=end.0 {
            if row > start.0 {
                text.push('\n');
            }
            let first = if row == start.0 { start.1 } else { 0 };
            let last = if row == end.0 {
                end.1
            } else {
                self.layout.cols - 1
            };
            let cells = self.row(row);
            let mut line = String::new();
            for cell in &cells[first..=last] {
                if cell.width > 0 {
                    line.push_str(&wire::passive(&cell.text));
                }
            }
            text.push_str(line.trim_end_matches(' '));
            if text.len() > 65536 {
                return Err("Selection exceeds 64 KiB; select a smaller region");
            }
        }
        Ok(text)
    }
    pub fn file_path(&self, cwd: &Path) -> Option<PathBuf> {
        if self.moved {
            return None;
        }
        let anchor = (self.anchor.0 - self.top) as usize * self.layout.cols + self.anchor.1;
        let candidates = paths::candidates(&self.cells, self.layout.cols, anchor, cwd);
        // Complete file captions with spaces take precedence over any shorter
        // directory token from the same clicked, wrapped caption.
        candidates
            .iter()
            .find(|p| p.is_file())
            .or_else(|| candidates.iter().rev().find(|p| p.is_dir()))
            .cloned()
    }
    pub fn paint(&self, canvas: &mut Canvas) {
        let (start, end) = self.range();
        for (i, original) in self.cells.iter().enumerate() {
            let mut cell = original.clone();
            let at = (
                self.top + (i / self.layout.cols) as u64,
                i % self.layout.cols,
            );
            if self.moved && at >= start && at <= end {
                cell.style = style(
                    Color::Rgb(240, 255, 250),
                    Color::Rgb(42, 92, 82),
                    cell.style.bold,
                );
            }
            let dest = (self.layout.terminal_y + i / self.layout.cols) * canvas.width
                + self.layout.terminal_x
                + i % self.layout.cols;
            canvas.cells[dest] = cell;
        }
    }
}
impl Ui {
    pub(super) fn selection_history(&self, selection: &mut Selection) {
        if self
            .previews
            .contains_key(&(self.snapshot.active, self.snapshot.tab))
        {
            return;
        }
        if let Some(s) = &self.scrollback {
            selection.attach_history(&s.page);
            return;
        }
        let Some(t) = self.snapshot.session() else {
            return;
        };
        if let Ok(bytes) = wire::request(
            &self.state,
            &[
                self.page_command("scrollback"),
                &t.id.to_string(),
                &t.run,
                "live",
                "0",
            ],
        ) && let Ok(page) = ScrollbackPage::decode(&bytes)
        {
            selection.attach_history(&page);
        }
    }
    pub(super) fn tick_selection(&mut self) -> bool {
        let Some(mut selection) = self.selection.take() else {
            return false;
        };
        if !selection.matches(&self.snapshot, self.terminal_layout()) {
            return true;
        }
        let mut changed = false;
        if selection.dragging
            && self.focus == Focus::Terminal
            && !self.menu
            && !self.board
            && self.form.is_none()
            && self.confirm.is_none()
            && !self.image_view()
        {
            let result = (|| -> Result<bool, &'static str> {
                let Some(target) = selection.scroll_target()? else {
                    return Ok(false);
                };
                if selection.cached(target) {
                    selection.move_view(target);
                    return Ok(true);
                }
                let Some(t) = self.snapshot.session() else {
                    return Ok(false);
                };
                let page = wire::request(
                    &self.state,
                    &[
                        self.page_command("scrollback"),
                        &t.id.to_string(),
                        &t.run,
                        &target.to_string(),
                        "0",
                    ],
                )
                .and_then(|b| ScrollbackPage::decode(&b))
                .map_err(|_| "Selection kept · history unavailable · release to copy")?;
                selection.accept_page(page, target)
            })();
            match result {
                Ok(true) => {
                    let page = selection.page();
                    self.scrollback = if page.position < page.end {
                        Some(scrollback::Scrollback::new(
                            &self.snapshot,
                            self.terminal_layout(),
                            page,
                        ))
                    } else {
                        None
                    };
                    changed = true;
                }
                Ok(false) => {}
                Err(message) => {
                    selection.stop_scroll(message);
                    changed = true;
                }
            }
        }
        self.selection = Some(selection);
        changed
    }
    pub(super) fn terminal_file_open(&mut self, path: &Path) {
        if path.is_dir() {
            if let Ok(path) = path.canonicalize()
                && let Some(t) = self.snapshot.session()
            {
                self.confirm = Some(confirmation::Confirmation::directory(
                    t,
                    &self.snapshot,
                    path,
                ));
            }
            return;
        }
        if image_preview::image_path(path) {
            self.image_open(path);
            return;
        }
        // Refresh the exact origin before an editor launch; do not retarget a
        // released click to another workspace if watch delivery was delayed.
        let origin = location(&self.snapshot);
        let editor_origin = self.editor_origin();
        self.flush_input();
        let refreshed = self.read_snapshot();
        let Ok(snapshot) = refreshed else {
            self.notice = "File click cancelled: cannot verify active terminal".into();
            return;
        };
        self.snapshot(snapshot);
        if location(&self.snapshot) != origin {
            self.notice = "File click cancelled: active terminal changed".into();
            return;
        }
        if !self.editor_open(&editor_origin, path, None) {
            return;
        }
        if self.snapshot.session().is_some_and(|t| t.kind == "editor") {
            self.previews.remove(&(origin.1, origin.2));
            self.scrollback = None;
            self.smooth_scroll = None;
            self.focus = Focus::Terminal;
            self.nav = false;
        }
    }
}
fn location(s: &Snapshot) -> (String, u64, u64, String) {
    (
        s.epoch.clone(),
        s.active,
        s.tab,
        s.session().map(|t| t.run.clone()).unwrap_or_default(),
    )
}
pub(super) fn clipboard_write(text: &str) -> Result<String, &'static str> {
    if text.len() > 65536 {
        return Err("Selection exceeds 64 KiB; select a smaller region");
    }
    // Base64 prevents selected terminal text from becoming an outer control string.
    // Never query/read the clipboard and never pass through a child's OSC 52.
    Ok(format!(
        "\x1b]52;c;{}\x07",
        STANDARD.encode(text.as_bytes())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Snapshot, Layout, Canvas) {
        let layout = Layout::new(20, 12);
        let mut canvas = Canvas::new(layout.width, layout.height);
        canvas.text(
            layout.terminal_x,
            layout.terminal_y,
            layout.cols,
            "abc界e\u{301}Z",
            Style::default(),
        );
        canvas.text(
            layout.terminal_x,
            layout.terminal_y + 1,
            layout.cols,
            "second line",
            Style::default(),
        );
        let snapshot = Snapshot {
            split: None,
            epoch: "test-epoch".into(),
            generation: 1,
            active: 1,
            tab: 2,
            workspaces: vec![],
            cols: layout.cols,
            rows: layout.rows,
            x: 0,
            y: 0,
            cursor: true,
            bracketed_paste: false,
            app_cursor: false,
            notice: String::new(),
            cells: vec![],
        };
        (snapshot, layout, canvas)
    }
    #[test]
    fn history_selection_retains_anchor_across_output_and_rejects_eviction_and_limits() {
        let (snapshot, l, canvas) = fixture();
        let mut s = Selection::new(&snapshot, l, &canvas, l.terminal_x, l.terminal_y + 2);
        s.cells = normalized(&s.cells);
        let page = ScrollbackPage {
            available: true,
            position: 100,
            end: 100,
            cols: l.cols,
            rows: l.rows,
            cells: s.cells.clone(),
        };
        let mut raced = ScrollbackPage::decode(&page.encode()).unwrap();
        raced.cells[0].text = "changed".into();
        s.attach_history(&raced);
        assert!(s.history.is_none());
        s.attach_history(&page);
        let anchor = s.anchor;
        s.extend(l.terminal_x, 0);
        let edge = s.edge.as_mut().unwrap();
        edge.entered = Instant::now() - Duration::from_secs(1);
        edge.checked = edge.entered;
        assert_eq!(s.scroll_target().unwrap(), Some(97));
        let mut earlier = ScrollbackPage::decode(&page.encode()).unwrap();
        earlier.position = 97;
        earlier.end = 120;
        earlier.cells.iter_mut().for_each(|c| c.text = "x".into());
        assert!(
            s.accept_page(ScrollbackPage::decode(&earlier.encode()).unwrap(), 97)
                .unwrap()
        );
        assert_eq!(s.anchor, anchor);
        assert_eq!(s.history.as_ref().unwrap().end, 100);
        assert_eq!(s.row(100), page.cells[..l.cols]); // frozen overlap never takes newer output
        assert!(s.text().unwrap().starts_with("x"));
        earlier.position = 110;
        assert!(
            s.accept_page(ScrollbackPage::decode(&earlier.encode()).unwrap(), 94)
                .is_err()
        );
        earlier.position = 94;
        s.history.as_mut().unwrap().bytes = CACHE_BYTES;
        assert!(s.accept_page(earlier, 94).is_err());
        assert_eq!(s.top, 97);
        s.dragging = false;
        assert_eq!(s.scroll_target().unwrap(), None);
        assert!(!s.text().unwrap().is_empty());
        let mut unavailable = Selection::new(&snapshot, l, &canvas, l.terminal_x, l.terminal_y + 2);
        unavailable.extend(l.terminal_x, 0);
        let edge = unavailable.edge.as_mut().unwrap();
        edge.entered = Instant::now() - Duration::from_secs(1);
        edge.checked = edge.entered;
        assert!(unavailable.scroll_target().is_err());
    }
    #[test]
    fn image_paths_use_clicked_visible_text_and_support_spaces_and_wrapping() {
        let mut c = Canvas::new(60, 5);
        c.text(0, 0, 60, "See (/home/test/captures/card-", Style::default());
        c.text(0, 1, 60, "    spacing 界.PNG)", Style::default());
        let paths = paths::candidates(&c.cells, 60, 60 + 13, Path::new("/workspace"));
        assert!(paths.contains(&PathBuf::from("/home/test/captures/card-spacing 界.PNG")));
        assert!(
            !paths::candidates(&c.cells, 60, 1, Path::new("/workspace"))
                .contains(&PathBuf::from("/home/test/captures/card-spacing 界.PNG"))
        );
        c = Canvas::new(80, 3);
        c.text(
            0,
            0,
            80,
            "./preview.png  https://example.invalid/private.png  /tmp/plain.png.bak",
            Style::default(),
        );
        assert!(
            paths::candidates(&c.cells, 80, 3, Path::new("/workspace"))
                .contains(&PathBuf::from("/workspace/./preview.png"))
        );
        assert!(paths::candidates(&c.cells, 80, 40, Path::new("/workspace")).is_empty());
        assert!(
            !paths::candidates(&c.cells, 80, 58, Path::new("/workspace"))
                .contains(&PathBuf::from("/tmp/plain.png"))
        );
    }
    #[test]
    fn selection_handles_reverse_multiline_wide_and_combining_cells() {
        let (snapshot, l, canvas) = fixture();
        let mut s = Selection::new(&snapshot, l, &canvas, l.terminal_x + 2, l.terminal_y + 1);
        assert_eq!(s.text().unwrap(), ""); // click alone never replaces the clipboard
        s.extend(l.terminal_x + 4, l.terminal_y); // starts on the wide character's continuation
        assert_eq!(s.text().unwrap(), "界e\u{301}Z\nsec");
        let mut selected = Canvas::new(l.width, l.height);
        s.paint(&mut selected);
        let start = l.terminal_y * l.width + l.terminal_x + 3;
        assert_eq!(
            selected.cells[start].style.bg,
            selected.cells[start + 1].style.bg
        );
        assert_eq!(selected.cells[start].style.bg, Color::Rgb(42, 92, 82));
        let mut stale = snapshot.clone();
        stale.epoch = "new-epoch".into();
        assert!(!s.matches(&stale, l));
        stale = snapshot.clone();
        stale.tab += 1;
        assert!(!s.matches(&stale, l));
        assert!(!s.matches(&snapshot, Layout::new(22, 12)));
    }
    #[test]
    fn clipboard_encoding_is_bounded_data_and_never_raw_escapes() {
        let text = "wide 界\n\x1b]52;c;injection\x07";
        let sequence = clipboard_write(text).unwrap();
        let payload = sequence
            .strip_prefix("\x1b]52;c;")
            .unwrap()
            .strip_suffix('\x07')
            .unwrap();
        assert!(
            payload
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
        );
        assert_eq!(STANDARD.decode(payload).unwrap(), text.as_bytes());
        assert!(clipboard_write(&"x".repeat(65537)).is_err());
    }
}
