use super::{MAX_JSON, private_dir, read_json, regular, run, run_inspection, sync_dir};
use crate::wire::invalid;
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub use super::manifest::{
    BuildMetadata, Compatibility, MAX_PAYLOAD, Manifest, PackageSource, ProtocolCompatibility,
    SourceReceipt, VersionRange,
};
use super::manifest::{Payload, digest_valid};

impl Manifest {
    pub fn load(directory: &Path) -> io::Result<Self> {
        let manifest: Self = read_json(&directory.join("manifest.json"))?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub fn verify(&self, directory: &Path) -> io::Result<()> {
        self.validate()?;
        let path = directory.join(&self.payload.file_name);
        if regular(&path, MAX_PAYLOAD)?.metadata()?.len() != self.payload.bytes
            || sha256(&path)? != self.payload.sha256
        {
            return Err(invalid(
                "package payload size or SHA-256 does not match manifest",
            ));
        }
        Ok(())
    }
}

/// Use the platform's established SHA-256 implementation; no custom crypto.
pub fn sha256(path: &Path) -> io::Result<String> {
    let input = regular(path, MAX_PAYLOAD)?;
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/shasum");
        c.args(["-a", "256"]);
        c
    };
    #[cfg(target_os = "linux")]
    let mut command = Command::new("/usr/bin/sha256sum");
    let bytes = run(command.stdin(input), 512, Duration::from_secs(30))?;
    let text = std::str::from_utf8(&bytes).map_err(io::Error::other)?;
    let digest = text
        .split_whitespace()
        .next()
        .ok_or_else(|| invalid("SHA-256 utility returned no digest"))?;
    if !digest_valid(digest) {
        return Err(invalid("SHA-256 utility returned an invalid digest"));
    }
    Ok(digest.into())
}

pub fn inspect_binary(binary: &Path) -> io::Result<BuildMetadata> {
    regular(binary, MAX_PAYLOAD)?;
    let bytes = run_inspection(
        Command::new(binary)
            .arg("--build-info")
            .stdin(Stdio::null()),
        MAX_JSON as usize,
        Duration::from_secs(3),
    )?;
    let build: BuildMetadata = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    build.validate()?;
    Ok(build)
}

/// Produce a directory package; fixed basenames remove archive path traversal,
/// links and unbounded extraction from the distribution format entirely.
pub fn package(
    binary: &Path,
    output: &Path,
    source: Option<SourceReceipt>,
) -> io::Result<Manifest> {
    if output.exists() {
        return Err(invalid("package output already exists"));
    }
    let build = inspect_binary(binary)?;
    private_dir(output)?;
    let result = (|| {
        let destination = output.join(&build.component);
        let input = regular(binary, MAX_PAYLOAD)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&destination)?;
        let bytes = io::copy(&mut input.take(MAX_PAYLOAD + 1), &mut file)?;
        file.sync_all()?;
        drop(file);
        let download_file = Some(format!("{}-{}", build.component, build.target));
        let manifest = Manifest {
            schema_version: 1,
            build,
            payload: Payload {
                file_name: destination
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                download_file,
                bytes,
                sha256: sha256(&destination)?,
            },
            source,
        };
        manifest.validate()?;
        if inspect_binary(&destination)? != manifest.build {
            return Err(invalid("candidate changed while packaging"));
        }
        super::atomic_json(&output.join("manifest.json"), &manifest)?;
        sync_dir(output)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

/// Fingerprint all Git-tracked and unignored files, including assets/config/tests.
/// This does not claim to include external patched dependencies or compiler state.
pub fn source_digest(root: &Path, scratch: &Path) -> io::Result<SourceReceipt> {
    private_dir(scratch)?;
    let git = |args: &[&str]| -> io::Result<Vec<u8>> {
        run(
            Command::new("git")
                .args(args)
                .current_dir(root)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_COMMON_DIR")
                .stdin(Stdio::null()),
            2 * 1024 * 1024,
            Duration::from_secs(10),
        )
    };
    let mut paths: Vec<_> = git(&[
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    ])?
    .split(|b| *b == 0)
    .filter(|s| !s.is_empty())
    .map(Vec::from)
    .collect();
    paths.sort();
    paths.dedup();
    let record_path = scratch.join(format!("source-{}", crate::os::nonce()?));
    let result = (|| {
        let mut record = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&record_path)?;
        for bytes in paths {
            use std::os::unix::ffi::OsStrExt;
            let relative = Path::new(std::ffi::OsStr::from_bytes(&bytes));
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return Err(invalid("invalid source path"));
            }
            record.write_all(&(bytes.len() as u64).to_be_bytes())?;
            record.write_all(&bytes)?;
            let path = root.join(relative);
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    record.write_all(&metadata.permissions().mode().to_be_bytes())?;
                    record.write_all(sha256(&path)?.as_bytes())?;
                }
                Ok(metadata) if metadata.is_symlink() => {
                    record.write_all(b"link")?;
                    record.write_all(fs::read_link(&path)?.as_os_str().as_bytes())?;
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    record.write_all(b"deleted")?
                }
                _ => return Err(invalid("source fingerprint contains a non-file entry")),
            }
            record.write_all(&[0])?;
        }
        record.sync_all()?;
        Ok(SourceReceipt {
            git_commit: String::from_utf8(git(&["rev-parse", "HEAD"])?)
                .map_err(io::Error::other)?
                .trim()
                .into(),
            dirty: !git(&["status", "--porcelain", "--untracked-files=normal"])?.is_empty(),
            source_sha256: sha256(&record_path)?,
            checks: Vec::new(),
        })
    })();
    let _ = fs::remove_file(&record_path);
    result
}

pub(crate) fn https(url: &str) -> io::Result<()> {
    if !url.starts_with("https://")
        || url.len() > 4096
        || !url.is_ascii()
        || url.bytes().any(|b| b.is_ascii_control() || b == b' ')
        || url.contains(['@', '#', '?', '\\'])
        || url
            .trim_start_matches("https://")
            .split('/')
            .next()
            .is_none_or(str::is_empty)
    {
        return Err(invalid(
            "release source must be an explicit HTTPS manifest URL without credentials, query or fragment",
        ));
    }
    Ok(())
}

/// The selected HTTPS origin is the trust root. Adjacent SHA-256 detects payload
/// corruption; it is not an independent publisher signature or provenance claim.
pub fn download(manifest_url: &str, output: &Path) -> io::Result<Manifest> {
    https(manifest_url)?;
    if !manifest_url.ends_with("manifest.json") || output.exists() {
        return Err(invalid(
            "download needs a manifest.json URL and a new output directory",
        ));
    }
    download_selected(manifest_url, output, None)
}

fn fetch(url: &str, limit: usize) -> io::Result<Vec<u8>> {
    run(
        Command::new("curl")
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
        limit,
        Duration::from_secs(65),
    )
}

pub(super) fn download_default(
    output: &Path,
    component: &str,
    current: Option<&str>,
) -> io::Result<Manifest> {
    let selection = super::channel::select_update(
        &fetch(super::channel::URL, super::channel::MAX_BYTES)?,
        crate::build_info::TARGET,
        component,
        current,
    )?;
    download_selected(&selection.url, output, Some(&selection))
}

fn download_selected(
    manifest_url: &str,
    output: &Path,
    selection: Option<&super::channel::Selection>,
) -> io::Result<Manifest> {
    download_with(manifest_url, output, selection, fetch)
}

fn download_with(
    manifest_url: &str,
    output: &Path,
    selection: Option<&super::channel::Selection>,
    mut fetch: impl FnMut(&str, usize) -> io::Result<Vec<u8>>,
) -> io::Result<Manifest> {
    if output.exists() {
        return Err(invalid("download output already exists"));
    }
    let bytes = fetch(manifest_url, MAX_JSON as usize)?;
    private_dir(output)?;
    let result = (|| {
        if let Some(selection) = selection {
            let path = output.join(".channel-manifest");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            selection.pin.verify(&bytes, &sha256(&path)?)?;
            fs::remove_file(path)?;
        }
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        manifest.validate()?;
        if let Some(selection) = selection {
            selection.verify_identity(
                &manifest.build.package_version,
                &manifest.build.target,
                &manifest.build.component,
            )?;
        }
        if manifest.build.target != crate::build_info::TARGET {
            return Err(invalid(
                "release package target/version differs from selection",
            ));
        }
        let url = format!(
            "{}/{}",
            manifest_url.rsplit_once('/').unwrap().0,
            manifest
                .payload
                .download_file
                .as_deref()
                .unwrap_or(&manifest.payload.file_name)
        );
        let payload = fetch(&url, manifest.payload.bytes as usize)?;
        let destination = output.join(&manifest.payload.file_name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&destination)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        manifest.verify(output)?;
        super::atomic_json(&output.join("manifest.json"), &manifest)?;
        sync_dir(output)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}

/// Probe only a previously verified immutable package; repeat its hash after the
/// bounded inspection before a new source kind may be persisted.
pub(super) fn require_source_support(
    manifest: &Manifest,
    executable: &Path,
    source: &PackageSource,
) -> io::Result<()> {
    if !matches!(source, PackageSource::DefaultChannel {}) {
        return Ok(());
    }
    super::channel::manifest_url(
        &manifest.build.package_version,
        &manifest.build.target,
        &manifest.build.component,
    )?;
    let verify = || -> io::Result<()> {
        if regular(executable, MAX_PAYLOAD)?.metadata()?.len() != manifest.payload.bytes
            || sha256(executable)? != manifest.payload.sha256
        {
            return Err(invalid("default-channel candidate identity changed"));
        }
        Ok(())
    };
    verify()?;
    let result = run_inspection(
        Command::new(executable)
            .args(super::channel::CAPABILITY_ARGS)
            .stdin(Stdio::null()),
        128,
        Duration::from_secs(3),
    );
    verify()?;
    if !result.is_ok_and(|bytes| bytes == super::channel::CAPABILITY) {
        return Err(invalid(
            "Candidate cannot retain default-channel intent; use a supporting package or explicitly replace the source policy before rollback. Installation retained.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod channel_download_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn pinned_manifest_is_checked_before_payload_and_explicit_source_stays_explicit() {
        let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("core-channel-{}", crate::os::nonce().unwrap()));
        private_dir(&root).unwrap();
        let payload = b"verified fixture bytes, never executed";
        let hash_file = root.join("hash");
        fs::write(&hash_file, payload).unwrap();
        let build: BuildMetadata = serde_json::from_str(crate::build_info::json()).unwrap();
        let manifest = json!({"schema_version":1,"build":build,"payload":{"file_name":"flere","bytes":payload.len(),"sha256":sha256(&hash_file).unwrap()}});
        let raw = serde_json::to_vec(&manifest).unwrap();
        fs::write(&hash_file, &raw).unwrap();
        let pin = json!({"bytes":raw.len(),"sha256":sha256(&hash_file).unwrap()});
        let index = serde_json::to_vec(&json!({"schema_version":1,"targets":{crate::build_info::TARGET:{"policy":"current","version":build.package_version,"manifests":{"flere":pin,"flere-connect":pin}}}})).unwrap();
        let selection =
            super::super::channel::select_update(&index, crate::build_info::TARGET, "flere", None)
                .unwrap();
        for (i, body) in [vec![b'!'; raw.len()], vec![b'!'; raw.len() + 1]]
            .into_iter()
            .enumerate()
        {
            let output = root.join(format!("invalid-{i}"));
            let mut calls = 0;
            assert!(
                download_with(&selection.url, &output, Some(&selection), |url, _| {
                    calls += 1;
                    assert_eq!(url, selection.url);
                    Ok(body.clone())
                })
                .is_err()
            );
            assert_eq!(calls, 1);
            assert!(!output.exists());
        }
        for (name, selected) in [("default", Some(&selection)), ("explicit", None)] {
            let output = root.join(name);
            let mut calls = Vec::new();
            let result = download_with(&selection.url, &output, selected, |url, bound| {
                calls.push(url.to_owned());
                if calls.len() == 1 {
                    assert_eq!(url, selection.url);
                    Ok(raw.clone())
                } else {
                    assert_eq!(bound, payload.len());
                    assert!(url.ends_with("/flere"));
                    Ok(payload.to_vec())
                }
            })
            .unwrap();
            assert_eq!(result.build, build);
            assert_eq!(calls.len(), 2);
            assert!(!calls.iter().any(|url| url == super::super::channel::URL));
            assert_eq!(fs::read(output.join("flere")).unwrap(), payload);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
