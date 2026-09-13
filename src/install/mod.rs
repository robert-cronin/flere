//! Explicit, per-user package installation. Package files and running processes
//! have separate identities; installation never starts a supervisor or a shell.
mod activation;
mod cli;
mod manifest;
mod package;
mod plan;
mod store;

pub use activation::ensure_update_endpoint;
pub use activation::{Activation, RuntimeIdentity, activate, runtime_identity};
pub use cli::command;
pub use package::{
    BuildMetadata, Compatibility, MAX_PAYLOAD, Manifest, PackageSource, ProtocolCompatibility,
    SourceReceipt, VersionRange, download, inspect_binary, package, sha256, source_digest,
};
pub use store::{InstallReceipt, InstalledPackage, StagedPackage, Store};

use crate::wire::invalid;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) const MAX_JSON: u64 = 64 * 1024;
pub const MAX_RECEIPT: u64 = 8 * 1024 * 1024;

pub(crate) fn regular(path: &Path, maximum: u64) -> io::Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > maximum {
        return Err(invalid(
            "package file is not regular or exceeds its size bound",
        ));
    }
    Ok(file)
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    read_bounded_json(path, MAX_JSON)
}

pub(crate) fn read_bounded_json<T: serde::de::DeserializeOwned>(
    path: &Path,
    maximum: u64,
) -> io::Result<T> {
    let mut bytes = Vec::new();
    regular(path, maximum)?
        .take(maximum + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(invalid("installation metadata exceeds bound"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir()
                || metadata.uid() != crate::os::uid()
                || metadata.permissions().mode() & 0o077 != 0
            {
                return Err(invalid(
                    "installation store must be a private, user-owned directory",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

pub(crate) fn sync_dir(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

pub(crate) fn atomic_json(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_RECEIPT {
        return Err(invalid("installation metadata exceeds bound"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("metadata path needs parent"))?;
    let temporary = parent.join(format!(".receipt-{}", crate::os::nonce()?));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        sync_dir(parent)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// Drain both pipes with a byte/time bound. An invalid candidate cannot deadlock
/// installation by filling stderr, inheriting stdin, or leaving a pipe open.
pub(crate) fn run(command: &mut Command, maximum: usize, timeout: Duration) -> io::Result<Vec<u8>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let deadline = Instant::now() + timeout;
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let result = (|| {
        crate::os::nonblock(stdout.as_raw_fd())?;
        crate::os::nonblock(stderr.as_raw_fd())?;
        loop {
            let drain = |reader: &mut dyn Read,
                         target: &mut Vec<u8>,
                         limit: usize|
             -> io::Result<()> {
                let mut bytes = [0; 16384];
                loop {
                    match reader.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            if target.len() + n > limit {
                                return Err(invalid("installer subprocess output exceeds bound"));
                            }
                            target.extend_from_slice(&bytes[..n]);
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) => return Err(error),
                    }
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "installer subprocess timed out",
                        ));
                    }
                }
                Ok(())
            };
            drain(&mut stdout, &mut output, maximum)?;
            drain(&mut stderr, &mut errors, 16384)?;
            if let Some(status) = child.try_wait()? {
                // Pipes may still hold the final write after try_wait observed exit.
                drain(&mut stdout, &mut output, maximum)?;
                drain(&mut stderr, &mut errors, 16384)?;
                return if status.success() {
                    Ok(output)
                } else {
                    Err(io::Error::other(format!(
                        "installer subprocess failed: {}",
                        crate::wire::passive(&String::from_utf8_lossy(&errors))
                    )))
                };
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "installer subprocess timed out",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
