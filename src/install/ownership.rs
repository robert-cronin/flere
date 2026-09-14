//! Read-only evidence for the installation containing the running executable.
//! A conventional pathname alone never establishes a package-manager owner.
use super::{InstallReceipt, Store, read_bounded_json};
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ManagerUpgrade {
    pub manager: &'static str,
    /// None means the ownership evidence needs review; never offer Apply then.
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
    if executable.file_name()? != "flere" {
        return None;
    }
    if let Some(owner) = homebrew(&executable, version).or_else(|| cargo(&executable, version)) {
        return Some(owner);
    }
    system_package(&executable)
}

fn receipt(path: &Path) -> Option<serde_json::Value> {
    // O_NOFOLLOW and a byte limit also keep malformed receipts passive/bounded.
    read_bounded_json(path, 1024 * 1024).ok()
}

fn word(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+_.-".contains(&b))
        && value.as_bytes()[0].is_ascii_alphanumeric()
}

fn homebrew(executable: &Path, version: &str) -> Option<ManagerUpgrade> {
    let bin = executable.parent()?;
    let keg = bin.parent()?;
    let formula = keg.parent()?;
    let cellar = formula.parent()?;
    if bin.file_name()? != "bin"
        || formula.file_name()? != "flere"
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
        command: Some(format!("brew upgrade {tap}/flere")),
        detail: "Run this in a shell, then reopen Flere. Running sessions stay alive.",
    })
}

fn cargo(executable: &Path, version: &str) -> Option<ManagerUpgrade> {
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
            entry["bins"]
                .as_array()
                .is_some_and(|bins| bins.iter().any(|bin| bin.as_str() == Some("flere")))
        })
        .collect();
    if owners.is_empty() {
        return None;
    }
    if owners.len() != 1 {
        return Some(ManagerUpgrade {
            manager: "Cargo",
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
        id.strip_prefix("flere ")?
            .strip_suffix(&format!(" ({source})"))
    })?;
    if !word(recorded_version) || entry["profile"].as_str()? != "release" {
        return None;
    }
    Some(ManagerUpgrade {
        manager: "Cargo",
        command: Some(format!(
            "cargo install --locked --registry crates-io --root {} flere",
            shell_words::quote(root_text)
        )),
        detail: if recorded_version == version {
            "This command keeps the recorded Cargo install root. Reopen Flere afterward."
        } else {
            "Cargo's receipt changed since this process was built. Reopen Flere before another upgrade; the Cargo install root is preserved."
        },
    })
}

#[cfg(target_os = "linux")]
fn system_package(executable: &Path) -> Option<ManagerUpgrade> {
    use std::{
        process::{Command, Stdio},
        time::Duration,
    };
    let query = |program: &str, args: &[&std::ffi::OsStr]| {
        super::run(
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
    };
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
    None
}

#[cfg(not(target_os = "linux"))]
fn system_package(_: &Path) -> Option<ManagerUpgrade> {
    None
}

#[cfg(any(target_os = "linux", test))]
fn debian_owner(output: &str, executable: &Path) -> Option<String> {
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
fn rpm_owner(output: &str) -> Option<String> {
    let package = output.trim();
    word(package).then(|| package.to_owned())
}

#[cfg(any(target_os = "linux", test))]
fn rpm_upgrade(package: &str, available: impl Fn(&str) -> bool) -> ManagerUpgrade {
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
        command: Some(command),
        detail: "Use a configured package source or download the new RPM; reopen Flere afterward.",
    }
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
