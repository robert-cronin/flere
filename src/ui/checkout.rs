//! Read-only checkout identity, cached off the UI loop. A branch name and the
//! first visible card never establish which directory is Git's main worktree.
use super::*;
use crate::model::WorkspaceView;
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

const LIMIT: usize = 128;
const RECHECK: Duration = Duration::from_secs(15);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Main,
    Linked,
}
fn git_path(cwd: &Path, option: &str) -> io::Result<PathBuf> {
    let bytes = crate::git::checkout_identity_output(
        cwd,
        &["rev-parse", "--path-format=absolute", option],
    )?;
    let value = String::from_utf8(bytes).map_err(io::Error::other)?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    if value.is_empty() {
        return Err(io::Error::other("missing Git directory"));
    }
    Path::new(value).canonicalize()
}
fn inspect(cwd: &Path) -> Option<Kind> {
    let inside =
        crate::git::checkout_identity_output(cwd, &["rev-parse", "--is-inside-work-tree"]).ok()?;
    if inside != b"true\n" {
        return None;
    }
    let directory = git_path(cwd, "--git-dir").ok()?;
    let common = git_path(cwd, "--git-common-dir").ok()?;
    Some(if directory == common {
        Kind::Main
    } else {
        Kind::Linked
    })
}
struct Entry {
    kind: Option<Kind>,
    at: Instant,
}
pub(super) struct Checkouts {
    values: HashMap<String, Entry>,
    pending: HashSet<String>,
    send: mpsc::SyncSender<String>,
    receive: mpsc::Receiver<(String, Option<Kind>)>,
    stopped: Arc<AtomicBool>,
}
impl Drop for Checkouts {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}
impl Checkouts {
    pub fn new() -> Self {
        let (send, requests) = mpsc::sync_channel::<String>(1);
        let (results, receive) = mpsc::sync_channel(1);
        let stopped = Arc::new(AtomicBool::new(false));
        let cancelled = stopped.clone();
        std::thread::spawn(move || {
            while let Ok(cwd) = requests.recv() {
                if cancelled.load(Ordering::Relaxed) {
                    break;
                }
                // Existing Git reads bound subprocess time and output. Only this
                // one worker can inspect a slow filesystem; UI input never waits.
                let kind = inspect(Path::new(&cwd));
                if cancelled.load(Ordering::Relaxed) || results.send((cwd, kind)).is_err() {
                    break;
                }
            }
        });
        Self {
            values: HashMap::new(),
            pending: HashSet::new(),
            send,
            receive,
            stopped,
        }
    }
    pub fn poll(&mut self, snapshot: &Snapshot) -> bool {
        let paths: HashSet<_> = snapshot.workspaces.iter().map(|w| w.cwd.as_str()).collect();
        self.values.retain(|cwd, _| paths.contains(cwd.as_str()));
        let mut changed = false;
        while let Ok((cwd, kind)) = self.receive.try_recv() {
            self.pending.remove(&cwd);
            if paths.contains(cwd.as_str()) {
                changed |= self.values.get(&cwd).and_then(|e| e.kind) != kind;
                self.values.insert(
                    cwd,
                    Entry {
                        kind,
                        at: Instant::now(),
                    },
                );
            }
        }
        // Inspect the active workspace first; cap both retained values and queued
        // work. Removed-directory results cannot become another card's identity.
        for workspace in snapshot
            .workspace()
            .into_iter()
            .chain(snapshot.workspaces.iter())
        {
            let cwd = &workspace.cwd;
            if self.pending.contains(cwd)
                || self
                    .values
                    .get(cwd)
                    .is_some_and(|e| e.at.elapsed() < RECHECK)
            {
                continue;
            }
            let occupied = self.values.len()
                + self
                    .pending
                    .iter()
                    .filter(|path| !self.values.contains_key(*path))
                    .count();
            if !self.values.contains_key(cwd) && occupied >= LIMIT {
                if workspace.id != snapshot.active {
                    continue;
                }
                // A newly active directory must still resolve after the cache
                // fills. Never evict it or an in-flight result's retained entry.
                let oldest = self
                    .values
                    .iter()
                    .filter(|(path, _)| *path != cwd && !self.pending.contains(*path))
                    .min_by_key(|(_, entry)| entry.at)
                    .map(|(path, _)| path.clone());
                let Some(oldest) = oldest else { continue };
                if let Some(entry) = self.values.remove(&oldest) {
                    changed |= entry.kind == Some(Kind::Main);
                }
            }
            if self.send.try_send(cwd.clone()).is_ok() {
                self.pending.insert(cwd.clone());
            }
        }
        changed
    }
}
impl Ui {
    pub(super) fn is_main_checkout(&self, workspace: &WorkspaceView) -> bool {
        self.checkouts
            .values
            .get(&workspace.cwd)
            .is_some_and(|entry| entry.kind == Some(Kind::Main))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::unix::fs::DirBuilderExt, process::Command};
    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args([
                "-c",
                "user.name=Flere Fixture",
                "-c",
                "user.email=fixture@invalid",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn main_checkout_tracks_git_directory_identity_not_branch_or_card_order() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("checkout-{}", crate::os::nonce().unwrap()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.join("project café/nested"))
            .unwrap();
        let main = root.join("project café");
        git(&main, &["init", "-q", "-b", "trunk"]);
        git(&main, &["commit", "--allow-empty", "-qm", "fixture"]);
        let linked = root.join("linked");
        git(
            &main,
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        assert_eq!(inspect(&linked), Some(Kind::Linked));
        assert_eq!(inspect(&main), Some(Kind::Main));
        assert_eq!(inspect(&main.join("nested")), Some(Kind::Main));
        git(&main, &["checkout", "--detach"]);
        assert_eq!(inspect(&main), Some(Kind::Main));
        assert_eq!(inspect(&root), None);
        assert_eq!(inspect(&root.join("missing")), None);
        let bare = root.join("bare");
        git(&root, &["init", "--bare", "-q", bare.to_str().unwrap()]);
        assert_eq!(inspect(&bare), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checkout_identity_ignores_inherited_repository_selectors() {
        const PROBE: &str = "FLERE_CHECKOUT_IDENTITY_TEST";
        if let Some(root) = std::env::var_os(PROBE) {
            let root = PathBuf::from(root);
            assert_eq!(inspect(&root.join("linked")), Some(Kind::Linked));
            assert_eq!(inspect(&root.join("main")), Some(Kind::Main));
            assert_eq!(inspect(&root), None);
            return;
        }
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("checkout-env-{}", crate::os::nonce().unwrap()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(root.join("main"))
            .unwrap();
        let main = root.join("main");
        git(&main, &["init", "-q"]);
        git(&main, &["commit", "--allow-empty", "-qm", "fixture"]);
        git(
            &main,
            &[
                "worktree",
                "add",
                "--detach",
                root.join("linked").to_str().unwrap(),
            ],
        );
        // Pollute a real child test process, without changing the parallel test
        // runner's environment or substituting a command-builder assertion.
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ui::checkout::tests::checkout_identity_ignores_inherited_repository_selectors",
                "--nocapture",
            ])
            .env(PROBE, &root)
            .env("GIT_DIR", main.join(".git"))
            .env("GIT_COMMON_DIR", main.join(".git"))
            .env("GIT_WORK_TREE", &main)
            .output()
            .unwrap();
        assert!(
            child.status.success(),
            "{}",
            String::from_utf8_lossy(&child.stderr)
        );
        assert!(String::from_utf8_lossy(&child.stdout).contains("1 passed"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn newly_active_checkout_resolves_after_cache_reaches_its_limit() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tmp")
            .join(format!("checkout-cap-{}", crate::os::nonce().unwrap()));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)
            .unwrap();
        git(&root, &["init", "-q"]);
        let mut cache = Checkouts::new();
        let mut workspaces = Vec::new();
        for i in 0..=LIMIT {
            let cwd = if i == LIMIT {
                root.to_string_lossy().into_owned()
            } else {
                root.join(format!("retained-{i}"))
                    .to_string_lossy()
                    .into_owned()
            };
            if i != LIMIT {
                cache.values.insert(
                    cwd.clone(),
                    Entry {
                        kind: Some(Kind::Linked),
                        at: Instant::now(),
                    },
                );
            }
            workspaces.push(WorkspaceView {
                id: i as u64 + 1,
                name: format!("Workspace {i}"),
                cwd,
                tabs: Vec::new(),
                meta: Default::default(),
            });
        }
        let snapshot = Snapshot {
            split: None,
            epoch: "checkout-cache-fixture".into(),
            generation: 0,
            active: LIMIT as u64 + 1,
            tab: 0,
            workspaces,
            cols: 1,
            rows: 1,
            x: 0,
            y: 0,
            cursor: false,
            bracketed_paste: false,
            app_cursor: false,
            notice: String::new(),
            cells: Vec::new(),
        };
        let current = &snapshot.workspace().unwrap().cwd;
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            cache.poll(&snapshot);
            if cache
                .values
                .get(current)
                .is_some_and(|entry| entry.kind == Some(Kind::Main))
            {
                break;
            }
            assert!(Instant::now() < until, "active checkout never resolved");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(cache.values.len(), LIMIT);
        drop(cache);
        fs::remove_dir_all(root).unwrap();
    }
}
