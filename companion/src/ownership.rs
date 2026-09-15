//! Local evidence only; no remote path or receipt value selects a process.
use super::*;
use crate::remote_update::{InstallationOwner, OwnerKind};
#[derive(Clone, Debug, PartialEq, Eq)]
struct ManagerUpgrade {
    manager: &'static str,
    verified: bool,
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
    #[cfg(windows)]
    let windows_manager = chocolatey(&executable, build, &owner.sha256)
        .or_else(|| winget(&executable, build, &owner.sha256));
    #[cfg(not(windows))]
    let windows_manager: Option<ManagerUpgrade> = None;
    if let Some(manager) = windows_manager
        .or_else(|| manager::nix(&executable))
        .or_else(|| manager::homebrew(&executable, &build.package_version))
        .or_else(|| manager::cargo(&executable, &build.package_version))
        .or_else(|| manager::system_package(&executable))
    {
        owner.kind = if manager.verified {
            OwnerKind::Manager
        } else {
            OwnerKind::Unknown
        };
        owner.guidance = format!(
            "Local {}: {}{}",
            manager.manager,
            manager
                .command
                .as_ref()
                .map_or(String::new(), |s| format!("{s} ")),
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

#[cfg(windows)]
fn chocolatey(executable: &Path, build: &BuildMetadata, hash: &str) -> Option<ManagerUpgrade> {
    let leaf = |path: &Path, name: &str| {
        path.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case(name))
    };
    if !leaf(executable, "flere.exe") && !leaf(executable, "flere-connect.exe") {
        return None;
    }
    let app = executable.parent()?;
    let tools = app.parent()?;
    let package = tools.parent()?;
    let lib = package.parent()?;
    if !leaf(app, "app")
        || !leaf(tools, "tools")
        || !leaf(package, "flere-connect")
        || !leaf(lib, "lib")
    {
        return None;
    }
    // The observed ordinary default installation is the trust anchor. A package
    // path must never choose arbitrary <prefix>/bin/choco.exe. Custom roots stay
    // unknown until independently established; no PATH or receipt command lookup.
    const ROOT: &str = r"C:\ProgramData\Chocolatey";
    let verified = (|| -> Option<()> {
        let root = fs::canonicalize(ROOT).ok()?;
        let expected_app = fs::canonicalize(root.join("lib/flere-connect/tools/app")).ok()?;
        if app != expected_app {
            return None;
        }
        let program = root.join("bin/choco.exe");
        regular(&program, manifest::MAX_PAYLOAD).ok()?;
        let manifest = receipt(&app.join("manifest.json"))?;
        let parsed: Manifest = serde_json::from_value(manifest.clone()).ok()?;
        parsed.validate().ok()?;
        let metadata = regular(executable, manifest::MAX_PAYLOAD)
            .ok()?
            .metadata()
            .ok()?;
        if !manager::chocolatey::manifest_matches(
            &manifest,
            &serde_json::to_value(build).ok()?,
            metadata.len(),
            hash,
        ) {
            return None;
        }
        let query = |args: &[&str], maximum| -> Option<String> {
            run(
                Command::new(&program)
                    .args(args)
                    .env("ChocolateyInstall", ROOT)
                    .stdin(Stdio::null()),
                maximum,
                Duration::from_secs(10),
            )
            .ok()
            .and_then(|v| String::from_utf8(v).ok())
        };
        let version = query(&["--version"], 128)?;
        let version = version.trim();
        if !manager::chocolatey::supported_version(version) {
            return None;
        }
        let output = query(&["list", "--limit-output"], 65536)?;
        manager::chocolatey::installed(&output, version, &build.package_version).then_some(())
    })()
    .is_some();
    Some(ManagerUpgrade {
        manager: "Chocolatey",
        verified,
        command: verified.then(|| "choco upgrade flere-connect".into()),
        detail: if verified {
            "Run this in a shell using the same Chocolatey package source, then reopen the companion. In-app Apply is disabled for this installation."
        } else {
            "Chocolatey ownership could not be verified. Reopen the companion from its installed command and review the package-manager installation; in-app Apply is disabled."
        },
    })
}

#[cfg(windows)]
fn winget(executable: &Path, build: &BuildMetadata, hash: &str) -> Option<ManagerUpgrade> {
    let leaf = |path: &Path, name: &str| {
        path.file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case(name))
    };
    let package = executable.parent()?;
    let packages = package.parent()?;
    if (!leaf(executable, "flere.exe") && !leaf(executable, "flere-connect.exe"))
        || !leaf(package, manager::winget::PRODUCT_CODE)
        || !leaf(packages, "Packages")
        || !leaf(packages.parent()?, "WinGet")
    {
        return None;
    }
    let verified = (|| -> Option<()> {
        let value = receipt(&package.join("manifest.json"))?;
        let parsed: Manifest = serde_json::from_value(value.clone()).ok()?;
        parsed.validate().ok()?;
        let metadata = regular(executable, manifest::MAX_PAYLOAD)
            .ok()?
            .metadata()
            .ok()?;
        // Both Windows aliases carry the same portable companion manifest.
        if !manager::chocolatey::manifest_matches(
            &value,
            &serde_json::to_value(build).ok()?,
            metadata.len(),
            hash,
        ) {
            return None;
        }
        let output = run(
            Command::new("powershell.exe")
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    manager::winget::QUERY,
                ])
                .stdin(Stdio::null()),
            65536,
            Duration::from_secs(5),
        )
        .ok()?;
        let output = String::from_utf8(output).ok()?;
        let record = manager::winget::record(&output, &build.package_version)?;
        // Anchor the normal AppData/Local default to the current SID's literal
        // HKLM profile path, not fixture HOME/LOCALAPPDATA or package metadata.
        if !Path::new(record.local_appdata).is_absolute()
            || !Path::new(record.install_location).is_absolute()
        {
            return None;
        }
        let root = fs::canonicalize(record.local_appdata).ok()?;
        let expected = fs::canonicalize(
            root.join("Microsoft/WinGet/Packages")
                .join(manager::winget::PRODUCT_CODE),
        )
        .ok()?;
        (package == expected && fs::canonicalize(record.install_location).ok()? == expected)
            .then_some(())
    })()
    .is_some();
    Some(ManagerUpgrade {
        manager: "WinGet",
        verified,
        command: None,
        detail: if verified {
            "This companion is installed as a WinGet portable package. Use WinGet with the next reviewed Flere manifest/package, then reopen the companion. In-app Apply is disabled."
        } else {
            "WinGet ownership could not be verified. Reopen the companion from its installed command and review the package-manager installation; in-app Apply is disabled."
        },
    })
}
