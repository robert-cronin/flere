//! Real PTY hover overlays; graphics commands are inspected, never displayed or copied.
use super::*;

pub(super) fn locate(screen: &Terminal, text: &str) -> (usize, usize) {
    (0..screen.grid.rows)
        .find_map(|y| {
            let line = screen.grid.line(y);
            line.find(text).map(|at| {
                (
                    line[..at]
                        .chars()
                        .map(|ch| flere::terminal::char_width(ch) as usize)
                        .sum(),
                    y,
                )
            })
        })
        .unwrap_or_else(|| panic!("missing {text:?}: {}", screen.capture(100)))
}

fn artifact(screen: &Terminal, name: &str, icon: Option<(usize, usize)>) {
    let Some(root) = std::env::var_os("FLERE_TEST_HOVER_ARTIFACTS") else {
        return;
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root).unwrap();
    let image = include_bytes!("../fixtures/local-image.png");
    let (width, height) = flere::image_preview::dimensions(image).unwrap();
    let scale = (24.0 / width as f64).min(20.0 / height as f64);
    let frame = flere::screenshot::Frame {
        width: screen.grid.cols,
        height: screen.grid.rows,
        cells: screen.grid.cells.clone(),
        // The Kitty placement is verified by the fixture before this composition.
        layers: icon
            .map(|(x, y)| flere::screenshot::Layer {
                x,
                y,
                columns: (width as f64 * scale).floor() / 8.0,
                rows: (height as f64 * scale).floor() / 20.0,
                pixels: flere::screenshot::Pixels::Encoded(image.to_vec()),
            })
            .into_iter()
            .collect(),
        cursor: None,
    };
    fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
    fs::write(root.join(format!("{name}.txt")), screen.capture(100)).unwrap();
}

#[test]
fn hover_tooltip_keeps_logo_and_inspector_while_pointer_events_and_typing_stay_owned() {
    let f = Fixture::new();
    history_git(&f.root, &["init", "-q"]);
    fs::write(
        f.root.join("logo.png"),
        include_bytes!("../fixtures/local-image.png"),
    )
    .unwrap();
    let tab = f.new_workspace("Tooltip fixture");
    f.send(&tab, b"printf 'HOVER_DRAFT");
    f.wait_text(&tab, "HOVER_DRAFT");
    fs::write(
        f.state.join("ui.json"),
        br#"{"left":38,"right":44,"compact_cards":true,"pet":true,"reduced_motion":true}"#,
    )
    .unwrap();
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&f.state)
        .arg("attach")
        .env("HOME", &f.root)
        .env_remove("XDG_CACHE_HOME");
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 160, 35).unwrap();
    let mut screen = Terminal::new(160, 35);
    drain_pty(&mut master, &mut screen, "HOVER_DRAFT");
    let (_, card_y) = locate(&screen, "Tooltip fixture");
    let icon = 0x5248_0065u32;
    master
        .write_all(format!("\x1b_Gi={};OK\x1b\\\x1b[6;20;8t", 0x5248_0000u32).as_bytes())
        .unwrap();
    let mut output = Vec::new();
    let end = Instant::now() + Duration::from_secs(4);
    loop {
        output.extend(pump_ui_bytes(&mut master, &mut screen, 40));
        if String::from_utf8_lossy(&output)
            .contains(&format!("\x1b[{};5H\x1b_Ga=p,i={icon}", card_y + 1))
        {
            break;
        }
        assert!(Instant::now() < end, "logo did not become ready");
    }
    pump_ui_bytes(&mut master, &mut screen, 250);
    let inspector: Vec<_> = (3..33)
        .flat_map(|y| screen.grid.cells[y * 160 + 116..y * 160 + 160].to_vec())
        .collect();
    let before = f.snapshot();
    master
        .write_all(format!("\x1b[<35;10;{}M", card_y + 1).as_bytes())
        .unwrap();
    let mut hovered = Vec::new();
    let end = Instant::now() + Duration::from_secs(3);
    while !screen.capture(100).contains("Preview · click to focus") {
        hovered.extend(pump_ui_bytes(&mut master, &mut screen, 40));
        assert!(Instant::now() < end, "hover did not open");
    }
    let hovered = String::from_utf8_lossy(&hovered);
    assert!(!hovered.contains("\x1b[2J"), "hover must retain graphics");
    assert!(!hovered.contains(&format!("a=d,d=i,i={icon}")));
    let title = locate(&screen, "Details · Tooltip fixture");
    assert_eq!(title.0, 41, "tooltip belongs beside the card");
    let after: Vec<_> = (3..33)
        .flat_map(|y| screen.grid.cells[y * 160 + 116..y * 160 + 160].to_vec())
        .collect();
    assert_eq!(inspector, after, "hover must not replace the inspector");
    artifact(&screen, "hover-wide", Some((4, card_y)));
    master
        .write_all(format!("\x1b[<65;{};{}M", title.0 + 1, title.1 + 3).as_bytes())
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(screen.capture(100).contains("Preview · click to focus"));
    assert_eq!(f.snapshot().tab, before.tab);
    assert_eq!(f.snapshot().session().unwrap().run, tab.run);
    // An overlay right-click dismisses it without opening a hidden terminal menu.
    master
        .write_all(
            format!(
                "\x1b[<2;{};{}M\x1b[<2;{};{}m",
                title.0 + 1,
                title.1 + 1,
                title.0 + 1,
                title.1 + 1
            )
            .as_bytes(),
        )
        .unwrap();
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(!screen.capture(100).contains("Preview · click to focus"));
    assert!(!screen.capture(100).contains("Close tab…"));
    master
        .write_all(format!("\x1b[<35;10;{}M", card_y + 1).as_bytes())
        .unwrap();
    drain_pty(&mut master, &mut screen, "Preview · click to focus");
    master.write_all(b"_TYPED").unwrap();
    drain_pty(&mut master, &mut screen, "HOVER_DRAFT_TYPED");
    assert!(!screen.capture(100).contains("Details · Tooltip fixture"));
    assert_eq!(f.snapshot().epoch, before.epoch);
    assert_eq!(f.snapshot().session().unwrap().pid, tab.pid);
    finish_ui(&mut master, &mut screen, &mut child);
}

#[test]
fn hover_tooltip_clamps_narrow_viewports_and_keeps_hovered_card_logo_gutter() {
    for (width, height) in [(60, 24), (36, 18), (24, 18)] {
        let f = Fixture::new();
        let tab = f.new_workspace("Hover");
        fs::write(
            f.state.join("ui.json"),
            br#"{"left":26,"compact_cards":true,"pet":false,"reduced_motion":true}"#,
        )
        .unwrap();
        let mut command = outer_ui_command();
        command.arg("--state").arg(&f.state).arg("attach");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut screen = Terminal::new(width as usize, height as usize);
        drain_pty(&mut master, &mut screen, "FLERE");
        if width < 56 {
            master.write_all(b"\0h").unwrap();
        }
        drain_pty(&mut master, &mut screen, "Hover");
        let (_, card_y) = locate(&screen, "Hover");
        let logo =
            screen.grid.cells[card_y * width as usize + 4..card_y * width as usize + 7].to_vec();
        master
            .write_all(format!("\x1b[<35;10;{}M", card_y + 1).as_bytes())
            .unwrap();
        drain_pty(&mut master, &mut screen, "Details · Hover");
        assert_eq!(
            logo,
            screen.grid.cells[card_y * width as usize + 4..card_y * width as usize + 7]
        );
        let title = locate(&screen, "Details · Hover");
        let x = title.0 - 2;
        let y = title.1 - 1;
        assert_eq!(screen.grid.cells[y * width as usize + x].text, "┌");
        let right = (x + 1..width as usize)
            .find(|&x| screen.grid.cells[y * width as usize + x].text == "┐")
            .expect("right edge stays on screen");
        let bottom = (y + 1..height as usize)
            .find(|&y| screen.grid.cells[y * width as usize + right].text == "┘")
            .expect("bottom edge stays on screen");
        assert!(bottom < height as usize - 1);
        artifact(&screen, &format!("hover-{width}"), None);
        // Resize an open tooltip down to the supported minimum without trapped input.
        os::resize(master.as_raw_fd(), 20, 8).unwrap();
        screen.resize(20, 8);
        pump_ui_bytes(&mut master, &mut screen, 300);
        assert!(screen.capture(100).contains("Details"));
        master.write_all(b"\x1b").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        assert!(!screen.capture(100).contains("Details"));
        assert_eq!(f.snapshot().session().unwrap().run, tab.run);
        finish_ui(&mut master, &mut screen, &mut child);
    }
}

#[test]
#[cfg(target_os = "linux")]
fn card_links_open_by_click_and_keyboard_locally_and_in_the_companion() {
    const ISSUE: &str = "https://github.com/demo/fixture/issues/7";
    const PR: &str = "https://github.com/demo/fixture/pull/9";
    for remote in [false, true] {
        let f = Fixture::new();
        let tab = f.new_workspace("Linked card");
        let before = f.snapshot();
        let mut meta = before.workspace().unwrap().meta.clone();
        meta.issue = ISSUE.into();
        meta.pr = PR.into();
        f.req(&[
            "metadata",
            &before.active.to_string(),
            &wire::hex(&serde_json::to_vec(&meta).unwrap()),
        ]);
        f.send(&tab, b"printf 'LINK_DRAFT");
        f.wait_text(&tab, "LINK_DRAFT");
        fs::write(
            f.state.join("ui.json"),
            br#"{"left":38,"right":44,"pet":false,"reduced_motion":true}"#,
        )
        .unwrap();
        let bin = f.root.join("bin");
        fs::create_dir(&bin).unwrap();
        let log = f.root.join("opened-links");
        fs::write(
            bin.join("xdg-open"),
            "#!/bin/sh\nprintf '%s %s\\n' \"$FLERE_OPEN_SIDE\" \"$1\" >> \"$FLERE_OPEN_LOG\"\n",
        )
        .unwrap();
        fs::write(
            bin.join("gh"),
            "#!/bin/sh\nprintf '%s' '{\"title\":\"Linked fixture\",\"state\":\"OPEN\"}'\n",
        )
        .unwrap();
        for name in ["xdg-open", "gh"] {
            fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut command = if remote {
            let ssh = f.root.join("fixture-ssh");
            fs::write(&ssh, "#!/usr/bin/python3\nimport os,sys,shlex\nassert sys.argv[1:4]==['-T','--','fixture-host']\nargs=shlex.split(sys.argv[4]);assert args.pop(0)=='exec'\nos.environ['FLERE_OPEN_SIDE']='remote'\nos.execv(args[0],args)\n").unwrap();
            fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
            let mut c = Command::new("/usr/bin/env");
            c.args(["-u", "FLERE"])
                .arg(crate::remote::companion_binary())
                .args([
                    "fixture-host",
                    "--remote",
                    env!("CARGO_BIN_EXE_flere"),
                    "--state",
                ])
                .arg(&f.state)
                .arg("--ssh")
                .arg(ssh);
            c
        } else {
            let mut c = outer_ui_command();
            c.arg("--state").arg(&f.state).arg("attach");
            c
        };
        command
            .env("HOME", &f.root)
            .env_remove("XDG_CACHE_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_DATA_HOME")
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .env("FLERE_OPEN_LOG", &log)
            .env("FLERE_OPEN_SIDE", "local");
        let (mut master, mut child) =
            os::spawn_command_pty(&f.root, &mut command, 160, 35).unwrap();
        let mut screen = Terminal::new(160, 35);
        drain_pty(&mut master, &mut screen, "LINK_DRAFT");
        let (_, y) = locate(&screen, "Linked card");
        master
            .write_all(format!("\x1b[<35;10;{}M", y + 1).as_bytes())
            .unwrap();
        drain_pty(&mut master, &mut screen, "[i] Issue #7 · open");
        pump_ui_bytes(&mut master, &mut screen, 150);
        assert!(screen.capture(100).contains("[p] PR #9 · open"));
        assert!(!log.exists(), "hover opened a browser");
        artifact(
            &screen,
            if remote {
                "links-hover-remote"
            } else {
                "links-hover-local"
            },
            None,
        );
        let (x, y) = locate(&screen, "[i] Issue #7");
        // Dragging away cancels activation even if release returns to the link.
        master
            .write_all(
                format!(
                    "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
                    x + 1,
                    y + 1,
                    x + 2,
                    y + 1,
                    x + 1,
                    y + 1
                )
                .as_bytes(),
            )
            .unwrap();
        pump_ui_bytes(&mut master, &mut screen, 150);
        assert!(!log.exists());
        master
            .write_all(
                format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes(),
            )
            .unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            fs::read_to_string(&log).unwrap_or_default() == format!("local {ISSUE}\n")
        });
        // Open/pin through Cards navigation without sending a key to the shell.
        master.write_all(b"\x1b").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        master.write_all(b"\0h?").unwrap();
        drain_pty(&mut master, &mut screen, "> [i] Open issue");
        pump_ui_bytes(&mut master, &mut screen, 150);
        master.write_all(b"p").unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            fs::read_to_string(&log).unwrap_or_default() == format!("local {ISSUE}\nlocal {PR}\n")
        });
        pump_ui_bytes(&mut master, &mut screen, 150);
        master.write_all(b"\r").unwrap();
        wait_current_ui(&mut master, &mut screen, |_| {
            fs::read_to_string(&log).unwrap_or_default()
                == format!("local {ISSUE}\nlocal {PR}\nlocal {ISSUE}\n")
        });
        master.write_all(b"\x1b[200~ip\r\x1b[201~").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 150);
        assert_eq!(
            fs::read_to_string(&log).unwrap().lines().count(),
            3,
            "paste activated a link"
        );
        assert!(!screen.capture(100).contains("Enter to apply"));
        let after = f.snapshot();
        assert_eq!(after.epoch, before.epoch);
        assert_eq!(after.session().unwrap().pid, tab.pid);
        assert_eq!(after.session().unwrap().run, tab.run);
        assert!(f.capture(&tab).contains("LINK_DRAFT"));
        master.write_all(b"\x1b").unwrap();
        pump_ui_bytes(&mut master, &mut screen, 100);
        finish_ui(&mut master, &mut screen, &mut child);
    }
}
