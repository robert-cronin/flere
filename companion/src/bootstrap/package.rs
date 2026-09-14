use super::{COMPONENT, MAX_JSON, Names, Request, Source, invalid, transport};
use crate::update::{self, Manifest};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;
pub(super) struct Package {
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub source_url: Option<String>,
}

fn regular(path: &Path, maximum: u64) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(invalid("Bootstrap package cannot be a reparse point"));
        }
    }
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum {
        return Err(invalid(
            "Bootstrap package file is linked, nonregular or too large",
        ));
    }
    Ok(file)
}

fn digest(path: &Path) -> io::Result<String> {
    let file = regular(path, MAX_PAYLOAD)?;
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut c = Command::new("/usr/bin/sha256sum");
        c.stdin(file);
        c
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/shasum");
        c.args(["-a", "256"]).stdin(file);
        c
    };
    #[cfg(windows)]
    let mut command = {
        drop(file);
        let path = path
            .to_str()
            .ok_or_else(|| invalid("Bootstrap package path must be Unicode"))?;
        update::windows_sha256_command(path)
    };
    let bytes = update::run(&mut command, 512, Duration::from_secs(30))?;
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let digest = text.split_whitespace().next().unwrap_or("");
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("System SHA-256 utility returned an invalid digest"));
    }
    Ok(digest.into())
}

fn load(directory: &Path, target: &str) -> io::Result<Manifest> {
    let mut bytes = Vec::new();
    regular(&directory.join("manifest.json"), MAX_JSON as u64)?
        .take(MAX_JSON as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_JSON {
        return Err(invalid("Bootstrap manifest exceeds its byte bound"));
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    validate(&manifest, target)?;
    if regular(&directory.join(COMPONENT), MAX_PAYLOAD)?
        .metadata()?
        .len()
        != manifest.payload.bytes
        || digest(&directory.join(COMPONENT))? != manifest.payload.sha256
    {
        return Err(invalid(
            "Bootstrap package does not match manifest size/SHA-256",
        ));
    }
    Ok(manifest)
}

fn validate(manifest: &Manifest, target: &str) -> io::Result<()> {
    manifest.validate()?;
    if manifest.build.component != COMPONENT || manifest.build.target != target {
        return Err(invalid(
            "Bootstrap package component or remote target differs",
        ));
    }
    Ok(())
}

pub(super) fn https(url: &str) -> io::Result<()> {
    let authority = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    if authority.is_empty()
        || url.len() > 4096
        || !url.is_ascii()
        || url.contains(['@', '?', '#', '\\'])
        || url.bytes().any(|b| b.is_ascii_control() || b == b' ')
        || !url.ends_with("manifest.json")
    {
        return Err(invalid(
            "Choose an explicit HTTPS manifest URL without credentials, query or fragment",
        ));
    }
    Ok(())
}

fn fetch(url: &str, maximum: usize) -> io::Result<Vec<u8>> {
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    update::run(
        Command::new(curl)
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--connect-timeout",
                "10",
                "--max-time",
                "60",
                "--url",
                url,
            ])
            .stdin(Stdio::null()),
        maximum,
        Duration::from_secs(65),
    )
}

fn cache(names: &Names) -> io::Result<PathBuf> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")));
    let base = base
        .filter(|p| p.is_absolute())
        .ok_or_else(|| invalid("An absolute user cache directory is required"))?;
    let cache = base.join(&names.cache_directory).join("bootstrap/packages");
    update::private_dir(&cache)?;
    Ok(cache)
}

fn create(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

pub(super) fn obtain(request: &Request, target: &str) -> io::Result<Package> {
    request.cancellation.check()?;
    if let Source::LocalPackage(directory) = &request.source {
        return Ok(Package {
            directory: directory.clone(),
            manifest: load(directory, target)?,
            source_url: None,
        });
    }
    let url = match &request.source {
        Source::DefaultChannel => format!(
            "https://github.com/robert-cronin/flere/releases/latest/download/flere-{target}.manifest.json"
        ),
        Source::HttpsManifest(url) => url.clone(),
        Source::LocalPackage(_) => unreachable!(),
    };
    https(&url)?;
    let bytes = fetch(&url, MAX_JSON).map_err(|error| invalid(&format!(
        "Cannot download the Flere release manifest from {url}: {error}. The release may be unpublished or this machine offline; choose a published manifest or a local verified package",
    )))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    validate(&manifest, target)?;
    request.cancellation.check()?;
    let cache = cache(&request.names)?;
    let directory = cache.join(manifest.id());
    if directory.try_exists()? {
        if load(&directory, target)? != manifest {
            return Err(invalid("Cached bootstrap package identity differs"));
        }
        return Ok(Package {
            directory,
            manifest,
            source_url: Some(url),
        });
    }
    let temporary = cache.join(format!(".download-{}", update::nonce()?));
    update::private_dir(&temporary)?;
    let result = (|| {
        let asset = manifest
            .payload
            .download_file
            .as_deref()
            .unwrap_or(COMPONENT);
        let asset_url = format!("{}/{}", url.rsplit_once('/').unwrap().0, asset);
        let payload = fetch(&asset_url, manifest.payload.bytes as usize)?;
        request.cancellation.check()?;
        create(&temporary.join(COMPONENT), &payload)?;
        create(&temporary.join("manifest.json"), &bytes)?;
        if load(&temporary, target)? != manifest {
            return Err(invalid("Downloaded bootstrap package changed"));
        }
        fs::rename(&temporary, &directory)?;
        Ok(Package {
            directory,
            manifest,
            source_url: Some(url),
        })
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

pub(super) struct Uploaded<'a> {
    request: &'a Request,
    pub directory: String,
}
impl Drop for Uploaded<'_> {
    fn drop(&mut self) {
        // Once the receiver returned a validated exact directory, clean only its
        // two fixed files on success and every later error. Store packages and
        // receipts remain untouched. A disconnected host can retain this one path.
        let mut cleanup_request = self.request.clone();
        cleanup_request.cancellation = super::Cancellation::default();
        let result =
            transport::script(include_str!("cleanup.sh"), &[&self.directory]).and_then(|command| {
                transport::run_bounded(
                    &cleanup_request,
                    "staging cleanup",
                    &command,
                    None,
                    1024,
                    Duration::from_secs(10),
                )
            });
        if let Err(error) = result {
            eprintln!("Bootstrap staging retained at {}: {error}", self.directory);
        }
    }
}

pub(super) fn upload<'a>(
    request: &'a Request,
    package: &Package,
    cache: &str,
) -> io::Result<Uploaded<'a>> {
    // Revalidate immediately before transfer; never execute a foreign target locally.
    if load(&package.directory, &package.manifest.build.target)? != package.manifest {
        return Err(invalid("Bootstrap package changed before transfer"));
    }
    let manifest = serde_json::to_vec(&package.manifest).map_err(io::Error::other)?;
    if manifest.len() > MAX_JSON {
        return Err(invalid("Bootstrap manifest exceeds its bound"));
    }
    let command = transport::script(
        include_str!("receive.sh"),
        &[
            cache,
            &manifest.len().to_string(),
            &package.manifest.payload.bytes.to_string(),
            &package.manifest.payload.sha256,
        ],
    )?;
    let input =
        io::Cursor::new(manifest).chain(regular(&package.directory.join(COMPONENT), MAX_PAYLOAD)?);
    let output = transport::run(
        request,
        "package transfer",
        &command,
        Some(Box::new(input)),
        16384,
    )?;
    let text = std::str::from_utf8(&output).map_err(io::Error::other)?;
    let mut lines = text.lines();
    if lines.next() != Some("FLERE-UPLOAD-1") {
        return Err(invalid("Invalid bootstrap transfer response"));
    }
    let directory = super::decode_hex(lines.next().unwrap_or(""))?;
    if lines.next() != Some("END")
        || lines.next().is_some()
        || !text.ends_with('\n')
        || !directory
            .strip_prefix(&format!("{cache}/bootstrap."))
            .is_some_and(|tail| tail.len() == 10 && tail.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return Err(invalid(
            "Remote bootstrap directory differs from the selected private cache",
        ));
    }
    Ok(Uploaded { request, directory })
}
