//! Bounded regular-file streams. Uploads publish atomically without replacement;
//! cancelling, timing out or dropping an incomplete stream removes only its part.
use crate::remote_services::{self as service, CHUNK, FILE_LIMIT};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

fn options() -> OpenOptions {
    let options = OpenOptions::new();
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = options;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        options
    };
    options
}
fn parent(path: &Path) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return Err(service::invalid("transfer path must be absolute"));
    }
    let dir = path
        .parent()
        .ok_or_else(|| service::invalid("transfer path has no parent"))?;
    for part in dir.ancestors() {
        let meta = fs::symlink_metadata(part)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(service::invalid("transfer directories cannot be symlinks"));
        }
    }
    fs::canonicalize(dir)
}
#[derive(Clone, PartialEq, Eq)]
struct Identity {
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    inode: u64,
}
impl Identity {
    fn of(meta: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            length: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(unix)]
            dev: meta.dev(),
            #[cfg(unix)]
            inode: meta.ino(),
        }
    }
}
pub struct Source {
    file: File,
    path: PathBuf,
    identity: Identity,
    pub name: String,
    pub size: u64,
    offset: u64,
}
impl Source {
    pub fn open(path: &Path) -> io::Result<Self> {
        let dir = parent(path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| service::invalid("file name must be UTF-8"))?
            .to_owned();
        service::name(&name)?;
        let path = dir.join(&name);
        let before = fs::symlink_metadata(&path)?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err(service::invalid(
                "transfer requires a regular file, not a symlink",
            ));
        }
        let file = options().read(true).open(&path)?;
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.len() > FILE_LIMIT
            || Identity::of(&meta) != Identity::of(&before)
        {
            return Err(service::invalid("file changed or exceeds 128 MiB"));
        }
        Ok(Self {
            file,
            path,
            identity: Identity::of(&meta),
            name,
            size: meta.len(),
            offset: 0,
        })
    }
    pub fn read(&mut self, offset: u64) -> io::Result<Vec<u8>> {
        if offset != self.offset {
            return Err(service::invalid("file read offset changed"));
        }
        self.validate()?;
        let count = (self.size - self.offset).min(CHUNK as u64) as usize;
        let mut bytes = vec![0; count];
        self.file.read_exact(&mut bytes)?;
        self.offset += count as u64;
        Ok(bytes)
    }
    fn validate(&self) -> io::Result<()> {
        let current = fs::symlink_metadata(&self.path)?;
        if current.file_type().is_symlink()
            || Identity::of(&current) != self.identity
            || Identity::of(&self.file.metadata()?) != self.identity
        {
            return Err(service::invalid(
                "file changed during transfer; select it again",
            ));
        }
        Ok(())
    }
    pub fn finish(&self) -> io::Result<()> {
        self.validate()?;
        if self.offset != self.size {
            return Err(service::invalid("download incomplete"));
        }
        Ok(())
    }
}
pub struct Sink {
    file: Option<File>,
    partial: PathBuf,
    destination: PathBuf,
    directory: PathBuf,
    directory_identity: Identity,
    expected: u64,
    received: u64,
    prepared: bool,
}
impl Sink {
    pub fn new(path: &Path, expected: u64, nonce: &str) -> io::Result<Self> {
        if expected > FILE_LIMIT
            || nonce.is_empty()
            || nonce.len() > 64
            || !nonce.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(service::invalid("invalid upload size or identity"));
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| service::invalid("file name must be UTF-8"))?;
        service::name(name)?;
        let directory = parent(path)?;
        let destination = directory.join(name);
        match fs::symlink_metadata(&destination) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "destination exists; choose another file name",
                ));
            }
        }
        // Directory identity excludes mtime/length: writing our own part changes those.
        let mut directory_identity = Identity::of(&fs::metadata(&directory)?);
        directory_identity.length = 0;
        directory_identity.modified = None;
        let partial = directory.join(format!(".flere-transfer-{nonce}.part"));
        let file = options().write(true).create_new(true).open(&partial)?;
        Ok(Self {
            file: Some(file),
            partial,
            destination,
            directory,
            directory_identity,
            expected,
            received: 0,
            prepared: false,
        })
    }
    pub fn write(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        if self.file.is_none()
            || self.prepared
            || offset != self.received
            || bytes.is_empty()
            || bytes.len() > CHUNK
            || self.received.saturating_add(bytes.len() as u64) > self.expected
        {
            return Err(service::invalid(
                "upload chunk is out of order or exceeds declared size",
            ));
        }
        self.file.as_mut().unwrap().write_all(bytes)?;
        self.received += bytes.len() as u64;
        Ok(())
    }
    pub fn prepare(&mut self) -> io::Result<()> {
        if self.received != self.expected {
            return Err(service::invalid("upload incomplete"));
        }
        self.file.as_mut().unwrap().sync_all()?;
        self.validate_staging()?;
        self.prepared = true;
        Ok(())
    }
    fn validate_staging(&self) -> io::Result<()> {
        let meta = fs::symlink_metadata(&self.directory)?;
        let mut identity = Identity::of(&meta);
        identity.length = 0;
        identity.modified = None;
        if meta.file_type().is_symlink() || identity != self.directory_identity {
            return Err(service::invalid(
                "destination directory changed during upload",
            ));
        }
        let partial_meta = fs::symlink_metadata(&self.partial)?;
        if partial_meta.file_type().is_symlink()
            || Identity::of(&partial_meta) != Identity::of(&self.file.as_ref().unwrap().metadata()?)
        {
            return Err(service::invalid("upload staging file changed"));
        }
        Ok(())
    }
    pub fn publish(mut self) -> io::Result<PathBuf> {
        if !self.prepared {
            return Err(service::invalid("upload was not prepared for publication"));
        }
        self.validate_staging()?;
        self.file.take();
        // hard_link is a no-replace atomic publication on supported filesystems.
        // A filesystem without hard links rejects safely, leaving the existing file intact.
        fs::hard_link(&self.partial, &self.destination)?;
        fs::remove_file(&self.partial)?;
        Ok(self.destination.clone())
    }
    pub fn finish(mut self) -> io::Result<PathBuf> {
        self.prepare()?;
        self.publish()
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.partial);
    }
}
