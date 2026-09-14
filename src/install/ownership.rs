//! Read-only evidence for the installation containing the running executable.
//! A conventional pathname alone never establishes a package-manager owner.
use super::{InstallReceipt, Store, read_bounded_json};
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManagerUpgrade {
    pub manager: &'static str,
    /// Ownership proof is independent of whether a generic upgrade command exists.
    pub verified: bool,
    pub command: Option<String>,
    pub detail: &'static str,
}

/// `managed` comes from Store::status(), which verifies ownership and payload
/// hashes. A stale Cargo receipt must not take precedence over a Flere install.
pub(crate) fn current_manager_upgrade(managed: Option<&InstallReceipt>) -> Option<ManagerUpgrade> {
    let Ok(executable) = crate::os::executable_path() else {
        return Some(unresolved_executable());
    };
    for_installation(&executable, env!("CARGO_PKG_VERSION"), managed)
}

fn unresolved_executable() -> ManagerUpgrade {
    ManagerUpgrade {
        manager: "current executable",
        verified: false,
        command: None,
        detail: "The running executable is missing or cannot be resolved. Reopen Flere from its installed command before updating.",
    }
}

fn for_installation(
    executable: &Path,
    version: &str,
    managed: Option<&InstallReceipt>,
) -> Option<ManagerUpgrade> {
    let Ok(executable) = fs::canonicalize(executable) else {
        return Some(unresolved_executable());
    };
    if managed.is_some_and(|receipt| {
        [&receipt.destination, &receipt.current.executable]
            .into_iter()
            .any(|path| fs::canonicalize(path).ok().as_ref() == Some(&executable))
    }) {
        return None;
    }
    for_executable(&executable, version)
}

pub(crate) fn update_manager_guard() -> Option<ManagerUpgrade> {
    let managed = Store::for_user("flere")
        .and_then(|store| store.status())
        .ok()
        .flatten();
    current_manager_upgrade(managed.as_ref())
}

fn for_executable(executable: &Path, version: &str) -> Option<ManagerUpgrade> {
    let Ok(executable) = fs::canonicalize(executable) else {
        return Some(unresolved_executable());
    };
    if !executable.is_file() {
        return Some(unresolved_executable());
    }
    if let Some(owner) = manager::nix(&executable) {
        return Some(owner);
    }
    if executable.file_name()? != "flere" {
        return None;
    }
    if let Some(owner) = homebrew(&executable, version).or_else(|| cargo(&executable, version)) {
        return Some(owner);
    }
    system_package(&executable)
}

mod manager;
use manager::{cargo, homebrew, system_package};
#[cfg(test)]
use manager::{debian_owner, rpm_owner, rpm_upgrade};
fn receipt(path: &Path) -> Option<serde_json::Value> {
    read_bounded_json(path, 1024 * 1024).ok()
}
#[cfg(target_os = "linux")]
fn query(program: &str, args: &[&std::ffi::OsStr]) -> Option<String> {
    super::run(
        std::process::Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .env_remove("DPKG_ROOT")
            .env_remove("DPKG_ADMINDIR")
            .stdin(std::process::Stdio::null()),
        65536,
        std::time::Duration::from_secs(2),
    )
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())
}

use super::{BuildMetadata, inspect_binary, sha256};
use crate::remote_update::{CoreOwners, InstallationOwner, OwnerKind};
use std::{io, os::unix::fs::MetadataExt};

fn selected_executable(
    store: &Store,
    executable: &Path,
    build: &BuildMetadata,
) -> io::Result<InstallationOwner> {
    let executable = fs::canonicalize(executable)?;
    let text = executable
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("executable path cannot be represented safely"))?
        .to_owned();
    let hash = sha256(&executable)?;
    let managed = store.status()?;
    let mut owner = InstallationOwner { kind: OwnerKind::Unknown, executable: text, sha256: hash, attempt: None,
        guidance: "Remote executable ownership is unknown. Upgrade the selected remote command manually, then reconnect.".into() };
    if let Some(receipt) = &managed
        && [&receipt.destination, &receipt.current.executable]
            .into_iter()
            .any(|p| fs::canonicalize(p).ok().as_ref() == Some(&executable))
        && receipt.current.manifest.build == *build
        && receipt.current.manifest.payload.sha256 == owner.sha256
    {
        owner.kind = OwnerKind::Managed;
        owner.attempt = Some(receipt.attempt.clone());
        owner.guidance.clear();
        return Ok(owner);
    }
    if let Some(manager) = for_executable(&executable, &build.package_version) {
        owner.kind = if manager.verified {
            OwnerKind::Manager
        } else {
            OwnerKind::Unknown
        };
        owner.guidance = format!(
            "Remote {}: {}{}",
            manager.manager,
            manager
                .command
                .as_ref()
                .map_or(String::new(), |s| format!("{s} ")),
            manager.detail
        );
        return Ok(owner);
    }
    // A path is insufficient: require the exact per-user command, no receipt
    // ambiguity, user ownership, safe modes and the running build's stateless ABI.
    let metadata = fs::symlink_metadata(store.destination());
    if managed.is_none()
        && fs::canonicalize(store.destination()).ok().as_ref() == Some(&executable)
        && metadata
            .is_ok_and(|m| m.is_file() && m.uid() == crate::os::uid() && m.mode() & 0o022 == 0)
        && !manager_metadata_present(&executable)
        && manager::system_unclaimed(&executable)
        && inspect_binary(&executable)? == *build
    {
        owner.kind = OwnerKind::Manual;
        owner.guidance =
            "Adopt the verified manual remote command and retain its previous bytes.".into();
    }
    Ok(owner)
}

fn manager_metadata_present(executable: &Path) -> bool {
    executable
        .parent()
        .and_then(Path::parent)
        .is_some_and(|root| {
            [
                root.join(".crates2.json"),
                root.join(".crates.toml"),
                root.join("INSTALL_RECEIPT.json"),
            ]
            .iter()
            .any(|p| !matches!(fs::symlink_metadata(p), Err(e) if e.kind() == io::ErrorKind::NotFound))
        })
}

/// The UI supplies its own PID to the isolated worker. No remote field supplies
/// a PID or path; the live registration must also match this helper build.
pub(super) fn selected(state: &Path, frontend_pid: u32) -> io::Result<CoreOwners> {
    let before = super::runtime_identity(state)?;
    let store = Store::for_user("flere")?;
    let frontend: BuildMetadata =
        serde_json::from_str(crate::build_info::json()).map_err(io::Error::other)?;
    let tracked: serde_json::Value =
        serde_json::from_slice(&crate::wire::request(state, &["frontends"])?)
            .map_err(io::Error::other)?;
    let identity = tracked["tracked"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|v| {
                v["pid"] == frontend_pid
                    && v["build"] == serde_json::to_value(&frontend).unwrap_or_default()
            })
        })
        .cloned()
        .ok_or_else(|| {
            io::Error::other("selected frontend registration changed; reconnect before updating")
        })?;
    let frontend = selected_executable(
        &store,
        &crate::os::process_executable(frontend_pid)?,
        &frontend,
    )?;
    let supervisor = selected_executable(
        &store,
        &crate::os::process_executable(before.pid)?,
        &before.build,
    )?;
    let after = super::runtime_identity(state)?;
    let tracked: serde_json::Value =
        serde_json::from_slice(&crate::wire::request(state, &["frontends"])?)
            .map_err(io::Error::other)?;
    if before.epoch != after.epoch
        || before.pid != after.pid
        || before.build != after.build
        || !tracked["tracked"]
            .as_array()
            .is_some_and(|items| items.contains(&identity))
    {
        return Err(io::Error::other(
            "selected frontend or supervisor changed during ownership check",
        ));
    }
    Ok(CoreOwners {
        schema_version: 1,
        epoch: before.epoch,
        pid: before.pid,
        frontend,
        supervisor,
    })
}

fn quote(value: &str) -> String {
    shell_words::quote(value).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!("ownership-{}", crate::os::nonce().unwrap()));
            fs::create_dir_all(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            Self(root)
        }
        fn file(&self, relative: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            path
        }
        fn json(&self, relative: &str, value: serde_json::Value) -> PathBuf {
            self.file(relative, &serde_json::to_vec(&value).unwrap())
        }
        fn cargo_receipt(&self, root: &str, id: &str) {
            self.json(
                &format!("{root}/.crates2.json"),
                serde_json::json!({"installs":{id:{"bins":["flere"],"profile":"release"}}}),
            );
        }
        fn brew_receipt(&self, tap: &str, version: &str) {
            self.json("Cellar/flere/0.3.0/INSTALL_RECEIPT.json", serde_json::json!({
                "homebrew_version":"5.0.0", "source":{"spec":"stable","versions":{"stable":version},"tap":tap}
            }));
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn coordinated_owners_distinguish_manual_manager_unknown_and_verified_store() {
        let f = Fixture::new();
        let binary = f.file(
            ".local/bin/flere",
            format!(
                "#!/bin/sh\nprintf '%s\\n' {}\n",
                shell_words::quote(crate::build_info::json())
            )
            .as_bytes(),
        );
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let store = Store::new(f.0.join("managed"), f.0.join(".local/bin"), "flere").unwrap();
        let build = serde_json::from_str(crate::build_info::json()).unwrap();
        assert_eq!(
            fs::canonicalize(&binary).unwrap(),
            fs::canonicalize(store.destination()).unwrap()
        );
        assert_eq!(
            fs::symlink_metadata(&binary).unwrap().uid(),
            crate::os::uid()
        );
        assert_eq!(fs::symlink_metadata(&binary).unwrap().mode() & 0o022, 0);
        assert!(!manager_metadata_present(&binary));
        assert_eq!(inspect_binary(&binary).unwrap(), build);
        let manual = selected_executable(&store, &binary, &build).unwrap();
        assert_eq!(manual.kind, OwnerKind::Manual);
        manual.require_update().unwrap();
        let other = f.file("other/flere", &fs::read(&binary).unwrap());
        assert_eq!(
            selected_executable(&store, &other, &build).unwrap().kind,
            OwnerKind::Unknown
        );
        f.cargo_receipt(
            ".local",
            &format!(
                "flere {} (registry+https://github.com/rust-lang/crates.io-index)",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let manager = selected_executable(&store, &binary, &build).unwrap();
        assert_eq!(manager.kind, OwnerKind::Manager);
        assert!(manager.require_update().is_err());
        assert!(manager.guidance.contains("cargo install"));
        f.file(".local/.crates2.json", b"{broken");
        assert_eq!(
            selected_executable(&store, &binary, &build).unwrap().kind,
            OwnerKind::Unknown
        );
        fs::remove_file(f.0.join(".local/.crates2.json")).unwrap();
        let package = f.0.join("package");
        crate::install::package(&binary, &package, None).unwrap();
        let staged = store.stage(&package).unwrap();
        // An ownership change after review is rejected inside the actual install
        // lock before adoption/replacement, including a candidate no-op.
        f.cargo_receipt(
            ".local",
            &format!(
                "flere {} (registry+https://github.com/rust-lang/crates.io-index)",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let before = fs::read(&binary).unwrap();
        assert!(
            store
                .install_guarded(&staged, crate::install::PackageSource::Local, true, || {
                    let current = selected_executable(&store, &binary, &build)?;
                    current.require_update()?;
                    if current != manual {
                        return Err(io::Error::other("ownership changed"));
                    }
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(fs::read(&binary).unwrap(), before);
        assert!(store.status().unwrap().is_none());
        fs::remove_file(f.0.join(".local/.crates2.json")).unwrap();
        let installed = store
            .install(&staged, crate::install::PackageSource::Local, true)
            .unwrap();
        f.cargo_receipt(
            ".local",
            &format!(
                "flere {} (registry+https://github.com/rust-lang/crates.io-index)",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let managed = selected_executable(&store, &installed.current.executable, &build).unwrap();
        assert_eq!(managed.kind, OwnerKind::Managed);
        assert_eq!(managed.attempt.as_deref(), Some(installed.attempt.as_str()));
        assert_eq!(
            selected_executable(&store, &other, &build).unwrap().kind,
            OwnerKind::Unknown
        );
    }
    #[test]
    fn homebrew_needs_receipt_version_tap_and_exact_keg_executable() {
        let f = Fixture::new();
        let exe = f.file("Cellar/flere/0.3.0/bin/flere", b"fixture");
        assert!(homebrew(&exe, "0.3.0").is_none());
        f.brew_receipt("robert-cronin/flere", "0.3.0");
        assert_eq!(
            homebrew(&exe, "0.3.0").unwrap().command.as_deref(),
            Some("brew upgrade robert-cronin/flere/flere")
        );
        assert!(homebrew(&exe, "0.4.0").is_none());
        let other_component = f.file("Cellar/flere/0.3.0/bin/flere-connect", b"fixture");
        assert!(homebrew(&other_component, "0.3.0").is_none());
        let foreign = f.file("Cellar/flere/0.3.0/bin/unrelated", b"fixture");
        assert!(homebrew(&foreign, "0.3.0").is_none());
        f.brew_receipt("robert-cronin/flere;bad", "0.3.0");
        assert!(homebrew(&exe, "0.3.0").is_none());
        f.brew_receipt("robert-cronin/flere", "0.4.0");
        assert!(homebrew(&exe, "0.3.0").is_none());
    }

    #[test]
    fn homebrew_symlink_resolves_to_receipted_keg_before_cargo() {
        let f = Fixture::new();
        let exe = f.file("Cellar/flere/0.3.0/bin/flere", b"fixture");
        f.brew_receipt("robert-cronin/flere", "0.3.0");
        f.cargo_receipt(
            "Cellar/flere/0.3.0",
            "flere 0.3.0 (registry+https://github.com/rust-lang/crates.io-index)",
        );
        let link = f.0.join("flere");
        symlink(&exe, &link).unwrap();
        assert_eq!(for_executable(&link, "0.3.0").unwrap().manager, "Homebrew");
    }

    #[test]
    fn cargo_uses_receipted_custom_root_and_shell_quotes_it() {
        let f = Fixture::new();
        let root = "custom root 'literal'";
        let exe = f.file(&format!("{root}/bin/flere"), b"fixture");
        assert!(cargo(&exe, "0.3.0").is_none());
        f.cargo_receipt(
            root,
            "flere 0.3.0 (registry+https://github.com/rust-lang/crates.io-index)",
        );
        let command = cargo(&exe, "0.3.0").unwrap().command.unwrap();
        let arguments = shell_words::split(&command).unwrap();
        assert_eq!(
            arguments,
            vec![
                "cargo",
                "install",
                "--locked",
                "--registry",
                "crates-io",
                "--root",
                f.0.join(root).to_str().unwrap(),
                "flere"
            ]
        );
    }

    #[test]
    fn cargo_preserves_local_git_other_registry_flow_and_blocks_ambiguous_bins() {
        let f = Fixture::new();
        let exe = f.file("cargo/bin/flere", b"fixture");
        for id in [
            "flere 0.3.0 (path+file:///fixture)",
            "flere 0.3.0 (git+https://example.invalid/flere)",
            "flere 0.3.0 (registry+https://example.invalid/index)",
        ] {
            f.cargo_receipt("cargo", id);
            assert!(cargo(&exe, "0.3.0").is_none());
        }
        f.json("cargo/.crates2.json", serde_json::json!({"installs":{
            "flere 0.3.0 (registry+https://github.com/rust-lang/crates.io-index)":{"bins":["other"],"profile":"release"}
        }}));
        assert!(cargo(&exe, "0.3.0").is_none());
        f.json("cargo/.crates2.json", serde_json::json!({"installs":{
            "flere 0.3.0 (registry+https://github.com/rust-lang/crates.io-index)":{"bins":["flere"],"profile":"release"},
            "another-package":{"bins":["flere"]}
        }}));
        let ambiguous = cargo(&exe, "0.3.0").unwrap();
        assert!(ambiguous.command.is_none());
        assert!(ambiguous.detail.contains("reopen"));
    }

    #[test]
    fn advancing_cargo_receipt_keeps_the_old_process_in_the_cargo_update_flow() {
        let f = Fixture::new();
        let exe = f.file("cargo/bin/flere", b"running version 0.3.0");
        f.cargo_receipt(
            "cargo",
            "flere 0.3.0 (registry+https://github.com/rust-lang/crates.io-index)",
        );
        let original = for_installation(&exe, "0.3.0", None).unwrap();
        // Cargo replaces its file and receipt without stopping the older UI.
        fs::write(&exe, b"installed version 0.4.0").unwrap();
        f.cargo_receipt(
            "cargo",
            "flere 0.4.0 (registry+https://github.com/rust-lang/crates.io-index)",
        );
        let after = for_installation(&exe, "0.3.0", None).unwrap();
        assert_eq!(after.manager, "Cargo");
        assert_eq!(after.command, original.command);
        assert!(after.detail.contains("Reopen Flere"));
        assert!(!f.0.join(".local").exists());
    }

    #[test]
    fn removed_homebrew_keg_requires_reopen_instead_of_a_local_update() {
        let f = Fixture::new();
        let exe = f.file("Cellar/flere/0.3.0/bin/flere", b"running older process");
        f.brew_receipt("robert-cronin/flere", "0.3.0");
        assert_eq!(
            for_installation(&exe, "0.3.0", None).unwrap().manager,
            "Homebrew"
        );
        // Homebrew may remove the old keg while its loaded process stays alive.
        fs::remove_dir_all(f.0.join("Cellar/flere/0.3.0")).unwrap();
        let unresolved = for_installation(&exe, "0.3.0", None).unwrap();
        assert!(unresolved.command.is_none());
        assert!(unresolved.detail.contains("Reopen Flere"));
        assert!(!f.0.join(".local").exists());
    }

    #[test]
    fn malformed_or_linked_receipt_cannot_claim_ownership() {
        let f = Fixture::new();
        let exe = f.file("cargo/bin/flere", b"fixture");
        let receipt = f.file("cargo/.crates2.json", b"{broken");
        assert!(cargo(&exe, "0.3.0").is_none());
        fs::remove_file(&receipt).unwrap();
        let data = f.json("other.json", serde_json::json!({"installs":{
            "flere 0.3.0 (registry+https://github.com/rust-lang/crates.io-index)":{"bins":["flere"],"profile":"release"}
        }}));
        symlink(data, receipt).unwrap();
        assert!(cargo(&exe, "0.3.0").is_none());
    }

    #[test]
    fn verified_flere_store_takes_precedence_over_stale_cargo_receipt() {
        let f = Fixture::new();
        let binary = f.file(
            "fixture-binary",
            format!(
                "#!/bin/sh\nprintf '%s\\n' {}\n",
                shell_words::quote(crate::build_info::json())
            )
            .as_bytes(),
        );
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let package = f.0.join("package");
        crate::install::package(&binary, &package, None).unwrap();
        let store = Store::new(f.0.join("managed"), f.0.join("cargo/bin"), "flere").unwrap();
        let staged = store.stage(&package).unwrap();
        store
            .install(&staged, crate::install::PackageSource::Local, false)
            .unwrap();
        f.cargo_receipt(
            "cargo",
            &format!(
                "flere {} (registry+https://github.com/rust-lang/crates.io-index)",
                env!("CARGO_PKG_VERSION")
            ),
        );
        let verified = store.status().unwrap().unwrap();
        assert_eq!(
            cargo(&verified.destination, env!("CARGO_PKG_VERSION"))
                .unwrap()
                .manager,
            "Cargo"
        );
        assert!(
            for_installation(
                &verified.destination,
                env!("CARGO_PKG_VERSION"),
                Some(&verified)
            )
            .is_none()
        );
        assert!(!f.0.join("state").exists());
    }

    #[test]
    fn debian_query_requires_exact_unambiguous_file_ownership() {
        let exe = Path::new("/usr/bin/flere");
        assert_eq!(
            debian_owner("flere: /usr/bin/flere\n", exe).as_deref(),
            Some("flere")
        );
        assert_eq!(
            debian_owner("flere:amd64: /usr/bin/flere\n", exe).as_deref(),
            Some("flere:amd64")
        );
        for output in [
            "flere: /usr/bin/flere-other\n",
            "flere, other: /usr/bin/flere\n",
            "diversion by flere from: /usr/bin/flere\n",
            "flere: /usr/bin/flere\nother: /usr/bin/flere\n",
            "bad;cmd: /usr/bin/flere\n",
        ] {
            assert!(debian_owner(output, exe).is_none());
        }
    }

    #[test]
    fn rpm_query_requires_one_package_and_uses_present_frontend() {
        assert_eq!(rpm_owner("flere-0\n").as_deref(), Some("flere-0"));
        for output in [
            "",
            "not owned by any package",
            "flere\nother",
            "-option",
            "flere;cmd",
        ] {
            assert!(rpm_owner(output).is_none());
        }
        assert_eq!(
            rpm_upgrade("flere", |p| p == "/usr/bin/zypper")
                .command
                .as_deref(),
            Some("sudo zypper update flere")
        );
        assert_eq!(
            rpm_upgrade("flere", |_| false).command.as_deref(),
            Some("sudo rpm --upgrade ./NEW_FLERE_PACKAGE.rpm")
        );
    }
}
