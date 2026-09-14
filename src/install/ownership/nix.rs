//! Fixed local Nix evidence shared by core and companion; no profile is guessed.
use super::super::ManagerUpgrade;
#[cfg(unix)]
use std::fs;
use std::path::Path;

const DETAIL: &str =
    "Update the owning Nix configuration or profile, then reopen Flere. In-app Apply is disabled.";
const UNKNOWN: &str = "Nix ownership could not be verified locally. Review the owning installation and reopen Flere; Apply is disabled.";

pub(super) fn detect(executable: &Path) -> Option<ManagerUpgrade> {
    if !executable.starts_with("/nix/store") {
        return None;
    }
    #[cfg(unix)]
    let verified = verify(executable, Path::new("/nix/store"), |output| {
        let program = trusted_program()?;
        if !trusted_socket() {
            return None;
        }
        super::super::super::run(
            &mut command(&program, output),
            65536,
            std::time::Duration::from_secs(2),
        )
        .ok()
    });
    #[cfg(not(unix))]
    let verified = false;
    Some(upgrade(verified))
}

fn upgrade(verified: bool) -> ManagerUpgrade {
    ManagerUpgrade {
        manager: "Nix",
        verified,
        command: None,
        detail: if verified { DETAIL } else { UNKNOWN },
    }
}

#[cfg(unix)]
fn output_path<'a>(executable: &'a Path, store: &Path) -> Option<&'a Path> {
    use std::path::Component;
    let relative = executable.strip_prefix(store).ok()?;
    let Component::Normal(name) = relative.components().next()? else {
        return None;
    };
    let name = name.to_str()?;
    let (hash, name) = name.split_once('-')?;
    if hash.len() != 32
        || !hash
            .bytes()
            .all(|c| b"0123456789abcdfghijklmnpqrsvwxyz".contains(&c))
        || name.is_empty()
        || executable.to_str()?.len() > 4096
        || executable.to_str()?.chars().any(char::is_control)
        || relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    executable.ancestors().find(|p| p.parent() == Some(store))
}

#[cfg(unix)]
#[derive(PartialEq)]
struct Identity {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
#[cfg(unix)]
fn identity(path: &Path) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).ok()?;
    (metadata.is_file() && fs::canonicalize(path).ok()?.as_path() == path).then_some(Identity {
        device: metadata.dev(),
        inode: metadata.ino(),
        bytes: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()),
        changed: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

#[cfg(unix)]
fn verify(executable: &Path, store: &Path, query: impl FnOnce(&Path) -> Option<Vec<u8>>) -> bool {
    let Some(output) = output_path(executable, store) else {
        return false;
    };
    let Some(before) = identity(executable) else {
        return false;
    };
    // Unlike --query --hash (a stored value), --verify-path checks the live NAR
    // against its registered hash. No --repair, realise, roots or profile writes.
    query(output).is_some_and(|bytes| bytes.is_empty()) && identity(executable) == Some(before)
}

#[cfg(unix)]
fn trusted_path(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    path.ancestors().all(|part| {
        fs::symlink_metadata(part).is_ok_and(|m| {
            m.uid() == 0
                && (m.file_type().is_symlink()
                    || m.mode() & 0o022 == 0
                    // The normal store is root:nixbld, group-writable and sticky.
                    || (part == Path::new("/nix/store")
                        && m.is_dir() && m.mode() & 0o1002 == 0o1000))
        })
    })
}

#[cfg(unix)]
fn trusted_program() -> Option<std::path::PathBuf> {
    use std::os::unix::fs::MetadataExt;
    [
        "/nix/var/nix/profiles/default/bin/nix-store",
        "/run/current-system/sw/bin/nix-store",
    ]
    .into_iter()
    .find_map(|fixed| {
        let fixed = Path::new(fixed);
        let canonical = fs::canonicalize(fixed).ok()?;
        let metadata = fs::symlink_metadata(&canonical).ok()?;
        (canonical.starts_with("/nix/store")
            && metadata.is_file()
            && metadata.mode() & 0o111 != 0
            && trusted_path(fixed)
            && trusted_path(&canonical))
        .then_some(canonical)
    })
}

#[cfg(unix)]
fn trusted_socket() -> bool {
    use std::os::unix::fs::FileTypeExt;
    let socket = Path::new("/nix/var/nix/daemon-socket/socket");
    fs::symlink_metadata(socket).is_ok_and(|m| m.file_type().is_socket())
        // A daemon socket normally allows user connections; its directory and
        // root ownership, not the socket's write bits, establish local routing.
        && {
            use std::os::unix::fs::MetadataExt;
            fs::symlink_metadata(socket).is_ok_and(|m| m.uid() == 0)
        }
        && socket.parent().is_some_and(trusted_path)
        && fs::canonicalize(socket).ok().as_deref() == Some(socket)
}

#[cfg(unix)]
fn command(program: &Path, output: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    configure(&mut command, output);
    command
}

#[cfg(unix)]
fn configure(command: &mut std::process::Command, output: &Path) {
    use std::os::unix::process::CommandExt;
    // nix-store may resolve to the unified nix executable; retain its dispatch
    // name without executing a mutable profile symlink or a shell.
    command
        .arg0("nix-store")
        .args([
            "--store",
            "unix:///nix/var/nix/daemon-socket/socket?store=/nix/store&real=/nix/store",
            "--verify-path",
        ])
        .arg(output)
        .env_clear()
        .env("LC_ALL", "C")
        .env("HOME", "/dev/null")
        .env("NIX_CONF_DIR", "/dev/null")
        .env("NIX_USER_CONF_FILES", "/dev/null")
        .env("NIX_CONFIG", "")
        .env("NIX_PATH", "")
        .env("XDG_CONFIG_HOME", "/dev/null")
        .env("XDG_CACHE_HOME", "/dev/null")
        .env("XDG_DATA_HOME", "/dev/null")
        .env("XDG_STATE_HOME", "/dev/null")
        .current_dir("/")
        .stdin(std::process::Stdio::null());
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        os::unix::fs::{PermissionsExt, symlink},
        path::PathBuf,
    };
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tmp")
                .join(format!("nix-owner-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            Self(fs::canonicalize(root).unwrap())
        }
        fn binary(&self, name: &str) -> PathBuf {
            let path = self
                .0
                .join("store/00000000000000000000000000000000-flere/bin")
                .join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, b"exact fixture executable").unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn both_components_require_registered_content_and_exact_live_identity() {
        let fixture = Fixture::new();
        for component in ["flere", "flere-connect"] {
            let executable = fixture.binary(component);
            let store = fixture.0.join("store");
            let verified = verify(&executable, &store, |output| {
                assert_eq!(output, executable.parent().unwrap().parent().unwrap());
                Some(Vec::new())
            });
            let owner = upgrade(verified);
            assert!(owner.verified);
            assert!(owner.command.is_none());
            assert!(owner.detail.contains("owning Nix configuration"));
            // Missing registration, a changed NAR, timeout, or noisy output is
            // never accepted as a verified manager result.
            assert!(!verify(&executable, &store, |_| None));
            assert!(!verify(&executable, &store, |_| Some(
                b"unexpected".to_vec()
            )));
            assert!(!verify(&executable, &store, |_| {
                fs::write(&executable, b"changed while checking").unwrap();
                Some(Vec::new())
            }));
            let alias = fixture.0.join(component);
            symlink(&executable, &alias).unwrap();
            assert!(!verify(&alias, &store, |_| panic!(
                "unresolved alias must not query"
            )));
        }
    }

    #[test]
    fn a_store_shaped_path_is_never_itself_ownership_evidence() {
        assert!(detect(Path::new("/ordinary/manual/flere")).is_none());
        for path in [
            "/nix/store",
            "/nix/store/not-a-store-output/bin/flere",
            "/nix/store/00000000000000000000000000000000-missing/bin/flere-connect",
        ] {
            let owner = detect(Path::new(path)).unwrap();
            assert!(!owner.verified);
            assert!(owner.command.is_none());
        }
    }

    #[test]
    fn query_is_fixed_local_read_only_and_cannot_inherit_configuration() {
        let output = Path::new("/nix/store/00000000000000000000000000000000-flere");
        let command = command(Path::new("/nix/store/trusted-nix/bin/nix"), output);
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(
            args,
            [
                "--store",
                "unix:///nix/var/nix/daemon-socket/socket?store=/nix/store&real=/nix/store",
                "--verify-path",
                output.to_str().unwrap()
            ]
        );
        assert_eq!(command.get_current_dir(), Some(Path::new("/")));
        let environment: BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
            .collect();
        assert_eq!(environment.len(), 10);
        for name in [
            "NIX_REMOTE",
            "NIX_DAEMON_SOCKET_PATH",
            "NIX_STORE_DIR",
            "NIX_STATE_DIR",
            "PATH",
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
        ] {
            assert!(!environment.contains_key(name));
        }
        assert_eq!(environment["NIX_USER_CONF_FILES"], "/dev/null");
        assert_eq!(environment["NIX_CONF_DIR"], "/dev/null");
        assert_eq!(environment["NIX_CONFIG"], "");
        // Also execute a harmless owned fixture after adding hostile selectors:
        // env_clear must remove pre-existing command overrides, not only omit
        // these names from get_envs() while still inheriting them.
        let fixture = Fixture::new();
        let script = fixture.0.join("query-fixture");
        fs::write(&script, b"#!/bin/sh\nprintf '%s\\n' \"${NIX_REMOTE-unset}\" \"${NIX_DAEMON_SOCKET_PATH-unset}\" \"$HOME\" \"$NIX_USER_CONF_FILES\"\n").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = std::process::Command::new(script);
        child
            .env("NIX_REMOTE", "ssh://must-not-connect.invalid")
            .env("NIX_DAEMON_SOCKET_PATH", "/foreign/socket")
            .env("NIX_USER_CONF_FILES", "/foreign/plugin-config");
        configure(&mut child, output);
        let bytes =
            super::super::super::super::run(&mut child, 4096, std::time::Duration::from_secs(2))
                .unwrap();
        assert_eq!(bytes, b"unset\nunset\n/dev/null\n/dev/null\n");
    }
}
