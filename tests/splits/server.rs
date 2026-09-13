use super::*;

#[test]
fn right_and_below_panes_get_distinct_kernel_sizes_and_both_stream_without_focus_changes() {
    for (op, first_size, second_size) in [
        ("split-right", (30, 31), (70, 31)),
        ("split-below", (101, 8), (101, 20)),
    ] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        resize(&f, 101, 31);
        pane(&f, op, Some("move"));
        let split = pane(&f, "ratio", Some("300"));
        assert_eq!(split.tab, tabs[1].id);
        wait(
            || size(&f, &tabs[0]) == Some(first_size) && size(&f, &tabs[1]) == Some(second_size),
            "both kernel PTY sizes must match their distinct panes",
        );
        let mut watcher = wire::connect(&f.state).unwrap();
        watcher.write_all(&wire::frame(b"watch-panes")).unwrap();
        assert_eq!(wire::read_frame(&mut watcher).unwrap()[0], 5);
        emit(&f, &tabs[0], 1, "\x1b[2J\x1b[HFIRST_PANE_OUTPUT");
        emit(&f, &tabs[1], 1, "\x1b[2J\x1b[HSECOND_PANE_OUTPUT");
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let update = Snapshot::decode(&wire::read_frame(&mut watcher).unwrap()).unwrap();
            let other = &update.split.as_ref().unwrap().other;
            assert_eq!(update.tab, tabs[1].id);
            assert_eq!(other.tab, tabs[0].id);
            if text(&update.cells).contains("SECOND_PANE_OUTPUT")
                && text(&other.cells).contains("FIRST_PANE_OUTPUT")
            {
                assert_eq!((update.cols, update.rows), second_size);
                assert_eq!((other.cols, other.rows), first_size);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "both pane outputs were not broadcast"
            );
        }
        let legacy = f.req(&["snapshot"]);
        assert_eq!(legacy[0], 4);
        let legacy = Snapshot::decode(&legacy).unwrap();
        assert!(legacy.split.is_none());
        assert!(text(&legacy.cells).contains("SECOND_PANE_OUTPUT"));
        assert!(wire::request(&f.state, &["resize", "80", "24"]).is_err());
        assert_eq!(size(&f, &tabs[0]), Some(first_size));
        assert_eq!(size(&f, &tabs[1]), Some(second_size));
        f.send(&tabs[0], b"FIRST_DRAFT");
        f.send(&tabs[1], b"SECOND_DRAFT");
        wait(
            || input(&f, &tabs[0]) == b"FIRST_DRAFT" && input(&f, &tabs[1]) == b"SECOND_DRAFT",
            "exact pane drafts must arrive separately",
        );
        unchanged(&f, &tabs);
    }
}

#[test]
fn group_local_new_tabs_move_and_merge_keep_processes_and_reject_stale_mutations() {
    let f = Fixture::new();
    let tabs = probes(&f, 3);
    resize(&f, 101, 31);
    let initial = pane(&f, "split-right", Some("move"));
    assert_eq!(
        initial.split.as_ref().unwrap().groups[0].tabs,
        [tabs[0].id, tabs[1].id]
    );
    assert_eq!(initial.split.as_ref().unwrap().groups[1].tabs, [tabs[2].id]);
    let resized = pane(&f, "ratio", Some("300"));
    assert!(pane_request(&f, &initial, "zoom", None).is_err());
    let mut wrong_run = resized.clone();
    wrong_run
        .workspaces
        .iter_mut()
        .flat_map(|w| &mut w.tabs)
        .find(|t| t.id == resized.tab)
        .unwrap()
        .run = "stale-run".into();
    assert!(pane_request(&f, &wrong_run, "merge", None).is_err());
    let mut wrong_epoch = resized.clone();
    wrong_epoch.epoch = "0".repeat(32);
    assert!(pane_request(&f, &wrong_epoch, "merge", None).is_err());
    assert_eq!(revision(&panes(&f)), revision(&resized));
    assert_eq!(identities(&panes(&f)), identities(&resized));
    pane(&f, "focus", Some("0"));
    f.req(&["tab", &resized.active.to_string()]);
    let with_new = panes(&f);
    let shell = with_new.session().unwrap().clone();
    assert_eq!(shell.kind, "shell");
    assert_eq!(
        with_new.split.as_ref().unwrap().groups[0].tabs,
        [tabs[0].id, tabs[1].id, shell.id]
    );
    assert_eq!(
        with_new.split.as_ref().unwrap().groups[1].selected,
        tabs[2].id
    );
    f.send(&shell, b"SHELL_GROUP_DRAFT");
    f.wait_text(&shell, "SHELL_GROUP_DRAFT");
    let moved = pane(&f, "move", None);
    let layout = moved.split.as_ref().unwrap();
    assert_eq!(layout.focused, 1);
    assert_eq!(moved.tab, shell.id);
    assert_eq!(layout.groups[0].tabs, [tabs[0].id, tabs[1].id]);
    assert_eq!(layout.groups[1].tabs, [tabs[2].id, shell.id]);
    let merged = pane(&f, "merge", None);
    assert!(merged.split.is_none());
    assert_eq!(merged.tab, shell.id);
    assert_eq!(
        merged
            .workspace()
            .unwrap()
            .tabs
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        [tabs[0].id, tabs[1].id, tabs[2].id, shell.id]
    );
    unchanged(&f, &tabs);
    unchanged(&f, std::slice::from_ref(&shell));
    assert!(f.capture(&shell).contains("SHELL_GROUP_DRAFT"));
    assert!(tabs.iter().all(|t| input(&f, t).is_empty()));
}

#[test]
fn pane_wheel_and_image_tickets_reject_changed_focus_layout_and_viewport() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    resize(&f, 101, 31);
    pane(&f, "split-right", Some("move"));
    pane(&f, "ratio", Some("300"));
    emit(
        &f,
        &tabs[1],
        1,
        "\x1b[?1049h\x1b[?1h\x1b[?1007h\x1b[?2004hALTERNATE_SECOND",
    );
    f.wait_text(&tabs[1], "ALTERNATE_SECOND");
    let wheel = |tab: &TabView, cols: usize, rows: usize| {
        wire::request(
            &f.state,
            &[
                "wheel",
                &tab.id.to_string(),
                &tab.run,
                &cols.to_string(),
                &rows.to_string(),
                "-2",
            ],
        )
    };
    assert!(wheel(&tabs[0], 30, 31).is_err());
    assert!(wheel(&tabs[1], 101, 31).is_err());
    assert!(tabs.iter().all(|t| input(&f, t).is_empty()));
    wheel(&tabs[1], 70, 31).unwrap();
    wait(
        || !input(&f, &tabs[1]).is_empty(),
        "focused alternate-screen pane receives wheel arrows",
    );
    let prior = input(&f, &tabs[1]);
    assert_eq!(prior, b"\x1bOA\x1bOA");
    assert!(!prior.contains(&b'\r'));
    assert!(input(&f, &tabs[0]).is_empty());
    let attachment_dir = f.root.join(".cache/flere/attachments");
    fs::create_dir_all(&attachment_dir).unwrap();
    fs::set_permissions(&attachment_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let image = attachment_dir.join(format!("{}.png", os::nonce().unwrap()));
    fs::write(&image, STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNg+H8HAALeAdxNj6zLAAAAAElFTkSuQmCC").unwrap()).unwrap();
    fs::set_permissions(&image, fs::Permissions::from_mode(0o600)).unwrap();
    let ticket = || {
        let s = panes(&f);
        String::from_utf8(f.req(&[
            "image-ticket",
            &s.epoch,
            &s.active.to_string(),
            &s.tab.to_string(),
            &s.session().unwrap().run,
        ]))
        .unwrap()
    };
    let paste = |token: &str| {
        wire::request(
            &f.state,
            &[
                "image-paste",
                token,
                &wire::hex(image.to_str().unwrap().as_bytes()),
            ],
        )
    };
    let old_layout = ticket();
    pane(&f, "ratio", Some("450"));
    assert!(paste(&old_layout).is_err());
    assert!(wheel(&tabs[1], 70, 31).is_err());
    let old_focus = ticket();
    pane(&f, "focus", Some("0"));
    pane(&f, "focus", Some("1"));
    assert!(
        paste(&old_focus).is_err(),
        "returning cannot revive a stale paste ticket"
    );
    assert_eq!(input(&f, &tabs[1]), prior);
    assert!(input(&f, &tabs[0]).is_empty());
    paste(&ticket()).unwrap();
    let expected = [
        prior.as_slice(),
        b"\x1b[200~",
        image.to_str().unwrap().as_bytes(),
        b"\x1b[201~",
    ]
    .concat();
    wait(
        || input(&f, &tabs[1]) == expected,
        "fresh exact-target paste reaches only the focused pane",
    );
    assert!(input(&f, &tabs[0]).is_empty());
    unchanged(&f, &tabs);
}

#[test]
fn live_refresh_preserves_split_order_geometry_and_every_exact_draft() {
    let f = Fixture::new();
    let tabs = probes(&f, 2);
    resize(&f, 101, 31);
    pane(&f, "split-below", Some("move"));
    let before = pane(&f, "ratio", Some("300"));
    for (tab, draft) in tabs
        .iter()
        .zip([b"FIRST_REFRESH_DRAFT".as_slice(), b"SECOND_REFRESH_DRAFT"])
    {
        f.send(tab, draft);
        wait(|| input(&f, tab) == draft, "draft ready before refresh");
    }
    let mut watcher = wire::connect(&f.state).unwrap();
    watcher.write_all(&wire::frame(b"watch-panes")).unwrap();
    wire::read_frame(&mut watcher).unwrap();
    f.req(&[
        "refresh",
        &wire::hex(env!("CARGO_BIN_EXE_flere").as_bytes()),
    ]);
    wait(
        || {
            wire::request(&f.state, &["refresh-status"]).is_ok_and(|b| {
                String::from_utf8_lossy(&b).contains("all terminal sessions preserved")
            })
        },
        "split supervisor refresh completed",
    );
    let after = panes(&f);
    assert_eq!(after.epoch, before.epoch);
    assert_eq!(identities(&after), identities(&before));
    let a = after.split.as_ref().unwrap();
    let b = before.split.as_ref().unwrap();
    assert_eq!(
        (&a.groups, a.axis, a.ratio, a.focused, a.revision),
        (&b.groups, b.axis, b.ratio, b.focused, b.revision)
    );
    assert_eq!(
        Snapshot::decode(&wire::read_frame(&mut watcher).unwrap())
            .unwrap()
            .epoch,
        before.epoch
    );
    assert_eq!(size(&f, &tabs[0]), Some((101, 8)));
    assert_eq!(size(&f, &tabs[1]), Some((101, 20)));
    assert_eq!(input(&f, &tabs[0]), b"FIRST_REFRESH_DRAFT");
    assert_eq!(input(&f, &tabs[1]), b"SECOND_REFRESH_DRAFT");
    unchanged(&f, &tabs);
    f.req(&["refresh", &wire::hex(b"/usr/bin/false")]);
    wait(
        || String::from_utf8_lossy(&f.req(&["refresh-status"])).contains("rejected"),
        "incompatible candidate rejected before exec",
    );
    assert_eq!(identities(&panes(&f)), identities(&after));
    assert_eq!(input(&f, &tabs[0]), b"FIRST_REFRESH_DRAFT");
    assert_eq!(input(&f, &tabs[1]), b"SECOND_REFRESH_DRAFT");
}
