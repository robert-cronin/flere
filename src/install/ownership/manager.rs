//! Receipt parsers shared by core and companion. No command comes from receipt text.
#[cfg(target_os = "linux")]
use super::query;
use super::{ManagerUpgrade, receipt};
use std::path::Path;
#[cfg(any(windows, test))]
#[path = "chocolatey.rs"]
pub(super) mod chocolatey;
#[path = "nix.rs"]
mod nix;
#[cfg(any(target_os = "linux", all(test, unix)))]
#[path = "pacman.rs"]
mod pacman;
#[cfg(any(windows, test))]
#[path = "scoop.rs"]
pub(super) mod scoop;
#[cfg(any(windows, test))]
#[path = "windows_profile.rs"]
pub(super) mod windows_profile;
#[cfg(any(windows, test))]
#[path = "winget.rs"]
pub(super) mod winget;
pub(super) fn nix(executable: &Path) -> Option<ManagerUpgrade> {
    nix::detect(executable)
}
fn word(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+_.-".contains(&b))
        && value.as_bytes()[0].is_ascii_alphanumeric()
}

pub(super) fn homebrew(executable: &Path, version: &str) -> Option<ManagerUpgrade> {
    let component = executable.file_name()?.to_str()?;
    if !matches!(component, "flere" | "flere-connect") {
        return None;
    }
    let bin = executable.parent()?;
    let keg = bin.parent()?;
    let formula = keg.parent()?;
    let cellar = formula.parent()?;
    if bin.file_name()? != "bin"
        || formula.file_name()? != component
        || cellar.file_name()? != "Cellar"
    {
        return None;
    }
    let keg_version = keg.file_name()?.to_str()?;
    if keg_version != version
        && !keg_version.strip_prefix(version).is_some_and(|revision| {
            revision
                .strip_prefix('_')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
    {
        return None;
    }
    let tab = receipt(&keg.join("INSTALL_RECEIPT.json"))?;
    if tab["homebrew_version"].as_str()?.is_empty()
        || tab["source"]["spec"].as_str()? != "stable"
        || tab["source"]["versions"]["stable"].as_str()? != version
    {
        return None;
    }
    let tap = tab["source"]["tap"].as_str()?;
    let (owner, repository) = tap.split_once('/')?;
    if !word(owner) || !word(repository) {
        return None;
    }
    Some(ManagerUpgrade {
        manager: "Homebrew",
        verified: true,
        command: Some(format!("brew upgrade {tap}/{component}")),
        detail: "Run this in a shell, then reopen Flere. Running sessions stay alive.",
    })
}

pub(super) fn cargo(executable: &Path, version: &str) -> Option<ManagerUpgrade> {
    if !matches!(executable.file_stem()?.to_str()?, "flere" | "flere-connect") {
        return None;
    }
    let bin = executable.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    let root = bin.parent()?;
    let root_text = root.to_str()?;
    if root_text.chars().any(char::is_control) {
        return None;
    }
    let metadata = receipt(&root.join(".crates2.json"))?;
    let installs = metadata["installs"].as_object()?;
    let owners: Vec<_> = installs
        .iter()
        .filter(|(_, entry)| {
            entry["bins"].as_array().is_some_and(|bins| {
                bins.iter()
                    .any(|bin| bin.as_str() == executable.file_name().and_then(|s| s.to_str()))
            })
        })
        .collect();
    if owners.is_empty() {
        return None;
    }
    if owners.len() != 1 {
        return Some(ManagerUpgrade {
            manager: "Cargo",
            verified: false,
            command: None,
            detail: "Multiple Cargo receipts claim this executable. Review the Cargo installation and reopen Flere; in-app Apply is disabled.",
        });
    }
    let (id, entry) = owners[0];
    // Git/path/alternate-registry installs retain their explicit development or
    // manual update flow; never redirect them to crates.io based on the filename.
    let recorded_version = [
        "registry+https://github.com/rust-lang/crates.io-index",
        "sparse+https://index.crates.io/",
    ]
    .into_iter()
    .find_map(|source| {
        id.strip_prefix(&format!("{} ", executable.file_stem()?.to_str()?))?
            .strip_suffix(&format!(" ({source})"))
    })?;
    if !word(recorded_version) || entry["profile"].as_str()? != "release" {
        return None;
    }
    Some(ManagerUpgrade {
        manager: "Cargo",
        verified: true,
        command: Some(format!(
            "cargo install --locked --registry crates-io --root {} {}",
            super::quote(root_text),
            executable.file_stem()?.to_str()?
        )),
        detail: if recorded_version == version {
            "This command keeps the recorded Cargo install root. Reopen Flere afterward."
        } else {
            "Cargo's receipt changed since this process was built. Reopen Flere before another upgrade; the Cargo install root is preserved."
        },
    })
}

#[cfg(target_os = "linux")]
pub(super) fn system_package(executable: &Path) -> Option<ManagerUpgrade> {
    if let Some(output) = query(
        "/usr/bin/dpkg-query",
        &[
            "--admindir=/var/lib/dpkg".as_ref(),
            "--search".as_ref(),
            "--".as_ref(),
            executable.as_os_str(),
        ],
    ) && let Some(package) = debian_owner(&output, executable)
        && let Some(status) = query(
            "/usr/bin/dpkg-query",
            &[
                "--admindir=/var/lib/dpkg".as_ref(),
                "--show".as_ref(),
                "--showformat=${db:Status-Status}".as_ref(),
                "--".as_ref(),
                package.as_ref(),
            ],
        )
        && status == "installed"
    {
        return Some(ManagerUpgrade {
            manager: "Debian package manager",
            verified: true,
            command: Some("sudo apt install ./NEW_FLERE_PACKAGE.deb".into()),
            detail: "Download the new .deb first and replace NEW_FLERE_PACKAGE.deb. No APT repository is assumed.",
        });
    }
    if let Some(output) = query(
        "/usr/bin/rpm",
        &[
            "--query".as_ref(),
            "--file".as_ref(),
            "--queryformat=%{NAME}\\n".as_ref(),
            "--".as_ref(),
            executable.as_os_str(),
        ],
    ) && let Some(package) = rpm_owner(&output)
    {
        return Some(rpm_upgrade(&package, |path| Path::new(path).is_file()));
    }
    pacman::detect(executable)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn system_package(_: &Path) -> Option<ManagerUpgrade> {
    None
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn debian_owner(output: &str, executable: &Path) -> Option<String> {
    let mut matches = output.lines().filter_map(|line| {
        let (package, path) = line.split_once(": ")?;
        let name = package.split(':').next()?;
        // Reject diversions, lists of owners and wildcard search near-matches.
        (Path::new(path) == executable && word(name) && package.split(':').all(word))
            .then(|| package.to_owned())
    });
    let owner = matches.next()?;
    matches.next().is_none().then_some(owner)
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn rpm_owner(output: &str) -> Option<String> {
    let package = output.trim();
    word(package).then(|| package.to_owned())
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn rpm_upgrade(package: &str, available: impl Fn(&str) -> bool) -> ManagerUpgrade {
    let command = if available("/usr/bin/dnf") {
        format!("sudo dnf upgrade {package}")
    } else if available("/usr/bin/zypper") {
        format!("sudo zypper update {package}")
    } else if available("/usr/bin/yum") {
        format!("sudo yum update {package}")
    } else {
        "sudo rpm --upgrade ./NEW_FLERE_PACKAGE.rpm".into()
    };
    ManagerUpgrade {
        manager: "RPM package manager",
        verified: true,
        command: Some(command),
        detail: "Use a configured package source or download the new RPM; reopen Flere afterward.",
    }
}

/// Only exact negative database answers establish absence. A missing tool means
/// that manager is unavailable; errors/timeouts/malformed output remain unknown.
#[cfg(target_os = "linux")]
pub(super) fn system_unclaimed(executable: &Path) -> bool {
    let Some(path) = executable.to_str() else {
        return false;
    };
    let script = "\"$@\" 2>&1; status=$?; printf '\\nFLERE_OWNER_STATUS=%s\\n' \"$status\"";
    for (program, args, expected) in [
        (
            "/usr/bin/dpkg-query",
            vec!["--admindir=/var/lib/dpkg", "--search", "--", path],
            format!("dpkg-query: no path found matching pattern {path}\n\nFLERE_OWNER_STATUS=1\n"),
        ),
        (
            "/usr/bin/rpm",
            vec!["--query", "--file", "--queryformat=%{NAME}\\n", "--", path],
            format!("file {path} is not owned by any package\n\nFLERE_OWNER_STATUS=1\n"),
        ),
    ] {
        match std::fs::symlink_metadata(program) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return false,
            Ok(_) => {}
        }
        // Fixed shell envelope preserves the query's exit status within the
        // bounded reader. Quoted positional arguments are never shell code.
        let mut command = vec!["-c", script, "flere-ownership", program];
        command.extend(args);
        let arguments: Vec<&std::ffi::OsStr> = command.iter().map(|s| s.as_ref()).collect();
        if query("/bin/sh", &arguments).as_deref() != Some(expected.as_str()) {
            return false;
        }
    }
    pacman::detect(executable).is_none()
}
#[cfg(not(target_os = "linux"))]
pub(super) fn system_unclaimed(_: &Path) -> bool {
    true
}

#[cfg(test)]
mod shared_tests {
    use super::*;
    #[test]
    fn native_manager_parsers_reject_ambiguous_or_command_like_names() {
        assert_eq!(
            debian_owner("flere:amd64: /usr/bin/flere\n", Path::new("/usr/bin/flere")).as_deref(),
            Some("flere:amd64")
        );
        assert!(
            debian_owner(
                "flere: /usr/bin/flere\nother: /usr/bin/flere\n",
                Path::new("/usr/bin/flere")
            )
            .is_none()
        );
        assert!(rpm_owner("flere; touch marker").is_none());
        assert!(rpm_owner("flere\nother").is_none());
        assert_eq!(
            rpm_upgrade("flere", |_| false).command.as_deref(),
            Some("sudo rpm --upgrade ./NEW_FLERE_PACKAGE.rpm")
        );
    }
}
