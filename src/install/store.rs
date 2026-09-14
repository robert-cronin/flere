use super::{
    Activation, BuildMetadata, MAX_PAYLOAD, Manifest, PackageSource, atomic_json, inspect_binary,
    private_dir, read_json, regular, sha256, sync_dir,
};
use crate::{os, wire::invalid};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledPackage {
    pub id: String,
    pub manifest: Manifest,
    pub executable: PathBuf,
}

#[derive(Clone, Debug)]
pub struct StagedPackage {
    pub package: InstalledPackage,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallReceipt {
    pub schema_version: u64,
    pub owner: String,
    pub component: String,
    pub destination: PathBuf,
    pub current: InstalledPackage,
    pub previous: Option<InstalledPackage>,
    pub source: PackageSource,
    pub attempt: String,
    /// File installation is distinct from the running supervisor/frontend result.
    pub status: String,
    pub activation: Option<Activation>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    next: InstallReceipt,
    previous_sha256: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LocalSource {
    schema_version: u64,
    checkout: PathBuf,
}

struct InstallationLock {
    file: File,
}
impl Drop for InstallationLock {
    fn drop(&mut self) {
        // A concurrent child can inherit this file description before exec.
        // Closing only our descriptor would leave its flock held by that child;
        // explicitly end this transaction's ownership before closing the file.
        let _ = self.file.unlock();
    }
}

/// One lock covers the package store, stable command and receipt/journal. The
/// store is independent of supervisor --state directories and retains recovery
/// packages even when activation is partial or a later rollback is incompatible.
#[derive(Clone, Debug)]
pub struct Store {
    pub root: PathBuf,
    pub bin_dir: PathBuf,
    pub component: String,
}
impl Store {
    pub fn new(root: PathBuf, bin_dir: PathBuf, component: &str) -> io::Result<Self> {
        if !root.is_absolute()
            || !bin_dir.is_absolute()
            || !matches!(component, "flere" | "flere-connect")
        {
            return Err(invalid(
                "installer needs absolute directories and a known component",
            ));
        }
        Ok(Self {
            root,
            bin_dir,
            component: component.into(),
        })
    }
    pub fn for_user(component: &str) -> io::Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| invalid("HOME is required for per-user installation"))?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Self::new(
            data.join("flere/install"),
            home.join(".local/bin"),
            component,
        )
    }
    fn receipt_path(&self) -> PathBuf {
        self.root.join(format!("{}.json", self.component))
    }
    fn pending_path(&self) -> PathBuf {
        self.root.join(format!("{}.pending.json", self.component))
    }
    pub fn destination(&self) -> PathBuf {
        self.bin_dir.join(&self.component)
    }
    fn package_path(&self, manifest: &Manifest) -> PathBuf {
        self.root.join("packages").join(manifest.id())
    }

    fn check_local_source(checkout: &Path) -> io::Result<()> {
        if !checkout.is_absolute() || fs::canonicalize(checkout)? != checkout {
            return Err(invalid(
                "local update source must be a canonical checkout path",
            ));
        }
        let directory = fs::symlink_metadata(checkout)?;
        let script = checkout.join("scripts/dev");
        let executable = fs::symlink_metadata(&script)?;
        if !directory.is_dir()
            || directory.uid() != os::uid()
            || directory.permissions().mode() & 0o022 != 0
            || !executable.is_file()
            || executable.uid() != os::uid()
            || executable.permissions().mode() & 0o022 != 0
            || executable.permissions().mode() & 0o100 == 0
            || fs::canonicalize(&script)? != script
        {
            return Err(invalid(
                "registered checkout/dev script must be user-owned, executable and not linked or writable by others",
            ));
        }
        Ok(())
    }

    /// Explicitly opt in to executing this checkout's development updater. It is
    /// never inferred from a workspace cwd or embedded package provenance.
    pub fn configure_local_source(&self, checkout: &Path) -> io::Result<()> {
        if self.component != "flere" {
            return Err(invalid(
                "development source belongs to the core installation",
            ));
        }
        let checkout = fs::canonicalize(checkout)?;
        Self::check_local_source(&checkout)?;
        let _lock = self.lock()?;
        atomic_json(
            &self.root.join("local-source.json"),
            &LocalSource {
                schema_version: 1,
                checkout,
            },
        )
    }

    /// Read-only; validates the registered executable again before the UI offers
    /// it. A registration never grants authority to an arbitrary active directory.
    pub fn local_source(&self) -> io::Result<Option<PathBuf>> {
        let path = self.root.join("local-source.json");
        let source: LocalSource = match read_json(&path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let metadata = fs::symlink_metadata(&path)?;
        if source.schema_version != 1
            || metadata.uid() != os::uid()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(invalid("invalid private local-source registration"));
        }
        Self::check_local_source(&source.checkout)?;
        Ok(Some(source.checkout))
    }

    fn lock(&self) -> io::Result<InstallationLock> {
        private_dir(&self.root)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(self.root.join("install.lock"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != os::uid()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(invalid("invalid installation lock ownership"));
        }
        os::lock(file.as_raw_fd()).map_err(|error| {
            io::Error::new(error.kind(), "another installation owns the update lock")
        })?;
        Ok(InstallationLock { file })
    }

    fn validate_package(&self, package: &InstalledPackage) -> io::Result<()> {
        package.manifest.validate()?;
        if package.id != package.manifest.id()
            || package.manifest.build.component != self.component
            || package.manifest.build.target != crate::build_info::TARGET
            || package.executable != self.package_path(&package.manifest).join(&self.component)
        {
            return Err(invalid("installation package identity/path differs"));
        }
        package
            .manifest
            .verify(&self.package_path(&package.manifest))
    }

    fn validate_receipt(&self, receipt: &InstallReceipt) -> io::Result<()> {
        if receipt.schema_version != 1
            || receipt.owner != "flere"
            || receipt.component != self.component
            || receipt.destination != self.destination()
            || receipt.attempt.len() != 32
            || !receipt.attempt.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(invalid("unsupported or foreign installation receipt"));
        }
        self.validate_package(&receipt.current)?;
        if let Some(previous) = &receipt.previous {
            self.validate_package(previous)?;
        }
        Ok(())
    }

    /// Read-only: does not create directories, acquire a lock or repair a journal.
    pub fn status(&self) -> io::Result<Option<InstallReceipt>> {
        match fs::symlink_metadata(&self.root) {
            Ok(metadata)
                if !metadata.is_dir()
                    || metadata.uid() != os::uid()
                    || metadata.permissions().mode() & 0o077 != 0 =>
            {
                return Err(invalid("installation store ownership/permissions changed"));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            _ => {}
        }
        if let Ok(metadata) = fs::symlink_metadata(self.receipt_path())
            && (metadata.uid() != os::uid() || metadata.permissions().mode() & 0o077 != 0)
        {
            return Err(invalid(
                "installation receipt is not private and user-owned",
            ));
        }
        let receipt: InstallReceipt =
            match super::read_bounded_json(&self.receipt_path(), super::MAX_RECEIPT) {
                Ok(receipt) => receipt,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            };
        self.validate_receipt(&receipt)?;
        if sha256(&receipt.destination)? != receipt.current.manifest.payload.sha256 {
            return Err(invalid(
                "installed command differs from its managed receipt; no file was changed",
            ));
        }
        if self.pending_path().try_exists()? {
            return Err(invalid(
                "installation transaction is interrupted; run the explicit install/update again to reconcile",
            ));
        }
        Ok(Some(receipt))
    }

    fn recover(&self) -> io::Result<()> {
        let pending: Pending =
            match super::read_bounded_json(&self.pending_path(), super::MAX_RECEIPT) {
                Ok(pending) => pending,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
        self.validate_receipt(&pending.next)?;
        let actual = match sha256(&self.destination()) {
            Ok(digest) => Some(digest),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if actual.as_deref() == Some(&pending.next.current.manifest.payload.sha256) {
            // A crash after rename does not justify downgrading a possibly active
            // process. Finish the file receipt; activation remains explicitly pending.
            atomic_json(&self.receipt_path(), &pending.next)?;
        } else if actual != pending.previous_sha256 {
            return Err(invalid(
                "interrupted update destination changed externally; refusing automatic replacement",
            ));
        }
        fs::remove_file(self.pending_path())?;
        sync_dir(&self.root)
    }

    pub fn stage(&self, directory: &Path) -> io::Result<StagedPackage> {
        let _lock = self.lock()?;
        self.stage_locked(directory)
    }

    fn stage_locked(&self, directory: &Path) -> io::Result<StagedPackage> {
        let manifest = Manifest::load(directory)?;
        if manifest.build.component != self.component
            || manifest.build.target != crate::build_info::TARGET
        {
            return Err(invalid(
                "package component or target differs from this installation",
            ));
        }
        manifest.verify(directory)?;
        private_dir(&self.root.join("packages"))?;
        let destination = self.package_path(&manifest);
        if destination.try_exists()? {
            let retained = Manifest::load(&destination)?;
            if retained.build != manifest.build || retained.payload != manifest.payload {
                return Err(invalid("retained package identity collision"));
            }
            retained.verify(&destination)?;
            return Ok(StagedPackage {
                package: InstalledPackage {
                    id: retained.id(),
                    executable: destination.join(&self.component),
                    manifest: retained,
                },
            });
        }
        let temporary = self
            .root
            .join("packages")
            .join(format!(".stage-{}", os::nonce()?));
        private_dir(&temporary)?;
        let result = (|| {
            let binary = temporary.join(&self.component);
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&binary)?;
            let length = io::copy(
                &mut regular(&directory.join(&self.component), MAX_PAYLOAD)?.take(MAX_PAYLOAD + 1),
                &mut file,
            )?;
            if length != manifest.payload.bytes {
                return Err(invalid("package changed while staging"));
            }
            file.sync_all()?;
            drop(file);
            manifest.verify(&temporary)?;
            if inspect_binary(&binary)? != manifest.build {
                return Err(invalid(
                    "verified payload reports different embedded metadata",
                ));
            }
            atomic_json(&temporary.join("manifest.json"), &manifest)?;
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o500))?;
            fs::set_permissions(
                temporary.join("manifest.json"),
                fs::Permissions::from_mode(0o400),
            )?;
            sync_dir(&temporary)?;
            fs::rename(&temporary, &destination)?;
            sync_dir(&self.root.join("packages"))?;
            Ok(StagedPackage {
                package: InstalledPackage {
                    id: manifest.id(),
                    executable: destination.join(&self.component),
                    manifest,
                },
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&temporary);
        }
        result
    }

    fn prepare_bin(&self) -> io::Result<()> {
        if !self.bin_dir.try_exists()? {
            // Only newly created directories get private permissions. Existing
            // user/package-manager paths still pass the ownership check below.
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&self.bin_dir)?;
        }
        let metadata = fs::symlink_metadata(&self.bin_dir)?;
        if !metadata.is_dir()
            || metadata.uid() != os::uid()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(invalid(
                "installation bin directory is not user-owned or is writable by others",
            ));
        }
        Ok(())
    }

    fn adopt(&self) -> io::Result<InstalledPackage> {
        // A symlink commonly denotes a package manager or another installation
        // owner. Never follow it or silently take ownership of its destination.
        let metadata = fs::symlink_metadata(self.destination())?;
        if !metadata.is_file()
            || metadata.uid() != os::uid()
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(invalid(
                "existing command is foreign-owned, linked or writable by others; use its installation manager",
            ));
        }
        let temporary = self.root.join(format!(".adopt-{}", os::nonce()?));
        let result = (|| {
            super::package(&self.destination(), &temporary, None)?;
            Ok(self.stage_locked(&temporary)?.package)
        })();
        let _ = fs::remove_dir_all(&temporary);
        result
    }

    fn replace(&self, next: &InstallReceipt, previous_sha256: Option<String>) -> io::Result<()> {
        self.validate_receipt(next)?;
        let temporary = self
            .bin_dir
            .join(format!(".{}-{}", self.component, os::nonce()?));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o700)
                .open(&temporary)?;
            io::copy(
                &mut regular(&next.current.executable, MAX_PAYLOAD)?.take(MAX_PAYLOAD + 1),
                &mut file,
            )?;
            file.sync_all()?;
            drop(file);
            if sha256(&temporary)? != next.current.manifest.payload.sha256 {
                return Err(invalid(
                    "staged installation copy failed SHA-256 verification",
                ));
            }
            fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
            atomic_json(
                &self.pending_path(),
                &Pending {
                    next: next.clone(),
                    previous_sha256: previous_sha256.clone(),
                },
            )?;
            // Recheck after staging: an external owner may have replaced the file
            // while the immutable package copy was being prepared.
            let actual = match sha256(&self.destination()) {
                Ok(hash) => Some(hash),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            if actual != previous_sha256 {
                return Err(invalid(
                    "installed command changed during update; replacement refused",
                ));
            }
            if previous_sha256.is_none() {
                // First installation never overwrites a command that appeared
                // after our last observation, including a noncooperating writer.
                // The temporary lives on the destination filesystem; an atomic
                // no-replace link fails safely where hard links are unsupported.
                fs::hard_link(&temporary, &next.destination)?;
                fs::remove_file(&temporary)?;
            } else {
                fs::rename(&temporary, &next.destination)?;
            }
            sync_dir(&self.bin_dir)?;
            atomic_json(&self.receipt_path(), next)?;
            fs::remove_file(self.pending_path())?;
            sync_dir(&self.root)
        })();
        let _ = fs::remove_file(&temporary);
        result
    }

    pub fn install(
        &self,
        staged: &StagedPackage,
        source: PackageSource,
        adopt_existing: bool,
    ) -> io::Result<InstallReceipt> {
        self.install_checked(staged, source, adopt_existing, false)
    }

    /// Bootstrap is permitted only while the destination is still absent. The
    /// guard shares the installation lock, so concurrent bootstrap clients cannot
    /// turn first installation into an implicit update or adoption.
    pub fn install_if_missing(
        &self,
        staged: &StagedPackage,
        source: PackageSource,
    ) -> io::Result<InstallReceipt> {
        self.install_checked(staged, source, false, true)
    }

    fn install_checked(
        &self,
        staged: &StagedPackage,
        source: PackageSource,
        adopt_existing: bool,
        if_missing: bool,
    ) -> io::Result<InstallReceipt> {
        let _lock = self.lock()?;
        self.recover()?;
        self.validate_package(&staged.package)?;
        self.prepare_bin()?;
        let current = self.status()?;
        if if_missing
            && (current.is_some()
                || match fs::symlink_metadata(self.destination()) {
                    Ok(_) => true,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                    Err(error) => return Err(error),
                })
        {
            return Err(invalid(
                "Flere is already installed; first installation refused; reconnect or choose an explicit update",
            ));
        }
        let previous = match current {
            Some(receipt) if receipt.current.id == staged.package.id => return Ok(receipt),
            Some(receipt) => Some(receipt.current),
            None if self.destination().try_exists()? => {
                if !adopt_existing {
                    return Err(invalid(
                        "existing command is unmanaged; explicit adoption is required to retain it before replacement",
                    ));
                }
                Some(self.adopt()?)
            }
            None => None,
        };
        let receipt = InstallReceipt {
            schema_version: 1,
            owner: "flere".into(),
            component: self.component.clone(),
            destination: self.destination(),
            current: staged.package.clone(),
            previous: previous.clone(),
            source,
            attempt: os::nonce()?,
            status: "installed_pending_activation".into(),
            activation: None,
        };
        self.replace(&receipt, previous.map(|p| p.manifest.payload.sha256))?;
        Ok(receipt)
    }

    pub fn rollback(&self, running: Option<&BuildMetadata>) -> io::Result<InstallReceipt> {
        let _lock = self.lock()?;
        self.recover()?;
        let current = self
            .status()?
            .ok_or_else(|| invalid("installation is not managed"))?;
        let previous = current
            .previous
            .ok_or_else(|| invalid("no verified previous package is retained"))?;
        if running.is_some_and(|build| !previous.manifest.build.accepts_runtime(build)) {
            return Err(invalid(
                "previous package cannot read current runtime handoff/state; installation and sessions retained",
            ));
        }
        let receipt = InstallReceipt {
            schema_version: 1,
            owner: "flere".into(),
            component: self.component.clone(),
            destination: self.destination(),
            current: previous,
            previous: Some(current.current.clone()),
            source: current.source,
            attempt: os::nonce()?,
            status: "installed_pending_activation".into(),
            activation: None,
        };
        self.replace(&receipt, Some(current.current.manifest.payload.sha256))?;
        Ok(receipt)
    }

    /// Restore files only after a proven pre-exec rejection. An unknown or
    /// successfully replaced process must never trigger an automatic downgrade.
    pub fn restore_rejected(
        &self,
        attempt: &str,
        mut activation: Activation,
    ) -> io::Result<InstallReceipt> {
        let _lock = self.lock()?;
        self.recover()?;
        let current = self
            .status()?
            .ok_or_else(|| invalid("installation is not managed"))?;
        if current.attempt != attempt || !activation.can_restore_installation {
            return Err(invalid(
                "installation changed or activation outcome does not permit automatic restoration",
            ));
        }
        let previous = current
            .previous
            .ok_or_else(|| invalid("no previous package is available for restoration"))?;
        let before = activation
            .before
            .as_ref()
            .ok_or_else(|| invalid("rejected update has no original runtime identity"))?;
        let actual = super::runtime_identity(&before.state)?;
        if actual.epoch != before.epoch
            || actual.pid != before.pid
            || actual.build != before.build
            || actual.sessions != before.sessions
            || !previous.manifest.build.accepts_runtime(&actual.build)
        {
            return Err(invalid(
                "runtime changed after rejection; installed files retained for explicit recovery",
            ));
        }
        activation.status = "restored_after_rejection".into();
        activation.supervisor = "retained".into();
        activation.initiating_frontend = "retained".into();
        activation.can_restore_installation = false;
        activation.detail = format!(
            "{}; prior installed package restored; existing runtime and sessions retained",
            activation.detail
        );
        let receipt = InstallReceipt {
            schema_version: 1,
            owner: "flere".into(),
            component: self.component.clone(),
            destination: self.destination(),
            current: previous,
            previous: Some(current.current.clone()),
            source: current.source,
            attempt: os::nonce()?,
            status: activation.status.clone(),
            activation: Some(activation),
        };
        self.replace(&receipt, Some(current.current.manifest.payload.sha256))?;
        Ok(receipt)
    }

    /// Do not overwrite the result of a later concurrent installation. The caller
    /// must pass the attempt that actually initiated this activation.
    pub fn record_activation(
        &self,
        attempt: &str,
        activation: Activation,
    ) -> io::Result<InstallReceipt> {
        let _lock = self.lock()?;
        self.recover()?;
        let mut receipt = self
            .status()?
            .ok_or_else(|| invalid("installation is not managed"))?;
        if receipt.attempt != attempt {
            return Err(invalid("installation changed while activation was running"));
        }
        receipt.status = activation.status.clone();
        receipt.activation = Some(activation);
        atomic_json(&self.receipt_path(), &receipt)?;
        Ok(receipt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn installation_lock_release_ignores_inherited_descriptors() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("core-install-lock-{}", os::nonce().unwrap()));
        private_dir(&root).unwrap();
        let f = Fixture(root);
        let store = Store::new(f.0.join("store"), f.0.join("bin"), "flere").unwrap();
        let lock = store.lock().unwrap();
        // A duplicate shares the open file description, like a concurrent child
        // between fork and exec. Its lifetime must not extend the transaction.
        let inherited = lock.file.try_clone().unwrap();
        assert!(store.lock().is_err());
        drop(lock);
        let next = store.lock().unwrap();
        drop(inherited);
        // Closing a stale duplicate must not release the next owner's lock.
        assert!(store.lock().is_err());
        drop(next);
        assert!(store.lock().is_ok());
    }
}
