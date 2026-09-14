use super::{
    Activation, Manifest, PackageSource, SourceReceipt, Store, activate, download, package,
    runtime_identity, source_digest,
};
use crate::wire::invalid;
use serde_json::json;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// The main parser delegates these commands before its workspace option parser.
/// No command here bootstraps application state or starts terminal sessions.
pub fn command(state: Option<&Path>, args: &[String]) -> io::Result<Vec<u8>> {
    let action = args.first().map(String::as_str).unwrap_or("");
    if action == "update-coordinated-v1" && args.len() == 3 {
        let frontend = args[1]
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid > 0)
            .ok_or_else(|| invalid("invalid selected frontend identity"))?;
        if args[2].len() > 8192 {
            return Err(invalid("coordinated request exceeds bound"));
        }
        let request: Vec<String> = serde_json::from_str(&args[2]).map_err(io::Error::other)?;
        return super::plan::coordinated_command(
            state.ok_or_else(|| invalid("coordinated update needs selected state"))?,
            &request,
            frontend,
        );
    }
    if matches!(action, "update-prepare" | "update-apply" | "update-plan") {
        return super::plan::command(
            state.ok_or_else(|| invalid("prepared update needs selected state"))?,
            args,
        );
    }
    let arg = |i: usize| {
        args.get(i)
            .map(String::as_str)
            .ok_or_else(|| invalid("missing installer argument"))
    };
    let absolute = |value: &str| -> io::Result<PathBuf> {
        let path = PathBuf::from(value);
        Ok(if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        })
    };
    let value = match action {
        "source-digest" if args.len() == 3 => {
            serde_json::to_value(source_digest(&absolute(arg(1)?)?, &absolute(arg(2)?)?)?)
                .map_err(io::Error::other)?
        }
        "package" if matches!(args.len(), 3 | 4) => {
            let source = if args.len() == 4 {
                Some(super::read_json::<SourceReceipt>(&absolute(arg(3)?)?)?)
            } else {
                None
            };
            serde_json::to_value(package(&absolute(arg(1)?)?, &absolute(arg(2)?)?, source)?)
                .map_err(io::Error::other)?
        }
        "install-status" if args.len() == 1 => match Store::for_user("flere")?.status()? {
            Some(receipt) => serde_json::to_value(receipt).map_err(io::Error::other)?,
            None => json!({"schema_version": 1, "status": "untracked"}),
        },
        "dev-source" if args.len() == 2 => {
            let store = Store::for_user("flere")?;
            store.configure_local_source(&absolute(arg(1)?)?)?;
            json!({"schema_version": 1, "status": "configured", "checkout": store.local_source()?})
        }
        "update-check" if args.len() == 1 => {
            let state = state.ok_or_else(|| invalid("update-check needs selected state"))?;
            match runtime_identity(state) {
                Ok(runtime) => {
                    super::ensure_update_endpoint(state)?;
                    json!({"schema_version":1,"status":"ready","runtime":runtime})
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    json!({"schema_version":1,"status":"not_running"})
                }
                Err(error) => return Err(error),
            }
        }
        "update-ack" if args.len() == 2 => {
            let store = Store::for_user("flere")?;
            let receipt = store
                .status()?
                .ok_or_else(|| invalid("installation is not managed"))?;
            let build: super::BuildMetadata =
                serde_json::from_str(crate::build_info::json()).map_err(io::Error::other)?;
            if receipt.attempt != arg(1)? || receipt.current.manifest.build != build {
                return Err(invalid(
                    "initiating frontend build/attempt differs from installed update",
                ));
            }
            let inventory: serde_json::Value = serde_json::from_slice(&crate::wire::request(
                state.ok_or_else(|| invalid("update acknowledgement needs selected state"))?,
                &["frontends"],
            )?)
            .map_err(io::Error::other)?;
            let expected_build = serde_json::to_value(&build).map_err(io::Error::other)?;
            if inventory["schema_version"] != 1
                || inventory["status"] != "known"
                || !inventory["tracked"].as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry["pid"] == std::process::id() && entry["build"] == expected_build
                    })
                })
            {
                return Err(invalid(
                    "this process is not a registered initiating frontend on the selected supervisor",
                ));
            }
            let activation = receipt
                .activation
                .ok_or_else(|| invalid("no supervisor activation is recorded"))?;
            let expected = activation
                .after
                .as_ref()
                .ok_or_else(|| invalid("supervisor activation was not verified"))?;
            let actual = runtime_identity(
                state.ok_or_else(|| invalid("update acknowledgement needs selected state"))?,
            )?;
            if actual.epoch != expected.epoch || actual.pid != expected.pid || actual.build != build
            {
                return Err(invalid(
                    "selected supervisor changed before frontend acknowledgement",
                ));
            }
            serde_json::to_value(store.record_activation(arg(1)?, activation.frontend_applied())?)
                .map_err(io::Error::other)?
        }
        "install" | "update" => {
            let mut adopt = false;
            let mut if_missing = false;
            let mut rollback = false;
            let mut frontend = false;
            let mut component = "flere";
            let mut source_arg = None;
            let mut from_url = None;
            let mut source_url = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--adopt" if !adopt => adopt = true,
                    "--if-missing" if !if_missing && action == "install" => if_missing = true,
                    "--rollback" if !rollback && action == "update" => rollback = true,
                    "--frontend" if !frontend && action == "update" => frontend = true,
                    "--component" if component == "flere" && action == "install" => {
                        i += 1;
                        component = arg(i)?;
                        if component != "flere-connect" {
                            return Err(invalid(
                                "--component supports flere-connect for file installation",
                            ));
                        }
                    }
                    "--from-url" if from_url.is_none() => {
                        i += 1;
                        from_url = Some(arg(i)?.to_owned());
                    }
                    "--source-url" if source_url.is_none() && action == "install" => {
                        i += 1;
                        super::package::https(arg(i)?)?;
                        source_url = Some(arg(i)?.to_owned());
                    }
                    value if !value.starts_with('-') && source_arg.is_none() => {
                        source_arg = Some(absolute(value)?)
                    }
                    _ => return Err(invalid("unsupported or duplicate installer argument")),
                }
                i += 1;
            }
            if if_missing && adopt {
                return Err(invalid("--if-missing cannot adopt an existing command"));
            }
            if (rollback && (source_arg.is_some() || from_url.is_some() || adopt))
                || (source_arg.is_some() && from_url.is_some())
            {
                return Err(invalid(
                    "choose exactly one local package, HTTPS source or rollback",
                ));
            }
            if source_url.is_some() && (source_arg.is_none() || from_url.is_some()) {
                return Err(invalid(
                    "--source-url labels one already verified local package",
                ));
            }
            let state = state.ok_or_else(|| invalid("installer requires selected state path"))?;
            let runtime = if action == "update" {
                match runtime_identity(state) {
                    Ok(runtime) => {
                        super::ensure_update_endpoint(state)?;
                        Some(runtime)
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                        ) =>
                    {
                        None
                    }
                    Err(error) => {
                        return Err(io::Error::other(format!(
                            "cannot establish update target identity: {error}"
                        )));
                    }
                }
            } else {
                None
            };
            let store = Store::for_user(component)?;
            let receipt = if rollback {
                store.rollback(runtime.as_ref().map(|r| &r.build))?
            } else {
                if source_arg.is_none() && from_url.is_none() {
                    from_url = match store.status()?.map(|r| r.source) {
                        Some(PackageSource::Public { manifest_url }) => Some(manifest_url),
                        _ => {
                            return Err(invalid(
                                "choose a local package or --from-url; local development installs are not switched to public packages automatically",
                            ));
                        }
                    };
                }
                let mut cleanup = None;
                let (directory, source) = if let Some(url) = from_url {
                    let home = std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .ok_or_else(|| invalid("HOME is required"))?;
                    let cache = std::env::var_os("XDG_CACHE_HOME")
                        .filter(|s| !s.is_empty())
                        .map(PathBuf::from)
                        .unwrap_or_else(|| home.join(".cache"));
                    let downloads = cache.join("flere/downloads");
                    super::private_dir(&downloads)?;
                    let directory = downloads.join(crate::os::nonce()?);
                    download(&url, &directory)?;
                    cleanup = Some(directory.clone());
                    (directory, PackageSource::Public { manifest_url: url })
                } else {
                    (
                        source_arg.unwrap(),
                        source_url.map_or(PackageSource::Local, |manifest_url| {
                            PackageSource::Public { manifest_url }
                        }),
                    )
                };
                let staged = store.stage(&directory);
                if let Some(directory) = cleanup {
                    let _ = fs::remove_dir_all(directory);
                }
                let staged = staged?;
                if runtime
                    .as_ref()
                    .is_some_and(|r| !staged.package.manifest.build.accepts_runtime(&r.build))
                {
                    return Err(invalid(
                        "candidate cannot read current runtime handoff/state; installation and sessions retained",
                    ));
                }
                if if_missing {
                    store.install_if_missing(&staged, source)?
                } else {
                    store.install(&staged, source, adopt)?
                }
            };
            let result = if let Some(runtime) = runtime {
                let mut activation = activate(state, &receipt.current, &runtime);
                if !frontend {
                    activation.initiating_frontend = "not_requested".into();
                    if activation.supervisor == "applied" {
                        activation.status = "applied".into();
                        activation.detail = "Installed and applied to the selected supervisor; exact session identities preserved; other attachments are reported separately".into();
                    }
                }
                if activation.can_restore_installation && receipt.previous.is_some() {
                    store.restore_rejected(&receipt.attempt, activation)?
                } else {
                    store.record_activation(&receipt.attempt, activation)?
                }
            } else {
                // An install command is explicitly file-only and starts nothing.
                let mut activation = Activation::not_running();
                if action == "install" {
                    activation.supervisor = "not_requested".into();
                    activation.detail =
                        "Installed; running components were not requested by first-time install"
                            .into();
                }
                store.record_activation(&receipt.attempt, activation)?
            };
            serde_json::to_value(result).map_err(io::Error::other)?
        }
        "verify-package" if args.len() == 2 => {
            let directory = absolute(arg(1)?)?;
            let manifest = Manifest::load(&directory)?;
            manifest.verify(&directory)?;
            json!({"schema_version": 1, "status": "verified", "manifest": manifest})
        }
        _ => {
            return Err(invalid(
                "usage: package BINARY OUTPUT [SOURCE-RECEIPT] | source-digest ROOT SCRATCH | verify-package DIRECTORY | install PACKAGE [--adopt|--if-missing] | update [PACKAGE|--from-url HTTPS_MANIFEST|--rollback] [--adopt] | install-status | update-ack ATTEMPT",
            ));
        }
    };
    serde_json::to_vec_pretty(&value).map_err(io::Error::other)
}
