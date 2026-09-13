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
