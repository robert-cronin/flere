//! Local companion → verified packages → real exec/refresh → exact frontend ACK.
use super::*;
use crate::splits::{input as probe_input, pane, panes, probes, unchanged};
use serde_json::Value;
use std::path::Path;

struct Ui {
    master: fs::File,
    child: os::Process,
    screen: Terminal,
}
impl Ui {
    fn attach(f: &Fixture, ssh: &Path, width: u16, height: u16) -> Self {
        let mut command = Command::new("/usr/bin/env");
        command
            .args(["-u", "FLERE"])
            .arg(companion_binary())
            .args([
                "fixture-host",
                "--remote",
                env!("CARGO_BIN_EXE_flere"),
                "--state",
                f.state.to_str().unwrap(),
                "--ssh",
                ssh.to_str().unwrap(),
            ])
            .env("HOME", &f.root)
            .env_remove("XDG_DATA_HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_STATE_HOME")
            .env_remove("XDG_CACHE_HOME")
            .env_remove("FLERE_UPDATE_ACK")
            .env_remove("FLERE_CONNECT_UPDATE_ACK")
            .env_remove("FLERE_COORDINATED_UPDATE")
            .env_remove("FLERE_NATIVE_LAUNCH_ID");
        let (master, child) = os::spawn_command_pty(&f.root, &mut command, width, height).unwrap();
        let mut ui = Self {
            master,
            child,
            screen: Terminal::new(width as usize, height as usize),
        };
        ui.wait("PANE_READY_");
        ui
    }
    fn key(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
        pump_ui_bytes(&mut self.master, &mut self.screen, 100);
    }
    fn wait(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(35);
        while !self.screen.capture(110).contains(text) {
            pump_ui_bytes(&mut self.master, &mut self.screen, 20);
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "companion exited before {text}: {}",
                self.screen.capture(110)
            );
            assert!(
                Instant::now() < deadline,
                "missing {text}: {}",
                self.screen.capture(110)
            );
        }
    }
    fn artifact(&self, name: &str) {
        let Some(root) = std::env::var_os("FLERE_TEST_REMOTE_UPDATE_ARTIFACTS") else {
            return;
        };
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let frame = flere::screenshot::Frame {
            width: self.screen.grid.cols,
            height: self.screen.grid.rows,
            cells: self.screen.grid.cells.clone(),
            layers: Vec::new(),
            cursor: None,
        };
        fs::write(root.join(format!("{name}.png")), frame.png().unwrap()).unwrap();
        fs::write(root.join(format!("{name}.txt")), self.screen.capture(110)).unwrap();
    }
    fn finish(mut self) {
        finish_ui(&mut self.master, &mut self.screen, &mut self.child);
    }
}
impl Drop for Ui {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            os::hangup(self.master.as_raw_fd(), &mut self.child);
        }
    }
}
fn plans(f: &Fixture) -> Vec<Value> {
    let root = f.root.join(".local/share/flere/install/coordinated");
    fs::read_dir(root)
        .unwrap()
        .map(|entry| serde_json::from_slice(&fs::read(entry.unwrap().path()).unwrap()).unwrap())
        .collect()
}

#[test]
fn coordinated_update_reexecs_companion_and_core_with_exact_session_and_frontend_receipts() {
    for (width, height) in [(180, 42), (60, 24)] {
        let f = Fixture::new();
        let tabs = probes(&f, 2);
        pane(&f, "split-right", Some("move"));
        f.send(&tabs[0], b"UPDATE_LEFT_DRAFT");
        f.send(&tabs[1], b"UPDATE_RIGHT_DRAFT");
        let before = panes(&f);
        let supervisor_pid = f.child.id();
        let core_package = f.root.join("core-package");
        let companion_package = f.root.join("companion-package");
        flere::install::package(Path::new(env!("CARGO_BIN_EXE_flere")), &core_package, None)
            .unwrap();
        flere::install::package(companion_binary(), &companion_package, None).unwrap();
        let ssh = f.root.join("fixture-ssh");
        fs::write(&ssh,r#"#!/usr/bin/python3
import os,sys,shlex,pathlib,json
assert sys.argv[1:4]==['-T','--','fixture-host']
args=shlex.split(sys.argv[4]);assert args.pop(0)=='exec'
with pathlib.Path('ssh-updates').open('a') as f:f.write(json.dumps({'pid':os.getpid(),'args':args})+'\n')
os.execv(args[0],args)
"#).unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o700)).unwrap();
        let mut ui = Ui::attach(&f, &ssh, width, height);
        let companion_pid = ui.child.id();
        ui.key(b"\0K");
        ui.wait("LOCAL · Update Flere + companion");
        ui.artifact(&format!("sources-{width}"));
        let fields = format!(
            "{}\t{}\r\r",
            core_package.display(),
            companion_package.display()
        );
        ui.key(fields.as_bytes());
        ui.wait("Packages verified.");
        ui.artifact(&format!("review-{width}"));
        assert!(!f.root.join(".local/bin/flere").exists());
        assert!(
            !f.root.join(".local/bin/flere-connect").exists(),
            "the Enter used to prepare, including its coalesced tail, cannot install"
        );
        let prepared = plans(&f);
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0]["phase"], "prepared");
        assert_eq!(probe_input(&f, &tabs[0]), b"UPDATE_LEFT_DRAFT");
        assert_eq!(probe_input(&f, &tabs[1]), b"UPDATE_RIGHT_DRAFT");
        ui.key(b"\rNEVER_NATIVE_DURING_UPDATE");
        ui.wait("Updated Flere and this companion;");
        ui.artifact(&format!("applied-{width}"));
        let core_store = flere::install::Store::new(
            f.root.join(".local/share/flere/install"),
            f.root.join(".local/bin"),
            "flere",
        )
        .unwrap();
        let receipt = core_store.status().unwrap().unwrap();
        let activation = receipt.activation.as_ref().unwrap();
        assert_eq!(activation.supervisor, "applied");
        assert_eq!(activation.initiating_frontend, "applied");
        assert_eq!(
            os::process_executable(supervisor_pid).unwrap(),
            receipt.current.executable
        );
        let companion_receipt: Value = serde_json::from_slice(
            &fs::read(f.root.join(".local/share/flere/install/flere-connect.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            os::process_executable(companion_pid).unwrap(),
            PathBuf::from(companion_receipt["current"]["executable"].as_str().unwrap())
        );
        let plan = plans(&f).pop().unwrap();
        assert_eq!(plan["phase"], "applied");
        assert_eq!(plan["core_attempt"], receipt.attempt);
        assert_eq!(plan["companion_attempt"], companion_receipt["attempt"]);
        let frontends: Value = serde_json::from_slice(&f.req(&["frontends"])).unwrap();
        let tracked = frontends["tracked"].as_array().unwrap();
        assert_eq!(
            tracked.len(),
            1,
            "old bridge watcher must retire after companion reconnect"
        );
        assert_eq!(
            tracked[0]["build"]["build_id"],
            receipt.current.manifest.build.build_id
        );
        let after = panes(&f);
        assert_eq!(before.epoch, after.epoch);
        assert_eq!(before.active, after.active);
        assert_eq!(before.tab, after.tab);
        assert_eq!(before.split.unwrap().groups, after.split.unwrap().groups);
        unchanged(&f, &tabs);
        assert_eq!(probe_input(&f, &tabs[0]), b"UPDATE_LEFT_DRAFT");
        assert_eq!(probe_input(&f, &tabs[1]), b"UPDATE_RIGHT_DRAFT");
        ui.key(b"AFTER_UPDATE");
        wait_current_ui(&mut ui.master, &mut ui.screen, |_| {
            probe_input(&f, &tabs[1]) == b"UPDATE_RIGHT_DRAFTAFTER_UPDATE"
        });
        let connections = fs::read_to_string(f.root.join("ssh-updates")).unwrap();
        assert_eq!(
            connections.lines().count(),
            2,
            "one actual companion restart reconnects once"
        );
        ui.finish();
    }
}
