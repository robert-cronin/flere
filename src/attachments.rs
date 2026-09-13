//! Private streamed attachments. Incomplete files are removed; retained drafts are never evicted.
use crate::{os, remote_protocol::IMAGE_LIMIT, wire::invalid};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::fd::AsRawFd,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
const CACHE_LIMIT: u64 = 256 * 1024 * 1024;
pub fn directory() -> io::Result<PathBuf> {
    private_directory("attachments")
}
fn private_directory(name: &str) -> io::Result<PathBuf> {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or_else(|| invalid("HOME or XDG_CACHE_HOME is required"))?;
    let dir = root.join("flere").join(name);
    if !dir.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let m = fs::symlink_metadata(&dir)?;
    if !m.is_dir() || m.file_type().is_symlink() || m.uid() != os::uid() || m.mode() & 0o077 != 0 {
        return Err(invalid(
            "attachment cache must be an owned private directory",
        ));
    }
    fs::canonicalize(dir)
}

/// A private destination reservation for the ordinary bounded file-transfer Sink.
/// The exclusive lock covers the reservation and stream, so concurrent workers
/// cannot spend the same remaining cache capacity. Completed paths are retained.
pub(crate) struct FileDrop {
    _lock: File,
    directory: PathBuf,
    pub(crate) destination: PathBuf,
}
const DROP_CACHE_LIMIT: u64 = 512 * 1024 * 1024;
const DROP_COUNT_LIMIT: usize = 256;
impl FileDrop {
    pub(crate) fn new(name: &str, expected: u64, nonce: &str) -> io::Result<Self> {
        crate::remote_services::name(name)?;
        if expected > crate::remote_services::FILE_LIMIT
            || nonce.len() != 32
            || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("invalid attachment size or identity"));
        }
        let root = private_directory("chat-attachments")?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(root.join(".lock"))?;
        let m = lock.metadata()?;
        if !m.is_file() || m.uid() != os::uid() || m.mode() & 0o077 != 0 || m.nlink() != 1 {
            return Err(invalid("invalid attachment reservation lock"));
        }
        os::lock(lock.as_raw_fd())
            .map_err(|_| invalid("another chat attachment upload is in progress"))?;
        let mut total = 0u64;
        let mut count = 0usize;
        for entry in fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name() == ".lock" {
                continue;
            }
            count += 1;
            if count >= DROP_COUNT_LIMIT {
                return Err(invalid(
                    "chat attachment count limit reached; retained files need review before cleanup",
                ));
            }
            let m = fs::symlink_metadata(entry.path())?;
            if !m.is_dir()
                || m.file_type().is_symlink()
                || m.uid() != os::uid()
                || m.mode() & 0o077 != 0
            {
                return Err(invalid("invalid private chat attachment directory"));
            }
            // Each nonce owns one completed file or its incomplete Sink staging file.
            // Never clean other transfers or evict paths possibly referenced by drafts.
            for (index, file) in fs::read_dir(entry.path())?.enumerate() {
                if index > 1 {
                    return Err(invalid("unexpected chat attachment directory contents"));
                }
                let m = fs::symlink_metadata(file?.path())?;
                if !m.is_file()
                    || m.file_type().is_symlink()
                    || m.uid() != os::uid()
                    || m.mode() & 0o077 != 0
                {
                    return Err(invalid("invalid private chat attachment file"));
                }
                total = total.saturating_add(m.len());
                if total.saturating_add(expected) > DROP_CACHE_LIMIT {
                    return Err(invalid(
                        "chat attachment cache full; retained files need review before cleanup",
                    ));
                }
            }
        }
        let directory = root.join(nonce);
        let destination = directory.join(name);
        if destination
            .to_str()
            .is_none_or(|s| s.len() > 4096 || s.chars().any(char::is_control))
        {
            return Err(invalid(
                "chat attachment path must be bounded printable UTF-8",
            ));
        }
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
        Ok(Self {
            _lock: lock,
            destination,
            directory,
        })
    }
}
impl Drop for FileDrop {
    fn drop(&mut self) {
        // Sink drops its partial first. An empty reservation can disappear, but
        // any published file survives even when its paste outcome is uncertain.
        let _ = fs::remove_dir(&self.directory);
    }
}
fn valid_name(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
        s.len() == 36 && s.ends_with(".png") && s[..32].bytes().all(|c| c.is_ascii_hexdigit())
    })
}
pub fn validate(path: &Path) -> io::Result<()> {
    if path.parent() != Some(directory()?.as_path()) || !valid_name(path) {
        return Err(invalid("image must be a completed Flere attachment"));
    }
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file()
        || m.uid() != os::uid()
        || m.mode() & 0o077 != 0
        || m.len() > IMAGE_LIMIT
        || m.nlink() != 1
    {
        return Err(invalid("invalid private image attachment"));
    }
    let mut header = [0; 33];
    f.read_exact(&mut header)?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n"
        || header[8..12] != 13u32.to_be_bytes()
        || &header[12..16] != b"IHDR"
    {
        return Err(invalid("clipboard image must be PNG"));
    }
    let w = u32::from_be_bytes(header[16..20].try_into().unwrap()) as u64;
    let h = u32::from_be_bytes(header[20..24].try_into().unwrap()) as u64;
    if w == 0 || h == 0 || w.saturating_mul(h) > 20_000_000 {
        return Err(invalid("clipboard image exceeds pixel limit"));
    }
    Ok(())
}
pub struct Upload {
    file: File,
    _lock: File,
    published: bool,
    partial: PathBuf,
    destination: PathBuf,
    expected: u64,
    received: u64,
    retained: bool,
}
impl Upload {
    pub fn new(expected: u64) -> io::Result<Self> {
        if !(33..=IMAGE_LIMIT).contains(&expected) {
            return Err(invalid("clipboard image must be 33 bytes–20 MiB"));
        }
        let dir = directory()?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(dir.join(".lock"))?;
        let m = lock.metadata()?;
        if !m.is_file() || m.uid() != os::uid() || m.mode() & 0o077 != 0 {
            return Err(invalid("invalid attachment lock"));
        }
        os::lock(lock.as_raw_fd()).map_err(|_| invalid("another image upload is in progress"))?;
        let mut total = 0u64;
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let m = fs::symlink_metadata(entry.path())?;
            // The exclusive cache lock proves no live upload owns these files.
            // Remove only private nonce partials; keep completed draft images.
            let name = entry.file_name();
            if name.to_str().is_some_and(|s| {
                s.len() == 37
                    && s.ends_with(".part")
                    && s[..32].bytes().all(|c| c.is_ascii_hexdigit())
            }) && m.is_file()
                && m.uid() == os::uid()
                && m.mode() & 0o077 == 0
                && m.nlink() == 1
            {
                fs::remove_file(entry.path())?;
                continue;
            }
            if m.is_file() {
                total = total.saturating_add(m.len());
            }
        }
        if total.saturating_add(expected) > CACHE_LIMIT {
            return Err(invalid(
                "attachment cache full; retained attachments need review before cleanup",
            ));
        }
        let name = os::nonce()?;
        let partial = dir.join(format!("{name}.part"));
        let destination = dir.join(format!("{name}.png"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&partial)?;
        let upload = Self {
            file,
            _lock: lock,
            published: false,
            partial,
            destination,
            expected,
            received: 0,
            retained: false,
        };
        upload.file.set_len(expected)?;
        Ok(upload)
    }
    pub fn append(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        if offset != self.received
            || bytes.is_empty()
            || self.received.saturating_add(bytes.len() as u64) > self.expected
        {
            return Err(invalid("image chunk offset/length mismatch"));
        }
        self.file.write_all(bytes)?;
        self.received += bytes.len() as u64;
        Ok(())
    }
    pub fn received(&self) -> u64 {
        self.received
    }
    pub fn finish(mut self) -> io::Result<PathBuf> {
        if self.received != self.expected {
            return Err(invalid("incomplete clipboard image"));
        }
        self.file.sync_all()?;
        // create_new reserved a nonce; link fails rather than replacing any completed image.
        fs::hard_link(&self.partial, &self.destination)?;
        self.published = true;
        fs::remove_file(&self.partial)?;
        if let Err(error) = validate(&self.destination) {
            let _ = fs::remove_file(&self.destination);
            return Err(error);
        }
        File::open(self.destination.parent().unwrap())?.sync_all()?;
        self.retained = true;
        Ok(self.destination.clone())
    }
}
impl Drop for Upload {
    fn drop(&mut self) {
        if !self.retained {
            let _ = fs::remove_file(&self.partial);
            if self.published {
                let _ = fs::remove_file(&self.destination);
            }
        }
    }
}
