//! Local retained screenshots. Received pixels never become terminal escapes or
//! a remote file path; only a fully received image can reach the local clipboard.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
const CACHE_LIMIT: u64 = 256 * 1024 * 1024;
fn root() -> io::Result<PathBuf> {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|p| p.join("Flere/screenshots"));
    #[cfg(unix)]
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map(|p| p.join("flere-connect/screenshots"));
    let root = root.ok_or_else(|| io::Error::other("local user cache directory is unavailable"))?;
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
        return Err(io::Error::other("invalid screenshot cache directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != crate::os::uid() || meta.mode() & 0o077 != 0 {
            return Err(io::Error::other(
                "screenshot cache must be owned and private",
            ));
        }
    }
    fs::canonicalize(root)
}
fn options() -> OpenOptions {
    let opts = OpenOptions::new();
    #[cfg(unix)]
    let opts = {
        let mut opts = opts;
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        opts
    };
    opts
}
pub fn read_owned(path: &Path) -> io::Result<Vec<u8>> {
    use std::io::Read;
    if path.parent() != Some(root()?.as_path())
        || !path.file_name().and_then(|v| v.to_str()).is_some_and(|s| {
            s.len() == 36 && s.ends_with(".png") && s[..32].bytes().all(|c| c.is_ascii_hexdigit())
        })
    {
        return Err(io::Error::other(
            "image must be a retained local screenshot",
        ));
    }
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(io::Error::other("invalid screenshot link"));
    }
    let mut file = options().read(true).open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > crate::protocol::IMAGE_LIMIT {
        return Err(io::Error::other("invalid screenshot file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != crate::os::uid() || meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
            return Err(io::Error::other("invalid private screenshot file"));
        }
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(crate::protocol::IMAGE_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > crate::protocol::IMAGE_LIMIT || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(io::Error::other("invalid screenshot PNG"));
    }
    crate::image_preview::dimensions(&bytes)?;
    Ok(bytes)
}
pub fn copy(bytes: &[u8]) -> io::Result<PathBuf> {
    save_and_copy(bytes, &root()?, crate::os::copy_png)
}
fn save_and_copy(
    bytes: &[u8],
    root: &Path,
    copy: fn(&[u8], &Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    if bytes.len() as u64 > crate::protocol::IMAGE_LIMIT || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err(io::Error::other("invalid screenshot PNG"));
    }
    crate::image_preview::dimensions(bytes)?;
    let lock = options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(".lock"))?;
    lock.try_lock().map_err(io::Error::other)?;
    let total = fs::read_dir(root)?.try_fold(0u64, |total, entry| {
        Ok::<_, io::Error>(total.saturating_add(fs::symlink_metadata(entry?.path())?.len()))
    })?;
    if total + bytes.len() as u64 > CACHE_LIMIT {
        return Err(io::Error::other(
            "screenshot cache full; retained draft images need review before cleanup",
        ));
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos()
        ^ ((std::process::id() as u128) << 64)
        ^ COUNTER.fetch_add(1, Ordering::Relaxed) as u128;
    let path = root.join(format!("{stamp:032x}.png"));
    let partial = path.with_extension("part");
    let mut file = options().write(true).create_new(true).open(&partial)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Publish atomically without replacing a retained draft image.
        fs::hard_link(&partial, &path)?;
        Ok::<_, io::Error>(())
    })();
    let _ = fs::remove_file(&partial);
    result?;
    if let Err(error) = copy(bytes, &path) {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_images_are_retained_and_failed_clipboards_leave_no_image() {
        let cache = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .unwrap();
        let root = PathBuf::from(cache)
            .join(".cache/flere/tests")
            .join(format!(
                "companion-screenshot-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../tests/fixtures/local-image.png");
        let path = save_and_copy(bytes, &root, |bytes, path| {
            assert_eq!(fs::read(path)?, bytes);
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                assert_eq!(fs::metadata(path)?.mode() & 0o777, 0o600);
                assert_eq!(fs::metadata(path)?.nlink(), 1);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(
            save_and_copy(bytes, &root, |_, _| Err(io::Error::other("clipboard busy"))).is_err()
        );
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2); // Retained PNG + cache lock.
        assert!(save_and_copy(b"not an image", &root, |_, _| unreachable!()).is_err());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        fs::remove_dir_all(root).unwrap();
    }
}
