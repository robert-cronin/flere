//! Static Ghostline sidebar study using Flere's portable terminal-cell renderer.
//! No supervisor, application state, native session, or desktop capture is involved.
use flere::{
    screenshot::Frame,
    terminal::{Cell, Color, Style},
};
use std::{fs, io, path::PathBuf};
use unicode_width::UnicodeWidthChar;

const WIDTH: usize = 82;
const HEIGHT: usize = 43;
const BG: Color = Color::Rgb(6, 13, 22);
const PANEL: Color = Color::Rgb(10, 20, 31);
const TEXT: Color = Color::Rgb(218, 229, 248);
const MUTED: Color = Color::Rgb(133, 155, 185);
const BORDER: Color = Color::Rgb(43, 64, 82);
const CYAN: Color = Color::Rgb(47, 231, 238);
const AMBER: Color = Color::Rgb(242, 193, 104);
const PURPLE: Color = Color::Rgb(187, 161, 245);
const SELECTED: Color = Color::Rgb(9, 43, 57);

fn style(fg: Color, bg: Color, bold: bool) -> Style {
    Style {
        fg,
        bg,
        bold,
        ..Style::default()
    }
}

struct Canvas {
    cells: Vec<Cell>,
}
impl Canvas {
    fn new() -> Self {
        Self {
            cells: vec![
                Cell {
                    style: style(TEXT, BG, false),
                    ..Cell::default()
                };
                WIDTH * HEIGHT
            ],
        }
    }
    fn text(&mut self, x: usize, y: usize, limit: usize, text: &str, style: Style) {
        assert!(y < HEIGHT && x + limit <= WIDTH);
        let chars: Vec<_> = text.chars().collect();
        // Every symbol in this static illustration must occupy exactly one cell.
        assert!(chars.iter().all(|c| c.width() == Some(1)));
        for (offset, character) in chars.iter().take(limit).enumerate() {
            let elided = chars.len() > limit && offset + 1 == limit;
            self.cells[y * WIDTH + x + offset] = Cell {
                text: if elided { '…' } else { *character }.to_string(),
                width: 1,
                style,
            };
        }
    }
    fn plain_text(&self) -> String {
        self.cells
            .chunks(WIDTH)
            .map(|row| {
                row.iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

struct Sidebar<'a> {
    canvas: &'a mut Canvas,
    left: usize,
    top: usize,
    width: usize,
    row: usize,
}
impl Sidebar<'_> {
    fn line(&mut self, bg: Color) {
        self.canvas.text(
            self.left,
            self.top + self.row,
            self.width,
            &" ".repeat(self.width),
            style(TEXT, bg, false),
        );
        self.put(self.width - 1, 1, "│", BORDER, bg, false);
    }
    fn put(&mut self, x: usize, width: usize, text: &str, fg: Color, bg: Color, bold: bool) {
        assert!(x + width <= self.width);
        self.canvas.text(
            self.left + x,
            self.top + self.row,
            width,
            text,
            style(fg, bg, bold),
        );
    }
    fn blank(&mut self, bg: Color, selected: bool) {
        self.line(bg);
        if selected {
            self.put(1, 1, "┃", CYAN, bg, false);
        }
        self.row += 1;
    }
    fn section(&mut self, label: &str, count: usize, color: Color, bg: Color) {
        self.line(bg);
        self.put(0, 1, "▌", color, bg, false);
        self.put(4, self.width - 11, label, color, bg, true);
        self.put(self.width - 5, 3, &format!("[{count}]"), color, bg, true);
        self.row += 1;
    }
    fn end_section(&mut self, color: Color) {
        self.line(PANEL);
        self.put(0, 1, "└", color, PANEL, false);
        self.put(
            1,
            self.width - 2,
            &"─".repeat(self.width - 2),
            color,
            PANEL,
            false,
        );
        self.put(self.width - 1, 1, "┘", color, PANEL, false);
        self.row += 1;
        self.blank(PANEL, false);
    }
    fn card(&mut self, card: &Card) {
        let bg = if card.selected { SELECTED } else { PANEL };
        let accent = if card.selected { CYAN } else { TEXT };
        self.line(bg);
        if card.selected {
            self.put(1, 1, "┃", CYAN, bg, false);
        }
        self.put(
            2,
            1,
            if card.children.is_empty() {
                "▸"
            } else {
                "▾"
            },
            accent,
            bg,
            false,
        );
        self.put(4, 3, card.icon, accent, bg, false);
        self.put(8, self.width - 12, card.title, accent, bg, true);
        self.put(
            self.width - 3,
            1,
            &card.children.len().to_string(),
            MUTED,
            bg,
            false,
        );
        self.row += 1;

        self.line(bg);
        if card.selected {
            self.put(1, 1, "┃", CYAN, bg, false);
        }
        self.put(8, self.width - 10, card.project, MUTED, bg, false);
        self.row += 1;

        for (index, (kind, name)) in card.children.iter().enumerate() {
            let current = card.selected && index == 0;
            let bg = if current { Color::Rgb(12, 56, 71) } else { bg };
            let fg = if current { CYAN } else { TEXT };
            self.line(bg);
            if card.selected {
                self.put(1, 1, "┃", CYAN, bg, false);
            }
            let branch = if index + 1 == card.children.len() {
                "└─"
            } else {
                "├─"
            };
            self.put(4, 2, branch, BORDER, bg, false);
            self.put(8, 1, kind, if current { CYAN } else { MUTED }, bg, false);
            let limit = if self.width >= 40 {
                self.width - 21
            } else {
                self.width - 13
            };
            self.put(10, limit, name, fg, bg, current);
            if self.width >= 40 {
                let state = if current {
                    "current"
                } else if *kind == "≡" {
                    "editor"
                } else {
                    "running"
                };
                self.put(self.width - 10, 8, state, MUTED, bg, false);
            } else {
                self.put(self.width - 3, 1, "●", MUTED, bg, false);
            }
            self.row += 1;
        }
        self.blank(bg, card.selected);
    }
}

struct Card {
    title: &'static str,
    icon: &'static str,
    project: &'static str,
    selected: bool,
    children: &'static [(&'static str, &'static str)],
}

fn sidebar(canvas: &mut Canvas, left: usize, width: usize) {
    let mut s = Sidebar {
        canvas,
        left,
        top: 6,
        width,
        row: 0,
    };
    s.line(PANEL);
    s.put(1, width - 4, "WORKSPACES", TEXT, PANEL, true);
    s.put(width - 3, 1, "4", CYAN, PANEL, true);
    s.row += 1;
    s.line(PANEL);
    s.put(1, width - 4, "6 open tabs", MUTED, PANEL, false);
    s.row += 1;
    s.blank(PANEL, false);

    s.section("Pinned", 1, CYAN, Color::Rgb(12, 39, 48));
    s.card(&Card {
        title: "Flere",
        icon: "[R]",
        project: "flere / main",
        selected: true,
        children: &[("›", "zsh"), ("›", "zsh"), ("≡", "TODO-ACTIVE.md")],
    });
    s.end_section(Color::Rgb(34, 107, 119));
    s.section("Needs me", 1, AMBER, Color::Rgb(43, 34, 26));
    s.card(&Card {
        title: "Workspace 1",
        icon: "[S]",
        project: "sample-project-fixture",
        selected: false,
        children: &[("›", "zsh"), ("›", "zsh")],
    });
    s.end_section(Color::Rgb(118, 86, 43));
    s.section("Todo", 2, PURPLE, Color::Rgb(34, 29, 49));
    s.card(&Card {
        title: "Session restore",
        icon: "[S]",
        project: "sample-project-fixture",
        selected: false,
        children: &[("›", "codex")],
    });
    s.card(&Card {
        title: "ExampleCo",
        icon: "[E]",
        project: "stopped · no open tabs",
        selected: false,
        children: &[],
    });
    s.end_section(Color::Rgb(87, 69, 122));
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 1 {
        return Err(io::Error::other("usage: sidebar_study OUTPUT.png"));
    }
    let output = PathBuf::from(&args[0]);
    let mut canvas = Canvas::new();
    canvas.text(
        2,
        1,
        78,
        "GHOSTLINE / A · SECTION BARS",
        style(CYAN, BG, true),
    );
    canvas.text(
        2,
        2,
        78,
        "Design illustration · fixed terminal cells · exactly four cards",
        style(MUTED, BG, false),
    );
    canvas.text(
        2,
        4,
        46,
        "46 columns · window width ≥134",
        style(TEXT, BG, true),
    );
    canvas.text(52, 4, 28, "28 columns · window ≥62", style(TEXT, BG, true));
    sidebar(&mut canvas, 2, 46);
    sidebar(&mut canvas, 52, 28);
    canvas.text(
        2,
        38,
        78,
        "FIXED GUTTERS / ZERO-BASED COLUMNS",
        style(CYAN, BG, true),
    );
    canvas.text(
        2,
        39,
        78,
        "Disclosure 2 · icon 4–6 · title 8 · child name 10 · count W−3",
        style(TEXT, BG, false),
    );
    canvas.text(
        2,
        40,
        78,
        "Narrow metadata truncates; selected parent and children share one surface.",
        style(MUTED, BG, false),
    );
    canvas.text(
        2,
        41,
        78,
        "Portable Frame renderer / no live app state or role semantics",
        style(MUTED, BG, false),
    );
    let text = canvas.plain_text();
    let frame = Frame {
        width: WIDTH,
        height: HEIGHT,
        cells: canvas.cells,
        layers: Vec::new(),
        cursor: None,
    };
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(&output, frame.png()?)?;
    fs::write(output.with_extension("txt"), text)?;
    println!(
        "{}: {WIDTH}×{HEIGHT} terminal cells; 46/28-column sidebars",
        output.display()
    );
    Ok(())
}
