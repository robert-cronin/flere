//! Explicit read-only file/terminal-click preview, scoped to one attachment and exact target.
use super::*;
use crate::remote_protocol::{self as protocol, Packet};
mod gallery;
pub(super) use gallery::Loader;
pub(super) struct ImagePreview {
    pub(super) id: u64,
    target: (String, u64, u64, String),
    name: String,
    gallery: gallery::Gallery,
    loading: bool,
    pending_steps: i64,
    navigated: bool,
    pub(super) bytes: Vec<u8>,
    offset: usize,
    pub(super) started: bool,
    pub(super) finished: bool,
    pub(super) error: String,
}
impl ImagePreview {
    pub(super) fn clear_id(&self) -> Option<u64> {
        self.started.then_some(self.id)
    }
}
impl Ui {
    fn active_image(&self) -> Option<&ImagePreview> {
        self.remote
            .as_ref()
            .and_then(|r| r.preview.as_ref())
            .or_else(|| {
                self.local_graphics
                    .as_ref()
                    .and_then(|g| g.preview.as_ref())
            })
    }
    fn active_image_mut(&mut self) -> Option<&mut ImagePreview> {
        self.remote
            .as_mut()
            .and_then(|r| r.preview.as_mut())
            .or_else(|| {
                self.local_graphics
                    .as_mut()
                    .and_then(|g| g.preview.as_mut())
            })
    }
    pub(super) fn image_view(&self) -> bool {
        self.active_image().is_some()
    }
    pub(super) fn screenshot_preview(&self) -> Option<&[u8]> {
        self.active_image()
            .filter(|p| p.finished && p.error.is_empty())
            .map(|p| p.bytes.as_slice())
    }
    fn image_target(&self) -> (String, u64, u64, String) {
        (
            self.snapshot.epoch.clone(),
            self.snapshot.active,
            self.snapshot.tab,
            self.snapshot
                .session()
                .map(|s| s.run.clone())
                .unwrap_or_default(),
        )
    }
    pub(super) fn image_key(&mut self, key: &Key) -> bool {
        if self.image_view() {
            if matches!(key, Key::Bytes(b) if b == b"s") {
                self.copy_screenshot();
                return true;
            }
            if matches!(key, Key::Bytes(b) if b == b"q" || b == b"\x1b" || b == b"\0") {
                self.image_close();
            } else if let Some(direction) = image_direction(key) {
                self.image_step(direction);
            }
            return true; // Every modal input, including paste/mouse and close, is consumed.
        }
        if self.focus != Focus::Files
            || self.prefs.inspector != Inspector::Files
            || !matches!(key, Key::Bytes(b) if b == b"p")
        {
            return false;
        }
        let selected = self
            .explorers
            .get(&self.snapshot.active)
            .and_then(|e| e.entries.get(e.selected))
            .cloned();
        if let Some((_, path, false)) = selected {
            self.image_open(&path);
        } else {
            self.notice = "Select a PNG or JPEG file".into();
        }
        true
    }
    pub(super) fn image_open(&mut self, path: &Path) {
        if !self.image_capable() {
            self.notice = if self.remote.is_none() {
                "Image preview needs a terminal with Kitty graphics, such as Ghostty"
            } else {
                "Image preview unavailable: use Windows Terminal 1.22+ with the Windows companion"
            }
            .into();
            return;
        }
        let result = (|| -> io::Result<ImagePreview> {
            Ok(ImagePreview {
                id: self.image_next_id()?,
                target: self.image_target(),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
                gallery: gallery::Gallery::new(path.into()),
                loading: true,
                pending_steps: 0,
                navigated: false,
                bytes: Vec::new(),
                offset: 0,
                started: false,
                finished: false,
                error: String::new(),
            })
        })();
        match result {
            Ok(preview) => {
                self.flush_input();
                self.paste_target = None;
                self.selection = None;
                crate::diagnostics::record(
                    "preview-open",
                    &format!("id={} bytes={}", preview.id, preview.bytes.len()),
                );
                self.image_close();
                self.image_install(preview);
            }
            Err(e) => {
                crate::diagnostics::error("preview-error", &e);
                self.notice = e.to_string();
            }
        }
    }
    fn image_next_id(&self) -> io::Result<u64> {
        self.remote
            .as_ref()
            .map(|r| r.preview_counter)
            .or_else(|| self.local_graphics.as_ref().map(|g| g.preview_counter))
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| wire::invalid("preview counter exhausted"))
    }
    fn image_install(&mut self, preview: ImagePreview) {
        if let Some(r) = &mut self.remote {
            r.preview_counter = preview.id;
            r.preview = Some(preview);
        } else if let Some(g) = &mut self.local_graphics {
            g.preview_counter = preview.id;
            g.preview = Some(preview);
        }
    }
    fn image_take(&mut self) -> Option<ImagePreview> {
        if let Some(g) = &mut self.local_graphics {
            let preview = g.preview.take();
            if preview.is_some() {
                g.cancel_preview();
                g.preview_clear = true;
            }
            return preview;
        }
        let r = self.remote.as_mut()?;
        let preview = r.preview.take()?;
        if preview.started {
            crate::diagnostics::record("preview-close", &format!("id={}", preview.id));
            r.preview_clear = Some(preview.id);
        }
        Some(preview)
    }
    fn image_close(&mut self) {
        self.image_take();
    }
    fn image_step(&mut self, direction: i64) {
        let Some(p) = self.active_image_mut() else {
            return;
        };
        if !p.gallery.discovered {
            p.pending_steps = p.pending_steps.saturating_add(direction);
            return;
        }
        if !p.gallery.step(direction) {
            return;
        }
        let id = match self.image_next_id() {
            Ok(id) => id,
            Err(error) => {
                self.notice = error.to_string();
                return;
            }
        };
        let mut p = self.image_take().unwrap();
        p.id = id;
        p.name = p
            .gallery
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into();
        p.loading = true;
        p.pending_steps = 0;
        p.navigated = true;
        p.bytes.clear();
        p.error.clear();
        p.offset = 0;
        p.started = false;
        p.finished = false;
        self.image_install(p);
    }
    pub(super) fn image_validate(&mut self) -> io::Result<bool> {
        let target = self.image_target();
        if self.active_image().is_some_and(|p| p.target != target) {
            self.image_close();
        }
        let preview = self
            .remote
            .as_mut()
            .and_then(|r| r.preview.as_mut())
            .or_else(|| {
                self.local_graphics
                    .as_mut()
                    .and_then(|g| g.preview.as_mut())
            });
        let dirty = self.image_loader.poll(preview);
        let steps = self
            .active_image_mut()
            .filter(|p| p.gallery.discovered)
            .map(|p| std::mem::take(&mut p.pending_steps))
            .unwrap_or(0);
        if steps != 0 {
            self.image_step(steps);
        }
        if let Some(error) = self
            .active_image()
            .filter(|p| dirty && !p.navigated && !p.error.is_empty())
            .map(|p| p.error.clone())
        {
            self.image_close();
            self.notice = error;
        }
        let preview = self
            .remote
            .as_ref()
            .and_then(|r| r.preview.as_ref())
            .or_else(|| {
                self.local_graphics
                    .as_ref()
                    .and_then(|g| g.preview.as_ref())
            });
        if let Some(preview) = preview {
            self.image_loader.request(preview);
        }
        if let Some(id) = self.remote.as_ref().and_then(|r| r.preview_clear) {
            Packet::new(protocol::PREVIEW_CLEAR, id, Vec::new()).write(&mut output::writer())?;
            // Retain the owned ID for destructor cleanup if admission fails.
            self.remote.as_mut().unwrap().preview_clear = None;
            return Ok(true);
        }
        let cleared = self
            .local_graphics
            .as_mut()
            .is_some_and(|g| std::mem::take(&mut g.preview_clear));
        Ok(dirty || cleared)
    }
    pub(super) fn image_error(&mut self, packet: Packet) -> io::Result<()> {
        if packet.data.len() > 1024 {
            return Err(wire::invalid("preview error exceeds bound"));
        }
        let r = self.remote.as_mut().unwrap();
        if let Some(p) = r.preview.as_mut().filter(|p| p.id == packet.id) {
            p.error = wire::passive(std::str::from_utf8(&packet.data).map_err(io::Error::other)?);
            p.finished = true;
            p.bytes.clear();
        } else if packet.id == 0 || packet.id > r.preview_counter {
            return Err(wire::invalid("preview error has no matching offer"));
        }
        Ok(())
    }
    pub(super) fn image_canvas(&self) -> Option<Canvas> {
        let p = self.active_image()?;
        let l = self.layout;
        let mut c = Canvas::new(l.width, l.height);
        c.text(
            1,
            1,
            l.width - 2,
            "FLERE · IMAGE PREVIEW",
            style(CYAN, BG, true),
        );
        c.text(
            1,
            2,
            l.width - 2,
            &format!(
                "{}/{} · {}",
                p.gallery.selected + 1,
                p.gallery.paths.len(),
                p.name
            ),
            style(CYAN, BG, true),
        );
        c.text(
            1,
            l.height - 2,
            l.width - 2,
            "h/k ←/↑ previous · l/j →/↓ next · s screenshot · q / Esc close",
            style(MUTED, BG, false),
        );
        if !p.error.is_empty() {
            c.text(1, 3, l.width - 2, &p.error, style(GOLD, BG, false));
        } else if p.loading {
            c.text(1, 3, l.width - 2, "Loading image…", style(MUTED, BG, false));
        }
        if !p.gallery.warning.is_empty() {
            c.text(
                1,
                l.height - 3,
                l.width - 2,
                &p.gallery.warning,
                style(GOLD, BG, false),
            );
        }
        Some(c)
    }
    pub(super) fn image_emit(&mut self) -> io::Result<()> {
        if self
            .active_image()
            .is_some_and(|p| p.loading || !p.error.is_empty())
        {
            return Ok(());
        }
        if let Some(g) = &mut self.local_graphics {
            return g.emit_preview();
        }
        let Some(p) = self.remote.as_mut().and_then(|r| r.preview.as_mut()) else {
            return Ok(());
        };
        if p.finished {
            return Ok(());
        }
        let mut out = output::writer();
        if !p.started {
            Packet::new(
                protocol::PREVIEW_BEGIN,
                p.id,
                (p.bytes.len() as u64).to_be_bytes(),
            )
            .write(&mut out)?;
            p.started = true;
        }
        // Bounded work per event-loop turn keeps close/target changes responsive over SSH.
        for _ in 0..4 {
            if p.offset == p.bytes.len() {
                Packet::new(protocol::PREVIEW_END, p.id, Vec::new()).write(&mut out)?;
                p.finished = true;
                // Keep only the current bounded source for screenshot export.
                break;
            }
            let end = (p.offset + protocol::CHUNK).min(p.bytes.len());
            let mut data = (p.offset as u64).to_be_bytes().to_vec();
            data.extend_from_slice(&p.bytes[p.offset..end]);
            Packet::new(protocol::PREVIEW_DATA, p.id, data).write(&mut out)?;
            p.offset = end;
        }
        Ok(())
    }
}

fn image_direction(key: &Key) -> Option<i64> {
    match key {
        Key::Bytes(b)
            if matches!(
                b.as_slice(),
                b"h" | b"k" | b"\x1b[D" | b"\x1b[A" | b"\x1bOD" | b"\x1bOA"
            ) =>
        {
            Some(-1)
        }
        Key::Bytes(b)
            if matches!(
                b.as_slice(),
                b"l" | b"j" | b"\x1b[C" | b"\x1b[B" | b"\x1bOC" | b"\x1bOB"
            ) =>
        {
            Some(1)
        }
        _ => None,
    }
}

pub(super) fn image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| matches!(s.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}
