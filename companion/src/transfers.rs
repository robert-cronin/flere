//! Explicit local file transfer endpoint. At most one file is in flight; incomplete
//! downloads are removed and local applications open only after explicit completion.
use crate::{
    protocol::Packet,
    remote_files::{Sink, Source},
    remote_services as service,
};
use std::{
    collections::VecDeque,
    fs, io,
    path::{Path, PathBuf},
    time::Instant,
};
enum Body {
    Upload {
        source: Source,
        ready: bool,
        ended: bool,
    },
    Download {
        sink: Sink,
        open: bool,
    },
}
struct Active {
    id: u64,
    body: Body,
    offset: u64,
    touched: Instant,
}
#[derive(Default)]
pub struct Transfers {
    active: Option<Active>,
    last: u64,
}
fn downloads() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| service::invalid("local home directory unavailable"))?;
    let root = home.join("Downloads");
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&root)?;
    let meta = fs::symlink_metadata(&root)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(service::invalid("Downloads must be a real directory"));
    }
    Ok(root)
}
pub fn open_allowed(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "html" | "htm" | "pdf"
        )
    })
}
pub fn launch(value: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(windows)]
    let program = "explorer.exe";
    std::process::Command::new(program)
        .arg(value)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    Ok(())
}
fn nonce(id: u64) -> String {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:032x}",
        stamp ^ (u128::from(std::process::id()) << 64) ^ u128::from(id)
    )
}
impl Transfers {
    pub fn reset(&mut self) {
        self.active = None;
        self.last = 0;
    }
    pub fn packet(&mut self, packet: Packet, queue: &mut VecDeque<Packet>) {
        if packet.tag == service::REQUEST {
            queue.push_back(Packet::new(
                service::RESULT,
                packet.id,
                service::result(false, "A fresh local choice is required"),
            ));
            return;
        }
        self.authorized_packet(packet, queue);
    }
    /// Called for a request only after the local prompt captured fresh user input.
    pub fn authorized_packet(&mut self, packet: Packet, queue: &mut VecDeque<Packet>) {
        let id = packet.id;
        if let Err(error) = self.receive(packet, queue) {
            if self.active.as_ref().is_some_and(|a| a.id == id) {
                self.active = None;
            }
            queue.push_back(Packet::new(
                service::RESULT,
                id,
                service::result(false, &error.to_string()),
            ));
        }
    }
    fn receive(&mut self, packet: Packet, queue: &mut VecDeque<Packet>) -> io::Result<()> {
        let id = packet.id;
        if packet.tag == service::REQUEST {
            if id == 0 || id <= self.last {
                return Err(service::invalid("stale file request"));
            }
            self.last = id;
            if self.active.is_some() {
                return Err(service::invalid("another file transfer is in progress"));
            }
            let value = service::decode(&packet.data)?;
            let body = match value.get("op").and_then(serde_json::Value::as_str) {
                Some("upload" | "attach") => {
                    let path = value
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| service::printable(s))
                        .ok_or_else(|| service::invalid("choose an absolute local upload path"))?;
                    let source = Source::open(Path::new(path))?;
                    queue.push_back(Packet::new(
                        service::METADATA,
                        id,
                        service::metadata(&source.name, source.size)?,
                    ));
                    Body::Upload {
                        source,
                        ready: false,
                        ended: false,
                    }
                }
                Some("download") => {
                    let (name, size) = service::file_metadata(&packet.data)?;
                    let open = value
                        .get("open")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    let path = downloads()?.join(name);
                    if open && !open_allowed(&path) {
                        return Err(service::invalid(
                            "Open locally supports images, HTML reports and PDF files",
                        ));
                    }
                    let sink = Sink::new(&path, size, &nonce(id))?;
                    queue.push_back(Packet::new(service::READY, id, Vec::new()));
                    Body::Download { sink, open }
                }
                _ => return Err(service::invalid("unknown local file request")),
            };
            self.active = Some(Active {
                id,
                body,
                offset: 0,
                touched: Instant::now(),
            });
            return Ok(());
        }
        if !self.active.as_ref().is_some_and(|a| a.id == id) {
            if packet.tag == service::RESULT {
                service::decode(&packet.data)?;
                return Ok(());
            }
            return Err(service::invalid("file request is no longer active"));
        }
        let mut active = self.active.take().unwrap();
        match (packet.tag, &mut active.body) {
            (service::READY, Body::Upload { ready, .. }) if packet.data.is_empty() && !*ready => {
                *ready = true;
            }
            (service::DATA, Body::Download { sink, .. }) => {
                let (offset, bytes) = service::unchunk(&packet.data)?;
                sink.write(offset, bytes)?;
                active.offset += bytes.len() as u64;
            }
            (service::END, Body::Download { .. }) if packet.data.is_empty() => {
                let Body::Download { sink, open } = active.body else {
                    unreachable!()
                };
                let path = sink.finish()?;
                if open {
                    launch(
                        path.to_str()
                            .ok_or_else(|| service::invalid("local path is not UTF-8"))?,
                    )?;
                }
                queue.push_back(Packet::new(
                    service::RESULT,
                    id,
                    service::result(
                        true,
                        &format!(
                            "Downloaded {}{}",
                            path.display(),
                            if open { " · opened locally" } else { "" }
                        ),
                    ),
                ));
                return Ok(());
            }
            (service::RESULT, Body::Upload { ended: true, .. }) => {
                service::decode(&packet.data)?;
                return Ok(());
            }
            (service::CANCEL, _) => return Ok(()),
            _ => return Err(service::invalid("invalid file packet direction or state")),
        }
        active.touched = Instant::now();
        self.active = Some(active);
        Ok(())
    }
    pub fn tick(&mut self, queue: &mut VecDeque<Packet>) {
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.touched.elapsed().as_secs() >= service::IDLE_SECONDS)
        {
            let id = self.active.take().unwrap().id;
            queue.push_back(Packet::new(service::CANCEL, id, b"file transfer timed out"));
        }
        let Some(active) = &mut self.active else {
            return;
        };
        let Body::Upload {
            source,
            ready,
            ended,
        } = &mut active.body
        else {
            return;
        };
        if !*ready || *ended || queue.len() >= 8 {
            return;
        }
        let result = if active.offset == source.size {
            source.finish().map(|()| {
                *ended = true;
                Packet::new(service::END, active.id, Vec::new())
            })
        } else {
            source.read(active.offset).and_then(|bytes| {
                let data = service::chunk(active.offset, &bytes)?;
                active.offset += bytes.len() as u64;
                *ready = false;
                Ok(Packet::new(service::DATA, active.id, data))
            })
        };
        match result {
            Ok(packet) => {
                active.touched = Instant::now();
                queue.push_back(packet);
            }
            Err(error) => {
                let id = self.active.take().unwrap().id;
                queue.push_back(Packet::new(
                    service::CANCEL,
                    id,
                    error.to_string().into_bytes(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let root = PathBuf::from(
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("LOCALAPPDATA"))
                .unwrap(),
        )
        .join(".cache/flere/tests")
        .join(format!("transfer-{}", nonce(0)));
        fs::create_dir_all(&root).unwrap();
        root
    }
    fn receiving(path: &Path, size: u64, id: u64) -> Transfers {
        Transfers {
            active: Some(Active {
                id,
                body: Body::Download {
                    sink: Sink::new(path, size, &nonce(id)).unwrap(),
                    open: false,
                },
                offset: 0,
                touched: Instant::now(),
            }),
            last: id,
        }
    }
    #[test]
    fn stale_chunks_do_not_discard_current_transfer_and_bad_offsets_abort() {
        let root = root();
        let path = root.join("result.bin");
        let mut transfer = receiving(&path, 4, 10);
        let mut queue = VecDeque::new();
        transfer.packet(
            Packet::new(service::DATA, 9, service::chunk(0, b"old").unwrap()),
            &mut queue,
        );
        assert_eq!(queue.pop_front().unwrap().id, 9);
        assert!(transfer.active.is_some());
        transfer.packet(
            Packet::new(service::DATA, 10, service::chunk(0, b"good").unwrap()),
            &mut queue,
        );
        assert!(!path.exists());
        transfer.packet(Packet::new(service::END, 10, Vec::new()), &mut queue);
        assert!(
            service::decode(&queue.pop_front().unwrap().data).unwrap()["success"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(fs::read(&path).unwrap(), b"good");
        assert!(transfer.active.is_none());
        let failed = root.join("bad.bin");
        let mut transfer = receiving(&failed, 4, 11);
        transfer.packet(
            Packet::new(service::DATA, 11, service::chunk(1, b"bad").unwrap()),
            &mut queue,
        );
        assert!(transfer.active.is_none());
        assert!(!failed.exists());
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            1,
            "bad chunks clean staging files"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cancelled_uploads_stop_reading_and_open_locally_rejects_executables() {
        let root = root();
        let path = root.join("upload.bin");
        fs::write(&path, vec![42; service::CHUNK + 4]).unwrap();
        let mut transfer = Transfers::default();
        let mut queue = VecDeque::new();
        transfer.authorized_packet(
            Packet::new(
                service::REQUEST,
                1,
                serde_json::to_vec(&serde_json::json!({"op":"upload","path":path})).unwrap(),
            ),
            &mut queue,
        );
        assert_eq!(queue.pop_front().unwrap().tag, service::METADATA);
        transfer.tick(&mut queue);
        assert!(queue.is_empty(), "source waits for acceptance");
        transfer.packet(Packet::new(service::READY, 1, Vec::new()), &mut queue);
        transfer.tick(&mut queue);
        assert_eq!(queue.pop_front().unwrap().tag, service::DATA);
        transfer.packet(Packet::new(service::CANCEL, 1, Vec::new()), &mut queue);
        transfer.tick(&mut queue);
        assert!(transfer.active.is_none() && queue.is_empty());
        for name in [
            "run.exe",
            "script.sh",
            "shortcut.url",
            "file.desktop",
            "report.pdf.exe",
        ] {
            assert!(!open_allowed(Path::new(name)));
        }
        for name in ["picture.PNG", "report.html", "report.PDF"] {
            assert!(open_allowed(Path::new(name)));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
