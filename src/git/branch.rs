//! A read-only comparison against the locally known PR base/default branch.
//! Resolve refs once; file actions use the captured commit identities thereafter.
use super::{
    history::{self, Details},
    output,
};
use crate::wire::invalid;
use std::{collections::BTreeMap, io, path::Path};

#[derive(Clone, Debug)]
pub struct BranchDiff {
    pub target: String,
    pub details: Details,
}

fn text(cwd: &Path, args: &[&str]) -> io::Result<String> {
    String::from_utf8(output(cwd, args)?)
        .map(|s| s.trim_end_matches('\n').to_owned())
        .map_err(io::Error::other)
}
fn optional(cwd: &Path, args: &[&str]) -> io::Result<Option<String>> {
    match text(cwd, args) {
        Ok(s) if s.is_empty() => Ok(None),
        Ok(s) => Ok(Some(s)),
        // Git's quiet lookup commands return 1 and no diagnostic for absence.
        Err(e) if e.kind() == io::ErrorKind::Other && e.to_string().is_empty() => Ok(None),
        Err(e) => Err(e),
    }
}
fn config(cwd: &Path, key: &str) -> io::Result<Option<String>> {
    optional(cwd, &["config", "--get", key])
}

pub fn inspect(cwd: &Path) -> io::Result<Option<BranchDiff>> {
    let Some(head) = optional(cwd, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])? else {
        return Ok(None); // An unborn repository has working changes, but no branch diff.
    };
    history::oid(&head)?;
    let branch = optional(cwd, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let preferred = match &branch {
        Some(name) => config(cwd, &format!("branch.{name}.gh-merge-base"))?,
        None => None,
    };
    let remote = match &branch {
        Some(name) => config(cwd, &format!("branch.{name}.remote"))?,
        None => None,
    }
    .filter(|s| s != ".")
    .unwrap_or_else(|| "origin".into());
    let listing = text(
        cwd,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)",
            "refs/heads/",
            "refs/remotes/",
        ],
    )?;
    let mut refs = BTreeMap::new();
    let mut defaults = BTreeMap::new();
    for line in listing.lines() {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 3 {
            return Err(invalid("malformed Git reference listing"));
        }
        history::oid(fields[1])?;
        refs.insert(fields[0].to_owned(), fields[1].to_owned());
        if !fields[2].is_empty() && fields[0].ends_with("/HEAD") {
            defaults.insert(fields[0].to_owned(), fields[2].to_owned());
        }
    }
    let (target, target_sha) = if let Some(base) = preferred {
        // An explicit missing base is an error, not permission to compare another branch.
        let candidates = [
            format!("refs/remotes/{remote}/{base}"),
            format!("refs/heads/{base}"),
            format!("refs/remotes/{base}"),
            base.clone(),
        ];
        let sha = candidates
            .iter()
            .find_map(|r| refs.get(r))
            .cloned()
            .ok_or_else(|| {
                invalid(&format!(
                    "comparison base {} is unavailable locally",
                    crate::wire::passive(&base)
                ))
            })?;
        (base, sha)
    } else {
        let default = defaults
            .get(&format!("refs/remotes/{remote}/HEAD"))
            .or_else(|| defaults.get("refs/remotes/origin/HEAD"))
            .or_else(|| {
                if defaults.len() == 1 {
                    defaults.values().next()
                } else {
                    None
                }
            });
        if let Some(reference) = default {
            let sha = refs
                .get(reference)
                .cloned()
                .ok_or_else(|| invalid("default branch ref is unavailable"))?;
            let label = reference.strip_prefix("refs/remotes/").unwrap_or(reference);
            (label.to_owned(), sha)
        } else {
            let initial = config(cwd, "init.defaultBranch")?;
            let mut names = vec!["main".to_owned(), "master".to_owned()];
            if let Some(name) = initial {
                names.insert(0, name);
            }
            let found = names.iter().find_map(|name| {
                [
                    format!("refs/remotes/{remote}/{name}"),
                    format!("refs/heads/{name}"),
                ]
                .into_iter()
                .find_map(|r| refs.get(&r).map(|sha| (name.clone(), sha.clone())))
            });
            let Some(found) = found else {
                return Ok(None);
            };
            found
        }
    };
    let bases = optional(cwd, &["merge-base", "--all", &target_sha, &head])?
        .ok_or_else(|| invalid("comparison branches have no common ancestor"))?;
    let mut bases = bases.lines();
    let base = bases
        .next()
        .ok_or_else(|| invalid("comparison branches have no common ancestor"))?
        .to_owned();
    if bases.next().is_some() {
        return Err(invalid("comparison has multiple merge bases"));
    }
    history::oid(&base)?;
    // Equal trees can have different histories (e.g. squash/revert). Hide the
    // section in that case too, while excluding upstream-only changes otherwise.
    let head_tree = text(cwd, &["rev-parse", &format!("{head}^{{tree}}")])?;
    let target_tree = text(cwd, &["rev-parse", &format!("{target_sha}^{{tree}}")])?;
    let files = if base == head || head_tree == target_tree {
        Vec::new()
    } else {
        history::parse_files(&output(
            cwd,
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--name-status",
                "-z",
                "-M",
                &base,
                &head,
                "--",
            ],
        )?)?
    };
    Ok(Some(BranchDiff {
        target: crate::wire::passive(&target),
        details: Details {
            message: format!(
                "Branch comparison with {}\nBase: {base}\nHead: {head}\n",
                crate::wire::passive(&target)
            ),
            sha: head,
            parent: Some(base),
            files,
        },
    }))
}
