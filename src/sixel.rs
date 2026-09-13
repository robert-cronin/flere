//! Bounded, generated Sixel only. Fixed RGB palette, ordered dithering and run coding.
use std::{fmt::Write, io};
pub struct Raster {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}
pub fn fit(source: (u32, u32), area: (usize, usize)) -> (usize, usize) {
    let w = area.0.clamp(1, 1280).min(source.0 as usize);
    let h = area.1.clamp(1, 720).min(source.1 as usize);
    if w as u64 * source.1 as u64 <= h as u64 * source.0 as u64 {
        (
            w,
            (source.1 as u64 * w as u64 / source.0 as u64).max(1) as usize,
        )
    } else {
        (
            (source.0 as u64 * h as u64 / source.1 as u64).max(1) as usize,
            h,
        )
    }
}
fn color(rgb: &[u8], x: usize, y: usize) -> usize {
    const BAYER: [[i32; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let bias = BAYER[y % 4][x % 4] * 3 - 22;
    let q = |v: u8| ((v as i32 + bias).clamp(0, 255) + 25) as usize / 51;
    q(rgb[0]) * 36 + q(rgb[1]) * 6 + q(rgb[2])
}
fn runs(out: &mut String, values: &[u8]) {
    let mut i = 0;
    while i < values.len() {
        let mut j = i + 1;
        while j < values.len() && values[j] == values[i] {
            j += 1;
        }
        let ch = (values[i] + 63) as char;
        if j - i > 3 {
            let _ = write!(out, "!{}{ch}", j - i);
        } else {
            for _ in i..j {
                out.push(ch);
            }
        }
        i = j;
    }
}
pub fn encode(r: &Raster) -> io::Result<String> {
    if r.width == 0
        || r.height == 0
        || r.width > 1280
        || r.height > 720
        || r.rgb.len() != r.width * r.height * 3
    {
        return Err(io::Error::other("preview raster outside bounds"));
    }
    let mut out = format!("\x1bP7;1q\"1;1;{};{}", r.width, r.height);
    for n in 0..216 {
        let _ = write!(
            out,
            "#{n};2;{};{};{}",
            n / 36 * 20,
            n / 6 % 6 * 20,
            n % 6 * 20
        );
    }
    let mut band = vec![0u8; 216 * r.width];
    for y in (0..r.height).step_by(6) {
        band.fill(0);
        for dy in 0..6.min(r.height - y) {
            for x in 0..r.width {
                let p = ((y + dy) * r.width + x) * 3;
                band[color(&r.rgb[p..p + 3], x, y + dy) * r.width + x] |= 1 << dy;
            }
        }
        let mut first = true;
        for n in 0..216 {
            let values = &band[n * r.width..(n + 1) * r.width];
            if let Some(last) = values.iter().rposition(|v| *v != 0) {
                if !first {
                    out.push('$');
                }
                first = false;
                let _ = write!(out, "#{n}");
                runs(&mut out, &values[..last + 1]);
            }
        }
        if y + 6 < r.height {
            out.push('-');
        }
        if out.len() > 32 * 1024 * 1024 {
            return Err(io::Error::other("encoded preview exceeds bound"));
        }
    }
    out.push_str("\x1b\\");
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit((4000, 2000), (800, 600)), (800, 400));
        assert_eq!(fit((2000, 4000), (800, 600)), (300, 600));
        assert_eq!(fit((10, 5), (800, 600)), (10, 5));
        assert_eq!(fit((10000, 1), (5000, 2000)), (1280, 1));
    }
    #[test]
    fn raster_encodes_odd_bands_runs_and_no_embedded_controls() {
        let r = Raster {
            width: 9,
            height: 7,
            rgb: vec![255; 9 * 7 * 3],
        };
        let encoded = encode(&r).unwrap();
        assert!(encoded.starts_with("\x1bP7;1q\"1;1;9;7"));
        assert!(encoded.ends_with("#215!9~-#215!9@\x1b\\"));
        assert!(
            encoded[2..encoded.len() - 2]
                .bytes()
                .all(|b| (32..=126).contains(&b))
        );
        assert!(
            encode(&Raster {
                width: 1281,
                height: 1,
                rgb: vec![]
            })
            .is_err()
        );
    }
}
