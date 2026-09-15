use flere::{os, server, ui, wire};
use std::{
    env,
    fs::OpenOptions,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const HELP: &str = r#"Flere — a native terminal workbench

flere [--state DIR]                       Start/attach UI; first-use intro, then a shell
flere [--state DIR] start                 Start supervisor only (never start saved work)
flere [--state DIR] attach                Attach UI to running supervisor
flere [--state DIR] intro                 Preview the animation; creates no state
flere ssh HOST [options]                  Connect through the local SSH companion
flere [--state DIR] add-project --cwd DIR [--name NAME] [--no-shell]
flere [--state DIR] new [--name NAME] [--cwd DIR] [--no-shell]
flere [--state DIR] tab WORKSPACE_ID       Explicitly start another shell in a workspace
flere [--state DIR] open-file WORKSPACE_ID PATH
flere [--state DIR] worktree NAME REPOSITORY BRANCH [BASE] [--no-shell]
flere [--state DIR] agent WORKSPACE_ID codex|claude|copilot [EXACT_UUID]
flere [--state DIR] refresh               Upgrade this supervisor, preserving sessions
flere [--state DIR] refresh-status        Read upgrade outcome
flere --build-info                       JSON identity of this executable build
flere [--state DIR] build-status          Compare this build with the supervisor
flere [--state DIR] coordinate WORKSPACE_ID OPERATION [JSON]
flere [--state DIR] agent-call OP [JSON]  Scoped MCP fallback using this native run
flere [--state DIR] list                  JSON workspace/session identities
flere [--state DIR] capture --session ID --run TOKEN [--lines 80]
flere [--state DIR] send --session ID --run TOKEN --text TEXT
flere [--state DIR] send --session ID --run TOKEN --key Enter|Escape|Ctrl-C|Tab|Backspace
flere [--state DIR] focus WORKSPACE_ID [TAB_ID]
flere [--state DIR] close --session ID --run TOKEN --terminate
flere [--state DIR] stop --terminate       Hang up Flere shells; retain workspaces
flere [--state DIR] serve                  Foreground supervisor for diagnostics

Ctrl+Space: navigation; h/l: panes; j/k: cards/tabs/files; Space: actions.
G: add project; n/N: new worktree; t: shell tab; b/f: history; x: close tab; q: detach.
s: copy the full Flere view as an image to your local clipboard.
Files: Enter opens directories/editor tabs; - or Backspace goes up.
Space actions supports j/k selection, direct shortcuts and Enter. R refreshes.
Terminal: wheel or Shift+Page Up browses history; Shift+Home jumps oldest; Esc/End returns live.
Detaching leaves shells alive. Opening the UI restores saved tabs; supervisor-only startup starts none.
Stopped terminal: S starts an agent, W retries saved tabs, Enter reopens, t opens a shell, Space opens actions.
Socket: STATE/control.sock. Default state: ~/.local/state/flere.
"#;
fn state_default() -> io::Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is unset; use --state"))?;
    Ok(env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".local/state"))
        .join("flere"))
}
fn ensure(state: &Path) -> io::Result<()> {
    if wire::request(state, &["ping"]).is_ok() {
        return Ok(());
    }
    server::private_state(state)?;
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(state.join("supervisor.log"))?;
    let mut cmd = Command::new(env::current_exe()?);
    cmd.arg("--state")
        .arg(state)
        .arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log));
    os::clean_environment(&mut cmd);
    os::detach(&mut cmd);
    let mut child = cmd.spawn()?;
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        if wire::request(state, &["ping"]).is_ok() {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "supervisor exited {status}; see {}",
                state.join("supervisor.log").display()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(io::Error::other(
        "supervisor did not become ready; inspect supervisor.log; no replacement sessions started",
    ))
}
fn run() -> io::Result<()> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    // SSH is a local companion action. Do not inspect/create local supervisor
    // state or apply the nested-UI guard before handing it the original argv.
    if args.first().is_some_and(|arg| arg == "ssh") {
        let sibling = env::current_exe()?.with_file_name("flere-connect");
        let companion = if sibling.try_exists()? {
            sibling
        } else {
            PathBuf::from("flere-connect")
        };
        let error = Command::new(companion).args(&args).exec();
        return Err(if error.kind() == io::ErrorKind::NotFound {
            io::Error::other(
                "flere ssh needs the local flere-connect companion; install the matching companion package or run scripts/dev update --with-companion",
            )
        } else {
            error
        });
    }
    // Package inspection must also work before first use, without HOME or state.
    if args.len() == 1 && args[0] == "--coordinated-update-info" {
        io::stdout().write_all(flere::remote_update::CANDIDATE_CAPABILITY)?;
        return Ok(());
    }
    if args.len() == 1 && args[0] == "--build-info" {
        println!("{}", flere::build_info::json());
        return Ok(());
    }
    if args
        .first()
        .is_some_and(|a| matches!(a.as_str(), "package" | "source-digest" | "verify-package"))
    {
        io::stdout().write_all(&flere::install::command(None, &args)?)?;
        return Ok(());
    }
    let mut state = if args.first().is_some_and(|a| a == "--state") {
        let i = 0;
        if i + 1 >= args.len() {
            return Err(wire::invalid("--state requires a directory"));
        }
        let state = PathBuf::from(args.remove(i + 1));
        args.remove(i);
        state
    } else {
        state_default()?
    };
    if !state.is_absolute() {
        state = env::current_dir()?.join(state)
    }
    let action = args.first().map_or("open", String::as_str);
    if action == "_shell-codex" {
        let run = args
            .get(1)
            .ok_or_else(|| wire::invalid("missing shell identity"))?;
        return flere::native::launcher::run(&state, run, &args[2..]);
    }
    if matches!(
        action,
        "package"
            | "source-digest"
            | "verify-package"
            | "install"
            | "update"
            | "install-status"
            | "update-ack"
            | "update-check"
            | "update-prepare"
            | "update-apply"
            | "update-plan"
            | "update-coordinated-v1"
            | "dev-source"
    ) {
        io::stdout().write_all(&flere::install::command(Some(&state), &args)?)?;
        return Ok(());
    }
    // Reject before connecting or bootstrapping state. Child shells/native hosts
    // inherit this marker; diagnostic and coordination commands remain usable.
    if matches!(action, "open" | "attach" | "intro" | "_bridge") && env::var_os("FLERE").is_some() {
        return Err(wire::invalid(
            "already inside Flere; use Ctrl+Space for navigation/actions. To attach elsewhere, open Flere from an ordinary terminal outside this workspace",
        ));
    }
    let mut options = std::collections::HashMap::new();
    let mut positional = vec![action];
    let mut i = 1;
    while i < args.len() {
        let key = args[i].as_str();
        if matches!(key, "--terminate" | "--no-shell") {
            if options.insert(key, "").is_some() {
                return Err(wire::invalid("duplicate option"));
            }
            i += 1;
        } else if matches!(
            key,
            "--name" | "--cwd" | "--session" | "--run" | "--lines" | "--text" | "--key"
        ) {
            let value = args
                .get(i + 1)
                .ok_or_else(|| wire::invalid(&format!("missing value for {key}")))?;
            if options.insert(key, value.as_str()).is_some() {
                return Err(wire::invalid("duplicate option"));
            }
            i += 2;
        } else if key.starts_with("--") {
            return Err(wire::invalid(&format!("unknown option {key}")));
        } else {
            positional.push(key);
            i += 1;
        }
    }
    let opt = |name: &str| {
        options
            .get(name)
            .copied()
            .ok_or_else(|| wire::invalid(&format!("missing {name}")))
    };
    let arg = |i: usize| {
        positional
            .get(i)
            .copied()
            .ok_or_else(|| wire::invalid("missing argument"))
    };
    if options.contains_key("--no-shell") && !matches!(action, "new" | "worktree") {
        return Err(wire::invalid(
            "--no-shell applies only to new/worktree preparation",
        ));
    }
    if matches!(action, "serve" | "_restore") {
        server::private_state(&state)?;
        if let Err(e) = flere::diagnostics::init(&state.join("diagnostics"), "supervisor") {
            eprintln!("Flere diagnostics unavailable: {e}");
        }
    }
    let output = match action {
        #[cfg(target_os = "linux")]
        "_clipboard" => {
            let path = Path::new(arg(1)?);
            flere::attachments::validate(path)?;
            return flere::clipboard_owner::serve(std::fs::read(path)?, path);
        }
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            return Ok(());
        }
        "--version" => {
            println!(
                "flere {} ({} {}, custom UI; build {})",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                flere::build_info::BUILD_ID,
            );
            return Ok(());
        }
        "--build-info" if positional.len() == 1 && options.is_empty() => {
            flere::build_info::json().as_bytes().to_vec()
        }
        "build-status" if positional.len() == 1 && options.is_empty() => {
            flere::build_status::report(&state)?
        }
        "serve" => return server::serve(&state),
        "_bridge" => return ui::bridge(&state),
        "mcp" => return flere::mcp::serve(&state),
        "_inbox-hook" => {
            return flere::inbox_hook::run(
                &state,
                std::io::stdin().lock(),
                std::io::stdout().lock(),
            );
        }
        "coordinate" => wire::request(
            &state,
            &[
                "coordinate",
                arg(1)?,
                arg(2)?,
                &wire::hex(positional.get(3).copied().unwrap_or("{}").as_bytes()),
            ],
        )?,
        "agent-call" => {
            let session = env::var("FLERE_SESSION")
                .map_err(|_| wire::invalid("agent-call requires this native session identity"))?;
            let run = env::var("FLERE_RUN")
                .map_err(|_| wire::invalid("agent-call requires this native run identity"))?;
            let operation = if arg(1)? == "get_context" {
                "context"
            } else {
                arg(1)?
            };
            wire::request(
                &state,
                &[
                    "agent-operation",
                    &session,
                    &run,
                    operation,
                    &wire::hex(positional.get(2).copied().unwrap_or("{}").as_bytes()),
                ],
            )?
        }
        "_host" => return flere::native::host(&state, arg(1)?),
        "_restore" => return server::restore(&state, arg(1)?),
        "_validate-refresh" => return server::validate_refresh(&state, arg(1)?),
        "refresh-status" => wire::request(&state, &["refresh-status"])?,
        "refresh" => wire::request(
            &state,
            &[
                "refresh",
                &wire::hex(os::executable_path()?.to_string_lossy().as_bytes()),
            ],
        )?,
        "worktree" => wire::request(
            &state,
            &[
                if options.contains_key("--no-shell") {
                    "worktree-stopped"
                } else {
                    "worktree"
                },
                &wire::hex(arg(1)?.as_bytes()),
                &wire::hex(arg(2)?.as_bytes()),
                &wire::hex(arg(3)?.as_bytes()),
                &wire::hex(positional.get(4).copied().unwrap_or("HEAD").as_bytes()),
                &wire::hex(options.get("--name").copied().unwrap_or("").as_bytes()),
            ],
        )?,
        "lead" => {
            return Err(wire::invalid(
                "Lead roles were removed; use new --name NAME --no-shell, then pin the workspace",
            ));
        }
        "agent" => wire::request(
            &state,
            &[
                "native",
                arg(1)?,
                arg(2)?,
                positional.get(3).copied().unwrap_or(""),
            ],
        )?,
        "start" => {
            ensure(&state)?;
            format!("Supervisor ready: {}", state.join("control.sock").display()).into_bytes()
        }
        "intro" => return ui::intro(&state).map(|_| ()),
        "open" => {
            ensure(&state)?;
            let mut s = flere::model::Snapshot::decode(&wire::request(&state, &["snapshot"])?)?;
            if s.workspaces.is_empty() {
                if !ui::intro(&state)? {
                    return Ok(());
                }
                // Another outer UI may have completed first use while this one waited.
                s = flere::model::Snapshot::decode(&wire::request(&state, &["snapshot"])?)?;
            }
            if s.workspaces.is_empty() {
                let cwd = env::current_dir()?;
                wire::request(
                    &state,
                    &[
                        "new",
                        &wire::hex(b"Workspace 1"),
                        &wire::hex(
                            cwd.to_str()
                                .ok_or_else(|| wire::invalid("cwd must be UTF-8"))?
                                .as_bytes(),
                        ),
                    ],
                )?;
            }
            return ui::attach(&state);
        }
        "attach" => return ui::attach(&state),
        "add-project" => {
            ensure(&state)?;
            let cwd = opt("--cwd")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            wire::request(
                &state,
                &[
                    if options.contains_key("--no-shell") {
                        "add-project-stopped"
                    } else {
                        "add-project"
                    },
                    &wire::hex(
                        cwd.to_str()
                            .ok_or_else(|| wire::invalid("directory must be UTF-8"))?
                            .as_bytes(),
                    ),
                    &wire::hex(opt("--name").unwrap_or("").as_bytes()),
                ],
            )?
        }
        "new" => {
            ensure(&state)?;
            let cwd = opt("--cwd")
                .map(PathBuf::from)
                .unwrap_or(env::current_dir()?);
            let name = opt("--name").unwrap_or("Workspace");
            wire::request(
                &state,
                &[
                    if options.contains_key("--no-shell") {
                        "new-stopped"
                    } else {
                        "new"
                    },
                    &wire::hex(name.as_bytes()),
                    &wire::hex(
                        cwd.to_str()
                            .ok_or_else(|| wire::invalid("cwd must be UTF-8"))?
                            .as_bytes(),
                    ),
                ],
            )?
        }
        "open-file" => wire::request(&state, &["open", arg(1)?, &wire::hex(arg(2)?.as_bytes())])?,
        "tab" => wire::request(&state, &["tab", arg(1)?])?,
        "list" => wire::request(&state, &["list"])?,
        "capture" => wire::request(
            &state,
            &[
                "capture",
                opt("--session")?,
                opt("--run")?,
                opt("--lines").unwrap_or("80"),
            ],
        )?,
        "send" => {
            let text = options.contains_key("--text");
            let key = options.contains_key("--key");
            if text == key {
                return Err(wire::invalid("specify exactly one of --text or --key"));
            }
            let bytes = if text {
                opt("--text")?.as_bytes()
            } else {
                match opt("--key")? {
                    "Enter" => b"\r",
                    "Escape" => b"\x1b",
                    "Ctrl-C" => b"\x03",
                    "Tab" => b"\t",
                    "Backspace" => b"\x7f",
                    _ => return Err(wire::invalid("unsupported key")),
                }
            };
            wire::request(
                &state,
                &[
                    if text { "text" } else { "input" },
                    opt("--session")?,
                    opt("--run")?,
                    &wire::hex(bytes),
                ],
            )?
        }
        "focus" => wire::request(
            &state,
            &["focus", arg(1)?, positional.get(2).copied().unwrap_or("0")],
        )?,
        "close" => {
            if !options.contains_key("--terminate") {
                return Err(wire::invalid(
                    "close requires --terminate; it hangs up the exact terminal",
                ));
            }
            wire::request(&state, &["close", opt("--session")?, opt("--run")?])?
        }
        "stop" => {
            if !options.contains_key("--terminate") {
                return Err(wire::invalid(
                    "stop requires --terminate; detach preserves shells",
                ));
            }
            wire::request(&state, &["stop"])?
        }
        _ => return Err(wire::invalid("unknown command; see flere --help")),
    };
    io::stdout().write_all(&output)?;
    println!();
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        flere::diagnostics::error("process-error", &e);
        eprintln!("flere: {}", wire::passive(&e.to_string()));
        std::process::exit(1)
    }
}
