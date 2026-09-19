//! Attachment-owned Kitty graphics. No companion, paths, or child escapes on stdout.
//! Only capability replies enable graphics; all transfers use bounded inline data.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{collections::HashSet, sync::mpsc};
const BASE: u32 = 0x5248_0000;
pub(super) const PET: u32 = BASE + 1;
const PREVIEW: u32 = BASE + 2;
const ICON: u32 = BASE + 100;
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum AssetKey {
    Icon(u32),
    Preview(u64),
}
struct Bitmap {
    format: u32,
    width: usize,
    height: usize,
    compressed: bool,
    data: String,
}
impl Bitmap {
    fn decode(bytes: &[u8], preview: bool) -> io::Result<Self> {
        let (width, height) = crate::image_preview::dimensions(bytes)?;
        // Small repository PNGs are already compact and preserve their alpha.
        if bytes.starts_with(b"\x89PNG") && (!preview || !cfg!(target_os = "macos")) {
            return Ok(Self {
                format: 100,
                width: width as usize,
                height: height as usize,
                compressed: false,
                data: STANDARD.encode(bytes),
            });
        }
        #[cfg(target_os = "macos")]
        {
            let (width, height, rgba) = os::image_pixels(bytes)?;
            Ok(Self {
                format: 32,
                width,
                height,
                compressed: true,
                data: STANDARD.encode(os::compress_pixels(&rgba)?),
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            let (width, height, rgba) = crate::screenshot::decode(bytes)?;
            let mut encoded = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut encoded, width as u32, height as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header()?.write_image_data(&rgba)?;
            }
            Ok(Self {
                format: 100,
                width,
                height,
                compressed: false,
                data: STANDARD.encode(encoded),
            })
        }
    }
    fn header(&self, id: u32) -> String {
        format!(
            "a=t,t=d,f={},s={},v={},i={},q=2{}",
            self.format,
            self.width,
            self.height,
            id,
            if self.compressed { ",o=z" } else { "" }
        )
    }
}
struct Transfer {
    bitmap: Bitmap,
    generation: u64,
    offset: usize,
}
pub(super) struct Graphics {
    supported: bool,
    pub cell: Option<(usize, usize)>,
    pub preview: Option<image_preview::ImagePreview>,
    pub preview_counter: u64,
    pub preview_clear: bool,
    send: mpsc::SyncSender<(AssetKey, Vec<u8>)>,
    receive: mpsc::Receiver<(AssetKey, io::Result<Bitmap>)>,
    pending: HashSet<AssetKey>,
    icons: HashMap<u32, Option<Bitmap>>,
    uploaded: HashSet<u32>,
    owned: HashSet<u32>,
    transfer: Option<Transfer>,
    preview_size: Option<(usize, usize)>,
    preview_uploaded: Option<u64>,
    placed: Vec<crate::avatar::Badge>,
    pet_data: Vec<u8>,
    invalidated: bool,
}
impl Graphics {
    pub fn new() -> Self {
        let (send, requests) = mpsc::sync_channel::<(AssetKey, Vec<u8>)>(crate::avatar::LIMIT + 1);
        let (results, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok((key, bytes)) = requests.recv() {
                let image = Bitmap::decode(&bytes, matches!(key, AssetKey::Preview(_)));
                if results.send((key, image)).is_err() {
                    break;
                }
            }
        });
        Self {
            supported: false,
            cell: os::cell_dimensions(0),
            preview: None,
            preview_counter: 0,
            preview_clear: false,
            send,
            receive,
            pending: HashSet::new(),
            icons: HashMap::new(),
            uploaded: HashSet::new(),
            owned: HashSet::new(),
            transfer: None,
            preview_size: None,
            preview_uploaded: None,
            placed: Vec::new(),
            pet_data: Vec::new(),
            invalidated: false,
        }
    }
    pub fn query() -> String {
        format!("\x1b_Ga=q,t=d,f=24,s=1,v=1,i={BASE};AAAA\x1b\\\x1b[16t")
    }
    pub fn has_icon(&self, key: &str) -> bool {
        key.strip_prefix("repo-")
            .and_then(|s| s.parse::<u32>().ok())
            .and_then(|n| self.icons.get(&(ICON + n)))
            .is_some_and(|b| b.is_some())
    }
    pub fn capable(&self) -> bool {
        self.supported && self.cell.is_some()
    }
    pub fn reply(&mut self, bytes: &[u8]) {
        if bytes == format!("\x1b_Gi={BASE};OK\x1b\\").as_bytes() {
            self.supported = true;
            self.invalidated = true;
            crate::diagnostics::record("local-graphics", "Kitty inline graphics confirmed");
        } else if let Some(size) = bytes
            .strip_prefix(b"\x1b[6;")
            .and_then(|b| b.strip_suffix(b"t"))
            && let Ok(size) = std::str::from_utf8(size)
            && let Some((h, w)) = size.split_once(';')
            && let (Ok(h), Ok(w)) = (h.parse::<usize>(), w.parse::<usize>())
            && crate::avatar::cell_bytes((w, h)).is_ok()
        {
            self.update_cell((w, h));
        }
    }
    pub fn update_cell(&mut self, cell: (usize, usize)) {
        if self.cell != Some(cell) {
            self.cell = Some(cell);
            self.invalidated = true;
        }
    }
    pub fn poll(&mut self) -> bool {
        let mut dirty = self.transfer.is_some();
        while let Ok((key, result)) = self.receive.try_recv() {
            self.pending.remove(&key);
            dirty = true;
            match key {
                AssetKey::Icon(id) => {
                    self.icons.insert(id, result.ok());
                }
                AssetKey::Preview(generation) => {
                    if let Some(p) = self.preview.as_mut().filter(|p| p.id == generation) {
                        match result {
                            Ok(bitmap) => {
                                self.transfer = Some(Transfer {
                                    bitmap,
                                    generation,
                                    offset: 0,
                                });
                            }
                            Err(e) => {
                                p.error = e.to_string();
                                p.finished = true;
                            }
                        }
                    }
                }
            }
        }
        dirty || self.invalidated
    }
    pub fn request_preview(&mut self) {
        let Some(p) = self.preview.as_mut() else {
            return;
        };
        let key = AssetKey::Preview(p.id);
        if p.started
            || self
                .pending
                .iter()
                .any(|k| matches!(k, AssetKey::Preview(_)))
            || self.transfer.is_some()
        {
            return;
        }
        if self.send.try_send((key, p.bytes.clone())).is_ok() {
            self.pending.insert(key);
            // Keep this one bounded source image for full-view screenshots.
            // Navigation and close release it with the current preview.
            p.started = true;
        }
    }
    pub fn cancel_preview(&mut self) {
        // End this attachment's old Kitty chunk sequence before sending any
        // replacement graphics. A late decoder result retains its old generation
        // and poll() discards it instead of placing it over the new preview.
        let mut out = String::new();
        if self.transfer.is_some() {
            out.push_str("\x1b_Gq=2,m=0;\x1b\\");
        }
        out.push_str(&delete(PREVIEW, true));
        match remote::emit(false, out.as_bytes()) {
            Ok(()) => {
                self.transfer = None;
                self.owned.remove(&PREVIEW);
            }
            Err(error) => {
                // Keep the pending chunk and owned ID for reserved destructor
                // cleanup; failed admission has not sent the terminator/delete.
                crate::diagnostics::error("preview-cancel", &error);
            }
        }
        self.preview_uploaded = None;
        self.preview_size = None;
        self.invalidated = true;
    }
    pub fn emit_preview(&mut self) -> io::Result<()> {
        self.request_preview();
        let Some(t) = &mut self.transfer else {
            return Ok(());
        };
        let mut out = String::new();
        // Finish an in-flight image even if closed; never place a stale generation.
        // No other graphics commands may interrupt a Kitty chunk sequence.
        for _ in 0..32 {
            if t.offset == t.bitmap.data.len() {
                break;
            }
            let end = (t.offset + 4096).min(t.bitmap.data.len());
            out.push_str(&format!(
                "\x1b_G{}m={};{}\x1b\\",
                if t.offset == 0 {
                    format!("{},", t.bitmap.header(PREVIEW))
                } else {
                    "q=2,".into()
                },
                u8::from(end < t.bitmap.data.len()),
                &t.bitmap.data[t.offset..end]
            ));
            t.offset = end;
        }
        remote::emit(false, out.as_bytes())?;
        if t.offset == t.bitmap.data.len() {
            self.owned.insert(PREVIEW);
            self.preview_size = Some((t.bitmap.width, t.bitmap.height));
            self.preview_uploaded = Some(t.generation);
            if let Some(p) = self.preview.as_mut().filter(|p| p.id == t.generation) {
                p.finished = true;
            }
            self.transfer = None;
            self.invalidated = true;
        }
        Ok(())
    }
    pub fn frame(
        &mut self,
        badges: &[crate::avatar::Badge],
        projects: &avatars::Projects,
        size: (usize, usize),
        reset: bool,
    ) -> String {
        self.invalidated |= reset;
        if reset {
            // Ghostty discards image data on ED2, not just placements. Cached CPU
            // bitmaps remain valid, but every visible badge needs a new upload.
            self.uploaded.clear();
        }
        if !self.capable() || self.transfer.is_some() {
            return String::new();
        }
        let mut out = String::new();
        if self.invalidated || self.placed != badges {
            for id in &self.owned {
                out.push_str(&delete(*id, false));
            }
            self.pet_data.clear();
            self.placed.clear();
            self.invalidated = false;
        }
        if let Some(p) = &self.preview {
            if self.preview_uploaded == Some(p.id)
                && let Some(source) = self.preview_size
            {
                let (cw, ch) = self.cell.unwrap();
                let (width, _) = fit(source, ((size.0 - 4) * cw, (size.1 - 7) * ch));
                let columns = width.div_ceil(cw);
                out.push_str(&place(
                    PREVIEW,
                    1,
                    (size.0 - columns) / 2,
                    3,
                    &aspect(source, ((size.0 - 4) * cw, (size.1 - 7) * ch), (cw, ch)),
                ));
            }
            return out;
        }
        if self.preview_uploaded.take().is_some() {
            out.push_str(&delete(PREVIEW, true));
            self.owned.remove(&PREVIEW);
        }
        let mut transmitted = false;
        for (index, b) in badges.iter().enumerate() {
            let Some(id) = b
                .key
                .strip_prefix("repo-")
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| (1..=crate::avatar::LIMIT as u32).contains(n))
                .map(|n| ICON + n)
            else {
                continue;
            };
            let key = AssetKey::Icon(id);
            if !self.icons.contains_key(&id)
                && !self.pending.contains(&key)
                && let Some(bytes) = projects.bytes(&b.key)
                && self.send.try_send((key, bytes.to_vec())).is_ok()
            {
                self.pending.insert(key);
            }
            let Some(Some(bitmap)) = self.icons.get(&id) else {
                continue;
            };
            if !self.uploaded.contains(&id) {
                if transmitted {
                    self.invalidated = true;
                    continue;
                }
                out.push_str(&transmit(&bitmap.header(id), &bitmap.data));
                self.uploaded.insert(id);
                self.owned.insert(id);
                transmitted = true;
            }
            // Native height and aspect ratio; transparent pixels keep the card colour.
            out.push_str(&place(
                id,
                index as u32 + 1,
                b.x as usize,
                b.y as usize,
                &aspect(
                    (bitmap.width, bitmap.height),
                    (
                        b.columns as usize * self.cell.unwrap().0,
                        self.cell.unwrap().1,
                    ),
                    self.cell.unwrap(),
                ),
            ));
        }
        self.placed = badges.to_vec();
        out
    }
    pub fn pet(&mut self, raster: &crate::sixel::Raster, x: usize, y: usize) -> io::Result<String> {
        if !self.capable() || self.transfer.is_some() || self.preview.is_some() {
            return Ok(String::new());
        }
        if self.pet_data == raster.rgb {
            return Ok(String::new());
        }
        let data = STANDARD.encode(os::compress_pixels(&raster.rgb)?);
        let mut out = transmit(
            &format!(
                "a=t,t=d,f=24,s={},v={},i={PET},o=z,q=2",
                raster.width, raster.height
            ),
            &data,
        );
        out.push_str(&place(PET, 1, x, y, ""));
        self.pet_data.clone_from(&raster.rgb);
        self.owned.insert(PET);
        Ok(out)
    }
}
impl Drop for Graphics {
    fn drop(&mut self) {
        // Terminate a pending inline transfer before cleanup; an invalid/truncated
        // image is silently rejected (q=2). Delete only IDs owned by this attachment.
        if self.transfer.is_some() {
            let _ = remote::cleanup(false, b"\x1b_Gq=2,m=0;\x1b\\");
            self.owned.insert(PREVIEW);
        }
        let out: String = self.owned.iter().map(|id| delete(*id, true)).collect();
        if !out.is_empty() {
            let _ = remote::cleanup(false, out.as_bytes());
        }
    }
}
fn transmit(header: &str, data: &str) -> String {
    let mut out = String::new();
    for (index, chunk) in data.as_bytes().chunks(4096).enumerate() {
        out.push_str(&format!(
            "\x1b_G{}m={};{}\x1b\\",
            if index == 0 {
                format!("{header},")
            } else {
                "q=2,".into()
            },
            u8::from((index + 1) * 4096 < data.len()),
            std::str::from_utf8(chunk).unwrap()
        ));
    }
    out
}
fn delete(id: u32, data: bool) -> String {
    format!(
        "\x1b_Ga=d,d={},i={id},q=2\x1b\\",
        if data { "I" } else { "i" }
    )
}
fn place(id: u32, placement: u32, x: usize, y: usize, geometry: &str) -> String {
    format!(
        "\x1b7\x1b[{};{}H\x1b_Ga=p,i={id},p={placement},C=1,q=2{}\x1b\\\x1b8",
        y + 1,
        x + 1,
        if geometry.is_empty() {
            String::new()
        } else {
            format!(",{geometry}")
        }
    )
}
fn aspect(source: (usize, usize), max: (usize, usize), cell: (usize, usize)) -> String {
    if source.0 <= max.0 && source.1 <= max.1 {
        return String::new();
    }
    if max.0 as f64 / source.0 as f64 <= max.1 as f64 / source.1 as f64 {
        format!("c={}", (max.0 / cell.0).max(1))
    } else {
        format!("r={}", (max.1 / cell.1).max(1))
    }
}
pub(super) fn fit(source: (usize, usize), max: (usize, usize)) -> (usize, usize) {
    let scale = (max.0 as f64 / source.0 as f64)
        .min(max.1 as f64 / source.1 as f64)
        .min(1.0);
    (
        (source.0 as f64 * scale).floor().max(1.0) as usize,
        (source.1 as f64 * scale).floor().max(1.0) as usize,
    )
}
impl Ui {
    pub(super) fn graphics_cell(&self) -> Option<(usize, usize)> {
        self.remote
            .as_ref()
            .and_then(|r| r.avatar_capable.then_some(r.avatar_cell).flatten())
            .or_else(|| {
                self.local_graphics
                    .as_ref()
                    .filter(|g| g.capable())
                    .and_then(|g| g.cell)
            })
    }
    pub(super) fn image_capable(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.preview_capable)
            || self.local_graphics.as_ref().is_some_and(|g| g.capable())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejected_preview_cancel_retains_partial_transfer_for_destructor_cleanup() {
        let bytes = crate::ui::output::tests::saturated(|| {
            let mut graphics = super::Graphics::new();
            graphics.transfer = Some(super::Transfer {
                bitmap: super::Bitmap {
                    format: 100,
                    width: 1,
                    height: 1,
                    compressed: false,
                    data: "AAAA".into(),
                },
                generation: 1,
                offset: 2,
            });
            graphics.owned.insert(super::PREVIEW);
            graphics.cancel_preview();
            assert!(graphics.transfer.is_some());
            assert!(graphics.owned.contains(&super::PREVIEW));
            drop(graphics);
        });
        let expected = format!(
            "\x1b_Gq=2,m=0;\x1b\\{}",
            super::delete(super::PREVIEW, true)
        );
        assert_eq!(bytes, expected.as_bytes());
    }

    use super::*;
    #[test]
    fn capability_requires_a_matching_positive_reply_and_valid_cell_metrics() {
        let mut g = Graphics::new();
        g.cell = None;
        g.reply(b"\x1b_Gi=99;OK\x1b\\");
        g.reply(b"\x1b[6;20;8t");
        assert!(!g.capable());
        g.reply(format!("\x1b_Gi={BASE};ENOENT\x1b\\").as_bytes());
        assert!(!g.capable());
        g.reply(format!("\x1b_Gi={BASE};OK\x1b\\").as_bytes());
        assert!(g.capable());
        g.reply(b"\x1b[6;0;9000t");
        assert_eq!(g.cell, Some((8, 20)));
    }
    #[test]
    fn inline_transfer_is_bounded_quiet_and_preserves_every_source_byte() {
        let bytes: Vec<_> = (0..16000).map(|n| (n % 251) as u8).collect();
        let data = STANDARD.encode(&bytes);
        let encoded = transmit("a=t,t=d,f=100,i=7,q=2", &data);
        let chunks: Vec<_> = encoded.split("\x1b\\").filter(|s| !s.is_empty()).collect();
        let mut combined = String::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let (header, payload) = chunk.split_once(';').unwrap();
            assert!(payload.len() <= 4096 && payload.len() % 4 == 0);
            assert!(header.contains("q=2"));
            assert!(header.ends_with(if i + 1 == chunks.len() { "m=0" } else { "m=1" }));
            combined.push_str(payload);
        }
        assert_eq!(STANDARD.decode(combined).unwrap(), bytes);
        assert!(!encoded.contains("t=f"));
        assert!(delete(PET, true).contains(&format!("d=I,i={PET}")));
        assert!(!delete(PET, true).contains("d=A"));
    }
    #[test]
    fn screen_clear_reuploads_badges_but_ordinary_frames_reuse_image_data() {
        let projects = avatars::Projects::new();
        let mut g = Graphics::new();
        g.supported = true;
        g.cell = Some((8, 20));
        g.icons.insert(
            ICON + 1,
            Some(
                Bitmap::decode(
                    include_bytes!("../../tests/fixtures/local-image.png"),
                    false,
                )
                .unwrap(),
            ),
        );
        let badges = vec![crate::avatar::Badge {
            x: 2,
            y: 4,
            columns: 3,
            key: "repo-1".into(),
            bg: [9, 18, 27],
        }];
        let first = g.frame(&badges, &projects, (100, 24), false);
        assert!(first.contains("a=t,t=d"));
        assert!(first.contains("a=p"));
        let unchanged = g.frame(&badges, &projects, (100, 24), false);
        assert!(!unchanged.contains("a=t,t=d"));
        let redraw = g.frame(&badges, &projects, (100, 24), true);
        assert!(redraw.contains("a=t,t=d"));
        assert!(redraw.find("a=t,t=d").unwrap() < redraw.find("a=p").unwrap());
        assert!(
            !g.frame(&badges, &projects, (100, 24), false)
                .contains("a=t,t=d")
        );
        g.owned.clear();
    }
    #[test]
    fn aspect_preserves_shape_and_bounds_without_upscaling_small_images() {
        assert_eq!(aspect((442, 491), (24, 20), (8, 20)), "r=1");
        assert_eq!(aspect((1000, 100), (24, 20), (8, 20)), "c=3");
        assert_eq!(aspect((2, 2), (24, 20), (8, 20)), "");
        assert_eq!(fit((4000, 2000), (800, 600)), (800, 400));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_decode_preserves_top_left_origin_and_straight_alpha() {
        let (w, h, rgba) =
            os::image_pixels(include_bytes!("../../tests/fixtures/local-image.png")).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(&rgba[..4], &[255, 0, 0, 255]);
        assert_eq!(&rgba[4..8], &[0, 255, 0, 128]);
        assert_eq!(&rgba[8..12], &[0, 0, 255, 255]);
        assert_eq!(rgba[15], 0);
        let (w, h, jpeg) =
            os::image_pixels(include_bytes!("../../tests/fixtures/local-image.jpg")).unwrap();
        assert_eq!((w, h), (16, 16));
        assert!(jpeg[0] > 240 && jpeg[1] < 15 && jpeg[2] < 15);
        assert!(jpeg.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        assert!(
            os::image_pixels(&include_bytes!("../../tests/fixtures/local-image.png")[..33])
                .is_err()
        );
        assert!(os::compress_pixels(&vec![0; 8 * 1024 * 1024 + 1]).is_err());
    }
}
