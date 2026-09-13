//! Platform FFI is isolated here. Handles returned by clipboard APIs remain borrowed;
//! console modes, GDI+ objects, streams and clipboard locks have explicit RAII ownership.
use std::{io, path::Path};
#[cfg(unix)]
#[path = "../../src/image_decode.rs"]
mod image_decode;
#[cfg(unix)]
pub fn uid() -> u32 {
    // POSIX uid_t is u32 on the supported Linux/macOS targets; no owned resource.
    unsafe { libc::getuid() }
}
/// Fresh local update/approval identifiers. These use the operating system RNG,
/// independently of any SSH peer or untrusted terminal content.
pub fn nonce() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    #[cfg(unix)]
    {
        use std::io::Read;
        std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    }
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                algorithm: *mut std::ffi::c_void,
                buffer: *mut u8,
                size: u32,
                flags: u32,
            ) -> i32;
        }
        // BCrypt's documented user-mode ABI uses 32-bit ULONG/NTSTATUS and a
        // caller-owned output buffer. SYSTEM_PREFERRED_RNG (2) requires a null
        // algorithm handle; no resource is acquired and the pointer is not retained.
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                2,
            )
        };
        if status < 0 {
            return Err(io::Error::other(format!("Windows RNG failed: {status:#x}")));
        }
    }
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn copy_png(bytes: &[u8], path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let _ = bytes;
        crate::clipboard_owner::publish(path)
    }
    #[cfg(windows)]
    {
        platform::copy_screenshot(bytes, path)
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
        let text = path
            .to_str()
            .filter(|s| !s.chars().any(char::is_control))
            .ok_or_else(|| io::Error::other("screenshot path must be printable UTF-8"))?;
        let context = ClipboardContext::new().map_err(io::Error::other)?;
        let format = "public.png";
        let contents = vec![
            ClipboardContent::Other(format.into(), bytes.to_vec()),
            ClipboardContent::Text(text.into()),
        ];
        context.set(contents).map_err(io::Error::other)?;
        if context.get_buffer(format).map_err(io::Error::other)? != bytes
            || context.get_text().map_err(io::Error::other)? != text
        {
            return Err(io::Error::other(
                "clipboard screenshot could not be verified",
            ));
        }
        Ok(())
    }
}
/// Encode the Windows bitmap fallback without acquiring any native resources.
/// Keep alpha in the original PNG; the 24-bit BI_RGB DIB uses a black matte.
#[cfg(any(windows, test))]
fn screenshot_dib(width: usize, height: usize, rgba: &[u8]) -> io::Result<Vec<u8>> {
    let pixels = width
        .checked_mul(height)
        .filter(|&n| n > 0 && n as u64 <= crate::image_preview::PIXELS)
        .ok_or_else(|| io::Error::other("invalid screenshot bitmap dimensions"))?;
    if rgba.len() != pixels * 4 {
        return Err(io::Error::other("invalid screenshot RGBA buffer"));
    }
    let stride = (width * 3).next_multiple_of(4);
    let mut dib = vec![0u8; 40 + stride * height];
    for (offset, value) in [(0, 40), (4, width as u32), (8, height as u32)] {
        dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&24u16.to_le_bytes());
    for (y, row) in dib[40..].chunks_exact_mut(stride).enumerate() {
        // Positive DIB height means bottom-up BGR rows with DWORD alignment.
        let start = (height - 1 - y) * width * 4;
        for (pixel, source) in row[..width * 3]
            .as_chunks_mut::<3>()
            .0
            .iter_mut()
            .zip(rgba[start..start + width * 4].as_chunks::<4>().0)
        {
            for (out, channel) in pixel.iter_mut().zip([source[2], source[1], source[0]]) {
                *out = ((u16::from(channel) * u16::from(source[3]) + 127) / 255) as u8;
            }
        }
    }
    Ok(dib)
}
#[cfg(unix)]
mod platform {
    use super::*;
    #[cfg(any(target_os = "linux", test))]
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    #[cfg(any(target_os = "linux", test))]
    fn clipboard_output(command: &mut Command, timeout: Duration) -> io::Result<Option<Vec<u8>>> {
        // Own the helper through every error/timeout; never leave a clipboard reader running.
        let mut child = crate::Ssh(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let mut stdout = child.0.stdout.take().unwrap();
        let fd = stdout.as_raw_fd();
        // Owned pipe descriptor; O_NONBLOCK bounds reads independently of child behavior.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let started = Instant::now();
        let mut bytes = Vec::new();
        let mut eof = false;
        loop {
            let mut chunk = [0; 8192];
            if !eof {
                match stdout.read(&mut chunk) {
                    Ok(0) => eof = true,
                    Ok(n) => {
                        if bytes.len() + n > crate::protocol::IMAGE_LIMIT as usize {
                            return Err(io::Error::other("clipboard image exceeds 20 MiB"));
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        continue;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            if eof && let Some(status) = child.0.try_wait()? {
                return Ok(
                    (status.success() && bytes.starts_with(b"\x89PNG\r\n\x1a\n")).then_some(bytes),
                );
            }
            if started.elapsed() >= timeout {
                return Err(io::Error::other(
                    "clipboard reader timed out; try paste again",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub struct Console {
        input: Option<libc::termios>,
    }
    impl Console {
        pub fn enter() -> io::Result<Self> {
            let mut value = std::mem::MaybeUninit::<libc::termios>::uninit();
            // Platform termios is supplied by libc; redirected input has no modes.
            if unsafe { libc::tcgetattr(0, value.as_mut_ptr()) } != 0 {
                return Ok(Self { input: None });
            }
            let original = unsafe { value.assume_init() };
            let mut raw = original;
            unsafe {
                libc::cfmakeraw(&mut raw);
            }
            if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                input: Some(original),
            })
        }
    }
    impl Drop for Console {
        fn drop(&mut self) {
            if let Some(original) = &self.input {
                unsafe {
                    libc::tcsetattr(0, libc::TCSANOW, original);
                }
            }
        }
    }
    pub fn size() -> (u16, u16) {
        let mut w = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe { libc::ioctl(0, libc::TIOCGWINSZ, &mut w) } == 0 && w.ws_col > 0 && w.ws_row > 0
        {
            (w.ws_col, w.ws_row)
        } else {
            (100, 30)
        }
    }
    pub fn decode_preview(bytes: &[u8], area: (usize, usize)) -> io::Result<crate::sixel::Raster> {
        let (source_w, source_h, rgba) = super::image_decode::decode(bytes)?;
        let (width, height) = crate::sixel::fit((source_w as u32, source_h as u32), area);
        let mut rgb = Vec::with_capacity(width * height * 3);
        for y in 0..height {
            for x in 0..width {
                let at = (y * source_h / height * source_w + x * source_w / width) * 4;
                let alpha = rgba[at + 3] as u32;
                for color in &rgba[at..at + 3] {
                    rgb.push(((*color as u32 * alpha + 12 * (255 - alpha) + 127) / 255) as u8);
                }
            }
        }
        Ok(crate::sixel::Raster { width, height, rgb })
    }
    #[cfg(target_os = "linux")]
    pub fn image() -> io::Result<Option<Vec<u8>>> {
        // Optional installed desktop tools; no shell interpolation or dependency installation.
        for (name, args) in [
            ("wl-paste", vec!["--no-newline", "--type", "image/png"]),
            (
                "xclip",
                vec!["-selection", "clipboard", "-t", "image/png", "-o"],
            ),
        ] {
            if std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .any(|p| p.join(name).is_file())
            {
                return clipboard_output(Command::new(name).args(args), Duration::from_secs(2));
            }
        }
        Ok(None)
    }
    #[cfg(target_os = "macos")]
    pub fn image() -> io::Result<Option<Vec<u8>>> {
        super::mac_clipboard::read("com.apple.pasteboard.clipboard")
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn unix_preview_decodes_png_jpeg_and_bounds_the_fitted_raster() {
            for bytes in [
                include_bytes!("../../tests/fixtures/local-image.png").as_slice(),
                include_bytes!("../../tests/fixtures/local-image.jpg").as_slice(),
            ] {
                let source = super::super::image_decode::decode(bytes).unwrap();
                let image = decode_preview(bytes, (1280, 720)).unwrap();
                assert_eq!((image.width, image.height), (source.0, source.1));
                assert_eq!(image.rgb.len(), image.width * image.height * 3);
                assert!(image.rgb[0] > 240 && image.rgb[1] < 15 && image.rgb[2] < 15);
                let small = decode_preview(bytes, (1, 1)).unwrap();
                assert_eq!((small.width, small.height, small.rgb.len()), (1, 1, 3));
            }
            let mut alpha = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut alpha, 1, 1);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&[255, 0, 0, 128])
                    .unwrap();
            }
            assert_eq!(decode_preview(&alpha, (1, 1)).unwrap().rgb, [134, 6, 6]);
            let mut bomb = alpha.clone();
            bomb[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
            assert!(decode_preview(&bomb, (1280, 720)).is_err());
            assert!(decode_preview(&alpha[..33], (10, 10)).is_err());
            assert!(decode_preview(b"invalid", (10, 10)).is_err());
            assert!(
                decode_preview(
                    &vec![0; crate::protocol::IMAGE_LIMIT as usize + 1],
                    (10, 10)
                )
                .is_err()
            );
        }
        #[test]
        fn clipboard_reader_bounds_output_and_kills_stalled_children() {
            let started = Instant::now();
            let error = clipboard_output(
                Command::new("/bin/sleep").arg("10"),
                Duration::from_millis(40),
            )
            .unwrap_err();
            assert!(error.to_string().contains("timed out"));
            assert!(started.elapsed() < Duration::from_secs(2));
            let error = clipboard_output(&mut Command::new("/usr/bin/yes"), Duration::from_secs(5))
                .unwrap_err();
            assert!(error.to_string().contains("20 MiB"));
            assert!(
                clipboard_output(
                    Command::new("/usr/bin/printf").arg("ordinary text"),
                    Duration::from_secs(2)
                )
                .unwrap()
                .is_none()
            );
        }
    }
}
/// On-demand macOS pasteboard access, never desktop/window capture. Darwin's
/// 64-bit CFIndex/opaque-reference ABI matches core's existing clipboard code.
/// All Create/Copy references are owned; borrowed flavors/keys never outlive them.
#[cfg(target_os = "macos")]
mod mac_clipboard {
    use super::*;
    use std::{ffi::c_void, ptr};
    type Ref = *const c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: Ref);
        fn CFStringCreateWithBytes(
            allocator: Ref,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> Ref;
        fn CFDataCreateMutable(allocator: Ref, capacity: isize) -> Ref;
        fn CFDataGetLength(data: Ref) -> isize;
        fn CFDataGetBytePtr(data: Ref) -> *const u8;
        fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(number: Ref, kind: isize, out: *mut i64) -> u8;
        #[cfg(test)]
        fn CFDataCreate(allocator: Ref, bytes: *const u8, len: isize) -> Ref;
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn PasteboardCreate(name: Ref, out: *mut Ref) -> i32;
        fn PasteboardSynchronize(board: Ref) -> u32;
        fn PasteboardGetItemCount(board: Ref, out: *mut u32) -> i32;
        fn PasteboardGetItemIdentifier(board: Ref, index: isize, item: *mut *mut c_void) -> i32;
        fn PasteboardCopyItemFlavorData(
            board: Ref,
            item: *mut c_void,
            kind: Ref,
            data: *mut Ref,
        ) -> i32;
        #[cfg(test)]
        fn PasteboardClear(board: Ref) -> i32;
        #[cfg(test)]
        fn PasteboardPutItemFlavor(
            board: Ref,
            item: *mut c_void,
            kind: Ref,
            data: Ref,
            flags: u32,
        ) -> i32;
    }
    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageSourceCreateWithData(data: Ref, options: Ref) -> Ref;
        fn CGImageSourceCopyPropertiesAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
        static kCGImagePropertyPixelWidth: Ref;
        static kCGImagePropertyPixelHeight: Ref;
        fn CGImageDestinationCreateWithData(
            data: Ref,
            kind: Ref,
            count: usize,
            options: Ref,
        ) -> Ref;
        fn CGImageDestinationAddImageFromSource(
            destination: Ref,
            source: Ref,
            index: usize,
            properties: Ref,
        );
        fn CGImageDestinationFinalize(destination: Ref) -> bool;
    }
    struct Owned(Ref);
    impl Owned {
        fn new(value: Ref) -> io::Result<Self> {
            if value.is_null() {
                Err(io::Error::other("native clipboard image is unavailable"))
            } else {
                Ok(Self(value))
            }
        }
        fn string(text: &str) -> io::Result<Self> {
            Self::new(unsafe {
                CFStringCreateWithBytes(
                    ptr::null(),
                    text.as_ptr(),
                    text.len() as isize,
                    0x08000100,
                    0,
                )
            })
        }
        fn bytes(&self) -> io::Result<Vec<u8>> {
            let n = unsafe { CFDataGetLength(self.0) };
            if !(1..=crate::protocol::IMAGE_LIMIT as isize).contains(&n) {
                return Err(io::Error::other("clipboard image must be at most 20 MiB"));
            }
            let bytes = unsafe { CFDataGetBytePtr(self.0) };
            if bytes.is_null() {
                return Err(io::Error::other("clipboard image bytes are unavailable"));
            }
            Ok(unsafe { std::slice::from_raw_parts(bytes, n as usize) }.to_vec())
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }
    fn board(name: &str) -> io::Result<Owned> {
        let name = Owned::string(name)?;
        let mut value = ptr::null();
        if unsafe { PasteboardCreate(name.0, &mut value) } != 0 {
            return Err(io::Error::other("macOS clipboard is unavailable"));
        }
        Owned::new(value)
    }
    fn transcode(data: Ref, format: &str) -> io::Result<Vec<u8>> {
        let source = Owned::new(unsafe { CGImageSourceCreateWithData(data, ptr::null()) })?;
        let properties =
            Owned::new(unsafe { CGImageSourceCopyPropertiesAtIndex(source.0, 0, ptr::null()) })?;
        let dimension = |key| -> io::Result<u64> {
            let value = unsafe { CFDictionaryGetValue(properties.0, key) };
            let mut number = 0i64;
            if value.is_null()
                || unsafe { CFGetTypeID(value) != CFNumberGetTypeID() }
                || unsafe { CFNumberGetValue(value, 4, &mut number) } == 0
                || number <= 0
            {
                return Err(io::Error::other("invalid clipboard image dimensions"));
            }
            Ok(number as u64)
        };
        let (w, h) = unsafe {
            (
                dimension(kCGImagePropertyPixelWidth)?,
                dimension(kCGImagePropertyPixelHeight)?,
            )
        };
        if w.checked_mul(h)
            .is_none_or(|n| n > crate::image_preview::PIXELS)
        {
            return Err(io::Error::other(
                "clipboard image exceeds 20 million pixels",
            ));
        }
        let output = Owned::new(unsafe { CFDataCreateMutable(ptr::null(), 0) })?;
        let kind = Owned::string(format)?;
        let destination = Owned::new(unsafe {
            CGImageDestinationCreateWithData(output.0, kind.0, 1, ptr::null())
        })?;
        unsafe {
            CGImageDestinationAddImageFromSource(destination.0, source.0, 0, ptr::null());
        }
        if !unsafe { CGImageDestinationFinalize(destination.0) } {
            return Err(io::Error::other("clipboard image conversion failed"));
        }
        output.bytes()
    }
    pub(super) fn read(name: &str) -> io::Result<Option<Vec<u8>>> {
        let board = board(name)?;
        unsafe {
            PasteboardSynchronize(board.0);
        }
        let mut count = 0;
        if unsafe { PasteboardGetItemCount(board.0, &mut count) } != 0 {
            return Err(io::Error::other("could not inspect macOS clipboard"));
        }
        for format in ["public.png", "public.tiff"] {
            let kind = Owned::string(format)?;
            for index in 1..=count.min(32) {
                let mut item = ptr::null_mut();
                if unsafe { PasteboardGetItemIdentifier(board.0, index as isize, &mut item) } != 0 {
                    continue;
                }
                let mut data = ptr::null();
                if unsafe { PasteboardCopyItemFlavorData(board.0, item, kind.0, &mut data) } != 0 {
                    continue;
                }
                let data = Owned::new(data)?;
                // Validate the encoded size before copying or decoding native data.
                let length = unsafe { CFDataGetLength(data.0) };
                if !(1..=crate::protocol::IMAGE_LIMIT as isize).contains(&length) {
                    return Err(io::Error::other("clipboard image must be at most 20 MiB"));
                }
                let bytes = if format == "public.png" {
                    data.bytes()?
                } else {
                    transcode(data.0, "public.png")?
                };
                super::image_decode::decode(&bytes)?;
                if unsafe { PasteboardSynchronize(board.0) } & 1 != 0 {
                    return Err(io::Error::other("clipboard changed; paste again"));
                }
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn private_mac_pasteboard_reads_png_and_tiff_without_changing_text() {
            let name = format!(
                "app.flere.companion.tests.{}.{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let board = board(&name).unwrap();
            let png = include_bytes!("../../tests/fixtures/local-image.png");
            let data =
                Owned::new(unsafe { CFDataCreate(ptr::null(), png.as_ptr(), png.len() as isize) })
                    .unwrap();
            let text = b"unsubmitted fixture text";
            let text_data = Owned::new(unsafe {
                CFDataCreate(ptr::null(), text.as_ptr(), text.len() as isize)
            })
            .unwrap();
            let text_kind = Owned::string("public.utf8-plain-text").unwrap();
            for format in ["public.png", "public.tiff"] {
                assert_eq!(unsafe { PasteboardClear(board.0) }, 0);
                assert_eq!(
                    unsafe {
                        PasteboardPutItemFlavor(
                            board.0,
                            ptr::without_provenance_mut(1),
                            text_kind.0,
                            text_data.0,
                            0,
                        )
                    },
                    0
                );
                assert!(read(&name).unwrap().is_none());
                let encoded = if format == "public.png" {
                    png.to_vec()
                } else {
                    transcode(data.0, format).unwrap()
                };
                let image = Owned::new(unsafe {
                    CFDataCreate(ptr::null(), encoded.as_ptr(), encoded.len() as isize)
                })
                .unwrap();
                let kind = Owned::string(format).unwrap();
                assert_eq!(
                    unsafe {
                        PasteboardPutItemFlavor(
                            board.0,
                            ptr::without_provenance_mut(1),
                            kind.0,
                            image.0,
                            0,
                        )
                    },
                    0
                );
                let result = read(&name).unwrap().unwrap();
                assert_eq!(
                    super::super::image_decode::decode(&result).unwrap(),
                    super::super::image_decode::decode(png).unwrap()
                );
                if format == "public.png" {
                    assert_eq!(result, png);
                }
                let mut actual = ptr::null();
                assert_eq!(
                    unsafe {
                        PasteboardCopyItemFlavorData(
                            board.0,
                            ptr::without_provenance_mut(1),
                            text_kind.0,
                            &mut actual,
                        )
                    },
                    0
                );
                assert_eq!(Owned::new(actual).unwrap().bytes().unwrap(), text);
            }
            assert_eq!(unsafe { PasteboardClear(board.0) }, 0);
        }
    }
}
#[cfg(windows)]
mod platform {
    use super::*;
    use std::{ffi::c_void, ptr};
    type Handle = *mut c_void;
    #[repr(C)]
    struct Coord {
        x: i16,
        y: i16,
    }
    #[repr(C)]
    struct Rect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }
    #[repr(C)]
    struct ScreenInfo {
        size: Coord,
        cursor: Coord,
        attributes: u16,
        window: Rect,
        max: Coord,
    }
    #[repr(C)]
    struct Bitmap {
        kind: i32,
        width: i32,
        height: i32,
        width_bytes: i32,
        planes: u16,
        bits_pixel: u16,
        bits: Handle,
    }
    #[repr(C)]
    struct Startup {
        version: u32,
        callback: Handle,
        suppress_thread: i32,
        suppress_codecs: i32,
    }
    #[repr(C)]
    struct Guid {
        a: u32,
        b: u16,
        c: u16,
        d: [u8; 8],
    }
    // WinHTTP's Windows x86_64 ABI uses opaque HINTERNET pointers, DWORD/u32,
    // INTERNET_PORT/u16 and DWORD_PTR/usize. Each successful handle is owned by Http.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetStdHandle(kind: u32) -> Handle;
        fn GetConsoleWindow() -> Handle;
        fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
        fn GetConsoleScreenBufferInfo(handle: Handle, info: *mut ScreenInfo) -> i32;
        fn GlobalSize(handle: Handle) -> usize;
        fn GlobalAlloc(flags: u32, bytes: usize) -> Handle;
        fn GlobalFree(handle: Handle) -> Handle;
        fn GlobalLock(handle: Handle) -> Handle;
        fn GlobalUnlock(handle: Handle) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(window: Handle) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, memory: Handle) -> Handle;
        fn RegisterClipboardFormatW(name: *const u16) -> u32;
        fn GetClipboardData(format: u32) -> Handle;
        fn IsClipboardFormatAvailable(format: u32) -> i32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetObjectW(object: Handle, count: i32, value: Handle) -> i32;
    }
    #[link(name = "gdiplus")]
    unsafe extern "system" {
        fn GdiplusStartup(token: *mut usize, input: *const Startup, output: Handle) -> i32;
        fn GdiplusShutdown(token: usize);
        fn GdipCreateBitmapFromHBITMAP(bitmap: Handle, palette: Handle, out: *mut Handle) -> i32;
        fn GdipDisposeImage(image: Handle) -> i32;
        fn GdipLoadImageFromStream(stream: Handle, image: *mut Handle) -> i32;
        fn GdipGetImageWidth(image: Handle, width: *mut u32) -> i32;
        fn GdipGetImageHeight(image: Handle, height: *mut u32) -> i32;
        fn GdipGetImageThumbnail(
            image: Handle,
            width: u32,
            height: u32,
            thumb: *mut Handle,
            callback: Handle,
            context: Handle,
        ) -> i32;
        fn GdipBitmapLockBits(
            image: Handle,
            rect: *const PixelRect,
            flags: u32,
            format: i32,
            data: *mut BitmapData,
        ) -> i32;
        fn GdipBitmapUnlockBits(image: Handle, data: *mut BitmapData) -> i32;
        fn GdipSaveImageToStream(
            image: Handle,
            stream: Handle,
            encoder: *const Guid,
            params: Handle,
        ) -> i32;
    }
    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CreateStreamOnHGlobal(
            memory: Handle,
            delete_on_release: i32,
            stream: *mut Handle,
        ) -> i32;
        fn GetHGlobalFromStream(stream: Handle, memory: *mut Handle) -> i32;
    }
    pub struct Console {
        input: Handle,
        output: Handle,
        input_mode: u32,
        output_mode: u32,
    }
    impl Console {
        pub fn enter() -> io::Result<Self> {
            // Windows x86_64 console HANDLEs are borrowed from the process, never closed here.
            unsafe {
                let input = GetStdHandle((-10i32) as u32);
                let output = GetStdHandle((-11i32) as u32);
                let mut input_mode = 0;
                let mut output_mode = 0;
                if GetConsoleMode(input, &mut input_mode) == 0
                    || GetConsoleMode(output, &mut output_mode) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                if SetConsoleMode(input, (input_mode & !(1 | 2 | 4 | 0x40)) | 0x200) == 0 {
                    return Err(io::Error::last_os_error());
                }
                if SetConsoleMode(output, output_mode | 4) == 0 {
                    SetConsoleMode(input, input_mode);
                    return Err(io::Error::last_os_error());
                }
                Ok(Self {
                    input,
                    output,
                    input_mode,
                    output_mode,
                })
            }
        }
    }
    impl Drop for Console {
        fn drop(&mut self) {
            unsafe {
                SetConsoleMode(self.input, self.input_mode);
                SetConsoleMode(self.output, self.output_mode);
            }
        }
    }
    pub fn size() -> (u16, u16) {
        unsafe {
            let mut info = std::mem::MaybeUninit::<ScreenInfo>::uninit();
            if GetConsoleScreenBufferInfo(GetStdHandle((-11i32) as u32), info.as_mut_ptr()) == 0 {
                return (100, 30);
            }
            let i = info.assume_init();
            (
                (i.window.right - i.window.left + 1).max(10) as u16,
                (i.window.bottom - i.window.top + 1).max(8) as u16,
            )
        }
    }
    // GDI+ flat API layouts, Windows x86_64: INT/UINT are 32-bit, pointers/ULONG_PTR 64-bit.
    #[repr(C)]
    struct PixelRect {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }
    #[repr(C)]
    struct BitmapData {
        width: u32,
        height: u32,
        stride: i32,
        format: i32,
        scan: Handle,
        reserved: usize,
    }
    struct Locked<'a> {
        image: &'a Picture,
        data: BitmapData,
    }
    impl Drop for Locked<'_> {
        fn drop(&mut self) {
            // Borrowed image stays alive; exactly one successful LockBits is released here.
            unsafe {
                GdipBitmapUnlockBits(self.image.0, &mut self.data);
            }
        }
    }
    pub fn decode_preview(bytes: &[u8], area: (usize, usize)) -> io::Result<crate::sixel::Raster> {
        let source = crate::image_preview::dimensions(bytes)?;
        let (width, height) = crate::sixel::fit(source, area);
        // The stream owns a fresh movable HGLOBAL; the image borrows it until disposed.
        // All handles remain on this thread, and guards drop in reverse dependency order.
        unsafe {
            let mut token = 0;
            let start = Startup {
                version: 1,
                callback: ptr::null_mut(),
                suppress_thread: 0,
                suppress_codecs: 0,
            };
            if GdiplusStartup(&mut token, &start, ptr::null_mut()) != 0 {
                return Err(io::Error::other("Windows image decoder unavailable"));
            }
            let _gdi = Gdi(token);
            let memory = GlobalAlloc(0x42, bytes.len());
            if memory.is_null() {
                return Err(io::Error::last_os_error());
            }
            let data = GlobalLock(memory);
            if data.is_null() {
                GlobalFree(memory);
                return Err(io::Error::last_os_error());
            }
            ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
            GlobalUnlock(memory);
            let mut stream = ptr::null_mut();
            if CreateStreamOnHGlobal(memory, 1, &mut stream) < 0 {
                GlobalFree(memory);
                return Err(io::Error::other("cannot allocate image stream"));
            }
            let stream = Stream(stream);
            let mut image = ptr::null_mut();
            if GdipLoadImageFromStream(stream.0, &mut image) != 0 {
                return Err(io::Error::other("Windows could not decode this image"));
            }
            let image = Picture(image);
            let (mut w, mut h) = (0, 0);
            if GdipGetImageWidth(image.0, &mut w) != 0
                || GdipGetImageHeight(image.0, &mut h) != 0
                || (w, h) != source
            {
                return Err(io::Error::other(
                    "decoded image dimensions do not match header",
                ));
            }
            let mut thumb = ptr::null_mut();
            if GdipGetImageThumbnail(
                image.0,
                width as u32,
                height as u32,
                &mut thumb,
                ptr::null_mut(),
                ptr::null_mut(),
            ) != 0
            {
                return Err(io::Error::other("cannot resize preview"));
            }
            let thumb = Picture(thumb);
            let rect = PixelRect {
                x: 0,
                y: 0,
                width: width as i32,
                height: height as i32,
            };
            let mut data = std::mem::MaybeUninit::<BitmapData>::zeroed();
            // PixelFormat32bppARGB = 0x26200a; read-only BGRA memory on little-endian x86_64.
            if GdipBitmapLockBits(thumb.0, &rect, 1, 0x26200a, data.as_mut_ptr()) != 0 {
                return Err(io::Error::other("cannot read preview pixels"));
            }
            let lock = Locked {
                image: &thumb,
                data: data.assume_init(),
            };
            let d = &lock.data;
            if d.scan.is_null()
                || d.width as usize != width
                || d.height as usize != height
                || (d.stride.unsigned_abs() as usize) < width * 4
            {
                return Err(io::Error::other("invalid Windows bitmap layout"));
            }
            let mut rgb = Vec::with_capacity(width * height * 3);
            for y in 0..height {
                // LockBits owns all rows including signed stride; borrow only each logical row.
                let row = std::slice::from_raw_parts(
                    d.scan.cast::<u8>().offset(y as isize * d.stride as isize),
                    width * 4,
                );
                for pixel in row.as_chunks::<4>().0 {
                    let a = pixel[3] as u32;
                    for (value, bg) in [(pixel[2], 8), (pixel[1], 12), (pixel[0], 20)] {
                        rgb.push(((value as u32 * a + bg * (255 - a) + 127) / 255) as u8);
                    }
                }
            }
            Ok(crate::sixel::Raster { width, height, rgb })
        }
    }
    struct Clipboard;
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    struct ClipboardMemory(Handle);
    impl ClipboardMemory {
        fn new(bytes: &[u8]) -> io::Result<Self> {
            // Windows x86_64 HGLOBAL/SIZE_T are pointer-sized. This movable,
            // zero-initialized allocation stays owned until SetClipboardData succeeds.
            unsafe {
                let memory = Self(GlobalAlloc(0x42, bytes.len()));
                if memory.0.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let data = GlobalLock(memory.0);
                if data.is_null() {
                    return Err(io::Error::last_os_error());
                }
                ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len());
                GlobalUnlock(memory.0);
                Ok(memory)
            }
        }
        fn publish(&mut self, format: u32) -> io::Result<()> {
            // Successful publication transfers ownership to Windows. Failure
            // leaves our allocation owned so Drop can release it.
            if unsafe { SetClipboardData(format, self.0) }.is_null() {
                return Err(io::Error::last_os_error());
            }
            self.0 = ptr::null_mut();
            Ok(())
        }
    }
    impl Drop for ClipboardMemory {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { GlobalFree(self.0) };
            }
        }
    }
    pub(super) fn copy_screenshot(bytes: &[u8], path: &Path) -> io::Result<()> {
        use clipboard_rs::common::{RustImage, RustImageData};
        let text = path
            .to_str()
            .filter(|s| !s.chars().any(char::is_control))
            .ok_or_else(|| io::Error::other("screenshot path must be printable UTF-8"))?;
        let dimensions = crate::image_preview::dimensions(bytes)?;
        let bitmap = RustImageData::from_bytes(bytes)
            .and_then(|image| image.to_rgba8())
            .map_err(io::Error::other)?;
        let (width, height) = (bitmap.width() as usize, bitmap.height() as usize);
        if (bitmap.width(), bitmap.height()) != dimensions {
            return Err(io::Error::other("screenshot bitmap dimensions differ"));
        }
        let dib = screenshot_dib(width, height, bitmap.as_raw())?;
        let text: Vec<u8> = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        // Prepare all representations before clearing the clipboard, then publish
        // under one lock; no nested clipboard-library call can release that lock.
        let mut png_memory = ClipboardMemory::new(bytes)?;
        let mut bitmap_memory = ClipboardMemory::new(&dib)?;
        let mut text_memory = ClipboardMemory::new(&text)?;
        let name: Vec<u16> = "PNG\0".encode_utf16().collect();
        unsafe {
            let format = RegisterClipboardFormatW(name.as_ptr());
            let window = GetConsoleWindow(); // Borrowed console HWND; never destroyed here.
            if format == 0 || window.is_null() || OpenClipboard(window) == 0 {
                return Err(io::Error::other(
                    "clipboard is unavailable; try screenshot again",
                ));
            }
            let _clipboard = Clipboard;
            if EmptyClipboard() == 0 {
                return Err(io::Error::last_os_error());
            }
            // Consumers may enumerate in publication order: richest format first.
            png_memory.publish(format)?;
            bitmap_memory.publish(8)?; // CF_DIB
            text_memory.publish(13)?; // CF_UNICODETEXT
        }
        Ok(())
    }
    struct Gdi(usize);
    impl Drop for Gdi {
        fn drop(&mut self) {
            unsafe {
                GdiplusShutdown(self.0);
            }
        }
    }
    struct Picture(Handle);
    impl Drop for Picture {
        fn drop(&mut self) {
            unsafe {
                GdipDisposeImage(self.0);
            }
        }
    }
    struct Stream(Handle);
    impl Drop for Stream {
        fn drop(&mut self) {
            // COM IStream starts with IUnknown's QueryInterface/AddRef/Release ABI.
            // CreateStreamOnHGlobal gives us one reference and owns the HGLOBAL on release.
            unsafe {
                let table = *(self.0 as *mut *mut usize);
                let release: unsafe extern "system" fn(Handle) -> u32 =
                    std::mem::transmute(*table.add(2));
                release(self.0);
            }
        }
    }
    fn copy_global(handle: Handle) -> io::Result<Vec<u8>> {
        unsafe {
            let size = GlobalSize(handle);
            if !(33..=crate::protocol::IMAGE_LIMIT as usize).contains(&size) {
                return Err(io::Error::other("clipboard image exceeds size limit"));
            }
            let data = GlobalLock(handle);
            if data.is_null() {
                return Err(io::Error::last_os_error());
            }
            let out = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
            GlobalUnlock(handle);
            Ok(out)
        }
    }
    pub fn image() -> io::Result<Option<Vec<u8>>> {
        unsafe {
            if OpenClipboard(ptr::null_mut()) == 0 {
                return Err(io::Error::other("clipboard is busy; try paste again"));
            }
            let _clipboard = Clipboard;
            let png_name: Vec<u16> = "PNG\0".encode_utf16().collect();
            let png = RegisterClipboardFormatW(png_name.as_ptr());
            if png != 0 && IsClipboardFormatAvailable(png) != 0 {
                let handle = GetClipboardData(png);
                if handle.is_null() {
                    return Err(io::Error::last_os_error());
                }
                return copy_global(handle).map(Some);
            }
            // Windows synthesizes CF_BITMAP from DIB clipboard formats. GDI+ handles PNG encoding.
            if IsClipboardFormatAvailable(2) == 0
                && IsClipboardFormatAvailable(8) == 0
                && IsClipboardFormatAvailable(17) == 0
            {
                return Ok(None);
            }
            let bitmap = GetClipboardData(2);
            if bitmap.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut info = std::mem::MaybeUninit::<Bitmap>::uninit();
            if GetObjectW(
                bitmap,
                std::mem::size_of::<Bitmap>() as i32,
                info.as_mut_ptr().cast(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let info = info.assume_init();
            if info.width <= 0
                || info.height <= 0
                || (info.width as u64) * (info.height as u64) > 20_000_000
            {
                return Err(io::Error::other("clipboard image exceeds pixel limit"));
            }
            let mut token = 0;
            let start = Startup {
                version: 1,
                callback: ptr::null_mut(),
                suppress_thread: 0,
                suppress_codecs: 0,
            };
            if GdiplusStartup(&mut token, &start, ptr::null_mut()) != 0 {
                return Err(io::Error::other("Windows image encoder unavailable"));
            }
            let _gdi = Gdi(token);
            let mut image = ptr::null_mut();
            if GdipCreateBitmapFromHBITMAP(bitmap, ptr::null_mut(), &mut image) != 0 {
                return Err(io::Error::other("cannot read clipboard bitmap"));
            }
            let image = Picture(image);
            let mut stream = ptr::null_mut();
            if CreateStreamOnHGlobal(ptr::null_mut(), 1, &mut stream) < 0 {
                return Err(io::Error::other("cannot allocate image stream"));
            }
            let stream = Stream(stream);
            let encoder = Guid {
                a: 0x557cf406,
                b: 0x1a04,
                c: 0x11d3,
                d: [0x9a, 0x73, 0, 0, 0xf8, 0x1e, 0xf3, 0x2e],
            };
            if GdipSaveImageToStream(image.0, stream.0, &encoder, ptr::null_mut()) != 0 {
                return Err(io::Error::other("cannot encode clipboard PNG"));
            }
            let mut memory = ptr::null_mut();
            if GetHGlobalFromStream(stream.0, &mut memory) < 0 {
                return Err(io::Error::other("cannot read encoded PNG"));
            }
            copy_global(memory).map(Some)
        }
    }
    #[cfg(test)]
    mod screenshot_tests {
        use super::*;
        use clipboard_rs::{Clipboard as _, ClipboardContext, common::RustImage};

        #[test]
        #[ignore = "requires an attached Windows console and explicit clipboard-test authority"]
        fn screenshot_publishes_original_png_bitmap_and_path_together() {
            let png = include_bytes!("../../tests/fixtures/local-image.png");
            let path = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
                .join("Flere/acceptance/clipboard-fixture.png");
            copy_screenshot(png, &path).unwrap();
            let context = ClipboardContext::new().unwrap();
            let actual = context.get_buffer("PNG").unwrap();
            // GlobalSize may include zero-initialized allocation padding; all
            // original PNG bytes and the independent text/bitmap formats must survive.
            assert!(actual.starts_with(png));
            assert!(actual[png.len()..].iter().all(|byte| *byte == 0));
            assert_eq!(context.get_text().unwrap(), path.to_str().unwrap());
            assert_eq!(context.get_image().unwrap().get_size(), (2, 2));
            // Reading PNG alone can hide a broken bitmap fallback. Ask Windows
            // to synthesize CF_BITMAP and verify its native dimensions separately.
            unsafe {
                assert_ne!(OpenClipboard(GetConsoleWindow()), 0);
                let _clipboard = super::Clipboard;
                let bitmap = GetClipboardData(2);
                assert!(!bitmap.is_null());
                let mut info = std::mem::MaybeUninit::<Bitmap>::uninit();
                assert_ne!(
                    GetObjectW(
                        bitmap,
                        std::mem::size_of::<Bitmap>() as i32,
                        info.as_mut_ptr().cast()
                    ),
                    0
                );
                let info = info.assume_init();
                assert_eq!((info.width, info.height), (2, 2));
            }
        }
    }
}
pub use platform::*;
pub fn image_file(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let f = options.open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || m.len() > crate::protocol::IMAGE_LIMIT {
        return Err(io::Error::other("image file exceeds 20 MiB"));
    }
    let mut bytes = Vec::new();
    f.take(crate::protocol::IMAGE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > crate::protocol::IMAGE_LIMIT || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(io::Error::other(
            "image must be a PNG no larger than 20 MiB",
        ));
    }
    Ok(bytes)
}

#[cfg(test)]
mod screenshot_dib_tests {
    use super::*;

    #[test]
    fn bitmap_fallback_has_bottom_up_bgr_rows_padding_and_black_alpha_matte() {
        // Three pixels require three padding bytes. The asymmetric rows expose
        // either a vertical flip or red/blue channel swap, independently of PNG.
        let rgba = [
            [255, 0, 0, 255],
            [0, 255, 0, 128],
            [0, 0, 255, 0],
            [0, 0, 255, 255],
            [10, 20, 30, 255],
            [200, 100, 50, 64],
        ]
        .concat();
        let dib = screenshot_dib(3, 2, &rgba).unwrap();
        assert_eq!(dib.len(), 64);
        assert_eq!(
            &dib[..16],
            &[40, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 1, 0, 24, 0]
        );
        assert!(dib[16..40].iter().all(|&b| b == 0)); // BI_RGB, no palette.
        assert_eq!(
            &dib[40..],
            &[
                255, 0, 0, 30, 20, 10, 13, 25, 50, 0, 0, 0, // Bottom row + padding.
                0, 0, 255, 0, 128, 0, 0, 0, 0, 0, 0, 0, // Top row + padding.
            ]
        );
    }

    #[test]
    fn bitmap_fallback_rejects_invalid_dimensions_and_rgba_lengths_before_allocation() {
        for (width, height) in [(0, 1), (1, 0), (20_000_001, 1), (usize::MAX, 2)] {
            assert!(screenshot_dib(width, height, &[]).is_err());
        }
        for bytes in [15, 17] {
            assert!(screenshot_dib(2, 2, &vec![0; bytes]).is_err());
        }
    }
}
