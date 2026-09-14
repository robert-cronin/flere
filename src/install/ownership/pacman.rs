//! Local pacman registration and metadata checks, shared by both components.
use super::super::ManagerUpgrade;
use std::{fs, os::unix::fs::MetadataExt, path::Path};

#[cfg(target_os = "linux")]
pub(super) fn detect(executable: &Path) -> Option<ManagerUpgrade> {
    match fs::symlink_metadata("/usr/bin/pacman") {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(upgrade(false)),
        Ok(_) => {}
    }
    detect_with(executable, |args| {
        // The same fixed status envelope as system_unclaimed preserves negative
        // database answers. Every path is a quoted positional argument, not code.
        let mut command = vec![
            "-c",
            "\"$@\" 2>&1; status=$?; printf '\\nFLERE_OWNER_STATUS=%s\\n' \"$status\"",
            "flere-ownership",
            "/usr/bin/pacman",
            "--config=/dev/null",
            "--root=/",
            "--dbpath=/var/lib/pacman",
            "--color=never",
        ];
        command.extend(args);
        let args: Vec<&std::ffi::OsStr> = command.iter().map(|s| s.as_ref()).collect();
        super::super::query("/bin/sh", &args)
    })
}

fn upgrade(verified: bool) -> ManagerUpgrade {
    ManagerUpgrade {
        manager: "pacman",
        verified,
        command: None,
        detail: if verified {
            "Use pacman to upgrade or remove this package, then reopen Flere. In-app Apply is disabled; no package repository or AUR helper is assumed."
        } else {
            "Pacman ownership could not be verified. Review the installed package and reopen Flere; in-app Apply is disabled."
        },
    }
}

fn identity(path: &Path) -> Option<(u64, u64, u64, i64, i64, i64, i64)> {
    let m = fs::symlink_metadata(path).ok()?;
    (m.is_file() && fs::canonicalize(path).ok()?.as_path() == path).then_some((
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

fn owner<'a>(output: &'a str, executable: &str) -> Option<&'a str> {
    let entry = output
        .strip_suffix("\n\nFLERE_OWNER_STATUS=0\n")?
        .strip_prefix(executable)?
        .strip_prefix(" is owned by ")?;
    let (package, version) = entry.split_once(' ')?;
    (super::word(package)
        && !version.is_empty()
        && version.len() <= 128
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+_.:-".contains(&b)))
    .then_some(package)
}

fn clean(output: &str, package: &str) -> bool {
    let Some(count) = output
        .strip_prefix(package)
        .and_then(|s| s.strip_prefix(": "))
        .and_then(|s| s.strip_suffix("\n\nFLERE_OWNER_STATUS=0\n"))
        .and_then(|s| {
            s.strip_suffix(" total files, 0 altered files")
                .or_else(|| s.strip_suffix(" total file, 0 altered files"))
        })
    else {
        return false;
    };
    // Non-quiet -Qkk is deliberate: missing mtree can exit zero and quiet mode
    // hides that fact. This proves pacman's metadata check passed, not a portable
    // byte-hash guarantee (checksum support depends on its libarchive build).
    count.bytes().all(|b| b.is_ascii_digit()) && count.parse::<u64>().is_ok_and(|n| n > 0)
}

fn detect_with(
    executable: &Path,
    mut query: impl FnMut(&[&str]) -> Option<String>,
) -> Option<ManagerUpgrade> {
    let unknown = || Some(upgrade(false));
    let Some(path) = executable
        .to_str()
        .filter(|s| !s.chars().any(char::is_control))
    else {
        return unknown();
    };
    let Some(before) = identity(executable) else {
        return unknown();
    };
    let Some(output) = query(&["--query", "--owns", "--", path]) else {
        return unknown();
    };
    if output == format!("error: No package owns {path}\n\nFLERE_OWNER_STATUS=1\n") {
        return if identity(executable) == Some(before) {
            None
        } else {
            unknown()
        };
    }
    let Some(package) = owner(&output, path) else {
        return unknown();
    };
    let verified = query(&["--query", "--check", "--check", "--", package])
        .is_some_and(|result| clean(&result, package))
        && identity(executable) == Some(before);
    Some(upgrade(verified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!("pacman-owner-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn executable(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"exact fixture executable").unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    fn claimed(path: &Path) -> String {
        format!(
            "{} is owned by flere-bin 0.3.3-1\n\nFLERE_OWNER_STATUS=0\n",
            path.display()
        )
    }
    // Summary from the retained real Arch makepkg/pacman validation fixture.
    const CLEAN: &str = "flere-bin: 13 total files, 0 altered files\n\nFLERE_OWNER_STATUS=0\n";

    #[test]
    fn pacman_both_components_require_exact_owner_and_clean_metadata() {
        let fixture = Fixture::new();
        for name in ["flere", "flere-connect"] {
            let path = fixture.executable(name);
            let mut calls = 0;
            let result = detect_with(&path, |args| {
                calls += 1;
                if calls == 1 {
                    assert_eq!(args, ["--query", "--owns", "--", path.to_str().unwrap()]);
                    Some(claimed(&path))
                } else {
                    assert_eq!(args, ["--query", "--check", "--check", "--", "flere-bin"]);
                    Some(CLEAN.into())
                }
            })
            .unwrap();
            assert_eq!(calls, 2);
            assert!(result.verified && result.command.is_none());
            assert_eq!(result.manager, "pacman");
            assert!(result.detail.contains("Use pacman to upgrade or remove"));
        }
    }

    #[test]
    fn pacman_only_exact_unowned_answer_preserves_manual_flow() {
        let fixture = Fixture::new();
        let path = fixture.executable("flere");
        let absent = format!(
            "error: No package owns {}\n\nFLERE_OWNER_STATUS=1\n",
            path.display()
        );
        assert!(detect_with(&path, |_| Some(absent.clone())).is_none());
        for output in [
            None,
            Some(String::new()),
            Some(absent.replace("STATUS=1", "STATUS=2")),
            Some(absent.replace("flere\n", "different\n")),
            Some(claimed(&fixture.0.join("different"))),
            Some(format!("{}{}", claimed(&path), claimed(&path))),
            Some(claimed(&path).replace("flere-bin", "flere;command")),
        ] {
            let result = detect_with(&path, |_| output.clone()).unwrap();
            assert!(!result.verified && result.command.is_none());
        }
    }

    #[test]
    fn pacman_refuses_failed_missing_or_noisy_metadata_and_changed_executable() {
        let fixture = Fixture::new();
        let path = fixture.executable("flere-connect");
        for check in [
            None,
            Some(String::new()),
            Some("flere-bin: no mtree file\n\nFLERE_OWNER_STATUS=0\n".into()),
            Some(CLEAN.replace("0 altered", "1 altered")),
            Some(CLEAN.replace("STATUS=0", "STATUS=1")),
            Some(CLEAN.replace("13 total", "0 total")),
            Some(CLEAN.replace("13 total", "x total")),
            Some(format!("warning: damaged metadata\n{CLEAN}")),
            Some(CLEAN.replace("flere-bin", "other")),
        ] {
            let result = detect_with(&path, |args| {
                if args[1] == "--owns" {
                    Some(claimed(&path))
                } else {
                    check.clone()
                }
            })
            .unwrap();
            assert!(!result.verified);
        }
        let result = detect_with(&path, |args| {
            if args[1] == "--owns" {
                Some(claimed(&path))
            } else {
                fs::remove_file(&path).unwrap();
                fs::write(&path, b"replaced executable").unwrap();
                Some(CLEAN.into())
            }
        })
        .unwrap();
        assert!(!result.verified);
    }
}
