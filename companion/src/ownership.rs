//! Local evidence only; no remote path or receipt value selects a process.
use super::*;
use crate::remote_update::{InstallationOwner, OwnerKind};
#[derive(Clone, Debug, PartialEq, Eq)]
struct ManagerUpgrade {
    manager: &'static str,
    command: Option<String>,
    detail: &'static str,
}
#[path = "../../src/install/ownership/manager.rs"]
mod manager;
fn receipt(path: &Path) -> Option<Value> {
    let mut bytes = Vec::new();
    regular(path, 1024 * 1024)
        .ok()?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= 1024 * 1024)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}
#[cfg(target_os = "linux")]
fn query(program: &str, args: &[&std::ffi::OsStr]) -> Option<String> {
    run(
        Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .env_remove("DPKG_ROOT")
            .env_remove("DPKG_ADMINDIR")
            .stdin(Stdio::null()),
        65536,
        Duration::from_secs(2),
    )
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())
}
pub(crate) fn current() -> io::Result<InstallationOwner> {
    let build: BuildMetadata =
        serde_json::from_str(crate::build_info::json()).map_err(io::Error::other)?;
    selected(&Store::standard()?, &std::env::current_exe()?, &build)
}
pub(super) fn selected(
    store: &Store,
    executable: &Path,
    build: &BuildMetadata,
) -> io::Result<InstallationOwner> {
    let executable = fs::canonicalize(executable)?;
    let text = executable
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
        .ok_or_else(|| invalid("local executable path cannot be represented safely"))?
        .to_owned();
    let mut owner = InstallationOwner { kind: OwnerKind::Unknown, executable: text, sha256: sha256(&executable)?, attempt: None,
        guidance: "Local companion ownership is unknown. Upgrade the selected local command manually, then reconnect.".into() };
    let managed = store.status()?;
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
    if let Some(manager) = manager::homebrew(&executable, &build.package_version)
        .or_else(|| manager::cargo(&executable, &build.package_version))
        .or_else(|| manager::system_package(&executable))
    {
        owner.kind = OwnerKind::Manager;
        owner.guidance = format!(
            "Local {}: {} {}",
            manager.manager,
            manager
                .command
                .as_deref()
                .unwrap_or("Review the installation ownership."),
            manager.detail
        );
        return Ok(owner);
    }
    let metadata = fs::symlink_metadata(store.destination());
    let receipts = executable
        .parent()
        .and_then(Path::parent)
        .is_some_and(|root| {
            [".crates2.json", ".crates.toml", "INSTALL_RECEIPT.json"]
                .iter()
                .any(|name| !matches!(fs::symlink_metadata(root.join(name)), Err(e) if e.kind() == io::ErrorKind::NotFound))
        });
    if managed.is_none()
        && fs::canonicalize(store.destination()).ok().as_ref() == Some(&executable)
        && metadata.is_ok_and(|m| m.is_file() && owned(&m, false))
        && !receipts
        && manager::system_unclaimed(&executable)
        && inspect(&executable)? == *build
    {
        store.require_aliases_absent()?;
        owner.kind = OwnerKind::Manual;
        owner.guidance =
            "Adopt the verified manual local companion and retain its previous bytes.".into();
    }
    Ok(owner)
}
pub(super) fn check(expected: &InstallationOwner) -> io::Result<()> {
    let actual = current()?;
    actual.require_update()?;
    if &actual != expected {
        return Err(invalid("local companion ownership changed; prepare again"));
    }
    Ok(())
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
