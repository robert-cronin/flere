use super::*;
use std::{
    os::unix::net::UnixListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

fn editor_request(
    f: &Fixture,
    origin: &Snapshot,
    before: Option<&std::path::Path>,
    path: &std::path::Path,
) -> std::io::Result<Vec<u8>> {
    let mut fields = vec![
        if before.is_some() {
            "open-diff-pane-epoch".to_string()
        } else {
            "open-pane-epoch".to_string()
        },
        origin.epoch.clone(),
        origin.active.to_string(),
        origin.tab.to_string(),
        origin.session().unwrap().run.clone(),
        revision(origin).to_string(),
    ];
    if let Some(before) = before {
        fields.push(wire::hex(before.to_str().unwrap().as_bytes()));
    }
    fields.push(wire::hex(path.to_str().unwrap().as_bytes()));
    wire::request(
        &f.state,
        &fields.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

#[test]
fn exact_editor_origin_rejects_pane_changes_and_only_valid_requests_join_the_intended_group() {
    for comparison in [false, true] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        resize(&f, 101, 31);
        let original = pane(&f, "split-right", Some("move"));
        let before = f.root.join("before.txt");
        let path = f.root.join("after.txt");
        fs::write(&before, "before\n").unwrap();
        fs::write(&path, "after\n").unwrap();
        if comparison {
            fs::set_permissions(&before, fs::Permissions::from_mode(0o400)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        }
        let comparison_path = comparison.then_some(before.as_path());
        for tab in &tabs {
            f.send(tab, b"KEEP_EXACT_DRAFT");
        }
        wait(
            || tabs.iter().all(|t| input(&f, t) == b"KEEP_EXACT_DRAFT"),
            "both original drafts arrive",
        );
        let reject = |origin: &Snapshot| {
            let state = panes(&f);
            let audit = fs::read(f.state.join("actions.log")).unwrap();
            let error = editor_request(&f, origin, comparison_path, &path).unwrap_err();
            assert!(
                error.to_string().contains("stale") && !error.to_string().contains("unknown"),
                "must reject the origin, not an unsupported command: {error}"
            );
            let after = panes(&f);
            assert_eq!(identities(&after), identities(&state));
            assert_eq!(after.tab, state.tab);
            assert_eq!(
                after.split.as_ref().unwrap().groups,
                state.split.as_ref().unwrap().groups
            );
            assert_eq!(revision(&after), revision(&state));
            assert_eq!(fs::read(f.state.join("actions.log")).unwrap(), audit);
        };
        pane(&f, "focus", Some("0"));
        reject(&original);
        pane(&f, "focus", Some("1"));
        assert_eq!(panes(&f).tab, original.tab);
        reject(&original); // Same tab/run after a round trip cannot revive old work.
        let current = panes(&f);
        let mut stale_run = current.clone();
        stale_run
            .workspaces
            .iter_mut()
            .flat_map(|w| &mut w.tabs)
            .find(|t| t.id == current.tab)
            .unwrap()
            .run = "old-run".into();
        reject(&stale_run);
        editor_request(&f, &current, comparison_path, &path).unwrap();
        let opened = panes(&f);
        let editor = opened.session().unwrap().clone();
        assert_eq!(editor.kind, "editor");
        assert_eq!(editor.path, path.to_str().unwrap());
        let groups = &opened.split.as_ref().unwrap().groups;
        assert_eq!(groups[0].tabs, [tabs[0].id]);
        assert_eq!(groups[1].tabs, [tabs[1].id, editor.id]);
        assert_eq!(groups[1].selected, editor.id);
        // Even an already-open file must revalidate before changing pane focus.
        reject(&current);
        pane(&f, "focus", Some("0"));
        editor_request(&f, &panes(&f), comparison_path, &path).unwrap();
        let reused = panes(&f);
        assert_eq!(reused.tab, editor.id);
        assert_eq!(reused.session().unwrap().pid, editor.pid);
        assert_eq!(reused.split.as_ref().unwrap().focused, 1);
        assert_eq!(identities(&reused), identities(&opened));
        unchanged(&f, &tabs);
        assert!(tabs.iter().all(|t| input(&f, t) == b"KEEP_EXACT_DRAFT"));
    }
}

// A real same-user bridge injects another attachment's focus after the first
// focus request succeeds but before its caller reads back the new snapshot.
// This makes the race deterministic without sleeps or production test hooks.
struct FocusRace {
    state: PathBuf,
    next: Arc<Mutex<Option<Vec<String>>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl FocusRace {
    fn new(f: &Fixture) -> Self {
        let state = f.root.join("focus-proxy");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(state.join("control.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let next = Arc::new(Mutex::new(None::<Vec<String>>));
        let stop = Arc::new(AtomicBool::new(false));
        let (pending, stopped, upstream) = (next.clone(), stop.clone(), f.state.clone());
        let worker = std::thread::spawn(move || {
            let mut clients = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                let (mut client, _) = match listener.accept() {
                    Ok(client) => client,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("fixture proxy accept: {e}"),
                };
                // The UI opens its watch socket before capability negotiation
                // on another socket. Neither connection may block admission of
                // the other; the injected focus still precedes its own reply.
                let (pending, stopped, upstream) =
                    (pending.clone(), stopped.clone(), upstream.clone());
                clients.push(std::thread::spawn(move || {
                    // UI probes may close before the proxy accepts them. macOS can
                    // report EINVAL when setting a timeout on that disconnected peer.
                    let setup = client
                        .set_nonblocking(false)
                        .and_then(|()| client.set_read_timeout(Some(Duration::from_secs(2))))
                        .and_then(|()| client.set_write_timeout(Some(Duration::from_secs(2))));
                    if let Err(e) = setup {
                        if disconnected(&e) || e.kind() == std::io::ErrorKind::InvalidInput {
                            return;
                        }
                        panic!("fixture proxy client setup: {e}");
                    }
                    let request = match wire::read_frame(&mut client) {
                        Ok(request) => request,
                        Err(e)
                            if disconnected(&e)
                                || matches!(
                                    e.kind(),
                                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                                ) =>
                        {
                            return;
                        }
                        Err(e) => panic!("fixture proxy client request: {e}"),
                    };
                    let mut server = wire::connect(&upstream).unwrap();
                    server.write_all(&wire::frame(&request)).unwrap();
                    let command = request.split(|b| *b == b'\t').next().unwrap();
                    if command == b"watch-panes" || command == b"watch-panes-build" {
                        server
                            .set_read_timeout(Some(Duration::from_millis(50)))
                            .unwrap();
                        let mut bytes = [0; 65536];
                        while !stopped.load(Ordering::SeqCst) {
                            match server.read(&mut bytes) {
                                Ok(0) => break,
                                Ok(n) => {
                                    if client.write_all(&bytes[..n]).is_err() {
                                        break;
                                    }
                                }
                                Err(e)
                                    if matches!(
                                        e.kind(),
                                        std::io::ErrorKind::WouldBlock
                                            | std::io::ErrorKind::TimedOut
                                    ) => {}
                                Err(_) => break,
                            }
                        }
                        return;
                    }
                    let response = wire::read_frame(&mut server).unwrap();
                    let fields = std::str::from_utf8(&request)
                        .unwrap()
                        .split('\t')
                        .collect::<Vec<_>>();
                    if fields.first() == Some(&"pane")
                        && fields.get(6) == Some(&"focus")
                        && let Some(change) = pending.lock().unwrap().take()
                    {
                        assert_ne!(response.first(), Some(&b'!'), "first focus must succeed");
                        wire::request(
                            &upstream,
                            &change.iter().map(String::as_str).collect::<Vec<_>>(),
                        )
                        .unwrap();
                    }
                    let _ = client.write_all(&wire::frame(&response));
                }));
            }
            let mut failure = None;
            for client in clients {
                if let Err(error) = client.join()
                    && failure.is_none()
                {
                    failure = Some(error);
                }
            }
            if let Some(error) = failure {
                std::panic::resume_unwind(error);
            }
        });
        Self {
            state,
            next,
            stop,
            worker: Some(worker),
        }
    }
    fn arm(&self, f: &Fixture, tab: &TabView) {
        let snapshot = panes(f);
        *self.next.lock().unwrap() = Some(vec![
            "focus-exact".into(),
            snapshot.epoch,
            snapshot.active.to_string(),
            tab.id.to_string(),
            tab.run.clone(),
        ]);
    }
}
fn disconnected(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::UnexpectedEof
    )
}
impl Drop for FocusRace {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take()
            && let Err(error) = worker.join()
            && !std::thread::panicking()
        {
            // Preserve a proxy failure when it is the original failure, but do
            // not abort the process by panicking again during a test's unwind.
            std::panic::resume_unwind(error);
        }
    }
}

#[test]
fn actual_ui_changed_pane_focus_readback_cancels_wheel_and_tab_close() {
    let f = Fixture::new();
    let tabs = probes(&f, 3);
    pane(&f, "split-right", Some("move"));
    for tab in &tabs {
        emit(
            &f,
            tab,
            1,
            &format!("\x1b[?1049h\x1b[?1h\x1b[?1007hRACE_READY_{}", tab.id),
        );
        f.wait_text(tab, &format!("RACE_READY_{}", tab.id));
    }
    let proxy = FocusRace::new(&f);
    let mut command = outer_ui_command();
    command
        .arg("--state")
        .arg(&proxy.state)
        .arg("attach")
        .env("HOME", &f.root);
    let (mut master, mut child) = os::spawn_command_pty(&f.root, &mut command, 180, 42).unwrap();
    let mut screen = Terminal::new(180, 42);
    wait_current_ui(&mut master, &mut screen, |s| {
        s.capture(100)
            .contains(&format!("RACE_READY_{}", tabs[2].id))
    });
    wait(
        || {
            let frontends: Value = serde_json::from_slice(&f.req(&["frontends"])).unwrap();
            frontends["tracked"]
                .as_array()
                .unwrap()
                .iter()
                .any(|frontend| frontend["pid"] == child.id())
        },
        "the metadata-bearing UI watch is registered",
    );
    let frontends: Value = serde_json::from_slice(&f.req(&["frontends"])).unwrap();
    let tracked = frontends["tracked"].as_array().unwrap();
    assert_eq!(tracked.len(), 1, "the proxy must keep the UI watch alive");
    assert_eq!(tracked[0]["pid"], child.id());
    assert_eq!(
        tracked[0]["build"],
        serde_json::from_str::<Value>(flere::build_info::json()).unwrap(),
        "the proxy must forward the original frontend metadata"
    );
    let layout = flere::ui::Layout::new(180, 42);
    proxy.arm(&f, &tabs[1]);
    master
        .write_all(
            format!(
                "\x1b[<64;{};{}M",
                layout.terminal_x + 3,
                layout.terminal_y + 3
            )
            .as_bytes(),
        )
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        proxy.next.lock().unwrap().is_none() && s.capture(100).contains("Pane changed")
    });
    assert!(
        proxy.next.lock().unwrap().is_none(),
        "the concurrent focus must have happened"
    );
    assert_eq!(panes(&f).tab, tabs[1].id);
    assert!(
        tabs.iter().all(|t| input(&f, t).is_empty()),
        "wheel cannot reach the newly substituted tab"
    );
    // Restore the original scene, then race an inactive pane's visible close X.
    pane(&f, "focus", Some("1"));
    wait_current_ui(&mut master, &mut screen, |s| {
        let y = layout.terminal_y - 1;
        s.grid.cells[y * 180 + layout.terminal_x..(y + 1) * 180]
            .iter()
            .rposition(|c| c.text == "━" && c.style.fg == flere::terminal::Color::Rgb(67, 227, 247))
            .is_some_and(|x| x > layout.cols / 2)
    });
    let y = layout.terminal_y - 2;
    let x = (layout.terminal_x..layout.terminal_x + layout.cols / 2)
        .find(|x| screen.grid.cells[y * 180 + x].text == "×")
        .unwrap();
    proxy.arm(&f, &tabs[0]);
    master
        .write_all(format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).as_bytes())
        .unwrap();
    wait_current_ui(&mut master, &mut screen, |s| {
        proxy.next.lock().unwrap().is_none() && s.capture(100).contains("Pane changed")
    });
    assert!(proxy.next.lock().unwrap().is_none());
    pump_ui_bytes(&mut master, &mut screen, 100);
    assert!(!screen.capture(100).contains("Close terminal?"));
    unchanged(&f, &tabs);
    assert!(tabs.iter().all(|t| input(&f, t).is_empty()));
    finish_ui(&mut master, &mut screen, &mut child);
}
