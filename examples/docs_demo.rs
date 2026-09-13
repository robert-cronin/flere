//! Record a real UI tour against a generated Git fixture; no user sessions or agents.
mod support;
use flere::{wire, workspace::Workflow};
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use support::{Fixture, View};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err("usage: docs_demo FLERE_EXE OUTPUT.jsonl; use a home-cache output".into());
    }
    let f = Fixture::new(&std::fs::canonicalize(&args[0])?)?;
    fs::create_dir(f.project.join("src"))?;
    fs::write(
        f.project.join("README.md"),
        "# Signal\n\nA small local project for the documentation tour.\nThree workspaces. One terminal. Real shell output.\n",
    )?;
    fs::write(
        f.project.join("src/routes.rs"),
        "pub fn health() -> &'static str {\n    \"ok\"\n}\n",
    )?;
    let git = |args: &[&str]| -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("git")
            .current_dir(&f.project)
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=Flere Demo",
                "-c",
                "user.email=demo@example.invalid",
            ])
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err("fixture Git command failed".into());
        }
        Ok(())
    };
    git(&["init", "-q", "-b", "main"])?;
    git(&["add", "."])?;
    git(&["commit", "-qm", "Create the Signal workspace"])?;
    fs::write(
        f.project.join("src/config.rs"),
        "pub const PORT: u16 = 8080;\n",
    )?;
    git(&["add", "."])?;
    git(&["commit", "-qm", "Add local configuration"])?;
    fs::write(
        f.project.join("src/routes.rs"),
        "pub fn health() -> &'static str {\n    \"ready\"\n}\n",
    )?;
    let mut workspaces = Vec::new();
    for (name, status) in [
        ("API", Workflow::InProgress),
        ("Console", Workflow::NeedsMe),
        ("Documentation", Workflow::Todo),
    ] {
        let s = f.workspace(name)?;
        let w = s.workspace().ok_or("no fixture workspace")?;
        let mut meta = w.meta.clone();
        meta.status = status;
        meta.project = "Signal".into();
        f.request(&[
            "metadata",
            &w.id.to_string(),
            &wire::hex(&serde_json::to_vec(&meta)?),
        ])?;
        f.input(format!("printf '\\033[2J\\033[H\\033[36mSignal / {name}\\033[0m\\n\\n'; cat README.md; printf '\\nSOURCE FILES\\n'; find src -type f | sort; printf '\\nWORKING TREE\\n'; git status --short; printf '\\nRECENT COMMITS\\n'; git log -2 --pretty=format:'%s'; printf '\\n\\n'\r").as_bytes())?;
        workspaces.push(w.id);
    }
    f.request(&["focus", &workspaces[0].to_string(), "0"])?;
    fs::write(
        f.state.join("ui.json"),
        serde_json::to_vec(&flere::workspace::Preferences {
            right: 40,
            ..Default::default()
        })?,
    )?;
    let mut view = View::new(&f, 140, 30, true)?;
    let mut output = File::create(PathBuf::from(&args[1]))?;
    let scenes = [
        ("01 / YOUR WORK, IN ONE PLACE", b"".as_slice()),
        ("02 / INSPECT LOCAL GIT HISTORY", b"\0g".as_slice()),
        ("03 / FIND THE NEXT WORKSPACE", b"\x1b".as_slice()),
        ("04 / KEEP THE BOARD IN VIEW", b"\0B".as_slice()),
    ];
    let start = Instant::now();
    for (i, (label, keys)) in scenes.iter().enumerate() {
        if i == 2 {
            view.key(keys)?;
            view.pump(Duration::from_millis(100))?;
            view.key(b"\0/Console")?;
        } else if i == 3 {
            view.key(b"\r")?;
            view.pump(Duration::from_millis(250))?;
            view.key(b"\x1b")?;
            view.pump(Duration::from_millis(100))?;
            view.key(keys)?;
        } else if !keys.is_empty() {
            view.key(keys)?;
        }
        let end = Instant::now() + Duration::from_secs(2);
        let mut next = Instant::now();
        while Instant::now() < end {
            view.read(10)?;
            if Instant::now() >= next && view.complete() {
                serde_json::to_writer(
                    &mut output,
                    &view.frame(start.elapsed().as_millis(), label),
                )?;
                output.write_all(b"\n")?;
                next = Instant::now() + Duration::from_millis(100);
            }
        }
    }
    println!("{}", PathBuf::from(&args[1]).display());
    Ok(())
}
