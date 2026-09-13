//! Bounded portable PNG/JPEG decoding shared by screenshots and Unix previews.
use std::io::{self, Cursor};

pub fn decode(bytes: &[u8]) -> io::Result<(usize, usize, Vec<u8>)> {
    if bytes.len() as u64 > crate::remote_protocol::IMAGE_LIMIT {
        return Err(io::Error::other("image exceeds 20 MiB"));
    }
    let (width, height) = crate::image_preview::dimensions(bytes)?;
    if bytes.starts_with(b"\x89PNG") {
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_limits(png::Limits {
            bytes: 96 * 1024 * 1024,
        });
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info()?;
        let size = reader
            .output_buffer_size()
            .filter(|n| *n <= 80_000_000)
            .ok_or_else(|| io::Error::other("image buffer exceeds bounds"))?;
        let mut raw = vec![0; size];
        let info = reader.next_frame(&mut raw)?;
        if (info.width, info.height) != (width, height) {
            return Err(io::Error::other("image dimensions changed"));
        }
        let raw = &raw[..info.buffer_size()];
        let rgba = match info.color_type {
            png::ColorType::Rgba => raw.to_vec(),
            png::ColorType::Rgb => raw
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::Grayscale => raw.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
            png::ColorType::GrayscaleAlpha => raw
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            _ => return Err(io::Error::other("unsupported screenshot image color")),
        };
        Ok((width as usize, height as usize, rgba))
    } else {
        let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
        decoder.set_max_decoding_buffer_size(80_000_000);
        let raw = decoder.decode().map_err(io::Error::other)?;
        let info = decoder
            .info()
            .ok_or_else(|| io::Error::other("missing JPEG information"))?;
        if (info.width as u32, info.height as u32) != (width, height) {
            return Err(io::Error::other("image dimensions changed"));
        }
        let rgba = match info.pixel_format {
            jpeg_decoder::PixelFormat::RGB24 => raw
                .as_chunks::<3>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            jpeg_decoder::PixelFormat::L8 => raw.iter().flat_map(|v| [*v, *v, *v, 255]).collect(),
            _ => return Err(io::Error::other("unsupported JPEG color format")),
        };
        Ok((width as usize, height as usize, rgba))
    }
}
