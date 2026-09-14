//! Projects use a main checkout; additional project cards use linked worktrees.
use super::*;

fn remote_name(remote: &str) -> Option<&str> {
    if remote.is_empty() || remote.chars().any(char::is_control) {
        return None;
    }
    let path = if let Some((scheme, address)) = remote.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git" | "file") {
            return None;
        }
        let (authority, path) = address.split_once('/')?;
        if (authority.is_empty() && scheme != "file") || path.contains(['?', '#', '@']) {
            return None;
        }
        path
    } else if let Some((host, path)) = remote.split_once(':') {
        if host.is_empty() || host.contains('/') || path.contains([':', '?', '#', '@']) {
            return None;
        }
        path
    } else {
        remote
    };
    let last = path.trim_end_matches('/').rsplit('/').next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 256
        && !name.chars().any(char::is_control))
    .then_some(name)
}

fn project_name(root: &Path) -> String {
    // GitHub SSH/HTTPS remotes carry the repository name locally. Reading the
    // config never contacts a host, invokes gh, or requires authentication.
    crate::git::checkout_identity_output(root, &["config", "--get", "remote.origin.url"])
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|remote| remote_name(remote.trim_end_matches('\n')).map(str::to_owned))
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("Project")
                .into()
        })
}

impl Server {
    pub(super) fn add_project(
        &mut self,
        directory: &Path,
        name: &str,
        shell: bool,
    ) -> io::Result<Vec<u8>> {
        let root = crate::git::primary_root(directory)?;
        let default = name.is_empty().then(|| project_name(&root));
        let name = default.as_deref().unwrap_or(name);
        if name.len() > 256 || name.chars().any(char::is_control) {
            return Err(invalid("invalid project name"));
        }
        let existing = self
            .workspaces
            .iter()
            .position(|w| w.cwd == root && !w.meta.archived);
        let wid = if let Some(index) = existing {
            let wid = self.workspaces[index].id;
            if self.workspaces[index].meta.project.is_empty() {
                self.workspaces[index].meta.project = name.into();
                if let Err(e) = self.persist() {
                    self.workspaces[index].meta.project.clear();
                    return Err(e);
                }
            }
            self.active = wid;
            if shell
                && self.workspaces[index].tabs.is_empty()
                && !self.restoration.pending.iter().any(|p| p.workspace == wid)
            {
                self.spawn(wid)?;
            }
            self.changed();
            wid
        } else {
            if self
                .workspaces
                .iter()
                .any(|w| w.cwd == root && w.meta.archived)
            {
                return Err(invalid(
                    "the primary project card is archived; restore it instead of creating a duplicate",
                ));
            }
            let result = self.create_workspace(
                name.into(),
                root.clone(),
                CardMeta {
                    project: name.into(),
                    ..Default::default()
                },
                shell,
            )?;
            serde_json::from_slice::<serde_json::Value>(&result).map_err(io::Error::other)?["workspace"].as_u64().ok_or_else(||invalid("missing created workspace"))?
        };
        self.audit(
            "add-project",
            wid,
            "primary checkout; ordinary agent capabilities",
        )?;
        serde_json::to_vec(&serde_json::json!({"workspace":wid,"primary":true,"directory":root}))
            .map_err(io::Error::other)
    }
    pub(super) fn prepare_project_card(
        &mut self,
        name: String,
        cwd: PathBuf,
        project: String,
    ) -> io::Result<Vec<u8>> {
        // Keep the low-level directory API useful outside Git. Within a project,
        // never make another card share its main checkout for implementation work.
        let root = match crate::git::primary_root(&cwd) {
            Ok(root) => root,
            Err(error) => {
                // A failed Git lookup must not silently make a loose card inside
                // a known checkout (permissions, unavailable main tree, bad Git).
                if cwd.ancestors().any(|p| p.join(".git").exists()) {
                    return Err(error);
                }
                return self.create_workspace(
                    name,
                    cwd,
                    CardMeta {
                        project,
                        ..Default::default()
                    },
                    false,
                );
            }
        };
        let checkout = crate::git::output(&cwd, &["rev-parse", "--show-toplevel"])?;
        let checkout = std::str::from_utf8(&checkout)
            .map_err(io::Error::other)?
            .trim_end_matches('\n');
        let cwd = fs::canonicalize(checkout)?;
        if cwd == root && !self.workspaces.iter().any(|w| w.cwd == root) {
            return self.add_project(&root, if project.is_empty() { "" } else { &project }, false);
        }
        if cwd != root && !self.workspaces.iter().any(|w| w.cwd == cwd) {
            return self.create_workspace(
                name,
                cwd,
                CardMeta {
                    project,
                    ..Default::default()
                },
                false,
            );
        }
        let project = if project.is_empty() {
            self.workspaces
                .iter()
                .find(|w| w.cwd == root)
                .map(|w| w.meta.project.clone())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| {
                    root.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
        } else {
            project
        };
        let slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .take(48)
            .collect();
        let slug = slug.trim_matches('-');
        let branch = format!(
            "flere/{}-{}",
            if slug.is_empty() { "workspace" } else { slug },
            self.next
        );
        self.command(
            &[
                "worktree-stopped".into(),
                wire::hex(name.as_bytes()),
                wire::hex(root.to_str().unwrap().as_bytes()),
                wire::hex(branch.as_bytes()),
                wire::hex(b"HEAD"),
                wire::hex(project.as_bytes()),
            ]
            .join("\t"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_come_from_repository_paths_never_url_credentials() {
        for remote in [
            "https://github.com/owner/actual-repo.git",
            "https://user:secret@github.com/owner/actual-repo.git",
            "git@github.com:owner/actual-repo.git",
            "ssh://git@github.com:2222/owner/actual-repo.git/",
            "/offline/actual-repo.git",
        ] {
            assert_eq!(remote_name(remote), Some("actual-repo"));
        }
        for remote in [
            "https://user:secret@github.com",
            "https://user:secret@github.com/",
            "https:///actual-repo.git",
            "https://github.com/repo.git?token=secret",
            "https://github.com/repo.git#secret",
            "ext::helper secret",
            "git@github.com:",
            "../",
            "\u{1b}[secret",
        ] {
            assert!(remote_name(remote).is_none(), "invalid remote accepted");
        }
    }
}
