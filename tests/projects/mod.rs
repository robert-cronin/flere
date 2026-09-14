use super::*;
use serde_json::{Value, json};
fn repo(f: &Fixture) -> PathBuf {
    let root = f.root.join("example project");
    fs::create_dir(&root).unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.name", "Fixture"],
        vec!["config", "user.email", "fixture@example.test"],
        vec!["config", "commit.gpgsign", "false"],
        vec!["config", "core.hooksPath", "/dev/null"],
    ] {
        assert!(
            Command::new("git")
                .current_dir(&root)
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    fs::write(root.join("README.md"), "Project fixture\n").unwrap();
    fs::create_dir(root.join("src")).unwrap();
    assert!(
        Command::new("git")
            .current_dir(&root)
            .args(["add", "README.md"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        Command::new("git")
            .current_dir(&root)
            .args(["commit", "-m", "Fixture base"])
            .output()
            .unwrap()
            .status
            .success()
    );
    root
}
fn added(f: &Fixture, root: &std::path::Path) -> u64 {
    serde_json::from_slice::<Value>(&f.req(&[
        "add-project-stopped",
        &wire::hex(root.to_str().unwrap().as_bytes()),
        "",
    ]))
    .unwrap()["workspace"]
        .as_u64()
        .unwrap()
}
#[test]
fn add_project_reuses_the_primary_from_subdirectories_and_linked_worktrees() {
    let f = Fixture::new();
    let root = repo(&f);
    let id = added(&f, &root.join("src"));
    assert_eq!(added(&f, &root), id);
    let linked = f.root.join("linked");
    assert!(
        Command::new("git")
            .current_dir(&root)
            .args(["worktree", "add", "-b", "linked"])
            .arg(&linked)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(added(&f, &linked), id);
    assert_eq!(f.snapshot().workspaces.len(), 1);
    let snapshot = f.snapshot();
    let primary = snapshot.workspace().unwrap();
    assert_eq!(primary.cwd, root.to_str().unwrap());
    assert_eq!(primary.name, "example project");
    assert_eq!(primary.meta.project, "example project");
    assert!(primary.tabs.is_empty());
    assert!(!primary.meta.pinned && !primary.meta.legacy_lead);
    f.req(&[
        "add-project",
        &wire::hex(root.to_str().unwrap().as_bytes()),
        "",
    ]);
    let t = f.snapshot().session().unwrap().clone();
    f.req(&[
        "add-project",
        &wire::hex(linked.to_str().unwrap().as_bytes()),
        "",
    ]);
    assert_eq!(f.snapshot().workspace().unwrap().tabs.len(), 1);
    assert_eq!(f.snapshot().session().unwrap().run, t.run);
    assert_eq!(f.snapshot().session().unwrap().pid, t.pid);
    assert!(flere::git::inspect(&root).primary);
    assert!(!flere::git::inspect(&linked).primary);
}

#[test]
fn project_name_uses_local_origin_without_a_host_or_a_chat() {
    let f = Fixture::new();
    let root = repo(&f);
    history_git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://fixture:secret@example.invalid/owner/repository-name.git",
        ],
    );
    fs::write(root.join("README.md"), "Uncommitted work stays here\n").unwrap();
    added(&f, &root.join("src"));
    let snapshot = f.snapshot();
    let card = snapshot.workspace().unwrap();
    assert_eq!(card.name, "repository-name");
    assert_eq!(card.meta.project, "repository-name");
    assert!(card.tabs.is_empty());
    assert!(!snapshot.encode().windows(6).any(|v| v == b"secret"));
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "Uncommitted work stays here\n"
    );
}

#[test]
fn malformed_origin_falls_back_to_the_checkout_directory() {
    let f = Fixture::new();
    let root = repo(&f);
    history_git(
        &root,
        &[
            "config",
            "remote.origin.url",
            "https://fixture:secret@example.invalid",
        ],
    );
    added(&f, &root);
    assert_eq!(f.snapshot().workspace().unwrap().name, "example project");
}
#[test]
fn ordinary_agent_adds_primary_without_focus_change_and_extra_cards_are_worktrees() {
    let f = Fixture::new();
    let root = repo(&f);
    let (actor, agent) = dispatch_fixture(&f);
    let primary = dispatch_call(
        &f,
        &agent,
        "add_project",
        json!({"cwd":root,"name":"Example"}),
    )
    .unwrap();
    let id = primary["workspace"].as_u64().unwrap();
    assert_eq!(f.snapshot().active, actor);
    assert!(
        f.snapshot()
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .unwrap()
            .tabs
            .is_empty()
    );
    let again = dispatch_call(&f, &agent, "add_project", json!({"cwd":root.join("src")})).unwrap();
    assert_eq!(again["workspace"], id);
    fs::write(root.join("README.md"), "Uncommitted main checkout work\n").unwrap();
    let prepared = dispatch_call(
        &f,
        &agent,
        "prepare_workspace",
        json!({"name":"Implement feature","cwd":root}),
    )
    .unwrap();
    let child = prepared["workspace"].as_u64().unwrap();
    assert_ne!(child, id);
    let end = Instant::now() + Duration::from_secs(5);
    let worktree = loop {
        let s = f.snapshot();
        let w = s.workspaces.iter().find(|w| w.id == child).unwrap();
        if w.meta.operation.is_empty() {
            break w.clone();
        }
        assert!(Instant::now() < end, "{}", w.meta.operation);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_ne!(worktree.cwd, root.to_str().unwrap());
    assert!(PathBuf::from(&worktree.cwd).join(".git").is_file());
    assert_eq!(
        flere::git::primary_root(PathBuf::from(&worktree.cwd).as_path()).unwrap(),
        root
    );
    assert!(worktree.tabs.is_empty());
    assert_eq!(f.snapshot().active, actor);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "Uncommitted main checkout work\n"
    );
    assert_eq!(
        fs::read_to_string(PathBuf::from(&worktree.cwd).join("README.md")).unwrap(),
        "Project fixture\n"
    );
}
#[test]
fn keyboard_add_project_opens_primary_and_new_card_uses_a_worktree() {
    let f = Fixture::new();
    let root = repo(&f);
    f.new_workspace("Loose shell");
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "Loose shell");
    master.write_all(b"\0G").unwrap();
    drain_pty(&mut master, &mut screen, "Project directory");
    master
        .write_all(format!("\x15{}\r", root.display()).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().cwd == root.to_str().unwrap()
    });
    let primary = f.snapshot().active;
    master.write_all(b"\0n").unwrap();
    drain_pty(&mut master, &mut screen, "New branch");
    master.write_all(b"UI task\t\ttask-ui\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        let s = f.snapshot();
        s.active != primary
            && s.workspace().unwrap().meta.operation.is_empty()
            && s.session().is_some()
    });
    let s = f.snapshot();
    let w = s.workspace().unwrap();
    assert!(PathBuf::from(&w.cwd).join(".git").is_file());
    assert_eq!(
        flere::git::primary_root(PathBuf::from(&w.cwd).as_path()).unwrap(),
        root
    );
    if screen.capture(100).contains(" NAV ") {
        master.write_all(b"\r").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
    }
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn project_directory_picker_completes_spaces_browses_parent_and_cancels() {
    let f = Fixture::new();
    let root = repo(&f);
    let original = f.new_workspace("Loose shell");
    let original_workspace = f.snapshot().active;
    let mut command = outer_ui_command();
    command.arg("--state").arg(&f.state).arg("attach");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 140, 38).unwrap();
    let mut screen = Terminal::new(140, 38);
    drain_pty(&mut master, &mut screen, "Loose shell");
    master.write_all(b"\0G\x15exam").unwrap();
    drain_pty(&mut master, &mut screen, "› example project/");
    assert!(!screen.capture(38).contains("Name (optional)"));
    master.write_all(b"\x1b[B\x1b[A\t").unwrap();
    drain_pty(&mut master, &mut screen, "› .git/");
    master.write_all(b"\x1b[D").unwrap();
    wait_current_ui(&mut master, &mut screen, |screen| {
        let text = screen.capture(38);
        text.contains("example project/") && !text.contains("› .git/")
    });
    master.write_all(b"\x1b").unwrap();
    wait_current_ui(&mut master, &mut screen, |screen| {
        !screen.capture(38).contains("Add project")
    });
    let unchanged = f.snapshot();
    assert_eq!(unchanged.workspaces.len(), 1);
    assert_eq!(unchanged.active, original_workspace);
    assert_eq!(unchanged.session().unwrap().run, original.run);
    assert_eq!(unchanged.session().unwrap().pid, original.pid);

    // The suggestion is only selected by Tab; Enter then adds that displayed
    // directory through the structured control request, preserving its spaces.
    master.write_all(b"G\x15exam").unwrap();
    drain_pty(&mut master, &mut screen, "› example project/");
    master.write_all(b"\r").unwrap();
    drain_pty(&mut master, &mut screen, "No such file or directory");
    assert_eq!(f.snapshot().workspaces.len(), 1);
    assert!(screen.capture(38).contains("Add project"));
    assert!(
        screen
            .capture(38)
            .lines()
            .any(|line| line.contains("  exam "))
    );
    master.write_all(b"\t").unwrap();
    drain_pty(&mut master, &mut screen, "› .git/");
    master.write_all(b"\r").unwrap();
    wait_current_ui(&mut master, &mut screen, |_| {
        f.snapshot().workspace().unwrap().cwd == root.to_str().unwrap()
    });
    let added = f.snapshot();
    assert_eq!(added.workspaces.len(), 2);
    assert_eq!(added.workspace().unwrap().name, "example project");
    assert_eq!(added.workspace().unwrap().tabs.len(), 1);
    finish_ui(&mut master, &mut screen, &mut child);
}
