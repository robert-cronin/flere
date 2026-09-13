//! Portable export of Flere's own composed frame. No desktop capture or
//! terminal-specific font APIs; cells, graphics, and cursor share grid coordinates.
use crate::terminal::{Cell, Color, DEFAULT_BG, DEFAULT_FG, Style};
use std::{collections::HashMap, io, sync::OnceLock};

pub const CELL_W: usize = 15;
pub const CELL_H: usize = 32;
const FONT_SIZE: f32 = 24.0;
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Cell>,
    pub layers: Vec<Layer>,
    pub cursor: Option<(usize, usize)>,
}
pub struct Layer {
    pub x: usize,
    pub y: usize,
    pub columns: f64,
    pub rows: f64,
    pub pixels: Pixels,
}
pub enum Pixels {
    Encoded(Vec<u8>),
    Rgb(crate::sixel::Raster),
}
fn font() -> io::Result<&'static fontdue::Font> {
    static FONT: OnceLock<Result<fontdue::Font, &'static str>> = OnceLock::new();
    FONT.get_or_init(|| {
        fontdue::Font::from_bytes(
            include_bytes!("assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf").as_slice(),
            fontdue::FontSettings::default(),
        )
    })
    .as_ref()
    .map_err(|e| io::Error::other(*e))
}
fn rgb(color: Color, default: Color) -> [u8; 3] {
    match color {
        Color::Default => rgb(default, Color::Rgb(0, 0, 0)),
        Color::Rgb(r, g, b) => [r, g, b],
        Color::Index(i) if i < 16 => [
            [0, 0, 0],
            [205, 0, 0],
            [0, 205, 0],
            [205, 205, 0],
            [0, 0, 238],
            [205, 0, 205],
            [0, 205, 205],
            [229, 229, 229],
            [127, 127, 127],
            [255, 0, 0],
            [0, 255, 0],
            [255, 255, 0],
            [92, 92, 255],
            [255, 0, 255],
            [0, 255, 255],
            [255, 255, 255],
        ][i as usize],
        Color::Index(i) if i < 232 => {
            let n = i - 16;
            let v = [0, 95, 135, 175, 215, 255];
            [
                v[(n / 36) as usize],
                v[((n / 6) % 6) as usize],
                v[(n % 6) as usize],
            ]
        }
        Color::Index(i) => [8 + (i - 232) * 10; 3],
    }
}
fn colors(style: Style) -> ([u8; 3], [u8; 3]) {
    let (mut fg, mut bg) = (rgb(style.fg, DEFAULT_FG), rgb(style.bg, DEFAULT_BG));
    if style.inverse {
        std::mem::swap(&mut fg, &mut bg);
    }
    if style.dim {
        for i in 0..3 {
            fg[i] = ((fg[i] as u16 + bg[i] as u16) / 2) as u8;
        }
    }
    (fg, bg)
}
fn blend(dest: &mut [u8], color: &[u8], alpha: u8) {
    for i in 0..3 {
        dest[i] = ((color[i] as u32 * alpha as u32 + dest[i] as u32 * (255 - alpha as u32) + 127)
            / 255) as u8;
    }
}
impl Frame {
    pub fn png(&self) -> io::Result<Vec<u8>> {
        if !(1..=320).contains(&self.width)
            || !(1..=106).contains(&self.height)
            || self.cells.len() != self.width * self.height
            || self.cells.iter().map(|c| c.text.len()).sum::<usize>() > 1024 * 1024
            || self.layers.len() > crate::avatar::LIMIT + 1
        {
            return Err(io::Error::other("screenshot frame exceeds bounds"));
        }
        let (width, height) = (self.width * CELL_W, self.height * CELL_H);
        let mut pixels = vec![0; width * height * 3];
        for (i, cell) in self.cells.iter().enumerate() {
            let (_, bg) = colors(cell.style);
            for y in i / self.width * CELL_H..(i / self.width + 1) * CELL_H {
                for x in i % self.width * CELL_W..(i % self.width + 1) * CELL_W {
                    pixels[(y * width + x) * 3..(y * width + x) * 3 + 3].copy_from_slice(&bg);
                }
            }
        }
        let font = font()?;
        let mut glyphs = HashMap::new();
        let baseline = 25i32;
        for (i, cell) in self.cells.iter().enumerate().filter(|(_, c)| c.width > 0) {
            let (fg, _) = colors(cell.style);
            let (left, top) = (i % self.width * CELL_W, i / self.width * CELL_H);
            let cell_width = (cell.width as usize).min(2) * CELL_W;
            for character in cell.text.chars().filter(|c| !c.is_control() && *c != ' ') {
                // Bound cache growth even for adversarial terminal Unicode.
                if glyphs.len() >= 4096 {
                    glyphs.clear();
                }
                let (metrics, mask) = glyphs
                    .entry(character)
                    .or_insert_with(|| font.rasterize(character, FONT_SIZE));
                for y in 0..metrics.height {
                    let gy = baseline - metrics.height as i32 - metrics.ymin + y as i32;
                    if !(0..CELL_H as i32).contains(&gy) {
                        continue;
                    }
                    let slant = if cell.style.italic {
                        (baseline - gy).max(0) / 5
                    } else {
                        0
                    };
                    for x in 0..metrics.width {
                        let gx = metrics.xmin + x as i32 + slant;
                        for bold in 0..=usize::from(cell.style.bold) {
                            let gx = gx + bold as i32;
                            if gx < 0 || gx as usize >= cell_width || left + gx as usize >= width {
                                continue;
                            }
                            let dest = ((top + gy as usize) * width + left + gx as usize) * 3;
                            blend(
                                &mut pixels[dest..dest + 3],
                                &fg,
                                mask[y * metrics.width + x],
                            );
                        }
                    }
                }
            }
            for (enabled, y) in [(cell.style.underline, 28), (cell.style.strikethrough, 16)] {
                if enabled {
                    for x in left..(left + cell_width).min(width) {
                        let dest = ((top + y) * width + x) * 3;
                        pixels[dest..dest + 3].copy_from_slice(&fg);
                    }
                }
            }
        }
        for layer in &self.layers {
            if layer.x >= self.width
                || layer.y >= self.height
                || !layer.columns.is_finite()
                || !layer.rows.is_finite()
                || !(0.0..=320.0).contains(&layer.columns)
                || !(0.0..=106.0).contains(&layer.rows)
            {
                return Err(io::Error::other("screenshot graphic exceeds bounds"));
            }
            let (sw, sh, rgba) = match &layer.pixels {
                Pixels::Encoded(bytes) => decode(bytes)?,
                Pixels::Rgb(r) => {
                    if r.width == 0
                        || r.height == 0
                        || r.width as u64 * r.height as u64 > crate::image_preview::PIXELS
                        || r.rgb.len() != r.width * r.height * 3
                    {
                        return Err(io::Error::other("invalid screenshot graphic"));
                    }
                    (
                        r.width,
                        r.height,
                        r.rgb
                            .as_chunks::<3>()
                            .0
                            .iter()
                            .flat_map(|p| [p[0], p[1], p[2], 255])
                            .collect(),
                    )
                }
            };
            let dw = (layer.columns * CELL_W as f64).round().max(1.0) as usize;
            let dh = (layer.rows * CELL_H as f64).round().max(1.0) as usize;
            let (left, top) = (layer.x * CELL_W, layer.y * CELL_H);
            for y in 0..dh.min(height - top) {
                for x in 0..dw.min(width - left) {
                    let source = ((y * sh / dh) * sw + x * sw / dw) * 4;
                    let dest = ((top + y) * width + left + x) * 3;
                    blend(
                        &mut pixels[dest..dest + 3],
                        &rgba[source..source + 3],
                        rgba[source + 3],
                    );
                }
            }
        }
        if let Some((x, y)) = self
            .cursor
            .filter(|(x, y)| *x < self.width && *y < self.height)
        {
            for py in y * CELL_H..(y + 1) * CELL_H {
                for px in x * CELL_W..x * CELL_W + 2 {
                    let dest = (py * width + px) * 3;
                    pixels[dest..dest + 3].copy_from_slice(&[204, 204, 204]);
                }
            }
        }
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header()?.write_image_data(&pixels)?;
        }
        if png.len() as u64 > crate::remote_protocol::IMAGE_LIMIT {
            return Err(io::Error::other("screenshot exceeds 20 MiB"));
        }
        Ok(png)
    }
}
#[path = "image_decode.rs"]
mod image_decode;
pub fn decode(bytes: &[u8]) -> io::Result<(usize, usize, Vec<u8>)> {
    image_decode::decode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(width: usize, height: usize) -> Frame {
        Frame {
            width,
            height,
            cells: vec![Cell::default(); width * height],
            layers: Vec::new(),
            cursor: None,
        }
    }
    #[test]
    fn portable_frame_preserves_edges_styles_graphics_and_cursor() {
        let mut frame = frame(4, 3);
        for (i, cell) in frame.cells.iter_mut().enumerate() {
            cell.style.bg = Color::Rgb(i as u8 * 10, 20, 30);
        }
        frame.cells[0].style.inverse = true;
        frame.cells[0].style.fg = Color::Rgb(200, 100, 50);
        frame.cells[1].text = "A".into();
        frame.cells[1].style.bold = true;
        frame.cells[1].style.italic = true;
        frame.cells[1].style.underline = true;
        frame.layers.push(Layer {
            x: 2,
            y: 1,
            columns: 1.0,
            rows: 1.0,
            pixels: Pixels::Rgb(crate::sixel::Raster {
                width: 1,
                height: 1,
                rgb: vec![45, 67, 89],
            }),
        });
        frame.cursor = Some((3, 2));
        let (w, h, image) = decode(&frame.png().unwrap()).unwrap();
        assert_eq!((w, h), (4 * CELL_W, 3 * CELL_H));
        let pixel = |x, y| &image[(y * w + x) * 4..(y * w + x) * 4 + 3];
        assert_eq!(pixel(0, 0), [200, 100, 50]);
        assert_eq!(pixel(w - 1, 0), [30, 20, 30]);
        assert_eq!(pixel(0, h - 1), [80, 20, 30]);
        assert_eq!(pixel(w - 1, h - 1), [110, 20, 30]);
        assert_eq!(pixel(2 * CELL_W, CELL_H), [45, 67, 89]);
        assert_eq!(pixel(3 * CELL_W, 2 * CELL_H), [204, 204, 204]);
        assert_eq!(pixel(CELL_W, 28), [204, 204, 204]);
        assert!((2..26).any(|y| (CELL_W..2 * CELL_W).any(|x| pixel(x, y) != [10, 20, 30])));
    }
    #[test]
    fn png_alpha_and_jpeg_decode_portably_and_invalid_frames_fail() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 128])
                .unwrap();
        }
        let mut frame = frame(1, 1);
        frame.layers.push(Layer {
            x: 0,
            y: 0,
            columns: 1.0,
            rows: 1.0,
            pixels: Pixels::Encoded(bytes),
        });
        assert_eq!(
            &decode(&frame.png().unwrap()).unwrap().2[..4],
            &[134, 6, 6, 255]
        );
        assert!(decode(include_bytes!("../tests/fixtures/local-image.png")).is_ok());
        assert!(decode(include_bytes!("../tests/fixtures/local-image.jpg")).is_ok());
        assert!(decode(b"invalid").is_err());
        assert!(decode(&include_bytes!("../tests/fixtures/local-image.png")[..33]).is_err());
        frame.layers[0].rows = f64::NAN;
        assert!(frame.png().is_err());
        frame.width = 321;
        assert!(frame.png().is_err());
        frame.width = 0;
        assert!(frame.png().is_err());
    }
}
