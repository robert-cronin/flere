//! State-owned immutable Git documents. The entry lock orders opens against
//! cleanup; durable reservations bridge creation/spawn to a saved tab checkpoint.
//! Unknown manifests and reservations surviving a crash are retained, never aged out.
use crate::{os, wire::invalid};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DIRECTORY: &str = "git-diffs-v1";
pub const RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const BATCH: usize = 16;
const MAX_DOCUMENT: u64 = 1024 * 1024;
const MAX_METADATA: u64 = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    device: u64,
    inode: u64,
}
impl Identity {
    fn of(meta: &fs::Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema_version: u32,
    state: PathBuf,
    state_identity: Identity,
    cache_identity: Identity,
    nonce: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    name: String,
    bytes: u64,
    sha256: String,
    identity: Identity,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    owner: String,
    identity: Identity,
    files: Vec<Document>,
    unused_since: Option<u64>,
    checked_at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn directory(path: &Path, create: bool) -> io::Result<fs::Metadata> {
    if create {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir() || m.uid() != os::uid() || m.mode() & 0o077 != 0 {
        return Err(invalid(
            "Git cache directory must be private, owned and not a symlink",
        ));
    }
    Ok(m)
}
fn regular(path: &Path, max: u64) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file()
        || m.uid() != os::uid()
        || m.mode() & 0o077 != 0
        || m.nlink() != 1
        || m.len() > max
    {
        return Err(invalid(
            "Git cache file is not a bounded private owned file",
        ));
    }
    Ok(f)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path, max: u64) -> io::Result<T> {
    let mut bytes = Vec::new();
    regular(path, max)?.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(invalid("Git cache metadata exceeds its limit"));
    }
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}
fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    crate::workspace::atomic_write(path, &serde_json::to_vec(value).map_err(io::Error::other)?)
}
struct CacheLock {
    file: File,
}
impl Drop for CacheLock {
    fn drop(&mut self) {
        // A concurrent child can briefly inherit this file description before
        // exec. Release the transaction explicitly so its copy cannot prolong
        // cache ownership after our guard ends.
        let _ = self.file.unlock();
    }
}

fn lock(path: &Path, create: bool) -> io::Result<CacheLock> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file() || m.uid() != os::uid() || m.mode() & 0o077 != 0 || m.nlink() != 1 {
        return Err(invalid("Git cache lock is not a private owned file"));
    }
    os::lock(f.as_raw_fd())?;
    Ok(CacheLock { file: f })
}
fn worker_lock(path: &Path, create: bool) -> io::Result<CacheLock> {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        match lock(path, create) {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < end => {
                std::thread::sleep(Duration::from_millis(10))
            }
            result => return result,
        }
    }
}

struct Namespace {
    root: PathBuf,
    owner: Owner,
}
impl Namespace {
    fn open(state: &Path, create: bool) -> io::Result<Self> {
        let state = fs::canonicalize(state)?;
        let sm = directory(&state, false)?;
        let root = state.join(DIRECTORY);
        let cm = directory(&root, create)?;
        let _guard = if create {
            Some(worker_lock(&root.join(".owner-lock"), true)?)
        } else {
            None
        };
        let owner_path = root.join("owner.json");
        if create && !owner_path.try_exists()? {
            write_json(
                &owner_path,
                &Owner {
                    schema_version: 1,
                    state: state.clone(),
                    state_identity: Identity::of(&sm),
                    cache_identity: Identity::of(&cm),
                    nonce: os::nonce()?,
                },
            )?;
        }
        let owner: Owner = read_json(&owner_path, MAX_METADATA)?;
        if owner.schema_version != 1
            || owner.state != state
            || owner.state_identity != Identity::of(&sm)
            || owner.cache_identity != Identity::of(&cm)
            || !token(&owner.nonce)
        {
            return Err(invalid(
                "Git cache ownership changed or is unknown; snapshots retained",
            ));
        }
        Ok(Self { root, owner })
    }
    fn validate(&self) -> io::Result<()> {
        let current = Self::open(&self.owner.state, false)?;
        if current.owner.nonce != self.owner.nonce || current.root != self.root {
            return Err(invalid("Git cache ownership changed; snapshots retained"));
        }
        Ok(())
    }
    fn manifest(&self, entry: &Path) -> io::Result<Manifest> {
        self.validate()?;
        let m: Manifest = read_json(&entry.join("manifest.json"), MAX_METADATA)?;
        if m.schema_version != 1
            || m.owner != self.owner.nonce
            || m.identity != Identity::of(&directory(entry, false)?)
            || !(1..=2).contains(&m.files.len())
            || m.files.iter().any(|f| {
                !basename(&f.name)
                    || f.bytes > MAX_DOCUMENT
                    || f.sha256.len() != 64
                    || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            })
            || m.files
                .iter()
                .map(|f| &f.name)
                .collect::<BTreeSet<_>>()
                .len()
                != m.files.len()
        {
            return Err(invalid(
                "Git cache manifest is unknown or changed; snapshots retained",
            ));
        }
        Ok(m)
    }
}
fn token(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn basename(s: &str) -> bool {
    !s.is_empty() && s.len() <= 240 && !s.starts_with('.') && !s.contains('/') && !s.contains('\0')
}

/// A durable open pin. Drop intentionally retains it: a failed checkpoint or
/// refresh/crash must never turn a running, unsaved editor into an unused file.
pub struct Reservation {
    path: PathBuf,
}
impl Reservation {
    pub fn protects(&self, path: &Path) -> bool {
        self.path.parent() == path.parent()
    }
    pub fn release(self) {
        let _ = fs::remove_file(&self.path);
    }
}
/// A UI worker owns this until its result is discarded or the open command has
/// returned. The server has its own independent pin before starting an editor.
pub struct Lease {
    reservation: Option<Reservation>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(r) = self.reservation.take() {
            r.release();
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    schema_version: u32,
    pid: u32,
    start: String,
    created_at: u64,
    foreign: bool,
}
fn reserve(entry: &Path, m: &mut Manifest, permanent: bool) -> io::Result<Reservation> {
    let path = entry.join(format!(
        "{}{}",
        if permanent { "foreign-" } else { "lease-" },
        os::nonce()?
    ));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let pin = Pin {
        schema_version: 1,
        pid: std::process::id(),
        start: os::child_identity(std::process::id())
            .map(|p| p.1)
            .unwrap_or_default(),
        created_at: now(),
        foreign: permanent,
    };
    f.write_all(&serde_json::to_vec(&pin).map_err(io::Error::other)?)?;
    f.sync_all()?;
    // Reset even when an earlier unused period was nearly expired.
    m.unused_since = None;
    m.checked_at = now();
    write_json(&entry.join("manifest.json"), m)?;
    File::open(entry)?.sync_all()?;
    Ok(Reservation { path })
}

/// Nonblocking server-side reservation, acquired after path resolution and
/// before reuse/spawn. Foreign-state references get a permanent conservative pin.
pub fn reserve_open(state: &Path, path: &Path) -> io::Result<Option<Reservation>> {
    let Some(entry) = path.parent() else {
        return Ok(None);
    };
    let Some(root) = entry
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == DIRECTORY))
    else {
        return Ok(None);
    };
    let Some(owner_state) = root.parent() else {
        return Err(invalid("invalid Git cache path"));
    };
    let ns = Namespace::open(owner_state, false)?;
    let _guard = lock(&entry.join(".lock"), false)?;
    let mut manifest = ns.manifest(entry)?;
    let doc = manifest
        .files
        .iter()
        .find(|f| path.file_name().is_some_and(|n| n == f.name.as_str()))
        .ok_or_else(|| invalid("file is not a managed Git snapshot"))?;
    let meta = regular(path, MAX_DOCUMENT)?.metadata()?;
    if Identity::of(&meta) != doc.identity || meta.len() != doc.bytes {
        return Err(invalid("Git snapshot changed; retained for inspection"));
    }
    let foreign = fs::canonicalize(state)? != ns.owner.state;
    let reservation = reserve(entry, &mut manifest, foreign)?;
    if foreign {
        Ok(None)
    } else {
        Ok(Some(reservation))
    }
}

/// Worker-only creation. Exact content comparison preserves safe canonical-tab
/// reuse even if the non-cryptographic filename key ever collides.
pub(super) fn create(
    state: &Path,
    key: &str,
    documents: &[(&str, &[u8])],
) -> io::Result<(Vec<PathBuf>, Lease)> {
    if !basename(key)
        || !(1..=2).contains(&documents.len())
        || documents
            .iter()
            .any(|(n, b)| !basename(n) || b.len() as u64 > MAX_DOCUMENT)
    {
        return Err(invalid("invalid Git snapshot document"));
    }
    let ns = Namespace::open(state, true)?;
    let entry = ns.root.join(key);
    directory(&entry, true)?;
    let _guard = worker_lock(&entry.join(".lock"), true)?;
    ns.validate()?;
    let manifest_path = entry.join("manifest.json");
    let mut m = if manifest_path.try_exists()? {
        let m = ns.manifest(&entry)?;
        if m.files.len() != documents.len() {
            return Err(invalid("Git snapshot identity collision"));
        }
        for (name, bytes) in documents {
            let mut existing = Vec::new();
            regular(&entry.join(name), MAX_DOCUMENT)?
                .take(MAX_DOCUMENT + 1)
                .read_to_end(&mut existing)?;
            if existing != *bytes || !m.files.iter().any(|f| f.name == *name) {
                return Err(invalid(
                    "Git snapshot changed or identity collided; retain and inspect",
                ));
            }
        }
        m
    } else {
        // An interrupted/unknown creation is not silently adopted or overwritten.
        if fs::read_dir(&entry)?.any(|e| e.is_err() || e.is_ok_and(|e| e.file_name() != ".lock")) {
            return Err(invalid(
                "incomplete Git cache entry retained for inspection",
            ));
        }
        let mut files = Vec::new();
        for (name, bytes) in documents {
            let path = entry.join(name);
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o400)
                .open(&path)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            files.push(Document {
                name: (*name).into(),
                bytes: bytes.len() as u64,
                sha256: crate::install::sha256(&path)?,
                identity: Identity::of(&f.metadata()?),
            });
        }
        Manifest {
            schema_version: 1,
            owner: ns.owner.nonce.clone(),
            identity: Identity::of(&directory(&entry, false)?),
            files,
            unused_since: None,
            checked_at: now(),
        }
    };
    let reservation = reserve(&entry, &mut m, false)?;
    Ok((
        documents.iter().map(|(n, _)| entry.join(n)).collect(),
        Lease {
            reservation: Some(reservation),
        },
    ))
}

#[derive(Default, Serialize)]
pub struct Report {
    schema_version: u32,
    checked_at: u64,
    scanned: usize,
    reclaimed_entries: usize,
    reclaimed_bytes: u64,
    referenced: usize,
    reserved: usize,
    reservation_markers: usize,
    confirmed_live_reservation_markers: usize,
    unconfirmed_reservation_markers: usize,
    foreign_reference_markers: usize,
    awaiting_retention: usize,
    uncertain: usize,
    error: Option<String>,
    legacy: &'static str,
    reservation_policy: &'static str,
}
impl Report {
    fn new(time: u64) -> Self {
        Self {
            schema_version: 1,
            checked_at: time,
            legacy: "shared legacy snapshots retained; ownership untracked",
            reservation_policy: "all outstanding reservations retained, including crash/unknown owners",
            ..Self::default()
        }
    }
}
fn references(state: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let v: serde_json::Value = read_json(&state.join("workspaces.v2.json"), 8 * 1024 * 1024)?;
    if v.get("version").and_then(|v| v.as_u64()) != Some(7) {
        return Err(invalid(
            "unknown saved layout version; Git snapshots retained",
        ));
    }
    let workspaces = v
        .get("workspaces")
        .and_then(|v| v.as_array())
        .filter(|v| v.len() <= 65536)
        .ok_or_else(|| invalid("unknown saved layout shape; Git snapshots retained"))?;
    let mut paths = BTreeSet::new();
    for w in workspaces {
        let tabs = w
            .get("tabs")
            .and_then(|t| t.as_array())
            .filter(|v| v.len() <= 512)
            .ok_or_else(|| invalid("unknown saved tabs; Git snapshots retained"))?;
        for t in tabs {
            let kind = t
                .get("kind")
                .and_then(|v| v.as_str())
                .filter(|v| matches!(*v, "shell" | "agent" | "editor"))
                .ok_or_else(|| invalid("unknown saved tab kind; Git snapshots retained"))?;
            if kind == "editor" {
                let p = t
                    .get("path")
                    .and_then(|v| v.as_str())
                    .filter(|p| p.len() <= 8192 && Path::new(p).is_absolute())
                    .ok_or_else(|| invalid("unknown saved editor path; Git snapshots retained"))?;
                paths.insert(p.into());
            }
        }
    }
    Ok(paths)
}
fn visit(ns: &Namespace, entry: &Path, time: u64, report: &mut Report) -> io::Result<()> {
    let _guard = lock(&entry.join(".lock"), false)?;
    let mut m = ns.manifest(entry)?;
    // This read MUST happen under the same lock as reservation creation and
    // deletion. An earlier reference snapshot cannot authorize later deletion.
    let refs = references(&ns.owner.state)?;
    let names = fs::read_dir(entry)?
        .take(70)
        .map(|e| e.map(|e| e.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    if names.len() >= 70 {
        return Err(invalid("too many cache entry files; retained"));
    }
    let mut reservations = 0;
    for name in &names {
        let name = name
            .to_str()
            .ok_or_else(|| invalid("unknown cache entry file"))?;
        if name.starts_with("lease-") || name.starts_with("foreign-") {
            reservations += 1;
            match read_json::<Pin>(&entry.join(name), MAX_METADATA) {
                Ok(pin) if pin.schema_version == 1 => {
                    report.foreign_reference_markers += usize::from(pin.foreign);
                    if !pin.start.is_empty()
                        && os::child_identity(pin.pid).is_ok_and(|p| p.1 == pin.start)
                    {
                        report.confirmed_live_reservation_markers += 1;
                    } else {
                        report.unconfirmed_reservation_markers += 1;
                    }
                }
                _ => report.unconfirmed_reservation_markers += 1,
            }
        } else if name != "manifest.json"
            && name != ".lock"
            && !m.files.iter().any(|f| f.name == name)
        {
            return Err(invalid("unknown cache entry content retained"));
        }
    }
    let protected = m.files.iter().any(|f| refs.contains(&entry.join(&f.name)));
    if protected || reservations > 0 {
        report.referenced += usize::from(protected);
        report.reserved += usize::from(reservations > 0);
        report.reservation_markers += reservations;
        if m.unused_since.is_some() || m.checked_at != time {
            m.unused_since = None;
            m.checked_at = time;
            write_json(&entry.join("manifest.json"), &m)?;
        }
        return Ok(());
    }
    if time < m.checked_at || m.unused_since.is_none() {
        m.unused_since = Some(time);
    }
    m.checked_at = time;
    write_json(&entry.join("manifest.json"), &m)?;
    if time.saturating_sub(m.unused_since.unwrap_or(time)) < RETENTION_SECONDS {
        report.awaiting_retention += 1;
        return Ok(());
    }
    // Check the complete pair before unlinking either side. Never recursively
    // remove a directory or follow a symlink. Modified/extra artifacts stay put.
    for doc in &m.files {
        let path = entry.join(&doc.name);
        let meta = regular(&path, MAX_DOCUMENT)?.metadata()?;
        if Identity::of(&meta) != doc.identity
            || meta.len() != doc.bytes
            || meta.mode() & 0o222 != 0
            || crate::install::sha256(&path)? != doc.sha256
        {
            return Err(invalid("changed Git snapshot retained"));
        }
    }
    ns.validate()?;
    if Identity::of(&directory(entry, false)?) != m.identity {
        return Err(invalid("Git entry ownership changed"));
    }
    for doc in &m.files {
        fs::remove_file(entry.join(&doc.name))?;
    }
    fs::remove_file(entry.join("manifest.json"))?;
    fs::remove_file(entry.join(".lock"))?;
    fs::remove_dir(entry)?;
    File::open(&ns.root)?.sync_all()?;
    report.reclaimed_entries += 1;
    report.reclaimed_bytes += m.files.iter().map(|f| f.bytes).sum::<u64>();
    Ok(())
}

/// One worker and one queued request per supervisor; sixteen entries per batch,
/// requested at most once a minute. The iterator advances instead of always
/// retrying the first entries. No filesystem reads occur in the PTY tick.
pub struct Cleaner {
    request: mpsc::SyncSender<()>,
    last: Instant,
}
impl Cleaner {
    pub fn new(state: &Path) -> Self {
        let state = state.to_owned();
        let (request, receive) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut scan: Option<fs::ReadDir> = None;
            while receive.recv().is_ok() {
                if !state.join(DIRECTORY).exists() {
                    continue;
                }
                let mut report = Report::new(now());
                let result = (|| -> io::Result<()> {
                    let ns = Namespace::open(&state, false)?;
                    if scan.is_none() {
                        scan = Some(fs::read_dir(&ns.root)?);
                    }
                    for _ in 0..BATCH {
                        let Some(entry) = scan.as_mut().and_then(Iterator::next) else {
                            scan = None;
                            break;
                        };
                        let entry = entry?;
                        if !entry.file_type()?.is_dir() {
                            continue;
                        }
                        report.scanned += 1;
                        if let Err(e) = visit(&ns, &entry.path(), report.checked_at, &mut report) {
                            report.uncertain += 1;
                            report.error = Some(e.to_string());
                        }
                    }
                    write_json(&ns.root.join("status.json"), &report)
                })();
                if let Err(e) = result {
                    scan = None;
                    report.error = Some(e.to_string());
                }
                crate::diagnostics::record(
                    "git-cache-retention",
                    &format!(
                        "scanned={} reclaimed={} bytes={} reserved={} uncertain={} error={}",
                        report.scanned,
                        report.reclaimed_entries,
                        report.reclaimed_bytes,
                        report.reserved,
                        report.uncertain,
                        report.error.as_deref().unwrap_or("none")
                    ),
                );
            }
        });
        Self {
            request,
            last: Instant::now() - Duration::from_secs(60),
        }
    }
    pub fn tick(&mut self) {
        if self.last.elapsed() >= Duration::from_secs(60) && self.request.try_send(()).is_ok() {
            self.last = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests;
