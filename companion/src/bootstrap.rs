//! Explicit `flere ssh` first use. Discovery never changes a running supervisor;
//! installation requires proved absence and the installer's locked --if-missing.
//! Native authentication remains OpenSSH's responsibility, before console setup.
mod package;
mod transport;

use crate::{connections::Connection, update::BuildMetadata};
use serde_json::Value;
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const COMPONENT: &str = "flere";
const MAX_JSON: usize = 65536;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Automatic,
    ExplicitExecutable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    DefaultChannel,
    HttpsManifest(String),
    LocalPackage(PathBuf),
}

pub fn help(args: &[String]) -> bool {
    args.first().is_some_and(|arg| arg == "ssh")
        && (args.len() == 1 || (args.len() == 2 && args[1] == "--help"))
}

/// Strip bootstrap-only source options from an explicit `ssh` invocation. A
/// direct host/profile invocation never acquires automatic install authority.
pub fn arguments(args: &mut Vec<String>) -> io::Result<Option<Source>> {
    if !args.first().is_some_and(|arg| arg == "ssh") {
        return Ok(None);
    }
    args.remove(0);
    let first = args
        .first()
        .ok_or_else(|| invalid("Use ssh EXISTING_ALIAS or ssh --connection NAME"))?;
    let mut index = if first == "--connection" {
        if args.len() < 2 || args[1].starts_with('-') {
            return Err(invalid("--connection requires one saved name"));
        }
        2
    } else if first == "reconnect" && args.get(1).is_some_and(|arg| !arg.starts_with('-')) {
        2
    } else {
        1
    };
    let mut source = Source::DefaultChannel;
    let mut seen = false;
    while index < args.len() {
        if matches!(args[index].as_str(), "--from-url" | "--package") {
            if seen || index + 1 >= args.len() {
                return Err(invalid(
                    "Choose one --from-url HTTPS_MANIFEST or --package LOCAL_CORE_PACKAGE",
                ));
            }
            let value = args.remove(index + 1);
            source = if args.remove(index) == "--package" {
                Source::LocalPackage(PathBuf::from(value))
            } else {
                Source::HttpsManifest(value)
            };
            seen = true;
        } else {
            index += 2;
        }
    }
    Ok(Some(source))
}

/// Presentation/filesystem names are separate from the fixed manifest component.
#[derive(Clone, Debug)]
pub struct Names {
    pub command: String,
    pub state_directory: String,
    pub cache_directory: String,
}
impl Default for Names {
    fn default() -> Self {
        Self {
            command: "flere".into(),
            state_directory: "flere".into(),
            cache_directory: "flere".into(),
        }
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "Optional caller-owned cancellation; the CLI currently retains OpenSSH's native Ctrl-C handling"
        )
    )]
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    fn check(&self) -> io::Result<()> {
        if self.0.load(Ordering::Relaxed) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "SSH bootstrap cancelled; an uncertain installation is not retried automatically",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone)]
pub struct Request {
    pub connection: Connection,
    pub selection: Selection,
    pub source: Source,
    pub names: Names,
    pub cancellation: Cancellation,
}
impl Request {
    pub fn new(connection: Connection, selection: Selection, source: Source) -> Self {
        Self {
            connection,
            selection,
            source,
            names: Names::default(),
            cancellation: Cancellation::default(),
        }
    }
    fn validate(&self) -> io::Result<()> {
        if !crate::connections::valid_host(&self.connection.host) {
            return Err(invalid("Use one existing OpenSSH alias or user@host"));
        }
        for name in [
            &self.names.command,
            &self.names.state_directory,
            &self.names.cache_directory,
        ] {
            if name.is_empty()
                || name.len() > 64
                || name.starts_with('.')
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
            {
                return Err(invalid("Invalid bootstrap command or directory name"));
            }
        }
        crate::quote(&self.connection.ssh)?;
        if self.selection == Selection::ExplicitExecutable {
            crate::quote(&self.connection.remote)?;
        }
        if let Some(state) = &self.connection.state {
            absolute(state)?;
        }
        if let Source::HttpsManifest(url) = &self.source {
            package::https(url)?;
        }
        self.cancellation.check()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Runtime {
    pub pid: u32,
    pub epoch: String,
    pub build: BuildMetadata,
}

pub struct Resolved {
    pub connection: Connection,
    pub bridge: BuildMetadata,
    pub runtime: Runtime,
}

/// Evidence only. The caller must obtain fresh local Update consent and use the
/// existing prepared-plan transaction; this module never refreshes a live core.
pub struct UpdateOffer {
    pub connection: Connection,
    pub bridge: BuildMetadata,
    pub runtime: Option<Runtime>,
    pub detail: String,
}

pub enum Outcome {
    Ready(Resolved),
    UpdateRequired(UpdateOffer),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Probe {
    target: String,
    home: String,
    cache: String,
    state: String,
    state_exists: bool,
    endpoint: bool,
    receipt: bool,
    executable: Option<String>,
    legacy_executable: Option<String>,
}

fn absolute(value: &str) -> io::Result<()> {
    // A remote Unix path must not use the local host's Windows path semantics.
    if !value.starts_with('/')
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || value.split('/').any(|part| matches!(part, "." | ".."))
    {
        return Err(invalid(
            "Remote bootstrap paths must be absolute and contain no controls or dot segments",
        ));
    }
    Ok(())
}

fn decode_hex(value: &str) -> io::Result<String> {
    if value.len() > 8192
        || !value.len().is_multiple_of(2)
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Invalid bounded SSH probe field encoding"));
    }
    let bytes: Vec<u8> = value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
            digit(pair[0]) * 16 + digit(pair[1])
        })
        .collect();
    let value = String::from_utf8(bytes).map_err(io::Error::other)?;
    if value.chars().any(char::is_control) {
        return Err(invalid("SSH probe fields cannot contain controls"));
    }
    Ok(value)
}

fn target(os: &str, arch: &str) -> io::Result<&'static str> {
    match (os, arch) {
        ("Linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("Darwin", "arm64" | "aarch64") => Ok("aarch64-apple-darwin"),
        ("Darwin", "x86_64") => Ok("x86_64-apple-darwin"),
        _ => Err(invalid(&format!(
            "No accepted prebuilt Flere core target for {os}/{arch}; Windows is a local companion platform"
        ))),
    }
}

fn parse_probe(bytes: &[u8]) -> io::Result<Probe> {
    if bytes.len() > MAX_JSON {
        return Err(invalid("SSH discovery response exceeds its bound"));
    }
    let text = std::str::from_utf8(bytes).map_err(io::Error::other)?;
    let mut lines = text.lines();
    if lines.next() != Some("FLERE-BOOTSTRAP-1") || !text.ends_with('\n') {
        return Err(invalid(
            "SSH discovery returned unexpected output; remove stdout banners from noninteractive remote commands",
        ));
    }
    let mut fields = Vec::new();
    for key in [
        "os",
        "arch",
        "home",
        "cache",
        "state",
        "state_presence",
        "endpoint",
        "receipt",
        "executable",
        "legacy_executable",
    ] {
        let value = lines
            .next()
            .and_then(|line| line.split_once('\t'))
            .filter(|(actual, _)| *actual == key)
            .ok_or_else(|| invalid("Missing, duplicate or out-of-order SSH probe field"))?
            .1;
        fields.push(decode_hex(value)?);
    }
    if lines.next() != Some("END") || lines.next().is_some() {
        return Err(invalid("Unexpected trailing SSH discovery output"));
    }
    for path in &fields[2..5] {
        absolute(path)?;
    }
    for path in &fields[8..10] {
        if !path.is_empty() {
            absolute(path)?;
        }
    }
    let presence = |value: &str| match value {
        "present" => Ok(true),
        "absent" => Ok(false),
        _ => Err(invalid("Invalid SSH presence field")),
    };
    let state_exists = match fields[5].as_str() {
        "missing" => false,
        "directory" => true,
        _ => {
            return Err(invalid(
                "Selected remote state is linked or is not a directory",
            ));
        }
    };
    Ok(Probe {
        target: target(&fields[0], &fields[1])?.into(),
        home: fields[2].clone(),
        cache: fields[3].clone(),
        state: fields[4].clone(),
        state_exists,
        endpoint: presence(&fields[6])?,
        receipt: presence(&fields[7])?,
        executable: (!fields[8].is_empty()).then(|| fields[8].clone()),
        legacy_executable: (!fields[9].is_empty()).then(|| fields[9].clone()),
    })
}

fn probe(request: &Request) -> io::Result<Probe> {
    let explicit = if request.selection == Selection::ExplicitExecutable {
        request.connection.remote.as_str()
    } else {
        ""
    };
    let command = transport::script(
        include_str!("bootstrap/probe.sh"),
        &[
            &request.names.command,
            "railhand",
            &request.names.state_directory,
            &request.names.cache_directory,
            explicit,
            request.connection.state.as_deref().unwrap_or(""),
        ],
    )?;
    let result = parse_probe(&transport::run(
        request,
        "discovery",
        &command,
        None,
        MAX_JSON,
    )?)?;
    if let Some(state) = &request.connection.state
        && *state != result.state
    {
        return Err(invalid(
            "Remote discovery changed the explicit selected state",
        ));
    }
    Ok(result)
}

fn invoke(request: &Request, executable: &str, args: &[&str], phase: &str) -> io::Result<Value> {
    let command = transport::command(executable, args)?;
    serde_json::from_slice(&transport::run(
        request,
        phase,
        &command,
        None,
        MAX_JSON * 4,
    )?)
    .map_err(|error| {
        invalid(&format!(
            "Invalid {phase} response from the selected remote core: {error}"
        ))
    })
}

fn metadata(value: Value, expected_target: &str) -> io::Result<BuildMetadata> {
    let build: BuildMetadata = serde_json::from_value(value).map_err(io::Error::other)?;
    build.validate()?;
    if build.component != COMPONENT || build.target != expected_target {
        return Err(invalid(
            "Selected executable is not a Flere core for this remote platform; legacy Railhand installations are not adopted",
        ));
    }
    Ok(build)
}

fn runtime(value: &Value, target: &str) -> io::Result<Runtime> {
    let pid = value["pid"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= u32::MAX as u64)
        .ok_or_else(|| invalid("Invalid remote supervisor PID"))? as u32;
    let epoch = value["epoch"]
        .as_str()
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| invalid("Invalid remote supervisor epoch"))?
        .to_owned();
    Ok(Runtime {
        pid,
        epoch,
        build: metadata(value["build"].clone(), target)?,
    })
}

struct Inspection {
    bridge: BuildMetadata,
    runtime: Option<Runtime>,
}

fn inspect(request: &Request, probe: &Probe, executable: &str) -> io::Result<Inspection> {
    let bridge = metadata(
        invoke(
            request,
            executable,
            &["--build-info"],
            "executable identity",
        )?,
        &probe.target,
    )?;
    let value = invoke(
        request,
        executable,
        &["--state", &probe.state, "build-status"],
        "supervisor identity",
    )?;
    if value["schema_version"] != 1
        || value["state"] != probe.state
        || metadata(value["executable"]["build"].clone(), &probe.target)? != bridge
    {
        return Err(invalid(
            "Remote executable or selected state changed during discovery",
        ));
    }
    if value["installation"]["status"] == "unknown" {
        return Err(invalid(
            "Remote installation is interrupted or unsafe; inspect install-status before bootstrap",
        ));
    }
    let runtime = match value["supervisor"]["status"].as_str() {
        Some("known") => Some(runtime(&value["supervisor"], &probe.target)?),
        Some("unreachable") => {
            // `unreachable` alone includes access/protocol errors. Only the core's
            // typed absence result permits supervisor-only startup.
            let check = invoke(
                request,
                executable,
                &["--state", &probe.state, "update-check"],
                "stopped supervisor check",
            )?;
            if check["schema_version"] != 1 || check["status"] != "not_running" {
                return Err(invalid(
                    "Cannot prove the selected supervisor is stopped; existing state was retained",
                ));
            }
            None
        }
        _ => {
            return Err(invalid(
                "Running supervisor identity is unknown; bootstrap leaves it unchanged and cannot prepare a guarded Update",
            ));
        }
    };
    Ok(Inspection { bridge, runtime })
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Install,
    Start,
    Attach,
    Update,
    Refuse,
}

fn decide(
    probe: &Probe,
    inspection: Option<&Inspection>,
    companion: &BuildMetadata,
    selection: Selection,
) -> Decision {
    let Some(inspection) = inspection else {
        return if probe.executable.is_some()
            || probe.endpoint
            || probe.receipt
            || selection == Selection::ExplicitExecutable
        {
            Decision::Refuse
        } else {
            Decision::Install
        };
    };
    if !companion.reads_core(&inspection.bridge) {
        return Decision::Update;
    }
    let Some(runtime) = &inspection.runtime else {
        return Decision::Start;
    };
    let bridge = &inspection.bridge.compatibility;
    let core = &runtime.build.compatibility;
    if !companion.reads_core(&runtime.build)
        || bridge.control_identity != core.control_identity
        || !bridge
            .snapshot
            .as_ref()
            .zip(core.snapshot.as_ref())
            .is_some_and(|(reader, writer)| reader.accepts(writer.current))
    {
        Decision::Update
    } else {
        Decision::Attach
    }
}

fn connection(request: &Request, probe: &Probe, executable: &str) -> Connection {
    Connection {
        remote: executable.into(),
        remote_explicit: true,
        state: Some(probe.state.clone()),
        ..request.connection.clone()
    }
}

fn offer(request: &Request, probe: &Probe, executable: &str, inspection: Inspection) -> Outcome {
    Outcome::UpdateRequired(UpdateOffer {
        connection: connection(request, probe, executable),
        bridge: inspection.bridge,
        runtime: inspection.runtime,
        detail: format!(
            "The selected core and companion cannot attach compatibly. Existing commands and sessions were retained. Select the same SSH host {}, executable {executable}, and state {} for an explicit guarded Update; update the local companion first if it cannot read both core versions",
            request.connection.host, probe.state
        ),
    })
}

/// Called only for the explicit new SSH workflow. Old direct/reconnect profiles
/// keep their exact executable route and do not implicitly enable installation.
pub fn prepare(request: &Request) -> io::Result<Outcome> {
    request.validate()?;
    let companion: BuildMetadata =
        serde_json::from_str(crate::build_info::json()).map_err(io::Error::other)?;
    companion.validate()?;
    let mut observed = probe(request)?;
    if let Some(legacy) = &observed.legacy_executable {
        eprintln!(
            "Legacy Railhand command at {legacy} is separate; Flere will not adopt its installation or state."
        );
    }
    let mut selected = observed.executable.clone();
    let mut inspection = selected
        .as_deref()
        .map(|path| inspect(request, &observed, path))
        .transpose()?;
    if decide(
        &observed,
        inspection.as_ref(),
        &companion,
        request.selection,
    ) == Decision::Install
    {
        eprintln!("Preparing a verified Flere core for {}…", observed.target);
        let package = package::obtain(request, &observed.target)?;
        if !companion.reads_core(&package.manifest.build) {
            return Err(invalid(
                "Published core requires a different companion; update the local companion before first installation",
            ));
        }
        let uploaded = package::upload(request, &package, &observed.cache)?;
        let directory = &uploaded.directory;
        let candidate = format!("{directory}/{COMPONENT}");
        let build = metadata(
            invoke(
                request,
                &candidate,
                &["--build-info"],
                "verified prebuilt execution",
            )?,
            &observed.target,
        )?;
        if build != package.manifest.build {
            return Err(invalid(
                "Transferred executable reports different embedded metadata",
            ));
        }
        let verified = invoke(
            request,
            &candidate,
            &["verify-package", directory],
            "remote package verification",
        )?;
        if verified["status"] != "verified"
            || verified["manifest"]
                != serde_json::to_value(&package.manifest).map_err(io::Error::other)?
        {
            return Err(invalid(
                "Remote package verification differs from the reviewed manifest",
            ));
        }
        if probe(request)? != observed {
            return Err(invalid(
                "Remote discovery changed during preparation; no automatic installation was attempted",
            ));
        }
        // The candidate diagnoses an existing stopped state before any install.
        // Its normal decoder/start path remains responsible for saved-state validation.
        let check = invoke(
            request,
            &candidate,
            &["--state", &observed.state, "update-check"],
            "first-install absence check",
        )?;
        if check["schema_version"] != 1 || check["status"] != "not_running" {
            return Err(invalid(
                "A selected runtime appeared before installation; it was left unchanged",
            ));
        }
        let mut args = vec![
            "--state",
            &observed.state,
            "install",
            &directory,
            "--if-missing",
        ];
        if let Some(url) = &package.source_url {
            args.extend(["--source-url", url]);
        }
        let receipt = invoke(request, &candidate, &args, "first installation")?;
        if receipt["schema_version"] != 1
            || receipt["component"] != COMPONENT
            || !receipt["previous"].is_null()
            || receipt["current"]["manifest"]
                != serde_json::to_value(&package.manifest).map_err(io::Error::other)?
        {
            return Err(invalid(
                "First installation returned an unexpected receipt; inspect install-status before retrying",
            ));
        }
        let destination = receipt["destination"]
            .as_str()
            .ok_or_else(|| invalid("First installation returned no command destination"))?
            .to_owned();
        absolute(&destination)?;
        if destination != format!("{}/.local/bin/{}", observed.home, request.names.command) {
            return Err(invalid(
                "Installed destination differs from the selected per-user command",
            ));
        }
        let fresh = probe(request)?;
        if fresh.state != observed.state
            || fresh.target != observed.target
            || fresh.home != observed.home
        {
            return Err(invalid(
                "Selected remote state or platform changed after installation",
            ));
        }
        observed = fresh;
        selected = Some(destination);
        inspection = Some(inspect(request, &observed, selected.as_deref().unwrap())?);
    }
    let executable = selected.ok_or_else(|| invalid(
        "Selected Flere executable is missing but absence cannot authorize installation; check explicit --remote, the selected state and install-status",
    ))?;
    let mut inspection =
        inspection.ok_or_else(|| invalid("Remote core inspection is unavailable"))?;
    match decide(&observed, Some(&inspection), &companion, request.selection) {
        Decision::Update => return Ok(offer(request, &observed, &executable, inspection)),
        Decision::Start => {
            // Re-inspect immediately before start. If another process has started
            // a core, use its actual identity; never refresh or replace it here.
            inspection = inspect(request, &observed, &executable)?;
            if inspection.runtime.is_none() {
                let command =
                    transport::command(&executable, &["--state", &observed.state, "start"])?;
                transport::run(request, "supervisor-only startup", &command, None, MAX_JSON)?;
                inspection = inspect(request, &observed, &executable)?;
            }
        }
        Decision::Attach => {}
        _ => {
            return Err(invalid(
                "Bootstrap cannot establish a safe start/attach decision",
            ));
        }
    }
    if decide(&observed, Some(&inspection), &companion, request.selection) != Decision::Attach {
        return Ok(offer(request, &observed, &executable, inspection));
    }
    Ok(Outcome::Ready(Resolved {
        connection: connection(request, &observed, &executable),
        bridge: inspection.bridge,
        runtime: inspection
            .runtime
            .ok_or_else(|| invalid("Supervisor did not become ready"))?,
    }))
}
#[cfg(test)]
mod tests;
