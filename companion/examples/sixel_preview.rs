//! Offline encoder diagnostic: encode a bounded raw RGB fixture (not a product decoder).
#[path = "../src/sixel.rs"]
mod sixel;
use std::io::{self, Read, Write};
fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err(io::Error::other(
            "usage: sixel_preview WIDTH HEIGHT RGB_FILE",
        ));
    }
    let width: usize = args[0].parse().map_err(io::Error::other)?;
    let height: usize = args[1].parse().map_err(io::Error::other)?;
    if width == 0 || height == 0 || width > 1280 || height > 720 {
        return Err(io::Error::other(
            "fixture dimensions outside preview limits",
        ));
    }
    assert_eq!(
        sixel::fit((width as u32, height as u32), (width, height)),
        (width, height)
    );
    let mut rgb = Vec::new();
    std::fs::File::open(&args[2])?
        .take(1280 * 720 * 3 + 1)
        .read_to_end(&mut rgb)?;
    let encoded = sixel::encode(&sixel::Raster { width, height, rgb })?;
    io::stdout().write_all(encoded.as_bytes())
}
