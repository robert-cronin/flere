//! Recreate only validated passive web links. The outer terminal owns activation.
use super::*;

pub(super) const CLOSE: &str = "\x1b]8;;\x1b\\";
const OPEN: &str = "\x1b]8;;";
const END: &str = "\x1b\\";
// Charge complete link runs before diffing, including unchanged rows. A stable
// plain-text fallback avoids unbounded output and repeated repaint attempts.
const PAINT_LINK_BYTES: usize = 256 * 1024;

pub(super) fn snapshot_command(panes: bool, links: bool) -> &'static str {
    if links {
        "snapshot-links"
    } else if panes {
        "snapshot-panes"
    } else {
        "snapshot"
    }
}

pub(super) fn initial_snapshot(state: &Path) -> io::Result<(Snapshot, bool, bool)> {
    for (command, panes, links) in [
        ("snapshot-links", true, true),
        ("snapshot-panes", true, false),
        ("snapshot", false, false),
    ] {
        match wire::request(state, &[command]) {
            Ok(bytes) => return Ok((Snapshot::decode(&bytes)?, panes, links)),
            Err(error) if error.to_string() == "unknown command" && command != "snapshot" => {}
            Err(error) => return Err(error),
        }
    }
    unreachable!("the legacy snapshot either succeeds or returns its error")
}

impl Ui {
    pub(super) fn page_command(&self, operation: &'static str) -> &'static str {
        match (self.link_capable, operation) {
            (true, "scrollback") => "scrollback-links",
            (true, "command-jump") => "command-jump-links",
            (true, "wheel") => "wheel-links",
            _ => operation,
        }
    }
}

fn paint_cells(c: &Canvas, mut budget: usize) -> Vec<Cell> {
    let mut cells = c.cells.clone();
    for row in cells.chunks_mut(c.width) {
        let mut first = 0;
        while first < row.len() {
            let mut end = first + 1;
            while end < row.len() && row[end].link == row[first].link {
                end += 1;
            }
            if let Some(link) = &row[first].link {
                let cost = OPEN.len() + link.as_str().len() + END.len() + CLOSE.len();
                if row[first..end].iter().all(|cell| cell.width == 0) || cost > budget {
                    for cell in &mut row[first..end] {
                        cell.link = None;
                    }
                } else {
                    budget -= cost;
                }
            }
            first = end;
        }
    }
    cells
}

pub(super) fn render(c: &Canvas, previous: &mut Vec<Cell>) -> String {
    render_bounded(c, previous, PAINT_LINK_BYTES)
}

fn render_bounded(c: &Canvas, previous: &mut Vec<Cell>, budget: usize) -> String {
    // Include the fixed row boundaries and display_frame's two resets in the
    // total OSC8 budget, even when only a subset of rows needs repainting.
    let boundary_bytes = (c.height.saturating_mul(2) + 2).saturating_mul(CLOSE.len());
    let cells = paint_cells(c, budget.saturating_sub(boundary_bytes));
    let mut out = String::new();
    for y in 0..c.height {
        let row = &cells[y * c.width..(y + 1) * c.width];
        if previous.len() == cells.len() && row == &previous[y * c.width..(y + 1) * c.width] {
            continue;
        }
        out.push_str(CLOSE);
        out.push_str(&format!("\x1b[{};1H", y + 1));
        let mut last_style = None;
        let mut last_link = None;
        let mut reanchor = false;
        for (x, cell) in row.iter().enumerate() {
            if previous.len() == cells.len()
                && previous[y * c.width + x] == *cell
                && c.badges.iter().any(|b| {
                    usize::from(b.y) == y
                        && x >= usize::from(b.x)
                        && x < usize::from(b.x + b.columns)
                })
            {
                reanchor = true;
                continue;
            }
            if cell.width == 0 {
                continue;
            }
            if cell.link.as_ref() != last_link {
                if last_link.is_some() {
                    out.push_str(CLOSE);
                }
                if let Some(link) = &cell.link {
                    out.push_str(OPEN);
                    out.push_str(link.as_str());
                    out.push_str(END);
                }
                last_link = cell.link.as_ref();
            }
            // Cursor positioning doesn't paint text. Keeping the same link open
            // across these positions avoids repeating a long URI per Unicode cell.
            let unicode = !cell.text.is_ascii();
            if reanchor || unicode {
                out.push_str(&format!("\x1b[{};{}H", y + 1, x + 1));
            }
            reanchor = unicode;
            if last_style != Some(cell.style) {
                ansi_style(cell.style, &mut out);
                last_style = Some(cell.style);
            }
            out.push_str(&cell.text);
        }
        out.push_str(CLOSE);
    }
    *previous = cells;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Terminal, hyperlinks::Hyperlink};

    fn link(c: &mut Canvas, row: usize, start: usize, end: usize, uri: &str) {
        let link = Hyperlink::new(uri).unwrap();
        for cell in &mut c.cells[row * c.width + start..row * c.width + end] {
            cell.link = Some(link.clone());
        }
    }

    fn outer(c: &Canvas, paint: &str) -> Terminal {
        let mut terminal = Terminal::new(c.width, c.height);
        terminal.feed(b"\x1b[?7l");
        for byte in paint.bytes() {
            terminal.feed(&[byte]);
        }
        terminal
    }

    #[test]
    fn negotiation_falls_back_only_for_unsupported_commands() {
        use std::os::unix::net::UnixListener;
        for mode in 0..4 {
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tests")
                .join(&os::nonce().unwrap()[..12]);
            fs::create_dir_all(&root).unwrap();
            let listener = UnixListener::bind(root.join("control.sock")).unwrap();
            let snapshot = Snapshot {
                epoch: "negotiation".into(),
                generation: 0,
                active: 0,
                tab: 0,
                workspaces: Vec::new(),
                cols: 2,
                rows: 2,
                x: 0,
                y: 0,
                cursor: false,
                bracketed_paste: false,
                app_cursor: false,
                notice: String::new(),
                cells: vec![Cell::default(); 4],
                split: None,
            };
            let worker = std::thread::spawn(move || {
                for (index, command) in ["snapshot-links", "snapshot-panes", "snapshot"]
                    .into_iter()
                    .enumerate()
                {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    assert_eq!(wire::read_frame(&mut socket).unwrap(), command.as_bytes());
                    let bytes = if mode == 3 {
                        b"malformed successful snapshot".to_vec()
                    } else if index < mode {
                        b"!unknown command".to_vec()
                    } else {
                        match mode {
                            0 => snapshot.encode_links(),
                            1 => snapshot.encode_panes(),
                            _ => snapshot.encode(),
                        }
                    };
                    socket.write_all(&wire::frame(&bytes)).unwrap();
                    if mode == 3 || index == mode {
                        break;
                    }
                }
            });
            let result = initial_snapshot(&root);
            worker.join().unwrap();
            fs::remove_dir_all(&root).unwrap();
            if mode == 3 {
                assert!(
                    result.is_err(),
                    "a malformed supported frame must not downgrade"
                );
            } else {
                let (snapshot, panes, links) = result.unwrap();
                assert_eq!(snapshot.epoch, "negotiation");
                assert_eq!((panes, links), (mode < 2, mode == 0));
            }
        }
    }

    #[test]
    fn terminal_hyperlinks_survive_unicode_runs_and_do_not_link_chrome() {
        let mut c = Canvas::new(48, 4);
        c.text(
            0,
            0,
            48,
            "左側  GitHub Releases    Inspector",
            Style::default(),
        );
        link(&mut c, 0, 0, 22, "https://example.com/releases");
        let output = render(&c, &mut Vec::new());
        assert_eq!(output.matches("https://example.com/releases").count(), 1);
        let mut terminal = outer(&c, &output);
        assert_eq!(terminal.grid.cells, c.cells);
        terminal.feed(b"\x1b[4;1Hprompt");
        assert!(
            terminal.grid.cells[3 * 48..3 * 48 + 6]
                .iter()
                .all(|cell| cell.link.is_none())
        );
    }

    #[test]
    fn link_only_changes_repaint_and_modal_erasure_removes_targets() {
        let mut c = Canvas::new(32, 4);
        c.text(2, 1, 20, "Open link", Style::default());
        link(&mut c, 1, 2, 11, "https://example.com/one");
        let mut previous = Vec::new();
        let mut terminal = outer(&c, &render(&c, &mut previous));
        assert!(render(&c, &mut previous).is_empty());
        link(&mut c, 1, 2, 11, "https://example.com/two");
        let changed = render(&c, &mut previous);
        assert!(changed.contains("https://example.com/two"));
        terminal.feed(changed.as_bytes());
        assert_eq!(terminal.grid.cells, c.cells);
        c.fill(0, 0, 32, 4, Style::default());
        c.text(0, 0, 32, "Modal prompt", Style::default());
        terminal.feed(render(&c, &mut previous).as_bytes());
        assert!(terminal.grid.cells.iter().all(|cell| cell.link.is_none()));
        assert_eq!(terminal.grid.cells, c.cells);
    }

    #[test]
    fn separate_runs_and_rows_keep_their_own_targets() {
        let mut c = Canvas::new(40, 4);
        c.text(0, 0, 40, "Left pane | Right pane", Style::default());
        c.text(0, 1, 40, "Next row", Style::default());
        link(&mut c, 0, 0, 9, "https://example.com/left");
        link(&mut c, 0, 12, 22, "https://example.com/right");
        link(&mut c, 1, 0, 8, "https://example.com/next");
        let output = render(&c, &mut Vec::new());
        assert_eq!(outer(&c, &output).grid.cells, c.cells);
    }

    #[test]
    fn output_budget_preserves_text_and_settles_without_repaint_loop() {
        let mut c = Canvas::new(40, 4);
        c.text(0, 0, 40, "First link then second link", Style::default());
        link(&mut c, 0, 0, 10, "https://example.com/first");
        link(&mut c, 0, 16, 27, "https://example.com/second");
        let budget = OPEN.len()
            + "https://example.com/first".len()
            + END.len()
            + CLOSE.len()
            + (c.height * 2 + 2) * CLOSE.len();
        let mut previous = Vec::new();
        let paint = render_bounded(&c, &mut previous, budget);
        assert!(paint.contains("https://example.com/first"));
        assert!(!paint.contains("https://example.com/second"));
        let osc_bytes: usize = paint
            .split("\x1b]8;")
            .skip(1)
            .map(|tail| "\x1b]8;".len() + tail.find(END).unwrap() + END.len())
            .sum();
        assert!(osc_bytes + 2 * CLOSE.len() <= budget);

        let terminal = outer(&c, &paint);
        assert_eq!(terminal.grid.line(0), "First link then second link");
        assert!(
            terminal.grid.cells[16..27]
                .iter()
                .all(|cell| cell.link.is_none())
        );
        assert!(render_bounded(&c, &mut previous, budget).is_empty());
    }

    #[test]
    fn long_url_is_not_repeated_at_each_unicode_cursor_anchor() {
        let mut c = Canvas::new(80, 4);
        c.text(0, 0, 80, &"界".repeat(35), Style::default());
        let uri = format!("https://example.com/{}", "a".repeat(1900));
        link(&mut c, 0, 0, 70, &uri);
        let paint = render(&c, &mut Vec::new());
        assert_eq!(paint.matches(&uri).count(), 1);
        assert!(paint.len() < 6000);
        assert_eq!(outer(&c, &paint).grid.cells, c.cells);
    }
}
