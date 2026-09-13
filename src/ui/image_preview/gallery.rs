//! One bounded reader per attachment. Rapid navigation coalesces to the latest
//! preview; directory reads and file reads never run on the UI thread.
use super::*;
use std::sync::mpsc::{self, Receiver, SyncSender};

const ENTRY_LIMIT: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Target {
    id: u64,
    owner: (String, u64, u64, String),
    path: PathBuf,
    discover: bool,
}

pub(super) struct Gallery {
    pub paths: Vec<PathBuf>,
    pub selected: usize,
    pub discovered: bool,
    pub warning: String,
}
impl Gallery {
    pub fn new(path: PathBuf) -> Self {
        Self {
            paths: vec![path],
            selected: 0,
            discovered: false,
            warning: String::new(),
        }
    }
    pub fn path(&self) -> &Path {
        &self.paths[self.selected]
    }
    pub fn step(&mut self, direction: i64) -> bool {
        let len = self.paths.len() as i64;
        let selected = (self.selected as i64 + direction.rem_euclid(len)).rem_euclid(len) as usize;
        let changed = selected != self.selected;
        self.selected = selected;
        changed
    }
}

struct Loaded {
    gallery: Option<Gallery>,
    bytes: io::Result<Vec<u8>>,
}

fn discover(path: &Path) -> Gallery {
    let mut gallery = Gallery::new(path.into());
    gallery.discovered = true;
    let result = (|| -> io::Result<()> {
        let directory = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        for (index, entry) in fs::read_dir(directory)?.take(ENTRY_LIMIT + 1).enumerate() {
            if index == ENTRY_LIMIT {
                gallery.warning = "First 512 directory entries · more images may exist".into();
                break;
            }
            let entry = entry?;
            let sibling = entry.path();
            if image_path(&sibling) && entry.file_type()?.is_file() && sibling != path {
                gallery.paths.push(sibling);
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        gallery.warning = format!(
            "Image list unavailable: {}",
            wire::passive(&error.to_string())
        );
    }
    // Match the explorer's case-insensitive filename ordering. Use the original
    // spelling as a stable tie break when names differ only by case.
    gallery.paths.sort_by_cached_key(|p| {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        (name.to_lowercase(), name)
    });
    gallery.paths.dedup();
    gallery.selected = gallery.paths.iter().position(|p| p == path).unwrap();
    gallery
}

fn read_image(path: &Path) -> io::Result<Vec<u8>> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || !(1..=protocol::IMAGE_LIMIT).contains(&meta.len()) {
        return Err(wire::invalid(
            "Preview requires a regular file no larger than 20 MiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(protocol::IMAGE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > protocol::IMAGE_LIMIT {
        return Err(wire::invalid("Image exceeds 20 MiB"));
    }
    crate::image_preview::dimensions(&bytes)?;
    Ok(bytes)
}

pub(in crate::ui) struct Loader {
    send: SyncSender<Target>,
    receive: Receiver<Loaded>,
    pending: Option<Target>,
}
impl Loader {
    pub fn new() -> Self {
        Self::with_reader(|target| Loaded {
            gallery: target.discover.then(|| discover(&target.path)),
            bytes: read_image(&target.path),
        })
    }
    fn with_reader(reader: impl Fn(&Target) -> Loaded + Send + 'static) -> Self {
        let (send, requests) = mpsc::sync_channel::<Target>(1);
        let (results, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(target) = requests.recv() {
                if results.send(reader(&target)).is_err() {
                    break;
                }
            }
        });
        Self {
            send,
            receive,
            pending: None,
        }
    }
    pub(super) fn request(&mut self, preview: &ImagePreview) {
        if !preview.loading || self.pending.is_some() {
            return;
        }
        let target = Target {
            id: preview.id,
            owner: preview.target.clone(),
            path: preview.gallery.path().into(),
            discover: !preview.gallery.discovered,
        };
        if self.send.try_send(target.clone()).is_ok() {
            self.pending = Some(target);
        }
    }
    pub(super) fn poll(&mut self, preview: Option<&mut ImagePreview>) -> bool {
        let loaded = match self.receive.try_recv() {
            Ok(loaded) => loaded,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => Loaded {
                gallery: None,
                bytes: Err(wire::invalid("Image reader unavailable")),
            },
        };
        let Some(target) = self.pending.take() else {
            return false;
        };
        let Some(preview) = preview.filter(|p| {
            p.id == target.id && p.target == target.owner && p.gallery.path() == target.path
        }) else {
            return false;
        };
        if let Some(gallery) = loaded.gallery {
            preview.gallery = gallery;
        }
        preview.loading = false;
        match loaded.bytes {
            Ok(bytes) => preview.bytes = bytes,
            Err(error) => {
                crate::diagnostics::error("preview-error", &error);
                preview.error = wire::passive(&error.to_string());
                preview.finished = true;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(id: u64) -> ImagePreview {
        ImagePreview {
            id,
            target: ("epoch".into(), 7, 9, "run".into()),
            name: "image.png".into(),
            gallery: Gallery::new("/image.png".into()),
            loading: true,
            pending_steps: 0,
            navigated: false,
            bytes: Vec::new(),
            offset: 0,
            started: false,
            finished: false,
            error: String::new(),
        }
    }

    #[test]
    fn blocked_reader_coalesces_navigation_and_ignores_stale_success_and_errors() {
        for fail in [false, true] {
            let (started, start) = mpsc::sync_channel(1);
            let (release, wait) = mpsc::sync_channel(1);
            let mut loader = Loader::with_reader(move |_| {
                started.send(()).unwrap();
                wait.recv().unwrap();
                Loaded {
                    gallery: None,
                    bytes: if fail {
                        Err(wire::invalid("stale failure"))
                    } else {
                        Ok(vec![1, 2, 3])
                    },
                }
            });
            let mut current = preview(1);
            loader.request(&current);
            start.recv_timeout(Duration::from_secs(1)).unwrap();
            for id in 2..102 {
                current.id = id;
                loader.request(&current);
                assert!(!loader.poll(Some(&mut current)));
            }
            assert!(start.try_recv().is_err());
            release.send(()).unwrap();
            let end = Instant::now() + Duration::from_secs(1);
            while loader.pending.is_some() {
                assert!(!loader.poll(Some(&mut current)));
                assert!(Instant::now() < end);
                std::thread::yield_now();
            }
            assert!(current.loading && current.bytes.is_empty() && current.error.is_empty());
            loader.request(&current);
            start.recv_timeout(Duration::from_secs(1)).unwrap();
            release.send(()).unwrap();
            while !loader.poll(Some(&mut current)) {
                assert!(Instant::now() < end);
                std::thread::yield_now();
            }
            assert!(!current.loading);
            assert_eq!(!current.error.is_empty(), fail);
            assert_eq!(!current.bytes.is_empty(), !fail);
        }
    }

    #[test]
    fn reader_result_requires_every_owner_field_and_the_selected_path() {
        for change in 0..5 {
            let mut loader = Loader::with_reader(|_| Loaded {
                gallery: None,
                bytes: Ok(vec![1]),
            });
            let mut current = preview(1);
            loader.request(&current);
            match change {
                0 => current.target.0 = "new epoch".into(),
                1 => current.target.1 += 1,
                2 => current.target.2 += 1,
                3 => current.target.3 = "new run".into(),
                _ => current.gallery.paths[0] = "/another.png".into(),
            }
            let end = Instant::now() + Duration::from_secs(1);
            while loader.pending.is_some() {
                assert!(!loader.poll(Some(&mut current)));
                assert!(Instant::now() < end);
                std::thread::yield_now();
            }
            assert!(current.loading && current.bytes.is_empty());
        }
    }
}
