//! Disposable, owned sessions for documentation tools. Never attaches to user state.
#![allow(dead_code)]
use flere::{model::Snapshot, os, terminal::Terminal, wire};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::{fd::AsRawFd, unix::fs::DirBuilderExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub struct Fixture {
    pub root: PathBuf,
    pub state: PathBuf,
    pub project: PathBuf,
    pub binary: PathBuf,
    pub server: Child,
}
impl Fixture {
    pub fn new(binary: &Path) -> io::Result<Self> {
        let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is required"))?;
        let root = PathBuf::from(home)
            .join(".cache/flere/docs")
            .join(&os::nonce()?[..10]);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&root)?;
        let state = root.join("s");
        let project = root.join("signal");
        fs::create_dir(&project)?;
        let log = File::create(root.join("supervisor.log"))?;
        let mut cmd = Command::new(binary);
        Self::environment(&mut cmd, &root);
        let server = cmd
            .args(["--state"])
            .arg(&state)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()?;
        let mut f = Self {
            root,
            state,
            project,
            binary: binary.into(),
            server,
        };
        let end = Instant::now() + Duration::from_secs(5);
        while wire::request(&f.state, &["ping"]).is_err() {
            if Instant::now() > end || f.server.try_wait()?.is_some() {
                return Err(io::Error::other("documentation supervisor did not start"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(f)
    }
    pub fn environment(cmd: &mut Command, root: &Path) {
        os::clean_environment(cmd);
        cmd.env("HOME", root)
            .env("SHELL", "/bin/sh")
            .env("ENV", "")
            .env("PS1", "$ ")
            .env("HISTFILE", "/dev/null")
            .env("XDG_CACHE_HOME", root.join(".cache"))
            .env("VISUAL", "/usr/bin/vim -Nu NONE -n --noplugin")
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH");
    }
    pub fn request(&self, fields: &[&str]) -> io::Result<Vec<u8>> {
        wire::request(&self.state, fields)
    }
    pub fn snapshot(&self) -> io::Result<Snapshot> {
        Snapshot::decode(&self.request(&["snapshot"])?)
    }
    pub fn workspace(&self, name: &str) -> io::Result<Snapshot> {
        self.request(&[
            "new",
            &wire::hex(name.as_bytes()),
            &wire::hex(self.project.as_os_str().as_encoded_bytes()),
        ])?;
        self.snapshot()
    }
    pub fn input(&self, bytes: &[u8]) -> io::Result<()> {
        let s = self.snapshot()?;
        let t = s
            .session()
            .ok_or_else(|| io::Error::other("no documentation shell"))?;
        self.request(&["input", &t.id.to_string(), &t.run, &wire::hex(bytes)])?;
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.request(&["stop"]);
        let end = Instant::now() + Duration::from_secs(3);
        while matches!(self.server.try_wait(), Ok(None)) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(10));
        }
        if matches!(self.server.try_wait(), Ok(None)) {
            let _ = self.server.kill();
        }
        let _ = self.server.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}
pub struct View {
    pub master: File,
    pub child: os::Process,
    pub screen: Terminal,
    raw: Vec<u8>,
    pub framed: bool,
    pub bytes_read: u64,
}
impl View {
    pub fn new(f: &Fixture, cols: u16, rows: u16, framed: bool) -> io::Result<Self> {
        let mut command = if framed {
            let mut c = Command::new("/usr/bin/env");
            c.args(["-u", "FLERE"])
                .arg(&f.binary)
                .arg("--state")
                .arg(&f.state)
                .arg("attach");
            c
        } else {
            let mut c = Command::new("/bin/sh");
            c.arg("-i");
            c
        };
        Fixture::environment(&mut command, &f.root);
        let (master, child) = os::spawn_command_pty(&f.project, &mut command, cols, rows)?;
        let mut view = Self {
            master,
            child,
            screen: Terminal::new(cols as usize, rows as usize),
            raw: Vec::new(),
            framed,
            bytes_read: 0,
        };
        view.until(|s| s.capture(100).contains(if framed { "FLERE" } else { "$" }))?;
        view.pump(Duration::from_millis(250))?;
        Ok(view)
    }
    pub fn complete(&self) -> bool {
        if !self.framed {
            return true;
        }
        let start = self.raw.windows(8).rposition(|w| w == b"\x1b[?2026h");
        let end = self.raw.windows(8).rposition(|w| w == b"\x1b[?2026l");
        end.is_some() && end > start
    }
    pub fn read(&mut self, timeout: i32) -> io::Result<()> {
        let mut fds = [os::PollFd {
            fd: self.master.as_raw_fd(),
            events: os::READ,
            revents: 0,
        }];
        os::wait(&mut fds, timeout)?;
        for _ in 0..128 {
            let mut b = [0; 65536];
            match self.master.read(&mut b) {
                Ok(0) => break,
                Ok(n) => {
                    self.bytes_read += n as u64;
                    self.screen.feed(&b[..n]);
                    self.raw.extend_from_slice(&b[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        // Keep the last complete frame boundary and any incomplete following frame.
        if self.raw.len() > 1024 * 1024
            && let Some(end) = self.raw.windows(8).rposition(|w| w == b"\x1b[?2026l")
        {
            self.raw.drain(..end);
        }
        Ok(())
    }
    pub fn until(&mut self, condition: impl Fn(&Terminal) -> bool) -> io::Result<()> {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            self.read(10)?;
            if self.complete() && condition(&self.screen) {
                return Ok(());
            }
        }
        Err(io::Error::other(format!(
            "documentation UI condition timed out: {}",
            self.screen.capture(50)
        )))
    }
    pub fn pump(&mut self, duration: Duration) -> io::Result<()> {
        let end = Instant::now() + duration;
        while Instant::now() < end {
            self.read(10)?;
        }
        if self.framed {
            self.until(|_| true)?;
        }
        Ok(())
    }
    pub fn key(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut left = bytes;
        let end = Instant::now() + Duration::from_secs(3);
        while !left.is_empty() && Instant::now() < end {
            match self.master.write(left) {
                Ok(0) => return Err(io::Error::other("zero PTY write")),
                Ok(n) => left = &left[n..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => self.read(1)?,
                Err(e) => return Err(e),
            }
        }
        if !left.is_empty() {
            return Err(io::Error::other("PTY input did not drain"));
        }
        Ok(())
    }
    pub fn frame(&self, ms: u128, label: &str) -> serde_json::Value {
        let mut runs = Vec::new();
        for y in 0..self.screen.grid.rows {
            let row =
                &self.screen.grid.cells[y * self.screen.grid.cols..(y + 1) * self.screen.grid.cols];
            let mut x = 0;
            while x < row.len() {
                let begin = x;
                let style = row[x].style;
                let mut text = String::new();
                while x < row.len() && row[x].style == style {
                    if row[x].width > 0 {
                        text.push_str(&row[x].text);
                    }
                    x += 1;
                }
                runs.push((y, begin, x - begin, text, style));
            }
        }
        serde_json::json!({"ms":ms,"label":label,"cols":self.screen.grid.cols,"rows":self.screen.grid.rows,"runs":runs})
    }
}
impl Drop for View {
    fn drop(&mut self) {
        os::hangup(self.master.as_raw_fd(), &mut self.child);
        let end = Instant::now() + Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < end {
            let mut b = [0; 65536];
            let _ = self.master.read(&mut b);
            std::thread::sleep(Duration::from_millis(5));
        }
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
    }
}
