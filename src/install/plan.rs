//! A reviewed, immutable candidate shared by local and companion update flows.
//! Preparation never replaces an installed command or starts a supervisor.
use super::*;
use serde::{Deserialize, Serialize};
use std::{
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const LIFETIME: u64 = 15 * 60;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u64,
    token: String,
    expires: u64,
    runtime: RuntimeIdentity,
    candidate: InstalledPackage,
    source: PackageSource,
    status: String,
    receipt: Option<InstallReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owners: Option<crate::remote_update::CoreOwners>,
}
fn valid_schema(plan: &Plan, guarded: bool) -> bool {
    matches!(
        (plan.schema_version, plan.owners.is_some(), guarded),
        (1, false, false) | (2, true, true)
    )
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn location(store: &Store, token: &str) -> io::Result<PathBuf> {
    if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid prepared update token"));
    }
    Ok(store.root.join("plans").join(format!("{token}.json")))
}
fn source_package(
    store: &Store,
    state: &Path,
    source: &str,
) -> io::Result<(StagedPackage, PackageSource)> {
    if source == "--rollback" {
        let receipt = store
            .status()?
            .ok_or_else(|| invalid("installation is not managed"))?;
        let previous = receipt
            .previous
            .ok_or_else(|| invalid("no previous package retained"))?;
        let staged = store.stage(
            previous
                .executable
                .parent()
                .ok_or_else(|| invalid("invalid retained package"))?,
        )?;
        return Ok((staged, receipt.source));
    }
    let source = if source.is_empty() {
        if let Some(receipt) = store.status()?
            && matches!(receipt.source, PackageSource::DefaultChannel {})
        {
            private_dir(&store.root)?;
            let path = store
                .root
                .join(format!(".download-{}", crate::os::nonce()?));
            super::package::download_default(
                &path,
                "flere",
                Some(&receipt.current.manifest.build.package_version),
            )?;
            let staged = store.stage(&path);
            let _ = fs::remove_dir_all(&path);
            return Ok((staged?, receipt.source));
        }
        if let Some(checkout) = store.local_source()? {
            let bytes = run(
                Command::new(checkout.join("scripts/dev"))
                    .args(["package", "--state"])
                    .arg(state)
                    .stdin(Stdio::null()),
                MAX_RECEIPT as usize,
                Duration::from_secs(1800),
            )?;
            let result: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            let artifact = result["artifact"]
                .as_str()
                .ok_or_else(|| invalid("development packaging did not return its artifact"))?;
            return Ok((
                store.stage(&Path::new(artifact).join("flere"))?,
                PackageSource::Local,
            ));
        }
        match store.status()?.map(|r| r.source) {
            Some(PackageSource::Public { manifest_url }) => manifest_url,
            _ => {
                return Err(invalid(
                    "choose a package or HTTPS manifest, or register a development checkout",
                ));
            }
        }
    } else {
        source.into()
    };
    if source.starts_with("https://") {
        private_dir(&store.root)?;
        let path = store
            .root
            .join(format!(".download-{}", crate::os::nonce()?));
        download(&source, &path)?;
        let staged = store.stage(&path);
        let _ = fs::remove_dir_all(&path);
        Ok((
            staged?,
            PackageSource::Public {
                manifest_url: source,
            },
        ))
    } else {
        let path = PathBuf::from(source);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        Ok((store.stage(&path)?, PackageSource::Local))
    }
}
pub(super) fn command(state: &Path, args: &[String]) -> io::Result<Vec<u8>> {
    execute(state, args, None, false)
}
pub(super) fn coordinated_command(
    state: &Path,
    args: &[String],
    frontend: u32,
) -> io::Result<Vec<u8>> {
    if args == ["update-ownership-v1"] {
        let owners = super::ownership::selected(state, frontend)?;
        let bytes = serde_json::to_vec(&owners).map_err(io::Error::other)?;
        if bytes.len() > crate::remote_update::OWNERSHIP_LIMIT {
            return Err(invalid("ownership report exceeds bound"));
        }
        return Ok(bytes);
    }
    let action = args.first().map(String::as_str).unwrap_or("");
    let (legacy, adopt) = match (action, args.len()) {
        ("update-prepare-v1", 2) => ("update-prepare", false),
        ("update-plan-v1", 2) => ("update-plan", false),
        ("update-apply-v1", 3) if matches!(args[2].as_str(), "adopt" | "managed") => {
            ("update-apply", args[2] == "adopt")
        }
        _ => return Err(invalid("unsupported ownership-checked update command")),
    };
    execute(
        state,
        &[legacy.into(), args[1].clone()],
        Some(frontend),
        adopt,
    )
}
fn check_owners(
    state: &Path,
    frontend: u32,
    expected: &crate::remote_update::CoreOwners,
) -> io::Result<()> {
    let current = super::ownership::selected(state, frontend)?;
    current.require_update()?;
    if &current != expected {
        return Err(invalid(
            "remote installation ownership changed; prepare again",
        ));
    }
    Ok(())
}
fn execute(
    state: &Path,
    args: &[String],
    frontend: Option<u32>,
    adopt: bool,
) -> io::Result<Vec<u8>> {
    let store = Store::for_user("flere")?;
    let action = args.first().map(String::as_str).unwrap_or("");
    if args.len() != 2 {
        return Err(invalid(
            "usage: update-prepare SOURCE | update-apply TOKEN | update-plan TOKEN",
        ));
    }
    if action == "update-prepare" {
        ensure_update_endpoint(state)?;
        let owners = if let Some(frontend) = frontend {
            let owners = super::ownership::selected(state, frontend)?;
            owners.require_update()?;
            Some(owners)
        } else {
            None
        };
        let before = runtime_identity(state)?;
        let (staged, source) = source_package(&store, state, &args[1])?;
        super::package::require_source_support(
            &staged.package.manifest,
            &staged.package.executable,
            &source,
        )?;
        if frontend.is_some() {
            let capability = run(
                Command::new(&staged.package.executable)
                    .arg("--coordinated-update-info")
                    .stdin(Stdio::null()),
                128,
                Duration::from_secs(3),
            );
            if !capability.is_ok_and(|bytes| bytes == crate::remote_update::CANDIDATE_CAPABILITY) {
                return Err(invalid(
                    "Core candidate cannot prove coordinated ownership support. Upgrade this endpoint manually first.",
                ));
            }
        }
        if !staged.package.manifest.build.accepts_runtime(&before.build) {
            return Err(invalid(
                "candidate cannot read the selected runtime handoff/state",
            ));
        }
        let actual = runtime_identity(state)?;
        if actual.epoch != before.epoch || actual.pid != before.pid || actual.build != before.build
        {
            return Err(invalid(
                "supervisor changed while preparing update; installation retained",
            ));
        }
        if let Some(owners) = &owners {
            check_owners(state, frontend.unwrap(), owners)?;
        }
        let token = crate::os::nonce()?;
        let plan = Plan {
            schema_version: if frontend.is_some() { 2 } else { 1 },
            token: token.clone(),
            expires: now() + LIFETIME,
            runtime: actual,
            candidate: staged.package,
            source,
            status: "prepared".into(),
            receipt: None,
            owners,
        };
        private_dir(&store.root.join("plans"))?;
        atomic_json(&location(&store, &token)?, &plan)?;
        return serde_json::to_vec_pretty(&plan).map_err(io::Error::other);
    }
    if !matches!(action, "update-apply" | "update-plan") {
        return Err(invalid("unknown prepared update command"));
    }
    let path = location(&store, &args[1])?;
    if action == "update-plan" {
        let plan: Plan = read_bounded_json(&path, MAX_RECEIPT)?;
        if !valid_schema(&plan, frontend.is_some())
            || plan.token != args[1]
            || plan.runtime.state != state
        {
            return Err(invalid(
                "prepared update does not belong to the selected state",
            ));
        }
        let readiness = (|| -> io::Result<()> {
            if plan.status != "prepared" || now() > plan.expires {
                return Err(invalid("plan is expired or no longer awaiting apply"));
            }
            if let Some(frontend) = frontend {
                check_owners(
                    state,
                    frontend,
                    plan.owners.as_ref().ok_or_else(|| {
                        invalid("legacy plan has no ownership evidence; prepare again")
                    })?,
                )?;
            }
            let actual = runtime_identity(state)?;
            if actual.epoch != plan.runtime.epoch
                || actual.pid != plan.runtime.pid
                || actual.build != plan.runtime.build
            {
                return Err(invalid("supervisor changed since preparation"));
            }
            plan.candidate.manifest.verify(
                plan.candidate
                    .executable
                    .parent()
                    .ok_or_else(|| invalid("invalid prepared package path"))?,
            )
        })();
        let mut value = serde_json::to_value(&plan).map_err(io::Error::other)?;
        value["ready"] = readiness.is_ok().into();
        value["readiness_detail"] = readiness
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
            .into();
        return serde_json::to_vec_pretty(&value).map_err(io::Error::other);
    }
    // One plan cannot be applied twice by concurrent SSH helpers. No filesystem
    // lock is held by the supervisor; installation's own lock is separate.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.with_extension("lock"))?;
    if !lock.metadata()?.is_file() {
        return Err(invalid("invalid update plan lock"));
    }
    lock.try_lock()
        .map_err(|_| invalid("prepared update is being applied by another process"))?;
    let mut plan: Plan = read_bounded_json(&path, MAX_RECEIPT)?;
    if !valid_schema(&plan, frontend.is_some())
        || plan.token != args[1]
        || plan.runtime.state != state
    {
        return Err(invalid(
            "prepared update does not belong to the selected state",
        ));
    }
    if frontend.is_some() && plan.owners.is_none() {
        return Err(invalid(
            "legacy plan has no ownership evidence; prepare again",
        ));
    }
    if frontend.is_none() && plan.owners.is_some() {
        return Err(invalid(
            "coordinated plan requires its local ownership confirmation",
        ));
    }
    if let Some(receipt) = &plan.receipt {
        return serde_json::to_vec_pretty(receipt).map_err(io::Error::other);
    }
    if plan.status != "prepared" {
        return Err(invalid(
            "prepared update has an uncertain prior attempt; inspect install-status before retrying",
        ));
    }
    if now() > plan.expires {
        return Err(invalid(
            "prepared update expired; prepare a fresh candidate",
        ));
    }
    if let Some(owners) = &plan.owners {
        check_owners(state, frontend.unwrap(), owners)?;
        if owners.manual() && !adopt {
            return Err(invalid(
                "manual remote installation requires explicit local adoption confirmation",
            ));
        }
    }
    ensure_update_endpoint(state)?;
    let before = runtime_identity(state)?;
    if before.epoch != plan.runtime.epoch
        || before.pid != plan.runtime.pid
        || before.build != plan.runtime.build
    {
        return Err(invalid(
            "prepared update targets a different supervisor generation; installation retained",
        ));
    }
    // Capture the current session set immediately before activation. Work opened
    // during review is preserved too; the plan never restores an old tab layout.
    let staged = store.stage(
        plan.candidate
            .executable
            .parent()
            .ok_or_else(|| invalid("invalid candidate directory"))?,
    )?;
    if staged.package != plan.candidate {
        return Err(invalid("prepared candidate changed before apply"));
    }
    plan.status = "applying".into();
    atomic_json(&path, &plan)?;
    let receipt = if let Some(owners) = &plan.owners {
        store.install_guarded(&staged, plan.source.clone(), adopt, || {
            check_owners(state, frontend.unwrap(), owners)
        })?
    } else {
        store.install(&staged, plan.source.clone(), true)?
    };
    let activation = activate(state, &receipt.current, &before);
    let receipt = if activation.can_restore_installation && receipt.previous.is_some() {
        store.restore_rejected(&receipt.attempt, activation)?
    } else {
        store.record_activation(&receipt.attempt, activation)?
    };
    plan.status = receipt.status.clone();
    plan.receipt = Some(receipt.clone());
    atomic_json(&path, &plan)?;
    serde_json::to_vec_pretty(&receipt).map_err(io::Error::other)
}
