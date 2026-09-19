//! Export the complete Flere frame; never inspect the desktop or crop a pane.
use super::*;
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub(super) type Job = Receiver<io::Result<Vec<u8>>>;
pub(super) struct Export {
    pub id: u64,
    png: Vec<u8>,
    offset: usize,
    ended: bool,
    started: Instant,
}

impl Ui {
    fn screenshot_layers(&self, canvas: &Canvas) -> Vec<crate::screenshot::Layer> {
        if self.arcade.is_some() && !self.arcade_obscured() {
            return Vec::new(); // The game is entirely composed into terminal cells.
        }
        use crate::screenshot::{Layer, Pixels};
        let Some((cw, ch)) = self.graphics_cell() else {
            return Vec::new();
        };
        let mut layers = Vec::new();
        if let Some(preview) = self.screenshot_preview() {
            if let Ok((width, height)) = crate::image_preview::dimensions(preview) {
                let (w, h) = local_graphics::fit(
                    (width as usize, height as usize),
                    ((canvas.width - 4) * cw, (canvas.height - 7) * ch),
                );
                layers.push(Layer {
                    x: (canvas.width - w.div_ceil(cw)) / 2,
                    y: 3,
                    columns: w as f64 / cw as f64,
                    rows: h as f64 / ch as f64,
                    pixels: Pixels::Encoded(preview.to_vec()),
                });
            }
            return layers;
        }
        for badge in &canvas.badges {
            if let Some(bytes) = self.project_icons.bytes(&badge.key)
                && let Ok((w, h)) = crate::image_preview::dimensions(bytes)
            {
                let (w, h) = local_graphics::fit(
                    (w as usize, h as usize),
                    (badge.columns as usize * cw, ch),
                );
                layers.push(Layer {
                    x: badge.x as usize,
                    y: badge.y as usize,
                    columns: w as f64 / cw as f64,
                    rows: h as f64 / ch as f64,
                    pixels: Pixels::Encoded(bytes.to_vec()),
                });
            }
        }
        if let Some(layer) = self.pet_screenshot() {
            layers.push(layer);
        }
        layers
    }
    pub(super) fn copy_screenshot(&mut self) {
        if self.remote.is_none()
            && (std::env::var_os("SSH_CONNECTION").is_some()
                || std::env::var_os("SSH_TTY").is_some())
        {
            self.notice =
                "Flere screenshots need a local clipboard; use flere-connect over SSH".into();
            return;
        }
        if self.remote.as_ref().is_some_and(|r| !r.screenshot_capable) {
            self.notice = "Update flere-connect to copy a Flere screenshot".into();
            return;
        }
        if self.screenshot_job.is_some()
            || self.remote.as_ref().is_some_and(|r| r.screenshot.is_some())
        {
            self.notice = "Creating Flere screenshot…".into();
            return;
        }
        // Close the action launcher before freezing the same complete frame used
        // for display. Preserve the board, preview, scrollback and chosen panes.
        self.menu = false;
        self.nav = false;
        self.focus = Focus::Terminal;
        self.nav_help_at = None;
        let canvas = self.frame_canvas();
        let layers = self.screenshot_layers(&canvas);
        let frame = crate::screenshot::Frame {
            width: canvas.width,
            height: canvas.height,
            cells: canvas.cells,
            layers,
            cursor: self.frame_cursor(),
        };
        let (send, receive) = mpsc::channel();
        match std::thread::Builder::new()
            .name("flere-screenshot".into())
            .spawn(move || {
                let _ = send.send(frame.png());
            }) {
            Ok(_) => {
                self.screenshot_job = Some(receive);
                // Return to the buffer at invocation, not when the worker finishes:
                // a later completion must not steal focus from another pane/menu.
                self.nav = false;
                self.focus = Focus::Terminal;
                self.nav_help_at = None;
                self.notice = "Creating Flere screenshot…".into();
            }
            Err(error) => self.notice = format!("Screenshot: {error}"),
        }
    }

    pub(super) fn tick_screenshot(&mut self, can_output: bool) -> bool {
        if !can_output {
            return false;
        }
        if let Some(remote) = &mut self.remote
            && let Some(export) = &mut remote.screenshot
        {
            use crate::remote_protocol as p;
            let packet = if export.started.elapsed() > Duration::from_secs(30) {
                Some(p::Packet::new(p::SCREENSHOT_CANCEL, export.id, []))
            } else if export.offset < export.png.len() {
                let end = (export.offset + p::CHUNK).min(export.png.len());
                let mut chunk = (export.offset as u64).to_be_bytes().to_vec();
                chunk.extend_from_slice(&export.png[export.offset..end]);
                export.offset = end;
                Some(p::Packet::new(p::SCREENSHOT_DATA, export.id, chunk))
            } else if !export.ended {
                export.ended = true;
                Some(p::Packet::new(p::SCREENSHOT_END, export.id, []))
            } else {
                None
            };
            if let Some(packet) = packet {
                let expired = packet.tag == p::SCREENSHOT_CANCEL;
                if let Err(error) = packet.write(&mut output::writer()) {
                    remote.screenshot = None;
                    self.notice = format!("Screenshot: {error}");
                } else if expired {
                    remote.screenshot = None;
                    self.notice = "Screenshot copy timed out".into();
                }
                return true;
            }
        }
        let Some(job) = &self.screenshot_job else {
            return false;
        };
        let result = match job.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => Err(io::Error::other("image worker stopped")),
        };
        self.screenshot_job = None;
        if let Some(remote) = &mut self.remote {
            self.notice = match result {
                Ok(png) => {
                    remote.screenshot_counter += 1;
                    let id = remote.screenshot_counter;
                    let packet = crate::remote_protocol::Packet::new(
                        crate::remote_protocol::SCREENSHOT_BEGIN,
                        id,
                        (png.len() as u64).to_be_bytes(),
                    );
                    match packet.write(&mut output::writer()) {
                        Ok(()) => {
                            remote.screenshot = Some(Export {
                                id,
                                png,
                                offset: 0,
                                ended: false,
                                started: Instant::now(),
                            });
                            "Copying Flere screenshot to your local clipboard…".into()
                        }
                        Err(error) => format!("Screenshot: {error}"),
                    }
                }
                Err(error) => format!("Screenshot: {error}"),
            };
            return true;
        }
        // Pasteboard APIs stay on the UI thread. Closing the UI drops a pending
        // image instead of allowing a detached worker to replace the clipboard.
        self.notice = match result.and_then(|png| copy(&png)) {
            Ok(()) => "Flere screenshot copied · paste in your chat".into(),
            Err(error) => format!("Screenshot: {error}"),
        };
        true
    }
    pub(super) fn screenshot_result(
        &mut self,
        packet: crate::remote_protocol::Packet,
    ) -> io::Result<()> {
        let remote = self.remote.as_mut().unwrap();
        let Some(export) = remote.screenshot.as_ref() else {
            return Ok(());
        };
        if packet.id != export.id
            || packet.data.is_empty()
            || packet.data.len() > 1024
            || packet.data[0] > 1
        {
            return Err(wire::invalid("invalid screenshot receipt"));
        }
        if packet.data[0] == 1 && !export.ended {
            return Err(wire::invalid("premature screenshot receipt"));
        }
        remote.screenshot = None;
        self.notice = if packet.data[0] == 1 {
            "Flere screenshot copied · paste in your chat".into()
        } else {
            format!(
                "Screenshot: {}",
                wire::passive(std::str::from_utf8(&packet.data[1..]).map_err(io::Error::other)?)
            )
        };
        Ok(())
    }

    pub(super) fn paint_preview(&self, c: &mut Canvas, l: Layout, p: &Preview) {
        c.fill(
            l.terminal_x,
            l.terminal_y,
            l.cols,
            l.rows,
            style(TEXT, BG, false),
        );
        c.text(
            l.terminal_x,
            l.terminal_y,
            l.cols,
            &format!("{}  [preview · q returns]", p.name),
            style(GOLD, BG, true),
        );
        let lines = p.wrapped(l.cols);
        for (i, line) in lines
            .iter()
            .skip(p.offset.min(lines.len().saturating_sub(1)))
            .take(l.rows.saturating_sub(1))
            .enumerate()
        {
            c.text(
                l.terminal_x,
                l.terminal_y + 1 + i,
                l.cols,
                line,
                style(TEXT, BG, false),
            );
        }
    }

    pub(super) fn paint_buffer(&self, c: &mut Canvas) {
        let l = self.terminal_layout();
        let tx = l.left;
        let tw = l.width - l.left - l.right;
        let bright = style(TEXT, BG, false);
        let quiet = style(MUTED, BG, false);
        if self.snapshot.session().is_none() {
            c.text(
                tx + 3,
                5,
                tw.saturating_sub(6),
                self.snapshot
                    .workspace()
                    .filter(|w| !w.meta.operation.is_empty())
                    .map_or("Workspace is stopped.", |w| w.meta.operation.as_str()),
                bright,
            );
            c.text(
                tx + 3,
                7,
                tw.saturating_sub(6),
                "S start agent  ·  t shell  ·  Space actions",
                quiet,
            );
        } else {
            for y in 0..l.rows.min(self.snapshot.rows) {
                for x in 0..l.cols.min(self.snapshot.cols) {
                    let dest = (l.terminal_y + y) * l.width + l.terminal_x + x;
                    if dest < c.cells.len() {
                        let mut cell = self.snapshot.cells[y * self.snapshot.cols + x].clone();
                        if cell.style.fg == Color::Default {
                            cell.style.fg = crate::terminal::DEFAULT_FG
                        }
                        if cell.style.bg == Color::Default {
                            cell.style.bg = crate::terminal::DEFAULT_BG
                        }
                        c.cells[dest] = cell
                    }
                }
            }
        }
        if let Some(view) = &self.scrollback
            && view.matches(&self.snapshot, self.terminal_layout())
        {
            view.paint(c);
        }
        if let Some(p) = self
            .previews
            .get(&(self.snapshot.active, self.snapshot.tab))
        {
            self.paint_preview(c, l, p);
        }
        if let Some(selection) = &self.selection
            && selection.matches(&self.snapshot, self.terminal_layout())
        {
            selection.paint(c);
        }
    }
}

fn copy(png: &[u8]) -> io::Result<()> {
    // Terminals paste text, so retain a private PNG alongside the image flavor.
    // Use the existing bounded attachment cache; never evict a file that might
    // still be referenced by a clipboard or an unsubmitted native draft.
    let mut upload = crate::attachments::Upload::new(png.len() as u64)?;
    upload.append(0, png)?;
    let path = upload.finish()?;
    if let Err(error) = os::copy_png(png, &path) {
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}
