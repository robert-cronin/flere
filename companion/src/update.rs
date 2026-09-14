//! Per-user companion packages. A Windows stable native launcher waits for its
//! immutable worker, so replacement never races a shell for the same console.
//! No command here launches/restarts a remote supervisor or replays native input.
#[path = "../../src/install/manifest.rs"]
#[allow(
    dead_code,
    reason = "Shared manifest includes core-only state compatibility and source receipts"
)]
mod manifest;
use manifest::MAX_PAYLOAD;
pub(crate) use manifest::{BuildMetadata, Manifest, PackageSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

const COMPONENT: &str = "flere-connect";
const MAX_JSON: u64 = 128 * 1024;
#[cfg(windows)]
pub const RESTART: i32 = 75;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
pub(crate) fn nonce() -> io::Result<String> {
    crate::os::nonce()
}
fn options() -> OpenOptions {
    let mut result = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        result
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        result.custom_flags(0x0020_0000);
    } // FILE_FLAG_OPEN_REPARSE_POINT
    result
}
fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn owned(metadata: &fs::Metadata, private: bool) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        !linked(metadata)
            && metadata.uid() == crate::os::uid()
            && metadata.mode() & if private { 0o077 } else { 0o022 } == 0
    }
    #[cfg(windows)]
    {
        let _ = private;
        !linked(metadata)
    }
}
fn regular(path: &Path, maximum: u64) -> io::Result<File> {
    let file = options().read(true).open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || linked(&metadata) || metadata.len() > maximum {
        return Err(invalid(
            "package file is linked, nonregular or exceeds its bound",
        ));
    }
    Ok(file)
}
pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || !owned(&metadata, true) {
        return Err(invalid(
            "companion installation directory must be private, user-owned and not linked",
        ));
    }
    // Windows directories inherit the selected per-user LOCALAPPDATA ACL. Exact
    // Windows ACL/console replacement acceptance is recorded separately from
    // portable source/cross-compilation tests; never claim Unix mode guarantees.
    Ok(())
}
fn executable(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        let _ = path;
    }
    Ok(())
}
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    } // std cannot open a directory for FlushFileBuffers.
}
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> io::Result<T> {
    let mut bytes = Vec::new();
    regular(path, MAX_JSON)?
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(invalid("companion installation metadata exceeds bound"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(invalid("companion installation metadata exceeds bound"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("receipt needs parent directory"))?;
    let temporary = parent.join(format!(".receipt-{}", nonce()?));
    let result = (|| {
        let mut file = options().write(true).create_new(true).open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_dir(parent)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

/// Portable bounded pipes. Readers stop at their byte bound; the owned child is
/// killed on timeout/error. No helper inherits the user's terminal input.
pub(crate) fn run(command: &mut Command, maximum: usize, timeout: Duration) -> io::Result<Vec<u8>> {
    let mut child = crate::Ssh(
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let stdout = child.0.stdout.take().unwrap();
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = mpsc::sync_channel(2);
    fn reader(
        reader: impl Read + Send + 'static,
        limit: usize,
        tag: usize,
        sender: mpsc::SyncSender<(usize, io::Result<Vec<u8>>)>,
    ) {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .and_then(|_| {
                    if bytes.len() > limit {
                        Err(invalid("installer helper output exceeds bound"))
                    } else {
                        Ok(bytes)
                    }
                });
            let _ = sender.send((tag, result));
        });
    }
    reader(stdout, maximum, 0, sender.clone());
    reader(stderr, 16384, 1, sender);
    let deadline = Instant::now() + timeout;
    let mut output = [None, None];
    loop {
        while let Ok((tag, bytes)) = receiver.try_recv() {
            output[tag] = Some(bytes?);
        }
        if output.iter().all(Option::is_some)
            && let Some(status) = child.0.try_wait()?
        {
            return if status.success() {
                Ok(output[0].take().unwrap())
            } else {
                Err(io::Error::other(format!(
                    "installer helper failed: {}",
                    String::from_utf8_lossy(output[1].as_ref().unwrap())
                        .chars()
                        .filter(|c| !c.is_control() || *c == '\n')
                        .take(2048)
                        .collect::<String>()
                )))
            };
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "installer helper timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn sha256(path: &Path) -> io::Result<String> {
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
            .ok_or_else(|| invalid("package path is not Unicode"))?;
        let script = format!(
            "$ErrorActionPreference='Stop';(Get-FileHash -LiteralPath '{}' -Algorithm SHA256).Hash.ToLowerInvariant()",
            path.replace('\'', "''")
        );
        let mut c = Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .stdin(Stdio::null());
        c
    };
    let bytes = run(&mut command, 512, Duration::from_secs(30))?;
    let digest = String::from_utf8(bytes)
        .map_err(io::Error::other)?
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    if !manifest::digest_valid(&digest) {
        return Err(invalid("system SHA-256 utility returned invalid digest"));
    }
    Ok(digest)
}
fn inspect(path: &Path) -> io::Result<BuildMetadata> {
    regular(path, MAX_PAYLOAD)?;
    let bytes = run(
        Command::new(path).arg("--build-info").stdin(Stdio::null()),
        65536,
        Duration::from_secs(3),
    )?;
    let build: BuildMetadata = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    build.validate()?;
    Ok(build)
}
fn package(
    binary: &Path,
    output: &Path,
    source: Option<manifest::SourceReceipt>,
) -> io::Result<Manifest> {
    if output.try_exists()? {
        return Err(invalid("package output already exists"));
    }
    let build = inspect(binary)?;
    if build.component != COMPONENT || build.target != crate::build_info::TARGET {
        return Err(invalid(
            "package producer requires a companion for this platform",
        ));
    }
    private_dir(output)?;
    let result = (|| {
        let path = output.join(COMPONENT);
        let mut destination = options().write(true).create_new(true).open(&path)?;
        let bytes = io::copy(
            &mut regular(binary, MAX_PAYLOAD)?.take(MAX_PAYLOAD + 1),
            &mut destination,
        )?;
        destination.sync_all()?;
        drop(destination);
        let manifest = Manifest {
            schema_version: 1,
            payload: manifest::Payload {
                file_name: COMPONENT.into(),
                download_file: Some(format!("{COMPONENT}-{}", build.target)),
                bytes,
                sha256: sha256(&path)?,
            },
            build,
            source,
        };
        manifest.validate()?;
        // Windows CreateProcess requires an .exe suffix. This temporary copy is
        // inspected before publication, while the shared package basename stays
        // identical on every platform.
        let inspection = output.join(if cfg!(windows) {
            ".inspect.exe"
        } else {
            ".inspect"
        });
        fs::copy(&path, &inspection)?;
        executable(&inspection)?;
        if inspect(&inspection)? != manifest.build
            || sha256(&inspection)? != manifest.payload.sha256
        {
            return Err(invalid("candidate changed while packaging"));
        }
        fs::remove_file(&inspection)?;
        write_json(&output.join("manifest.json"), &manifest)?;
        sync_dir(output)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}
fn https(url: &str) -> io::Result<()> {
    if !url.starts_with("https://")
        || url.len() > 4096
        || !url.is_ascii()
        || url.contains(['@', '?', '#', '\\'])
        || url.bytes().any(|b| b.is_ascii_control() || b == b' ')
        || !url.ends_with("manifest.json")
    {
        return Err(invalid(
            "choose an explicit HTTPS manifest URL without credentials/query/fragment",
        ));
    }
    Ok(())
}
fn download(url: &str, directory: &Path) -> io::Result<Manifest> {
    https(url)?;
    #[cfg(windows)]
    let curl = "curl.exe";
    #[cfg(not(windows))]
    let curl = "curl";
    let fetch = |url: &str, bound: usize| {
        run(
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
            bound,
            Duration::from_secs(65),
        )
    };
    let manifest: Manifest =
        serde_json::from_slice(&fetch(url, 65536)?).map_err(io::Error::other)?;
    manifest.validate()?;
    if manifest.build.component != COMPONENT || manifest.build.target != crate::build_info::TARGET {
        return Err(invalid("release companion target/component differs"));
    }
    private_dir(directory)?;
    let result = (|| {
        let asset = manifest
            .payload
            .download_file
            .as_deref()
            .unwrap_or(&manifest.payload.file_name);
        let bytes = fetch(
            &format!("{}/{}", url.rsplit_once('/').unwrap().0, asset),
            manifest.payload.bytes as usize,
        )?;
        let path = directory.join(COMPONENT);
        let mut file = options().write(true).create_new(true).open(&path)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if sha256(&path)? != manifest.payload.sha256 || bytes.len() as u64 != manifest.payload.bytes
        {
            return Err(invalid("download does not match manifest size/SHA-256"));
        }
        write_json(&directory.join("manifest.json"), &manifest)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(directory);
    }
    result
}

fn remote_build(connection: &crate::connections::Connection) -> io::Result<BuildMetadata> {
    let mut remote = format!("exec {}", crate::quote(&connection.remote)?);
    if let Some(state) = &connection.state {
        remote.push_str(&format!(" --state {}", crate::quote(state)?));
    }
    remote.push_str(" build-status");
    let bytes = run(
        Command::new(&connection.ssh)
            .args(["-T", "--", &connection.host, &remote])
            .stdin(Stdio::null()),
        8 * 1024 * 1024,
        Duration::from_secs(20),
    )?;
    let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    if value["schema_version"] != 1 || value["supervisor"]["status"] != "known" {
        return Err(invalid(
            "remote supervisor compatibility is unknown; companion installation was not changed",
        ));
    }
    let build: BuildMetadata =
        serde_json::from_value(value["supervisor"]["build"].clone()).map_err(io::Error::other)?;
    build.validate()?;
    if build.component != "flere" {
        return Err(invalid(
            "selected SSH target did not report a core supervisor",
        ));
    }
    Ok(build)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Package {
    pub id: String,
    pub manifest: Manifest,
    pub executable: PathBuf,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub schema_version: u64,
    pub owner: String,
    pub component: String,
    pub destination: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<PathBuf>,
    pub current: Package,
    pub previous: Option<Package>,
    pub source: PackageSource,
    pub attempt: String,
    pub status: String,
    pub activation: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_sha256: Option<String>,
}
pub(crate) type InstallResult = Receipt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Prepared {
    pub package: Package,
    pub source: PackageSource,
    baseline: InstallationBaseline,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum InstallationBaseline {
    Missing,
    Manual {
        bytes: u64,
        sha256: String,
    },
    Managed {
        attempt: String,
        package_id: String,
        destination_sha256: String,
    },
}

pub(crate) fn source_default() -> io::Result<Option<PackageSource>> {
    Ok(Store::standard()?.status()?.map(|r| r.source))
}
pub(crate) fn coordinated_dir() -> io::Result<PathBuf> {
    let store = Store::standard()?;
    private_dir(&store.root)?;
    let directory = store.root.join("coordinated");
    private_dir(&directory)?;
    Ok(directory)
}

pub(crate) fn prepare(source: &str) -> io::Result<Prepared> {
    let store = Store::standard()?;
    prepare_in(&store, source)
}
fn prepare_in(store: &Store, source: &str) -> io::Result<Prepared> {
    let _lock = store.lock()?;
    store.recover()?;
    let baseline = store.baseline()?;
    if source == "--rollback" {
        let receipt = store
            .status()?
            .ok_or_else(|| invalid("companion is not managed"))?;
        return Ok(Prepared {
            package: receipt
                .previous
                .ok_or_else(|| invalid("no previous companion retained"))?,
            source: receipt.source,
            baseline,
        });
    }
    let source = if source.is_empty() {
        match store.status()?.map(|r| r.source) {
            Some(PackageSource::Public { manifest_url }) => manifest_url,
            _ => {
                return Err(invalid(
                    "choose a companion package or HTTPS manifest source",
                ));
            }
        }
    } else {
        source.into()
    };
    if source.starts_with("https://") {
        let directory = store.root.join(format!(".download-{}", nonce()?));
        download(&source, &directory)?;
        let staged = store.stage(&directory);
        let _ = fs::remove_dir_all(directory);
        Ok(Prepared {
            package: staged?,
            source: PackageSource::Public {
                manifest_url: source,
            },
            baseline,
        })
    } else {
        let path = Path::new(&source);
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok(Prepared {
            package: store.stage(&path)?,
            source: PackageSource::Local,
            baseline,
        })
    }
}
pub(crate) fn install_prepared(prepared: &Prepared, adopt: bool) -> io::Result<InstallResult> {
    let store = Store::standard()?;
    install_prepared_in(&store, prepared, adopt)
}
fn install_prepared_in(
    store: &Store,
    prepared: &Prepared,
    adopt: bool,
) -> io::Result<InstallResult> {
    let _lock = store.lock()?;
    store.recover()?;
    if store.baseline()? != prepared.baseline {
        return Err(invalid(
            "companion installation changed after preparation; prepare again before applying",
        ));
    }
    store.install(prepared.package.clone(), prepared.source.clone(), adopt)
}
/// Recheck the exact locally installed attempt and the process actually executing
/// it. A matching build stamp or an old successful reconnect label is insufficient.
pub(crate) fn verify_applied(prepared: &Prepared, expected_attempt: &str) -> io::Result<()> {
    verify_applied_in(&Store::standard()?, prepared, expected_attempt)
}
fn verify_applied_in(store: &Store, prepared: &Prepared, expected_attempt: &str) -> io::Result<()> {
    let _lock = store.lock()?;
    let receipt = store
        .status()?
        .ok_or_else(|| invalid("companion managed installation disappeared"))?;
    if receipt.attempt != expected_attempt || receipt.current != prepared.package {
        return Err(invalid(
            "companion installed attempt/package changed before acknowledgement",
        ));
    }
    if !running_candidate(prepared) {
        return Err(invalid(
            "running companion is not the exact retained candidate executable/build",
        ));
    }
    Ok(())
}
pub(crate) fn running_candidate(prepared: &Prepared) -> bool {
    let metadata: Result<BuildMetadata, _> = serde_json::from_str(crate::build_info::json());
    metadata.is_ok_and(|m| m == prepared.package.manifest.build)
        && std::env::current_exe()
            .ok()
            .and_then(|p| fs::canonicalize(p).ok())
            .is_some_and(|current| {
                fs::canonicalize(&prepared.package.executable)
                    .is_ok_and(|candidate| candidate == current)
            })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RestartRecord {
    schema_version: u64,
    attempt: String,
    launch_id: String,
    connection: Vec<String>,
    resume: Option<String>,
}
static RESTART_REQUESTED: Mutex<Option<(String, String)>> = Mutex::new(None);
static DIRECT_LAUNCH_ID: Mutex<Option<String>> = Mutex::new(None);
fn valid_token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn launch_id() -> io::Result<String> {
    if let Some(value) = std::env::var_os("FLERE_NATIVE_LAUNCH_ID") {
        let value = value
            .into_string()
            .map_err(|_| invalid("invalid launcher identity"))?;
        if !valid_token(&value) {
            return Err(invalid("invalid launcher identity"));
        }
        return Ok(value);
    }
    let mut id = DIRECT_LAUNCH_ID
        .lock()
        .map_err(|_| invalid("restart identity lock failed"))?;
    if id.is_none() {
        *id = Some(nonce()?);
    }
    Ok(id.clone().unwrap())
}
fn restart_record(store: &Store, receipt: &Receipt, launch: &str) -> io::Result<RestartRecord> {
    if !valid_token(launch) {
        return Err(invalid("invalid restart launcher token"));
    }
    let record: RestartRecord = read_json(
        &store
            .root
            .join(format!("restart-{}-{launch}.json", receipt.attempt)),
    )?;
    if record.schema_version != 1
        || record.attempt != receipt.attempt
        || record.launch_id != launch
        || crate::connections::Connection::parse(&record.connection)?.args() != record.connection
        || record.resume.as_ref().is_some_and(|t| !valid_token(t))
    {
        return Err(invalid("invalid exact companion restart record"));
    }
    Ok(record)
}
pub(crate) fn request_restart(
    connection: &crate::connections::Connection,
    resume: Option<&str>,
    expected_attempt: &str,
) -> io::Result<()> {
    if resume.is_some_and(|t| !valid_token(t)) {
        return Err(invalid("invalid coordinated restart token"));
    }
    let store = Store::standard()?;
    let _lock = store.lock()?;
    store.recover()?;
    let receipt = store
        .status()?
        .ok_or_else(|| invalid("companion must be managed before restart"))?;
    if receipt.attempt != expected_attempt {
        return Err(invalid("companion installation changed before restart"));
    }
    #[cfg(windows)]
    if receipt.launcher_sha256.is_none() || std::env::var_os("FLERE_NATIVE_LAUNCH_ID").is_none() {
        return Err(invalid(
            "Windows live update requires the managed native launcher; installed package remains available for explicit reconnect",
        ));
    }
    let launch_id = launch_id()?;
    write_json(
        &store
            .root
            .join(format!("restart-{}-{launch_id}.json", receipt.attempt)),
        &RestartRecord {
            schema_version: 1,
            attempt: receipt.attempt.clone(),
            launch_id: launch_id.clone(),
            connection: connection.args(),
            resume: resume.map(String::from),
        },
    )?;
    *RESTART_REQUESTED
        .lock()
        .map_err(|_| invalid("restart request lock failed"))? = Some((receipt.attempt, launch_id));
    Ok(())
}
/// Call only after the event-loop stack has released SSH and console resources.
/// Exec/worker exit also destroys old stdin-reader threads, preventing key theft.
pub(crate) fn finish_restart() -> io::Result<()> {
    let Some((attempt, launch)) = RESTART_REQUESTED
        .lock()
        .map_err(|_| invalid("restart request lock failed"))?
        .take()
    else {
        return Ok(());
    };
    let store = Store::standard()?;
    let receipt = store
        .status()?
        .ok_or_else(|| invalid("companion install disappeared before restart"))?;
    if receipt.attempt != attempt {
        return Err(invalid(
            "companion installation changed after restart was requested",
        ));
    }
    let record = restart_record(&store, &receipt, &launch)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(&receipt.current.executable);
        command
            .args(&record.connection)
            .env("FLERE_CONNECT_UPDATE_ACK", &receipt.attempt)
            .env_remove("FLERE_COORDINATED_UPDATE");
        if let Some(token) = &record.resume {
            command.env("FLERE_COORDINATED_UPDATE", token);
        }
        Err(command.exec())
    }
    #[cfg(windows)]
    {
        let _ = record;
        std::process::exit(RESTART)
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    next: Receipt,
    previous_sha256: Option<String>,
}
struct InstallationLock {
    file: File,
}
impl Drop for InstallationLock {
    fn drop(&mut self) {
        // A concurrently spawned child can briefly inherit the descriptor before
        // exec. Closing our file alone would leave its shared flock held until
        // that child closes its copy. Release ownership when this guard ends.
        let _ = self.file.unlock();
    }
}
struct Store {
    root: PathBuf,
    bin: PathBuf,
    windows_launchers: bool,
}
impl Store {
    fn baseline(&self) -> io::Result<InstallationBaseline> {
        if let Some(receipt) = self.status()? {
            return Ok(InstallationBaseline::Managed {
                attempt: receipt.attempt,
                package_id: receipt.current.id,
                destination_sha256: receipt
                    .launcher_sha256
                    .unwrap_or(receipt.current.manifest.payload.sha256),
            });
        }
        self.require_aliases_absent()?;
        match fs::symlink_metadata(self.destination()) {
            Ok(metadata) => {
                if !metadata.is_file() || !owned(&metadata, false) {
                    return Err(invalid(
                        "unmanaged companion destination is linked or has foreign ownership/permissions",
                    ));
                }
                Ok(InstallationBaseline::Manual {
                    bytes: metadata.len(),
                    sha256: sha256(&self.destination())?,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Ok(InstallationBaseline::Missing)
            }
            Err(error) => Err(error),
        }
    }
    fn standard() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .ok_or_else(|| invalid("HOME is unavailable"))?;
            let data = std::env::var_os("XDG_DATA_HOME")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"));
            if !data.is_absolute() {
                return Err(invalid("XDG_DATA_HOME must be absolute"));
            }
            Ok(Self {
                root: data.join("flere/install"),
                bin: home.join(".local/bin"),
                windows_launchers: false,
            })
        }
        #[cfg(windows)]
        {
            let base = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .ok_or_else(|| invalid("LOCALAPPDATA is unavailable"))?
                .join("Flere");
            Ok(Self {
                root: base.join("install"),
                bin: base.join("bin"),
                windows_launchers: true,
            })
        }
    }
    fn destination(&self) -> PathBuf {
        self.bin.join(if self.windows_launchers {
            "flere.exe"
        } else {
            COMPONENT
        })
    }
    fn aliases(&self) -> Vec<PathBuf> {
        if self.windows_launchers {
            vec![self.bin.join("flere-connect.exe")]
        } else {
            Vec::new()
        }
    }
    fn require_aliases_absent(&self) -> io::Result<()> {
        for alias in self.aliases() {
            match fs::symlink_metadata(&alias) {
                Ok(_) => {
                    return Err(invalid(
                        "unmanaged flere-connect.exe alias exists; move it explicitly before installation; --adopt only adopts the primary command",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    fn validate_receipt_paths(&self, receipt: &Receipt) -> io::Result<()> {
        if receipt.schema_version != 1
            || receipt.owner != "flere"
            || receipt.component != COMPONENT
            || receipt.destination != self.destination()
            || receipt.aliases != self.aliases()
            || receipt.launcher_sha256.is_some() != self.windows_launchers
            || !valid_token(&receipt.attempt)
        {
            return Err(invalid("unsupported/foreign companion receipt"));
        }
        Ok(())
    }
    fn validate_aliases(&self, expected: &str) -> io::Result<()> {
        for alias in self.aliases() {
            let metadata = fs::symlink_metadata(&alias)?;
            if !metadata.is_file() || !owned(&metadata, false) || sha256(&alias)? != expected {
                return Err(invalid(
                    "installed companion alias differs from its receipt",
                ));
            }
        }
        Ok(())
    }
    fn publish_aliases(&self, expected: &str) -> io::Result<()> {
        if !self.windows_launchers {
            return Ok(());
        }
        // Both names are stable native launchers. Publish without replacement;
        // an unsupported filesystem or a racing unrelated file fails closed.
        // The pending receipt retains this operation across an interrupted install.
        if sha256(&self.destination())? != expected {
            return Err(invalid("primary launcher changed before alias publication"));
        }
        for alias in self.aliases() {
            match fs::hard_link(self.destination(), &alias) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        self.validate_aliases(expected)?;
        sync_dir(&self.bin)
    }
    #[cfg(any(windows, test))]
    fn is_launcher_path(&self, executable: &Path) -> io::Result<bool> {
        let executable = fs::canonicalize(executable)?;
        Ok(std::iter::once(self.destination())
            .chain(self.aliases())
            .any(|path| fs::canonicalize(path).is_ok_and(|path| path == executable)))
    }
    fn receipt(&self) -> PathBuf {
        self.root.join("flere-connect.json")
    }
    fn journal(&self) -> PathBuf {
        self.root.join("flere-connect.pending.json")
    }
    fn lock(&self) -> io::Result<InstallationLock> {
        private_dir(&self.root)?;
        let file = options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("install.lock"))?;
        if !file.metadata()?.is_file() || !owned(&file.metadata()?, true) {
            return Err(invalid("invalid installation lock"));
        }
        file.try_lock()
            .map_err(|_| io::Error::other("another installation owns the update lock"))?;
        Ok(InstallationLock { file })
    }
    fn validate(&self, package: &Package) -> io::Result<()> {
        package.manifest.validate()?;
        if package.id != package.manifest.id()
            || package.manifest.build.component != COMPONENT
            || package.manifest.build.target != crate::build_info::TARGET
            || package.executable
                != self
                    .root
                    .join("packages")
                    .join(&package.id)
                    .join(if self.windows_launchers {
                        "flere-connect.exe"
                    } else {
                        COMPONENT
                    })
            || regular(&package.executable, MAX_PAYLOAD)?.metadata()?.len()
                != package.manifest.payload.bytes
            || sha256(&package.executable)? != package.manifest.payload.sha256
        {
            return Err(invalid("retained companion package identity/hash differs"));
        }
        Ok(())
    }
    fn status(&self) -> io::Result<Option<Receipt>> {
        match fs::symlink_metadata(&self.root) {
            Ok(metadata) if !metadata.is_dir() || !owned(&metadata, true) => {
                return Err(invalid(
                    "companion installation ownership/permissions changed",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            _ => {}
        }
        if let Ok(metadata) = fs::symlink_metadata(self.receipt())
            && !owned(&metadata, true)
        {
            return Err(invalid("companion receipt is not private/user-owned"));
        }
        let receipt: Receipt = match read_json(&self.receipt()) {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        self.validate_receipt_paths(&receipt)?;
        self.validate(&receipt.current)?;
        if let Some(previous) = &receipt.previous {
            self.validate(previous)?;
        }
        let expected = receipt
            .launcher_sha256
            .as_deref()
            .unwrap_or(&receipt.current.manifest.payload.sha256);
        if sha256(&self.destination())? != expected {
            return Err(invalid(
                "installed companion/launcher differs from its receipt",
            ));
        }
        self.validate_aliases(expected)?;
        if self.journal().try_exists()? {
            return Err(invalid(
                "interrupted companion install; retry explicit install/update to reconcile",
            ));
        }
        Ok(Some(receipt))
    }
    fn recover(&self) -> io::Result<()> {
        let pending: Pending = match read_json(&self.journal()) {
            Ok(p) => p,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        self.validate(&pending.next.current)?;
        self.validate_receipt_paths(&pending.next)?;
        let actual = match sha256(&self.destination()) {
            Ok(hash) => Some(hash),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        let desired = pending
            .next
            .launcher_sha256
            .as_ref()
            .unwrap_or(&pending.next.current.manifest.payload.sha256);
        if actual.as_ref() == Some(desired) {
            self.publish_aliases(desired)?;
            write_json(&self.receipt(), &pending.next)?;
        } else if actual != pending.previous_sha256 {
            return Err(invalid(
                "interrupted companion destination changed externally",
            ));
        }
        fs::remove_file(self.journal())?;
        sync_dir(&self.root)
    }
    fn stage(&self, directory: &Path) -> io::Result<Package> {
        let manifest: Manifest = read_json(&directory.join("manifest.json"))?;
        manifest.validate()?;
        if manifest.build.component != COMPONENT
            || manifest.build.target != crate::build_info::TARGET
        {
            return Err(invalid("package target/component differs"));
        }
        let source = directory.join(&manifest.payload.file_name);
        if regular(&source, MAX_PAYLOAD)?.metadata()?.len() != manifest.payload.bytes
            || sha256(&source)? != manifest.payload.sha256
        {
            return Err(invalid("package size/SHA-256 does not match manifest"));
        }
        private_dir(&self.root.join("packages"))?;
        let path = self.root.join("packages").join(manifest.id());
        let binary_name = if self.windows_launchers {
            "flere-connect.exe"
        } else {
            COMPONENT
        };
        let package = Package {
            id: manifest.id(),
            executable: path.join(binary_name),
            manifest,
        };
        if path.try_exists()? {
            let retained: Manifest = read_json(&path.join("manifest.json"))?;
            if retained.build != package.manifest.build
                || retained.payload != package.manifest.payload
            {
                return Err(invalid("retained companion package identity collision"));
            }
            let package = Package {
                manifest: retained,
                ..package
            };
            self.validate(&package)?;
            return Ok(package);
        }
        let temporary = self
            .root
            .join("packages")
            .join(format!(".stage-{}", nonce()?));
        private_dir(&temporary)?;
        let result = (|| {
            let binary = temporary.join(binary_name);
            let mut file = options().write(true).create_new(true).open(&binary)?;
            let length = io::copy(
                &mut regular(&source, MAX_PAYLOAD)?.take(MAX_PAYLOAD + 1),
                &mut file,
            )?;
            file.sync_all()?;
            drop(file);
            executable(&binary)?;
            if length != package.manifest.payload.bytes
                || sha256(&binary)? != package.manifest.payload.sha256
                || inspect(&binary)? != package.manifest.build
            {
                return Err(invalid("candidate changed or embedded identity differs"));
            }
            write_json(&temporary.join("manifest.json"), &package.manifest)?;
            sync_dir(&temporary)?;
            fs::rename(&temporary, &path)?;
            sync_dir(&self.root.join("packages"))?;
            Ok(package)
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(temporary);
        }
        result
    }
    fn adopt(&self) -> io::Result<Package> {
        let path = self.destination();
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || !owned(&metadata, false) {
            return Err(invalid(
                "existing companion belongs to another installation owner",
            ));
        }
        let build = inspect(&path)?;
        if build.component != COMPONENT || build.target != crate::build_info::TARGET {
            return Err(invalid("manual predecessor component/target differs"));
        }
        let temporary = self.root.join(format!(".adopt-{}", nonce()?));
        private_dir(&temporary)?;
        let result = (|| {
            let target = temporary.join(COMPONENT);
            fs::copy(&path, &target)?;
            let manifest = Manifest {
                schema_version: 1,
                build,
                payload: manifest::Payload {
                    file_name: COMPONENT.into(),
                    download_file: None,
                    bytes: fs::metadata(&target)?.len(),
                    sha256: sha256(&target)?,
                },
                source: None,
            };
            write_json(&temporary.join("manifest.json"), &manifest)?;
            self.stage(&temporary)
        })();
        let _ = fs::remove_dir_all(temporary);
        result
    }
    fn install(&self, package: Package, source: PackageSource, adopt: bool) -> io::Result<Receipt> {
        self.validate(&package)?;
        if !self.bin.try_exists()? {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(&self.bin)?;
        }
        let bin = fs::symlink_metadata(&self.bin)?;
        if !bin.is_dir() || !owned(&bin, false) {
            return Err(invalid(
                "companion bin directory is linked or foreign-writable",
            ));
        }
        let prior = self.status()?;
        if prior.is_none() {
            self.require_aliases_absent()?;
        }
        if let Some(prior) = &prior
            && prior.current.id == package.id
        {
            return Ok(prior.clone());
        }
        let previous = match &prior {
            Some(p) => Some(p.current.clone()),
            None if self.destination().try_exists()? => {
                if !adopt {
                    return Err(invalid(
                        "explicit --adopt is required for an existing manual companion",
                    ));
                }
                Some(self.adopt()?)
            }
            None => None,
        };
        let previous_hash = if self.destination().try_exists()? {
            Some(sha256(&self.destination())?)
        } else {
            None
        };
        let receipt = Receipt {
            schema_version: 1,
            owner: "flere".into(),
            component: COMPONENT.into(),
            destination: self.destination(),
            aliases: self.aliases(),
            current: package,
            previous,
            source,
            attempt: nonce()?,
            status: "installed_pending_reconnect".into(),
            activation: None,
            launcher_sha256: None,
        };
        let receipt = if self.windows_launchers {
            let mut receipt = receipt;
            if let Some(hash) = prior.as_ref().and_then(|p| p.launcher_sha256.clone()) {
                // The existing native launcher proves it understands this package
                // before a worker with a newer schema becomes its launch target.
                run(
                    Command::new(self.destination())
                        .arg("_launcher-check")
                        .arg(receipt.current.executable.parent().unwrap())
                        .stdin(Stdio::null()),
                    1024,
                    Duration::from_secs(3),
                )?;
                receipt.launcher_sha256 = Some(hash);
                write_json(&self.receipt(), &receipt)?;
                return Ok(receipt);
            }
            receipt.launcher_sha256 = Some(receipt.current.manifest.payload.sha256.clone());
            receipt
        } else {
            receipt
        };
        let temporary = self.bin.join(format!(".companion-{}", nonce()?));
        let result = (|| {
            let mut file = options().write(true).create_new(true).open(&temporary)?;
            io::copy(
                &mut regular(&receipt.current.executable, MAX_PAYLOAD)?.take(MAX_PAYLOAD + 1),
                &mut file,
            )?;
            file.sync_all()?;
            drop(file);
            executable(&temporary)?;
            if sha256(&temporary)? != receipt.current.manifest.payload.sha256 {
                return Err(invalid("installed companion copy failed SHA-256"));
            }
            write_json(
                &self.journal(),
                &Pending {
                    next: receipt.clone(),
                    previous_sha256: previous_hash.clone(),
                },
            )?;
            let actual = if self.destination().try_exists()? {
                Some(sha256(&self.destination())?)
            } else {
                None
            };
            if actual != previous_hash {
                return Err(invalid("companion destination changed during installation"));
            }
            fs::rename(&temporary, self.destination()).map_err(|e| io::Error::new(e.kind(), "companion command is in use or replacement was denied; no process was killed; retry after detaching the old manual companion"))?;
            sync_dir(&self.bin)?;
            self.publish_aliases(&receipt.current.manifest.payload.sha256)?;
            write_json(&self.receipt(), &receipt)?;
            fs::remove_file(self.journal())?;
            sync_dir(&self.root)?;
            Ok(receipt)
        })();
        let _ = fs::remove_file(temporary);
        result
    }
}

/// Acknowledged only by the selected new worker after an actual successful HELLO.
pub fn connected(protocol: &str) -> io::Result<()> {
    let Some(attempt) = std::env::var_os("FLERE_CONNECT_UPDATE_ACK") else {
        return Ok(());
    };
    let store = Store::standard()?;
    let _lock = store.lock()?;
    store.recover()?;
    let mut receipt = store
        .status()?
        .ok_or_else(|| invalid("companion update has no managed receipt"))?;
    let current: BuildMetadata =
        serde_json::from_str(crate::build_info::json()).map_err(io::Error::other)?;
    if attempt != receipt.attempt.as_str()
        || current != receipt.current.manifest.build
        || fs::canonicalize(std::env::current_exe()?)?
            != fs::canonicalize(&receipt.current.executable)?
        || !current
            .compatibility
            .remote_protocol
            .accepts
            .iter()
            .any(|p| p == protocol)
    {
        return Err(invalid(
            "companion reconnect build/attempt/protocol differs",
        ));
    }
    receipt.status = "applied".into();
    write_json(&store.receipt(), &receipt)
}

/// On Windows the stable binary owns the foreground wait, while immutable workers
/// own SSH/console IO. Rust argv forwarding adds no cmd.exe/PowerShell evaluation.
pub fn launcher(args: &[String]) -> io::Result<bool> {
    #[cfg(not(windows))]
    {
        let _ = args;
        Ok(false)
    }
    #[cfg(windows)]
    {
        let store = Store::standard()?;
        if !store.is_launcher_path(&std::env::current_exe()?)? {
            return Ok(false);
        }
        let Some(receipt) = store.status()? else {
            return Ok(false);
        };
        if receipt.launcher_sha256.is_none() {
            return Ok(false);
        }
        let mut launch_args = args.to_vec();
        let launch_id = nonce()?;
        let mut resume = None;
        let mut current = receipt;
        for _ in 0..4 {
            let mut command = Command::new(&current.current.executable);
            command
                .args(&launch_args)
                .env("FLERE_CONNECT_UPDATE_ACK", &current.attempt)
                .env("FLERE_NATIVE_LAUNCH_ID", &launch_id)
                .env_remove("FLERE_COORDINATED_UPDATE");
            if let Some(token) = &resume {
                command.env("FLERE_COORDINATED_UPDATE", token);
            }
            let status = command.status()?;
            if status.code() != Some(RESTART) {
                return if status.success() {
                    Ok(true)
                } else {
                    Err(io::Error::other(format!(
                        "companion worker exited {status}"
                    )))
                };
            }
            let next = store
                .status()?
                .ok_or_else(|| invalid("managed companion disappeared during restart"))?;
            let record = restart_record(&store, &next, &launch_id)?;
            // Consume this launcher's exact request once. A deliberate reconnect
            // to the same package is valid; an unexplained repeated exit 75 has
            // no record and cannot replay its arguments or coordinated plan.
            fs::remove_file(
                store
                    .root
                    .join(format!("restart-{}-{launch_id}.json", next.attempt)),
            )?;
            sync_dir(&store.root)?;
            launch_args = record.connection;
            resume = record.resume;
            current = next;
        }
        Err(invalid("companion update restart bound reached"))
    }
}

/// Standalone installation/update commands execute before console/raw-input setup.
/// Returns true when handled. Reconnect is explicit and strips one-shot images.
pub fn command(args: &[String]) -> io::Result<bool> {
    let Some(action) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    if !matches!(
        action,
        "package" | "install" | "update" | "update-status" | "_launcher-check"
    ) {
        return Ok(false);
    }
    if action == "package" {
        if !(3..=4).contains(&args.len()) {
            return Err(invalid("usage: package BINARY OUTPUT [SOURCE_RECEIPT]"));
        }
        let source = args.get(3).map(|s| read_json(Path::new(s))).transpose()?;
        let manifest = package(Path::new(&args[1]), Path::new(&args[2]), source)?;
        println!(
            "{}",
            serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?
        );
        return Ok(true);
    }
    let store = Store::standard()?;
    if action == "_launcher-check" {
        if args.len() != 2 {
            return Err(invalid("invalid launcher check"));
        }
        let manifest: Manifest = read_json(&Path::new(&args[1]).join("manifest.json"))?;
        manifest.validate()?;
        let package = Package {
            id: manifest.id(),
            executable: Path::new(&args[1]).join(if cfg!(windows) {
                "flere-connect.exe"
            } else {
                COMPONENT
            }),
            manifest,
        };
        store.validate(&package)?;
        println!("launcher-compatible-v1");
        return Ok(true);
    }
    if action == "update-status" {
        if args.len() != 1 {
            return Err(invalid("update-status takes no arguments"));
        }
        println!("{}", serde_json::to_string_pretty(&json!({"schema_version":1,"running_build":serde_json::from_str::<Value>(crate::build_info::json()).map_err(io::Error::other)?,"installation":store.status()?,"other_frontends":"untracked"})).map_err(io::Error::other)?);
        return Ok(true);
    }
    let mut local = None;
    let mut url = None;
    let mut source_url = None;
    let mut adopt = false;
    let mut rollback = false;
    let mut reconnect: Option<Vec<String>> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--adopt" if !adopt => adopt = true,
            "--rollback" if action == "update" && !rollback => rollback = true,
            "--from-url" if url.is_none() => {
                i += 1;
                url = Some(
                    args.get(i)
                        .ok_or_else(|| invalid("--from-url requires URL"))?
                        .clone(),
                );
            }
            "--source-url" if action == "install" && source_url.is_none() => {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| invalid("--source-url requires URL"))?;
                https(value)?;
                source_url = Some(value.clone());
            }
            "--reconnect" if action == "update" && reconnect.is_none() => {
                let mut request = vec!["reconnect".into()];
                if args.get(i + 1).is_some_and(|a| !a.starts_with('-')) {
                    i += 1;
                    request.push(args[i].clone());
                }
                let resolved = crate::connections::prepare(&request)?
                    .ok_or_else(|| invalid("no saved connection to reconnect"))?;
                reconnect = Some(crate::connections::Connection::parse(&resolved)?.args());
            }
            value if !value.starts_with('-') && local.is_none() => {
                let p = PathBuf::from(value);
                local = Some(if p.is_absolute() {
                    p
                } else {
                    std::env::current_dir()?.join(p)
                });
            }
            _ => {
                return Err(invalid(
                    "usage: install PACKAGE [--adopt] | update [PACKAGE|--from-url URL|--rollback] [--adopt] [--reconnect [NAME]]",
                ));
            }
        }
        i += 1;
    }
    if (local.is_some() && url.is_some())
        || (rollback && (local.is_some() || url.is_some() || adopt))
        || (source_url.is_some() && (local.is_none() || url.is_some()))
    {
        return Err(invalid("choose exactly one package/source/rollback"));
    }
    let lock = store.lock()?;
    store.recover()?;
    let (package, source) = if rollback {
        let receipt = store
            .status()?
            .ok_or_else(|| invalid("companion is not managed"))?;
        (
            receipt
                .previous
                .ok_or_else(|| invalid("no previous companion package retained"))?,
            receipt.source,
        )
    } else {
        if local.is_none() && url.is_none() {
            url = match store.status()?.map(|r| r.source) {
                Some(PackageSource::Public { manifest_url }) => Some(manifest_url),
                _ => {
                    return Err(invalid(
                        "choose a local package or explicit HTTPS source; local builds are not switched to releases",
                    ));
                }
            };
        }
        if let Some(url) = url {
            let directory = store.root.join(format!(".download-{}", nonce()?));
            download(&url, &directory)?;
            let staged = store.stage(&directory);
            let _ = fs::remove_dir_all(directory);
            (staged?, PackageSource::Public { manifest_url: url })
        } else {
            (
                store.stage(&local.unwrap())?,
                source_url.map_or(PackageSource::Local, |manifest_url| PackageSource::Public {
                    manifest_url,
                }),
            )
        }
    };
    if let Some(args) = &reconnect {
        let connection = crate::connections::Connection::parse(args)?;
        let core = remote_build(&connection)?;
        if !package.manifest.build.reads_core(&core) {
            return Err(invalid(
                "selected companion cannot read the current remote core; installation and sessions retained",
            ));
        }
    }
    // A reconnect may fail to authenticate/reach the peer. Preserve the verified
    // installation and report pending; never restart a remote process to compensate.
    let mut receipt = store.install(package, source, adopt)?;
    if action == "install" {
        receipt.status = "installed".into();
        write_json(&store.receipt(), &receipt)?;
    }
    drop(lock);
    if let Some(connection) = reconnect {
        println!(
            "Companion installed; reconnecting using the selected saved SSH target. Remote sessions were not restarted."
        );
        #[cfg(windows)]
        {
            // The stable launcher must stay the sole foreground parent. A nested
            // worker could not hand its later update exit back to that launcher.
            request_restart(
                &crate::connections::Connection::parse(&connection)?,
                None,
                &receipt.attempt,
            )?;
            return Ok(true);
        }
        #[cfg(unix)]
        {
            let status = Command::new(&receipt.current.executable)
                .args(connection)
                .env("FLERE_CONNECT_UPDATE_ACK", &receipt.attempt)
                .status()?;
            if !status.success() {
                return Err(io::Error::other(
                    "companion installed but reconnect failed; package retained and remote sessions were not restarted",
                ));
            }
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&receipt).map_err(io::Error::other)?
        );
    }
    Ok(true)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    // Direct execution of fresh fixture scripts can serialize inside macOS
    // interpreter startup after spawn succeeds. Keep independent fixtures from
    // exhausting inspect's real deadline while retaining direct-exec coverage.
    #[cfg(target_os = "macos")]
    static SCRIPT_FIXTURES: Mutex<()> = Mutex::new(());

    struct Fixture {
        root: PathBuf,
        store: Store,
        #[cfg(target_os = "macos")]
        _scripts: std::sync::MutexGuard<'static, ()>,
    }
    impl Fixture {
        fn new() -> Self {
            #[cfg(target_os = "macos")]
            let scripts = SCRIPT_FIXTURES
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!("cu-{}", nonce().unwrap()));
            private_dir(&root).unwrap();
            let store = Store {
                root: root.join(".local/share/flere/install"),
                bin: root.join(".local/bin"),
                windows_launchers: false,
            };
            Self {
                root,
                store,
                #[cfg(target_os = "macos")]
                _scripts: scripts,
            }
        }
        fn package(&self, name: &str) -> PathBuf {
            let directory = self.root.join(name);
            private_dir(&directory).unwrap();
            let build: BuildMetadata = serde_json::from_str(crate::build_info::json()).unwrap();
            let json = serde_json::to_string(&build).unwrap();
            let binary = directory.join(COMPONENT);
            fs::write(&binary, format!("#!/bin/sh\nif [ \"$#\" -eq 2 ] && [ \"$1\" = _launcher-check ]; then printf 'launcher-compatible-v1\\n'; exit 0; fi\n[ \"$#\" -eq 1 ] && [ \"$1\" = --build-info ] || exit 2\nprintf '%s\\n' '{}'\n# {name}\n", json.replace('\'', "'\\''"))).unwrap();
            executable(&binary).unwrap();
            let manifest = Manifest {
                schema_version: 1,
                build,
                payload: manifest::Payload {
                    file_name: COMPONENT.into(),
                    download_file: Some(format!("{COMPONENT}-{}", crate::build_info::TARGET)),
                    bytes: fs::metadata(&binary).unwrap().len(),
                    sha256: sha256(&binary).unwrap(),
                },
                source: None,
            };
            write_json(&directory.join("manifest.json"), &manifest).unwrap();
            directory
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            if std::thread::panicking() {
                eprintln!("Retained companion update fixture {}", self.root.display());
            } else {
                fs::remove_dir_all(&self.root).unwrap();
            }
        }
    }
    #[test]
    fn windows_launcher_names_share_one_receipt_and_stay_stable_through_update_and_rollback() {
        // Exercise the actual Windows filesystem layout/transaction on a Unix
        // fixture with harmless executable stand-ins. This does not establish
        // Windows console ownership, ACL, or in-use executable behavior.
        let mut f = Fixture::new();
        f.store.windows_launchers = true;
        let _lock = f.store.lock().unwrap();
        let first = f.store.stage(&f.package("windows-first")).unwrap();
        let second = f.store.stage(&f.package("windows-second")).unwrap();
        let original = f
            .store
            .install(first.clone(), PackageSource::Local, false)
            .unwrap();
        assert_eq!(original.destination, f.store.bin.join("flere.exe"));
        assert_eq!(
            original.aliases,
            vec![f.store.bin.join("flere-connect.exe")]
        );
        assert_eq!(
            original.current.executable.file_name().unwrap(),
            "flere-connect.exe"
        );
        assert_eq!(original.component, "flere-connect");
        assert_eq!(
            original.launcher_sha256.as_ref(),
            Some(&first.manifest.payload.sha256)
        );
        for path in std::iter::once(original.destination.clone()).chain(original.aliases.clone()) {
            assert!(f.store.is_launcher_path(&path).unwrap());
            assert_eq!(sha256(&path).unwrap(), first.manifest.payload.sha256);
        }
        assert!(!f.store.is_launcher_path(&first.executable).unwrap());
        let updated = f
            .store
            .install(second.clone(), PackageSource::Local, false)
            .unwrap();
        assert_eq!(updated.previous.as_ref().unwrap().id, first.id);
        assert_eq!(updated.current.id, second.id);
        assert_ne!(updated.attempt, original.attempt);
        assert_eq!(updated.aliases, original.aliases);
        assert_eq!(updated.launcher_sha256, original.launcher_sha256);
        let repeated = f
            .store
            .install(second, PackageSource::Local, false)
            .unwrap();
        assert_eq!(repeated.attempt, updated.attempt);
        let rollback = f
            .store
            .install(first.clone(), PackageSource::Local, false)
            .unwrap();
        assert_eq!(rollback.previous.as_ref().unwrap().id, updated.current.id);
        assert_eq!(rollback.launcher_sha256, original.launcher_sha256);
        assert_eq!(f.store.status().unwrap().unwrap().current.id, first.id);
        for path in std::iter::once(rollback.destination).chain(rollback.aliases) {
            assert_eq!(sha256(&path).unwrap(), first.manifest.payload.sha256);
        }
    }
    #[test]
    fn windows_alias_conflicts_are_never_adopted_or_overwritten() {
        for linked in [false, true] {
            let mut f = Fixture::new();
            f.store.windows_launchers = true;
            let source = f.package("windows-conflict");
            let prepared = prepare_in(&f.store, source.to_str().unwrap()).unwrap();
            private_dir(&f.store.bin).unwrap();
            let alias = f.store.aliases().pop().unwrap();
            if linked {
                symlink(f.root.join("unrelated-missing-file"), &alias).unwrap();
            } else {
                fs::write(&alias, b"unmanaged companion alias").unwrap();
            }
            assert!(
                install_prepared_in(&f.store, &prepared, true)
                    .unwrap_err()
                    .to_string()
                    .contains("unmanaged flere-connect.exe alias")
            );
            for adopt in [false, true] {
                let _lock = f.store.lock().unwrap();
                assert!(
                    f.store
                        .install(prepared.package.clone(), PackageSource::Local, adopt)
                        .unwrap_err()
                        .to_string()
                        .contains("unmanaged flere-connect.exe alias")
                );
            }
            if linked {
                assert_eq!(
                    fs::read_link(&alias).unwrap(),
                    f.root.join("unrelated-missing-file")
                );
            } else {
                assert_eq!(fs::read(&alias).unwrap(), b"unmanaged companion alias");
            }
            assert!(!f.store.destination().exists());
            assert!(!f.store.receipt().exists());
            assert!(!f.store.journal().exists());
        }
    }
    #[test]
    fn interrupted_windows_alias_publication_recovers_without_replacing_a_foreign_file() {
        let mut f = Fixture::new();
        f.store.windows_launchers = true;
        let _lock = f.store.lock().unwrap();
        let package = f.store.stage(&f.package("windows-interrupted")).unwrap();
        let receipt = f
            .store
            .install(package, PackageSource::Local, false)
            .unwrap();
        let alias = receipt.aliases[0].clone();
        fs::remove_file(&alias).unwrap();
        fs::remove_file(f.store.receipt()).unwrap();
        write_json(
            &f.store.journal(),
            &Pending {
                next: receipt.clone(),
                previous_sha256: None,
            },
        )
        .unwrap();
        f.store.recover().unwrap();
        assert_eq!(f.store.status().unwrap().unwrap().attempt, receipt.attempt);
        assert_eq!(
            sha256(&alias).unwrap(),
            receipt.launcher_sha256.clone().unwrap()
        );
        assert!(!f.store.journal().exists());

        // Replacing an alias after publication invalidates managed status. A
        // pending recovery must retain the foreign bytes and the old receipt.
        let before = fs::read(f.store.receipt()).unwrap();
        fs::remove_file(&alias).unwrap();
        fs::write(&alias, b"different owner").unwrap();
        assert!(f.store.status().is_err());
        let mut next = receipt.clone();
        next.attempt = nonce().unwrap();
        write_json(
            &f.store.journal(),
            &Pending {
                next,
                previous_sha256: None,
            },
        )
        .unwrap();
        assert!(
            f.store
                .recover()
                .unwrap_err()
                .to_string()
                .contains("alias differs")
        );
        assert_eq!(fs::read(&alias).unwrap(), b"different owner");
        assert_eq!(fs::read(f.store.receipt()).unwrap(), before);
        assert_eq!(
            sha256(&f.store.destination()).unwrap(),
            receipt.launcher_sha256.unwrap()
        );
        assert!(f.store.journal().exists());
    }
    #[test]
    fn package_install_preserves_previous_and_shares_unix_core_receipt_shape() {
        let f = Fixture::new();
        assert!(f.store.status().unwrap().is_none());
        assert!(!f.store.root.exists());
        assert!(!f.store.bin.exists());
        let _lock = f.store.lock().unwrap();
        let first = f.store.stage(&f.package("first")).unwrap();
        let second = f.store.stage(&f.package("second")).unwrap();
        assert_eq!(first.manifest.build, second.manifest.build);
        assert_ne!(first.id, second.id);
        let a = f
            .store
            .install(first.clone(), PackageSource::Local, false)
            .unwrap();
        assert_eq!(
            fs::metadata(&f.store.bin).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let b = f
            .store
            .install(second.clone(), PackageSource::Local, false)
            .unwrap();
        assert_eq!(b.previous.unwrap().id, first.id);
        assert_ne!(a.attempt, b.attempt);
        let repeated = f
            .store
            .install(second, PackageSource::Local, false)
            .unwrap();
        assert_eq!(repeated.attempt, b.attempt);
        let restored = f
            .store
            .install(first.clone(), PackageSource::Local, false)
            .unwrap();
        assert_eq!(
            sha256(&f.store.destination()).unwrap(),
            first.manifest.payload.sha256
        );
        let json: Value = read_json(&f.store.receipt()).unwrap();
        assert_eq!(json["owner"], "flere");
        assert!(json.get("launcher_sha256").is_none());
        assert_eq!(
            json["current"]["executable"],
            restored.current.executable.to_str().unwrap()
        );
        assert!(f.store.status().unwrap().is_some());
        assert!(!f.root.join(".local/state").exists());
    }
    #[test]
    fn install_rejects_existing_writable_and_linked_bin_without_changing_them() {
        for linked in [false, true] {
            let f = Fixture::new();
            let _lock = f.store.lock().unwrap();
            let package = f.store.stage(&f.package("candidate")).unwrap();
            let target = if linked {
                let target = f.root.join("linked-bin");
                private_dir(&target).unwrap();
                symlink(&target, &f.store.bin).unwrap();
                target
            } else {
                private_dir(&f.store.bin).unwrap();
                fs::set_permissions(&f.store.bin, fs::Permissions::from_mode(0o775)).unwrap();
                f.store.bin.clone()
            };
            assert!(
                f.store
                    .install(package, PackageSource::Local, false)
                    .unwrap_err()
                    .to_string()
                    .contains("bin directory is linked or foreign-writable")
            );
            assert!(!f.store.receipt().exists());
            assert!(fs::read_dir(&target).unwrap().next().is_none());
            if linked {
                assert_eq!(fs::read_link(&f.store.bin).unwrap(), target);
            } else {
                assert_eq!(
                    fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                    0o775
                );
            }
        }
    }
    #[test]
    fn prepared_install_rejects_intervening_managed_install_and_aba() {
        let f = Fixture::new();
        let first_path = f.package("baseline-first");
        let candidate_path = f.package("baseline-candidate");
        let concurrent_path = f.package("baseline-concurrent");
        let first = {
            let _lock = f.store.lock().unwrap();
            let package = f.store.stage(&first_path).unwrap();
            f.store
                .install(package, PackageSource::Local, false)
                .unwrap()
        };
        let prepared = prepare_in(&f.store, candidate_path.to_str().unwrap()).unwrap();
        let mut legacy = serde_json::to_value(&prepared).unwrap();
        legacy.as_object_mut().unwrap().remove("baseline");
        assert!(serde_json::from_value::<Prepared>(legacy).is_err());
        let concurrent = {
            let _lock = f.store.lock().unwrap();
            let package = f.store.stage(&concurrent_path).unwrap();
            f.store
                .install(package, PackageSource::Local, false)
                .unwrap()
        };
        let before = fs::read(f.store.receipt()).unwrap();
        assert!(
            install_prepared_in(&f.store, &prepared, true)
                .unwrap_err()
                .to_string()
                .contains("changed after preparation")
        );
        assert_eq!(fs::read(f.store.receipt()).unwrap(), before);
        assert_eq!(
            sha256(&f.store.destination()).unwrap(),
            concurrent.current.manifest.payload.sha256
        );
        let restored = {
            let _lock = f.store.lock().unwrap();
            f.store
                .install(first.current.clone(), PackageSource::Local, false)
                .unwrap()
        };
        assert_eq!(restored.current.id, first.current.id);
        assert_ne!(restored.attempt, first.attempt);
        assert!(
            install_prepared_in(&f.store, &prepared, true)
                .unwrap_err()
                .to_string()
                .contains("changed after preparation")
        );

        let fresh = prepare_in(&f.store, candidate_path.to_str().unwrap()).unwrap();
        let installed = install_prepared_in(&f.store, &fresh, false).unwrap();
        let no_op = prepare_in(&f.store, candidate_path.to_str().unwrap()).unwrap();
        let repeated = install_prepared_in(&f.store, &no_op, false).unwrap();
        assert_eq!(repeated.attempt, installed.attempt);
        assert_eq!(repeated.status, installed.status);
        let before = fs::read(f.store.receipt()).unwrap();
        assert!(
            verify_applied_in(&f.store, &fresh, &nonce().unwrap())
                .unwrap_err()
                .to_string()
                .contains("attempt/package changed")
        );
        // These harmless executables intentionally report this same build stamp,
        // but the test process is not executing their retained package path.
        assert!(
            verify_applied_in(&f.store, &fresh, &installed.attempt)
                .unwrap_err()
                .to_string()
                .contains("exact retained candidate executable/build")
        );
        assert_eq!(fs::read(f.store.receipt()).unwrap(), before);
    }
    #[test]
    fn prepared_install_freezes_missing_and_manual_adoption_destinations() {
        let f = Fixture::new();
        let candidate_path = f.package("manual-candidate");
        let first_manual = f.package("manual-before");
        let second_manual = f.package("manual-after");
        let missing = prepare_in(&f.store, candidate_path.to_str().unwrap()).unwrap();
        assert_eq!(missing.baseline, InstallationBaseline::Missing);
        private_dir(&f.store.bin).unwrap();
        fs::copy(first_manual.join(COMPONENT), f.store.destination()).unwrap();
        let first_bytes = fs::read(f.store.destination()).unwrap();
        assert!(
            install_prepared_in(&f.store, &missing, true)
                .unwrap_err()
                .to_string()
                .contains("changed after preparation")
        );
        assert_eq!(fs::read(f.store.destination()).unwrap(), first_bytes);
        assert!(!f.store.receipt().exists());

        let manual = prepare_in(&f.store, candidate_path.to_str().unwrap()).unwrap();
        assert!(matches!(
            manual.baseline,
            InstallationBaseline::Manual { .. }
        ));
        fs::copy(second_manual.join(COMPONENT), f.store.destination()).unwrap();
        let second_hash = sha256(&f.store.destination()).unwrap();
        assert!(
            install_prepared_in(&f.store, &manual, true)
                .unwrap_err()
                .to_string()
                .contains("changed after preparation")
        );
        assert_eq!(sha256(&f.store.destination()).unwrap(), second_hash);
        assert!(!f.store.receipt().exists());

        let fresh = prepare_in(&f.store, candidate_path.to_str().unwrap()).unwrap();
        let adopted = install_prepared_in(&f.store, &fresh, true).unwrap();
        assert_eq!(
            adopted.previous.unwrap().manifest.payload.sha256,
            second_hash
        );
        assert_eq!(adopted.current.id, fresh.package.id);
    }
    #[test]
    fn corruption_identity_collision_links_and_fifo_are_rejected_before_replacement() {
        let f = Fixture::new();
        let _lock = f.store.lock().unwrap();
        let directory = f.package("first");
        let first = f.store.stage(&directory).unwrap();
        f.store
            .install(first.clone(), PackageSource::Local, false)
            .unwrap();
        let original = fs::read(f.store.destination()).unwrap();
        let mut forged: Manifest = read_json(&directory.join("manifest.json")).unwrap();
        forged.build.build_id = "forged-stamp-with-identical-payload".into();
        write_json(&directory.join("manifest.json"), &forged).unwrap();
        assert!(
            f.store
                .stage(&directory)
                .unwrap_err()
                .to_string()
                .contains("collision")
        );
        let corrupt = f.package("corrupt");
        fs::write(corrupt.join(COMPONENT), b"bad").unwrap();
        assert!(f.store.stage(&corrupt).is_err());
        let linked = f.package("linked");
        fs::remove_file(linked.join(COMPONENT)).unwrap();
        symlink(f.store.destination(), linked.join(COMPONENT)).unwrap();
        assert!(f.store.stage(&linked).is_err());
        let fifo = f.root.join("fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        let start = Instant::now();
        assert!(regular(&fifo, 1024).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(fs::read(f.store.destination()).unwrap(), original);
    }
    #[test]
    fn installation_lock_release_ignores_inherited_descriptors() {
        let f = Fixture::new();
        let lock = f.store.lock().unwrap();
        // A duplicate shares the open file description, like a child that has
        // inherited the descriptor during a concurrent spawn but not yet exec'd.
        let inherited = lock.file.try_clone().unwrap();
        assert!(f.store.lock().is_err());
        drop(lock);
        let next = f.store.lock().unwrap();
        drop(inherited);
        assert!(f.store.lock().is_err());
        drop(next);
        assert!(f.store.lock().is_ok());
    }
    #[test]
    fn lock_adoption_and_pending_recovery_preserve_exact_payloads() {
        let f = Fixture::new();
        let lock = f.store.lock().unwrap();
        assert!(f.store.lock().is_err());
        drop(lock);
        let _lock = f.store.lock().unwrap();
        let old_source = f.package("manual");
        private_dir(&f.store.bin).unwrap();
        fs::copy(old_source.join(COMPONENT), f.store.destination()).unwrap();
        let next = f.store.stage(&f.package("next")).unwrap();
        assert!(
            f.store
                .install(next.clone(), PackageSource::Local, false)
                .is_err()
        );
        let adopted = f
            .store
            .install(next.clone(), PackageSource::Local, true)
            .unwrap();
        assert!(adopted.previous.is_some());
        let final_package = f.store.stage(&f.package("final")).unwrap();
        let mut planned = adopted.clone();
        planned.previous = Some(next.clone());
        planned.current = final_package.clone();
        planned.attempt = nonce().unwrap();
        write_json(
            &f.store.journal(),
            &Pending {
                next: planned.clone(),
                previous_sha256: Some(next.manifest.payload.sha256),
            },
        )
        .unwrap();
        let copy = f.store.bin.join("staged-copy");
        fs::copy(&final_package.executable, &copy).unwrap();
        fs::rename(copy, f.store.destination()).unwrap();
        assert!(f.store.status().is_err());
        f.store.recover().unwrap();
        assert_eq!(f.store.status().unwrap().unwrap().attempt, planned.attempt);
    }
    #[test]
    fn system_digest_and_bounded_subprocesses_have_observable_limits() {
        let f = Fixture::new();
        let vector = f.root.join("abc");
        fs::write(&vector, b"abc").unwrap();
        assert_eq!(
            sha256(&vector).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let output = run(
            Command::new("/bin/sh")
                .args(["-c", "while :; do printf 0123456789; done"])
                .stdin(Stdio::null()),
            512,
            Duration::from_secs(2),
        );
        assert!(output.unwrap_err().to_string().contains("bound"));
        let start = Instant::now();
        let timeout = run(
            Command::new("/bin/sh")
                .args(["-c", "exec sleep 10"])
                .stdin(Stdio::null()),
            512,
            Duration::from_millis(100),
        );
        assert_eq!(timeout.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn saved_restart_arguments_keep_metacharacters_literal_and_drop_one_shot_image() {
        let args = [
            "user@host",
            "--remote",
            "C:/Tools/100% ! & ^ ' /flere",
            "--state",
            "work space/%!&^\\state",
            "--ssh",
            "C:/Open SSH/ssh.exe",
            "--image",
            "once %!&^.png",
        ]
        .map(String::from);
        let frozen = crate::connections::Connection::parse(&args).unwrap().args();
        assert!(!frozen.iter().any(|a| a == "--image" || a.contains("once")));
        assert!(frozen.iter().any(|a| a == "C:/Tools/100% ! & ^ ' /flere"));
        assert!(frozen.iter().any(|a| a == "work space/%!&^\\state"));
        assert_eq!(
            crate::connections::Connection::parse(&frozen)
                .unwrap()
                .args(),
            frozen
        );
    }
    #[test]
    fn restart_records_are_bound_to_install_attempt_and_independent_launcher() {
        let f = Fixture::new();
        let _lock = f.store.lock().unwrap();
        let package = f.store.stage(&f.package("restart")).unwrap();
        let receipt = f
            .store
            .install(package, PackageSource::Local, false)
            .unwrap();
        let first = nonce().unwrap();
        let second = nonce().unwrap();
        assert_ne!(first, second);
        let connection = crate::connections::Connection::parse(&[
            "user@exact-host".into(),
            "--state".into(),
            "state with %!&^ spaces".into(),
        ])
        .unwrap()
        .args();
        let path = f
            .store
            .root
            .join(format!("restart-{}-{first}.json", receipt.attempt));
        let mut record = RestartRecord {
            schema_version: 1,
            attempt: receipt.attempt.clone(),
            launch_id: first.clone(),
            connection: connection.clone(),
            resume: Some(nonce().unwrap()),
        };
        write_json(&path, &record).unwrap();
        assert_eq!(
            restart_record(&f.store, &receipt, &first)
                .unwrap()
                .connection,
            connection
        );
        assert!(restart_record(&f.store, &receipt, &second).is_err());
        record.launch_id = second;
        write_json(&path, &record).unwrap();
        assert!(restart_record(&f.store, &receipt, &first).is_err());
        record.launch_id = first.clone();
        record.attempt = nonce().unwrap();
        write_json(&path, &record).unwrap();
        assert!(restart_record(&f.store, &receipt, &first).is_err());
        record.attempt = receipt.attempt.clone();
        record
            .connection
            .extend(["--image".into(), "never-replay.png".into()]);
        write_json(&path, &record).unwrap();
        assert!(restart_record(&f.store, &receipt, &first).is_err());
        record.connection = connection;
        record.resume = Some("../foreign-plan".into());
        write_json(&path, &record).unwrap();
        assert!(restart_record(&f.store, &receipt, &first).is_err());
    }
}
