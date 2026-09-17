//! Explicit, per-user package installation. Package files and running processes
//! have separate identities; installation never starts a supervisor or a shell.
mod activation;
mod channel;
mod cli;
pub mod discovery;
pub use channel::{
    CAPABILITY as DEFAULT_CHANNEL_CAPABILITY, CAPABILITY_ARGS as DEFAULT_CHANNEL_ARGS,
};
mod manifest;
mod ownership;
mod package;
mod plan;
mod store;

pub use activation::ensure_update_endpoint;
pub use activation::{Activation, RuntimeIdentity, activate, runtime_identity};
pub use cli::command;
pub(crate) use ownership::{ManagerUpgrade, current_manager_upgrade, update_manager_guard};
pub use package::{
    BuildMetadata, Compatibility, MAX_PAYLOAD, Manifest, PackageSource, ProtocolCompatibility,
    SourceReceipt, VersionRange, download, inspect_binary, package, sha256, source_digest,
};
pub(crate) use package::{available_release, background_release};
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
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    collect(child, maximum, Instant::now() + timeout)
}

/// A concurrent fork can retain a CLOEXEC writer until that child execs/exits.
/// Only metadata inspection retries this pre-exec error; a started program is
/// never replayed, and spawning consumes the same deadline as output collection.
fn run_inspection(command: &mut Command, maximum: usize, timeout: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = retry_busy_spawn(|| command.spawn(), deadline)?;
    collect(child, maximum, deadline)
}

fn retry_busy_spawn<T>(
    mut spawn: impl FnMut() -> io::Result<T>,
    deadline: Instant,
) -> io::Result<T> {
    loop {
        match spawn() {
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(error);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(5)));
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
            result => return result,
        }
    }
}

fn collect(
    mut child: std::process::Child,
    maximum: usize,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut output = Vec::new();
    let mut errors = Vec::new();
    let result = (|| {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "installer subprocess timed out",
            ));
        }
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

#[cfg(test)]
mod inspection_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new(output: &str, exit: u8) -> Self {
            let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!(
                    "inspect-busy-{}",
                    &crate::os::nonce().unwrap()[..12]
                ));
            private_dir(&root).unwrap();
            let body = format!(
                "#!/bin/sh\nprintf x >> \"$0.marker\"\nprintf '%s\\n' '{}'\nexit {exit}\n",
                output.replace('\'', "'\\''")
            );
            fs::write(root.join("binary"), body).unwrap();
            fs::set_permissions(root.join("binary"), fs::Permissions::from_mode(0o700)).unwrap();
            Self(root)
        }
        fn command(&self) -> Command {
            let mut command = Command::new(self.0.join("binary"));
            command.arg("--build-info").stdin(Stdio::null());
            command
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if std::thread::panicking() {
                eprintln!("Retained inspection fixture {}", self.0.display());
            } else {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }

    #[test]
    fn only_busy_spawn_errors_retry_and_expired_busy_remains_bounded() {
        let mut attempts = 0;
        let value = retry_busy_spawn(
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(io::Error::from_raw_os_error(libc::ETXTBSY))
                } else {
                    Ok(42)
                }
            },
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!((value, attempts), (42, 2));
        for code in [
            libc::EACCES,
            libc::ENOENT,
            libc::ENOEXEC,
            libc::EIO,
            libc::ETXTBSY,
        ] {
            let mut attempts = 0;
            let deadline = if code == libc::ETXTBSY {
                Instant::now()
            } else {
                Instant::now() + Duration::from_secs(1)
            };
            let result: io::Result<()> = retry_busy_spawn(
                || {
                    attempts += 1;
                    Err(io::Error::from_raw_os_error(code))
                },
                deadline,
            );
            assert_eq!(result.unwrap_err().raw_os_error(), Some(code));
            assert_eq!(attempts, 1);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn persistent_real_writer_is_bounded_and_never_executes() {
        let f = Fixture::new(crate::build_info::json(), 0);
        let _writer = OpenOptions::new()
            .write(true)
            .open(f.0.join("binary"))
            .unwrap();
        let start = Instant::now();
        let error = run_inspection(
            &mut f.command(),
            MAX_JSON as usize,
            Duration::from_millis(30),
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ETXTBSY));
        assert!(start.elapsed() < Duration::from_secs(3));
        assert!(!f.0.join("binary.marker").exists());
        // Other installer commands retain their immediate spawn-error behavior.
        assert_eq!(
            run(&mut f.command(), MAX_JSON as usize, Duration::from_secs(1))
                .unwrap_err()
                .raw_os_error(),
            Some(libc::ETXTBSY)
        );
        assert!(!f.0.join("binary.marker").exists());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn closing_real_writer_after_observed_busy_allows_one_execution() {
        let f = Fixture::new(crate::build_info::json(), 0);
        let mut writer = Some(
            OpenOptions::new()
                .write(true)
                .open(f.0.join("binary"))
                .unwrap(),
        );
        let mut command = f.command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut attempts = 0;
        let child = retry_busy_spawn(
            || {
                attempts += 1;
                let result = command.spawn();
                if attempts == 1 {
                    assert_eq!(
                        result.as_ref().unwrap_err().raw_os_error(),
                        Some(libc::ETXTBSY)
                    );
                    assert!(!f.0.join("binary.marker").exists());
                    drop(writer.take());
                }
                result
            },
            deadline,
        )
        .unwrap();
        let output = collect(child, MAX_JSON as usize, deadline).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output).unwrap(),
            serde_json::from_str::<serde_json::Value>(crate::build_info::json()).unwrap()
        );
        assert!(attempts >= 2);
        assert_eq!(fs::read(f.0.join("binary.marker")).unwrap(), b"x");
    }

    #[test]
    fn started_failure_or_invalid_metadata_is_never_replayed() {
        for (output, exit) in [(crate::build_info::json(), 4), ("not json", 0)] {
            let f = Fixture::new(output, exit);
            assert!(inspect_binary(&f.0.join("binary")).is_err());
            assert_eq!(fs::read(f.0.join("binary.marker")).unwrap(), b"x");
        }
        let f = Fixture::new("output exceeds the bound", 0);
        assert!(run_inspection(&mut f.command(), 1, Duration::from_secs(3)).is_err());
        assert_eq!(fs::read(f.0.join("binary.marker")).unwrap(), b"x");
    }

    #[test]
    fn output_collection_does_not_reset_an_exhausted_spawn_deadline() {
        let mut command = Command::new("/bin/sh");
        let child = command
            .args(["-c", "while :; do :; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let error = collect(child, 64, Instant::now()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }
}
