use super::*;

#[test]
fn git_context_uses_the_clicked_commit_and_keeps_inspection_read_only() {
    for (width, height) in [(160, 40), (60, 24)] {
        let f = Fixture::new();
        let scene = scene(&f);
        history_git(&scene.source_dir, &["init", "-q", "-b", "main"]);
        history_git(&scene.source_dir, &["add", "."]);
        history_git(&scene.source_dir, &["commit", "-qm", "CTX-FIRST"]);
        let first = String::from_utf8(history_git(&scene.source_dir, &["rev-parse", "HEAD"]))
            .unwrap()
            .trim()
            .to_owned();
        fs::write(&scene.file, "Second revision\n").unwrap();
        history_git(&scene.source_dir, &["add", "."]);
        history_git(&scene.source_dir, &["commit", "-qm", "CTX-SECOND"]);
        let head = history_git(&scene.source_dir, &["rev-parse", "HEAD"]);
        let index = fs::read(scene.source_dir.join(".git/index")).unwrap();
        let mut ui = Ui::attach(&f, &scene, width, height);
        let before = current(&f);
        let audit = input_audit(&f);
        ui.key(b"\0g");
        ui.wait(|s| s.capture(100).contains("CTX-FIRST"));
        let row = ui.find("CTX-FIRST");
        ui.context(row.0, row.1, "Copy commit SHA");
        assert_eq!(current(&f), before);
        ui.artifact(&format!("context-git-{width}"));
        let copy = ui.find("Copy commit SHA");
        let raw = ui.click(copy.0, copy.1);
        assert_eq!(osc52(&raw), vec![first.as_bytes().to_vec()]);

        let row = ui.find("CTX-FIRST");
        ui.context(row.0, row.1, "Open / expand");
        ui.key(b"\r");
        let filename = scene.file.file_name().unwrap().to_str().unwrap();
        ui.wait(|s| s.capture(100).contains(filename));
        let file = ui.find(filename);
        ui.context(file.0, file.1, "Preview diff");
        let preview = ui.find("Preview diff");
        ui.click(preview.0, preview.1);
        ui.wait(|s| s.capture(100).contains("Git diff"));
        if width < 100 {
            ui.key(b"\x1b[6~"); // Wrapped commit metadata fills the narrow first page.
        }
        ui.wait(|s| s.capture(100).contains("EXACT_EDITOR_FILE"));
        assert_eq!(current(&f), before);
        assert_eq!(input_audit(&f), audit);
        assert!(f.capture(&scene.source).contains("SOURCE_DRAFT"));
        alive(&f, &scene.source);
        assert_eq!(history_git(&scene.source_dir, &["rev-parse", "HEAD"]), head);
        assert_eq!(
            fs::read(scene.source_dir.join(".git/index")).unwrap(),
            index
        );
        assert_eq!(
            fs::read_to_string(&scene.file).unwrap(),
            "Second revision\n"
        );
        ui.finish();
    }
}
