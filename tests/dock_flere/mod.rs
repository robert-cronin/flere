//! Actual dock selection and gestures are cosmetic, with no native-input ownership.
use super::*;

fn artifact(screen: &Terminal, name: &str) {
    let Some(root) = std::env::var_os("FLERE_TEST_DOCK_FLERE_ARTIFACTS") else {
        return;
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root).unwrap();
    let frame = flere::screenshot::Frame {
        width: screen.grid.cols,
        height: screen.grid.rows,
        cells: screen.grid.cells.clone(),
        layers: Vec::new(),
        cursor: None,
    };
    fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
}

#[test]
fn default_dock_flere_and_original_shortcuts_preserve_draft_through_petting_and_drag() {
    for quiet in [false, true] {
        let f = Fixture::new();
        let tab = f.new_workspace("Dock Flere");
        f.send(&tab, b"echo DOCK_FLERE_SAFE_DRAFT");
        fs::write(
            f.state.join("ui.json"),
            serde_json::to_vec(&serde_json::json!({
                "left":38,"right":33,"pet":true,"reduced_motion":quiet
            }))
            .unwrap(),
        )
        .unwrap();
        let before = f.snapshot();
        let mut command = outer_ui_command();
        command
            .arg("--state")
            .arg(&f.state)
            .arg("attach")
            .env("HOME", &f.root)
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_DATA_HOME");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, 188, 39).unwrap();
        let mut screen = Terminal::new(188, 39);
        drain_pty(&mut master, &mut screen, "DOCK_FLERE_SAFE_DRAFT");
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("PET / Flere")
        });
        master.write_all(b"\0O").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(38).contains("PET Ready")
        });
        for (key, label) in [
            (b'1', "Duck"),
            (b'2', "Robot"),
            (b'3', "Cat"),
            (b'4', "Flere"),
        ] {
            master.write_all(&[key]).unwrap();
            wait_current_ui(&mut master, &mut screen, |s| {
                s.capture(100).contains(&format!("PET / {label}"))
            });
            let prefs: flere::workspace::Preferences =
                serde_json::from_value(super::preferences::durable(&f)).unwrap();
            assert_eq!(prefs.pet_kind.label(), label);
        }
        // Hover, wheel and pasted commands cannot pet Flere or touch the native draft.
        let (x, y) = mascot_cell(&screen, 157, 30, 29);
        master
            .write_all(
                format!("\x1b[<35;{};{}M\x1b[<65;{};{}M", x + 1, y + 1, x + 1, y + 1).as_bytes(),
            )
            .unwrap();
        master
            .write_all(b"\x1b[200~p\rMUST_NOT_REACH_NATIVE\x1b[201~")
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert!(screen.grid.line(38).contains("PET Ready"));
        assert!(!f.capture(&tab).contains("MUST_NOT_REACH_NATIVE"));
        click_mascot(&mut master, &screen, 157, 30, 29);
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(38).contains("PET Satisfied") || s.grid.line(38).contains("PET Annoyed")
        });
        artifact(
            &screen,
            if quiet {
                "dock-flere-ui-quiet"
            } else {
                "dock-flere-ui"
            },
        );
        // A held sprite owns its drag/release, even outside the dock over the terminal.
        let (x, y) = mascot_cell(&screen, 157, 30, 29);
        master
            .write_all(format!("\x1b[<0;{};{}M", x + 1, y + 1).as_bytes())
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.grid.line(38).contains("PET Dangling")
        });
        master
            .write_all(b"\x1b[<32;65;10M\x1b[200~HELD_INPUT\x1b[201~\x1b[<0;65;10m")
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert!(!f.capture(&tab).contains("HELD_INPUT"));
        let after = f.snapshot();
        assert_eq!((after.active, after.tab), (before.active, before.tab));
        assert_eq!(
            (after.session().unwrap().pid, &after.session().unwrap().run),
            (tab.pid, &tab.run)
        );
        assert!(f.capture(&tab).contains("echo DOCK_FLERE_SAFE_DRAFT"));
        master.write_all(b"\x1b").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            !s.grid.line(38).contains("Esc back")
        });
        master.write_all(b"\x1b").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            !s.grid.line(38).trim_start().starts_with("NAV")
        });
        master.write_all(b"_FRESH").unwrap();
        wait_current_ui(&mut master, &mut screen, |s| {
            s.capture(100).contains("DOCK_FLERE_SAFE_DRAFT_FRESH")
        });
        finish_ui(&mut master, &mut screen, &mut child);
    }
}
