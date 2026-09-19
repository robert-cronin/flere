//! Repository-owned image inspection stays off the UI loop. No network or author lookup.
use super::*;
use std::{collections::HashSet, os::unix::fs::OpenOptionsExt, sync::mpsc};
const PATHS: &[&str] = &[
    ".flere/icon.png",
    ".flere/icon.jpg",
    "logo.png",
    "docs/logo.png",
    "assets/logo.png",
    "assets/icon.png",
    "frontend/public/logo.png",
    "public/logo.png",
    "website/static/img/logo.png",
    "docs/images/logo.png",
    "frontend/public/favicon.png",
    "public/favicon.png",
    "favicon.png",
];
fn inspect(cwd: &Path) -> Option<Vec<u8>> {
    let bytes = crate::git::output(cwd, &["rev-parse", "--show-toplevel"]).ok()?;
    let root = PathBuf::from(String::from_utf8(bytes).ok()?.trim_end());
    let root = root.canonicalize().ok()?;
    PATHS.iter().find_map(|name| {
        let path = root.join(name);
        if !path.canonicalize().ok()?.starts_with(&root) {
            return None;
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .ok()?;
        let meta = file.metadata().ok()?;
        if !meta.is_file() || meta.len() > crate::avatar::BYTE_LIMIT as u64 {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(crate::avatar::BYTE_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        crate::avatar::validate(&bytes).ok()?;
        Some(bytes)
    })
}
pub(super) struct Projects {
    values: HashMap<String, Option<String>>,
    pending: HashSet<String>,
    send: mpsc::SyncSender<String>,
    receive: mpsc::Receiver<(String, Option<Vec<u8>>)>,
    images: Vec<(String, Vec<u8>)>,
    sent: HashSet<String>,
}
impl Projects {
    pub fn new() -> Self {
        let (send, requests) = mpsc::sync_channel::<String>(128);
        let (results, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(cwd) = requests.recv() {
                let image = inspect(Path::new(&cwd));
                if results.send((cwd, image)).is_err() {
                    break;
                }
            }
        });
        Self {
            values: HashMap::new(),
            pending: HashSet::new(),
            send,
            receive,
            images: Vec::new(),
            sent: HashSet::new(),
        }
    }
    pub fn get(&self, w: &crate::model::WorkspaceView) -> Option<String> {
        self.values.get(&w.cwd).cloned().flatten()
    }
    pub fn bytes(&self, key: &str) -> Option<&[u8]> {
        self.images
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, b)| b.as_slice())
    }
    pub fn columns(&self, key: &str, cell: (usize, usize), max: usize) -> u16 {
        self.images
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, bytes)| crate::image_preview::dimensions(bytes).ok())
            .map_or(2, |source| crate::avatar::columns(source, cell, max))
    }
    pub fn poll(&mut self, snapshot: &Snapshot) -> bool {
        let mut changed = false;
        while let Ok((cwd, image)) = self.receive.try_recv() {
            self.pending.remove(&cwd);
            let key = image.and_then(|bytes| {
                if let Some((key, _)) = self.images.iter().find(|(_, b)| *b == bytes) {
                    return Some(key.clone());
                }
                if self.images.len() >= crate::avatar::LIMIT {
                    return None;
                }
                let key = format!("repo-{}", self.images.len() + 1);
                self.images.push((key.clone(), bytes));
                Some(key)
            });
            self.values.insert(cwd, key);
            changed = true;
        }
        for w in &snapshot.workspaces {
            if self.values.contains_key(&w.cwd)
                || self.pending.contains(&w.cwd)
                || self.values.len() + self.pending.len() >= 128
            {
                continue;
            }
            if self.send.try_send(w.cwd.clone()).is_ok() {
                self.pending.insert(w.cwd.clone());
            }
        }
        changed
    }
    /// Transfer at most one visible asset per loop, once per attachment. The next
    /// text/layout frame owns placement; image bytes never enter the native PTY.
    pub fn emit(&mut self, badges: &[crate::avatar::Badge]) -> io::Result<()> {
        use crate::remote_protocol as p;
        let Some((key, bytes)) = self
            .images
            .iter()
            .find(|(key, _)| !self.sent.contains(key) && badges.iter().any(|b| &b.key == key))
        else {
            return Ok(());
        };
        for (i, bytes_part) in bytes.chunks(p::CHUNK).enumerate() {
            p::Packet::new(
                p::AVATAR_IMAGE,
                0,
                crate::avatar::chunk(key, bytes.len(), i * p::CHUNK, bytes_part),
            )
            .write(&mut output::writer())?;
        }
        self.sent.insert(key.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_icons_stay_inside_repository_and_obey_image_bounds() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("project-icon-{}", crate::os::nonce().unwrap()));
        fs::create_dir_all(root.join("repo/.flere")).unwrap();
        let repo = root.join("repo");
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(442u32.to_be_bytes());
        png.extend(491u32.to_be_bytes());
        png.resize(33, 0);
        let icon = repo.join(".flere/icon.png");
        fs::write(&icon, &png).unwrap();
        assert_eq!(inspect(&repo.join(".flere")), Some(png.clone()));
        fs::remove_file(&icon).unwrap();
        fs::write(root.join("outside.png"), &png).unwrap();
        std::os::unix::fs::symlink(root.join("outside.png"), &icon).unwrap();
        assert!(inspect(&repo).is_none());
        fs::remove_file(&icon).unwrap();
        fs::write(&icon, vec![0; crate::avatar::BYTE_LIMIT + 1]).unwrap();
        assert!(inspect(&repo).is_none());
        png[16..20].copy_from_slice(&1025u32.to_be_bytes());
        fs::write(&icon, png).unwrap();
        assert!(inspect(&repo).is_none());
        fs::remove_dir_all(root).unwrap();
    }
}
