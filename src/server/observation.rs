//! Slice background discovery between client/PTY turns; never cache authorization.
use super::*;

const SLICE_TABS: usize = 8;
const SLICE_TIME: Duration = Duration::from_millis(2);

#[derive(Clone)]
struct Target {
    workspace: u64,
    session: u64,
    run: String,
}
struct Observation {
    target: Target,
    pid: u32,
    cwd: PathBuf,
    launcher: Option<(u32, String)>,
    previous: String,
    conversation: String,
}
pub(super) struct State {
    checked: Instant,
    pending: VecDeque<Target>,
    observed: Vec<Observation>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            checked: Instant::now(),
            pending: VecDeque::new(),
            observed: Vec::new(),
        }
    }
}
impl State {
    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
}
impl Server {
    pub(super) fn observe_native(&mut self) -> bool {
        if self.stop || os::stopping() {
            // Do not keep the event loop in immediate-poll mode during shutdown.
            // Already observed history is flushed before the final checkpoint.
            self.native_observation.pending.clear();
            return false;
        }
        let mut changed = false;
        if !self.native_observation.pending()
            && self.native_observation.checked.elapsed() < Duration::from_secs(1)
        {
            return false;
        }
        let _timing = crate::diagnostics::measure("supervisor-native-poll");
        let started = Instant::now();
        if !self.native_observation.pending() {
            self.native_observation.checked = started;
            self.sample_tab_directories();
            changed |= self.checkpoint_tabs().is_err();
            self.native_observation.pending = self
                .workspaces
                .iter()
                .flat_map(|w| {
                    w.tabs.iter().map(|s| Target {
                        workspace: w.id,
                        session: s.id,
                        run: s.run.clone(),
                    })
                })
                .collect();
        }
        for index in 0..SLICE_TABS {
            // A single discovery may exceed the budget. Always make progress,
            // then return to the event loop before inspecting another tab.
            if index > 0 && started.elapsed() >= SLICE_TIME {
                break;
            }
            let Some(target) = self.native_observation.pending.pop_front() else {
                break;
            };
            let Some(w) = self
                .workspaces
                .iter_mut()
                .find(|w| w.id == target.workspace)
            else {
                continue;
            };
            let Some(s) = w
                .tabs
                .iter_mut()
                .find(|s| s.id == target.session && s.run == target.run)
            else {
                continue;
            };
            if !s.alive || s.ended {
                if s.working {
                    s.working = false;
                    changed = true;
                }
                continue;
            }
            // This fresh result belongs only to this tab's current inspection.
            // Coordination operations continue to prove their own identities.
            let observed = crate::native::working_screen(&s.term).then(|| {
                crate::native::discover_codex(
                    s.child.id(),
                    s.native.as_ref().map_or(&w.cwd, |n| &n.cwd),
                )
            });
            let working = matches!(&observed, Some(Ok(Some(_))));
            if s.working != working {
                s.working = working;
                changed = true;
            }
            let Some(spec) = &s.native else {
                continue;
            };
            if spec.harness != "codex"
                || spec.launcher.as_ref().is_some_and(|(pid, start)| {
                    !os::child_identity(*pid).is_ok_and(|p| p.1 == *start)
                })
            {
                continue;
            }
            let conversation =
                observed.unwrap_or_else(|| crate::native::discover_codex(s.child.id(), &spec.cwd));
            if let Ok(Some(conversation)) = conversation
                && spec.conversation != conversation
            {
                // Apply at the round boundary so slicing does not multiply
                // durable writes. A later command may replace this identity.
                self.native_observation.observed.push(Observation {
                    target,
                    pid: s.child.id(),
                    cwd: spec.cwd.clone(),
                    launcher: spec.launcher.clone(),
                    previous: spec.conversation.clone(),
                    conversation,
                });
            }
        }
        if !self.native_observation.pending() {
            changed |= self.finish_native_observation();
            crate::diagnostics::slow("supervisor-native-round", self.native_observation.checked);
        }
        changed
    }
    pub(super) fn finish_native_observation(&mut self) -> bool {
        let mut recorded = false;
        for observation in std::mem::take(&mut self.native_observation.observed) {
            let Some(w) = self
                .workspaces
                .iter_mut()
                .find(|w| w.id == observation.target.workspace)
            else {
                continue;
            };
            let Some(s) = w.tabs.iter_mut().find(|s| {
                s.id == observation.target.session
                    && s.run == observation.target.run
                    && s.child.id() == observation.pid
            }) else {
                continue;
            };
            let Some(spec) = &mut s.native else {
                continue;
            };
            if spec.harness != "codex"
                || spec.cwd != observation.cwd
                || spec.launcher != observation.launcher
                || spec.conversation != observation.previous
                || (s.alive
                    && !s.ended
                    && spec.launcher.as_ref().is_some_and(|(pid, start)| {
                        !os::child_identity(*pid).is_ok_and(|p| p.1 == *start)
                    }))
            {
                continue;
            }
            // This is already observed history, never an authorization result.
            // Preserve it for the same exited run before its terminal is pruned.
            let saved = crate::native::Conversation {
                harness: "codex".into(),
                uuid: observation.conversation.clone(),
                cwd: spec.cwd.clone(),
            };
            if !w.meta.conversations.contains(&saved) {
                w.meta.conversations.push(saved.clone());
            }
            w.meta.last_conversation = Some(saved);
            spec.conversation = observation.conversation;
            recorded = true;
        }
        if recorded {
            if let Err(e) = self.persist() {
                let _ = self.audit("conversation-save-failed", 0, &e.to_string());
            } else {
                let _ = self.audit("conversation-linked", 0, "exact owned process metadata");
            }
        }
        recorded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
        server: Server,
    }
    impl Fixture {
        fn new(count: usize) -> Self {
            let root = PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tests")
                .join(&os::nonce().unwrap()[..12]);
            let state = root.join("s");
            private_state(&state).unwrap();
            let mut server = Server::load(&state).unwrap();
            server.active = 1;
            server.next = 2;
            server.workspaces.push(Workspace {
                id: 1,
                name: "Observation fixture".into(),
                cwd: root.clone(),
                meta: CardMeta::default(),
                tabs: Vec::new(),
                selected: 0,
                split: None,
            });
            let mut fixture = Self { root, server };
            for _ in 0..count {
                fixture.add_tab();
            }
            fixture
        }
        fn add_tab(&mut self) -> u64 {
            self.add_script("read fixture_input")
        }
        fn add_script(&mut self, script: &str) -> u64 {
            let mut command = std::process::Command::new("/bin/sh");
            command
                .args(["-c", script])
                .env("ENV", "")
                .env("HOME", &self.root);
            let (master, child) = os::spawn_command_pty(&self.root, &mut command, 2, 2).unwrap();
            let id = self.server.next;
            self.server.next += 1;
            let run = os::nonce().unwrap();
            self.server.workspaces[0].tabs.push(Session {
                id,
                run: run.clone(),
                shell_run: String::new(),
                master,
                child,
                term: Terminal::new(2, 2),
                input: VecDeque::new(),
                alive: true,
                ended: false,
                title: "fixture".into(),
                kind: "agent".into(),
                path: String::new(),
                native: Some(crate::native::HostSpec {
                    id,
                    run,
                    harness: "codex".into(),
                    argv: vec!["/bin/sh".into()],
                    cwd: self.root.clone(),
                    shell: "/bin/sh".into(),
                    conversation: String::new(),
                    dispatch: String::new(),
                    launcher: None,
                }),
                working: true,
            });
            id
        }
        fn result(&self, index: usize) -> Observation {
            let tab = &self.server.workspaces[0].tabs[index];
            let spec = tab.native.as_ref().unwrap();
            Observation {
                target: Target {
                    workspace: 1,
                    session: tab.id,
                    run: tab.run.clone(),
                },
                pid: tab.child.id(),
                cwd: spec.cwd.clone(),
                launcher: spec.launcher.clone(),
                previous: spec.conversation.clone(),
                conversation: format!("11111111-1234-5678-9012-{index:012}"),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for tab in &mut self.server.workspaces[0].tabs {
                let _ = tab.child.kill();
                let _ = tab.child.wait();
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn output_only_drain_leaves_layout_retry_to_periodic_observation() {
        let mut f = Fixture::new(1);
        // Control the round boundary explicitly, rather than racing its timer.
        f.server.native_observation.checked = Instant::now() + Duration::from_secs(3600);
        f.server.workspaces[0].tabs[0].term = Terminal::new(80, 8);
        f.server.workspaces[0].selected = f.server.workspaces[0].tabs[0].id;
        f.server.checkpoint_tabs().unwrap();
        let path = f.server.state.join("workspaces.v2.json");
        let saved = fs::read(&path).unwrap();
        let inode = fs::metadata(&path).unwrap().ino();
        // An unrelated pending change must not make output itself perform a save.
        f.server.workspaces[0].tabs[0].title = "pending title".into();
        f.server.workspaces[0].tabs[0]
            .master
            .write_all(b"output-token")
            .unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        let mut displayed = false;
        loop {
            displayed |= f.server.drain();
            let snapshot = f.server.snapshot();
            let text: String = snapshot.cells.iter().map(|c| c.text.as_str()).collect();
            if text.contains("output-token") {
                break;
            }
            assert!(Instant::now() < until, "owned shell echo was not displayed");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(displayed);
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);

        // Periodic observation still retries failed checkpoints and persists
        // the pending layout once storage is usable, without needing output.
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        f.server.native_observation.checked = Instant::now() - Duration::from_secs(1);
        assert!(f.server.observe_native());
        assert!(path.is_dir());
        fs::remove_dir(&path).unwrap();
        f.server.native_observation.checked = Instant::now() - Duration::from_secs(1);
        f.server.observe_native();
        let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.workspaces[0].tabs[0].title, "pending title");
    }

    #[test]
    fn pty_retirement_checkpoints_before_child_exit_including_deferred_io() {
        for deferred in [false, true] {
            let mut f = Fixture::new(0);
            let id = f.add_script("read fixture_input; exec 0<&- 1>&- 2>&-; exec /bin/sleep 30");
            f.server.workspaces[0].selected = id;
            f.server.native_observation.checked = Instant::now() + Duration::from_secs(3600);
            f.server.checkpoint_tabs().unwrap();
            let path = f.server.state.join("workspaces.v2.json");
            let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(saved.workspaces[0].tabs.len(), 1);
            f.server.workspaces[0].tabs[0]
                .master
                .write_all(b"\n")
                .unwrap();
            let until = Instant::now() + Duration::from_secs(3);
            while f.server.workspaces[0].tabs[0].alive {
                if deferred {
                    // The durable writer drains PTYs without lifecycle work.
                    let (_, retired) = f.server.drain_terminals();
                    f.server.io_retired |= retired;
                } else {
                    f.server.drain();
                }
                assert!(Instant::now() < until, "owned child did not close its PTY");
                std::thread::sleep(Duration::from_millis(2));
            }
            if deferred {
                assert!(f.server.io_retired);
                assert!(
                    f.server.drain(),
                    "deferred retirement must update the display"
                );
                assert!(!f.server.io_retired);
            }
            let tab = &mut f.server.workspaces[0].tabs[0];
            assert!(!tab.ended);
            assert!(tab.child.try_wait().unwrap().is_none());
            let saved: Saved = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert!(saved.workspaces[0].tabs.is_empty());
        }
    }

    #[test]
    fn slices_service_each_recorded_run_once_and_admit_new_runs_next_round() {
        let mut f = Fixture::new(2 * SLICE_TABS + 1);
        f.server.native_observation.checked = Instant::now() - Duration::from_secs(1);
        f.server.observe_native();
        let pending = f.server.native_observation.pending.len();
        assert!((SLICE_TABS + 1..=2 * SLICE_TABS).contains(&pending));
        let replaced = f.server.native_observation.pending.front().unwrap().clone();
        let new_run = os::nonce().unwrap();
        let tab = f.server.session(replaced.session, &replaced.run).unwrap();
        tab.run = new_run.clone();
        tab.native.as_mut().unwrap().run = new_run.clone();
        let added = f.add_tab();
        f.server.workspaces[0].tabs.reverse(); // Array positions are not identities.
        while f.server.native_observation.pending() {
            let before = f.server.native_observation.pending.len();
            f.server.observe_native();
            let after = f.server.native_observation.pending.len();
            assert!(after < before && before - after <= SLICE_TABS);
        }
        for tab in &f.server.workspaces[0].tabs {
            assert_eq!(tab.working, tab.id == added || tab.id == replaced.session);
        }
        assert_eq!(
            f.server.session(replaced.session, &new_run).unwrap().run,
            new_run
        );
        f.server.native_observation.checked = Instant::now() - Duration::from_secs(1);
        f.server.observe_native();
        while f.server.native_observation.pending() {
            f.server.observe_native();
        }
        assert!(f.server.workspaces[0].tabs.iter().all(|s| !s.working));
        assert!(f.server.workspaces[0].meta.conversations.is_empty());
    }

    #[test]
    fn round_commit_rejects_changed_targets_and_never_overwrites_newer_conversations() {
        let mut f = Fixture::new(10);
        let observations = (0..10).map(|i| f.result(i)).collect();
        f.server.native_observation.observed = observations;
        let tabs = &mut f.server.workspaces[0].tabs;
        tabs[1].run = os::nonce().unwrap();
        tabs[2].ended = true;
        let newer = "22222222-1234-5678-9012-123456789012";
        tabs[3].native.as_mut().unwrap().conversation = newer.into();
        tabs[4].native.as_mut().unwrap().launcher = Some((tabs[4].child.id(), "stale".into()));
        tabs[5].native.as_mut().unwrap().cwd = f.root.join("changed");
        f.server.native_observation.observed[6].pid = 0;
        tabs[7].native = None;
        f.server.native_observation.observed[8].target.workspace = 999;
        let launcher = Some((tabs[9].child.id(), "stale".into()));
        tabs[9].native.as_mut().unwrap().launcher = launcher.clone();
        f.server.native_observation.observed[9].launcher = launcher;
        assert!(f.server.finish_native_observation());
        let w = &f.server.workspaces[0];
        assert_eq!(w.meta.conversations.len(), 2);
        assert_eq!(
            w.meta.conversations[0].uuid,
            "11111111-1234-5678-9012-000000000000"
        );
        assert_eq!(w.tabs[3].native.as_ref().unwrap().conversation, newer);
        assert!(f.server.native_observation.observed.is_empty());
        let store = f.server.state.join("workspaces.v2.json");
        let metadata = fs::metadata(&store).unwrap();
        assert!(!f.server.finish_native_observation());
        assert_eq!(fs::metadata(&store).unwrap().ino(), metadata.ino());
        assert_eq!(
            fs::metadata(&store).unwrap().modified().unwrap(),
            metadata.modified().unwrap()
        );
    }

    #[test]
    fn lifecycle_boundaries_preserve_already_observed_conversations() {
        let mut f = Fixture::new(3);
        let first = f.result(0);
        let first_id = first.target.session;
        let first_run = first.target.run.clone();
        f.server.native_observation.observed.push(first);
        assert!(
            f.server
                .command(&format!("native-ended\t{first_id}\twrong"))
                .is_err()
        );
        assert!(f.server.workspaces[0].meta.conversations.is_empty());
        assert_eq!(f.server.native_observation.observed.len(), 1);
        f.server
            .command(&format!("native-ended\t{first_id}\t{first_run}"))
            .unwrap();
        assert!(
            f.server
                .session(first_id, &first_run)
                .unwrap()
                .native
                .is_none()
        );
        assert_eq!(f.server.workspaces[0].meta.conversations.len(), 1);

        let second = f.result(1);
        let second_id = second.target.session;
        let second_run = second.target.run.clone();
        f.server.native_observation.observed.push(second);
        f.server
            .command(&format!("close\t{second_id}\t{second_run}"))
            .unwrap();
        assert_eq!(f.server.workspaces[0].meta.conversations.len(), 2);

        let third = f.result(2);
        let third_id = third.target.session;
        f.server
            .native_observation
            .pending
            .push_back(third.target.clone());
        f.server.native_observation.observed.push(third);
        f.server.workspaces[0].tabs[2].child.kill().unwrap();
        f.server.workspaces[0].tabs[2].child.wait().unwrap();
        f.server.drain();
        assert_eq!(f.server.workspaces[0].meta.conversations.len(), 3);
        assert!(!f.server.workspaces[0].tabs.iter().any(|t| t.id == third_id));
        let saved: Saved =
            serde_json::from_slice(&fs::read(f.server.state.join("workspaces.v2.json")).unwrap())
                .unwrap();
        assert_eq!(saved.workspaces[0].meta.conversations.len(), 3);
    }

    #[test]
    fn refresh_flushes_observed_history_and_keeps_uninspected_targets_pending() {
        let mut f = Fixture::new(2);
        let observed = f.result(0);
        let pending = f.result(1).target;
        f.server.native_observation.observed.push(observed);
        f.server
            .native_observation
            .pending
            .push_back(pending.clone());
        assert!(f.server.command("refresh\t00").is_err());
        assert!(f.server.workspaces[0].meta.conversations.is_empty());
        let binary = std::env::current_exe().unwrap();
        f.server
            .command(&format!(
                "refresh\t{}",
                wire::hex(binary.to_str().unwrap().as_bytes())
            ))
            .unwrap();
        assert_eq!(f.server.refresh.as_ref(), Some(&binary));
        assert_eq!(f.server.workspaces[0].meta.conversations.len(), 1);
        assert_eq!(f.server.native_observation.pending.len(), 1);
        assert_eq!(f.server.native_observation.pending[0].run, pending.run);
        assert_eq!(
            f.server.native_observation.pending[0].session,
            pending.session
        );
    }

    #[test]
    fn stopping_ends_immediate_polling_without_discarding_observed_history() {
        let mut f = Fixture::new(1);
        let observation = f.result(0);
        f.server
            .native_observation
            .pending
            .push_back(observation.target.clone());
        f.server.native_observation.observed.push(observation);
        f.server.stop = true;
        assert!(!f.server.observe_native());
        assert!(!f.server.native_observation.pending());
        assert_eq!(f.server.native_observation.observed.len(), 1);
        assert!(f.server.workspaces[0].meta.conversations.is_empty());
        assert!(f.server.finish_native_observation());
        assert_eq!(f.server.workspaces[0].meta.conversations.len(), 1);
    }

    #[test]
    fn round_persistence_failure_retains_observed_history_for_checkpoint_retry() {
        let mut f = Fixture::new(2);
        f.server.native_observation.observed = vec![f.result(0), f.result(1)];
        let store = f.server.state.join("workspaces.v2.json");
        fs::create_dir(&store).unwrap();
        assert!(f.server.finish_native_observation());
        assert_eq!(f.server.workspaces[0].meta.conversations.len(), 2);
        assert!(f.server.checkpoint_tabs().is_err());
        fs::remove_dir(&store).unwrap();
        f.server.checkpoint_tabs().unwrap();
        let saved: Saved = serde_json::from_slice(&fs::read(&store).unwrap()).unwrap();
        assert_eq!(saved.workspaces[0].meta.conversations.len(), 2);
        assert_eq!(saved.workspaces[0].tabs.len(), 2);
        assert!(
            saved.workspaces[0]
                .tabs
                .iter()
                .all(|t| !t.conversation.is_empty())
        );
    }
}
