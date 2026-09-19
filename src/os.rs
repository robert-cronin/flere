//! Linux x86_64 and macOS arm64/x86_64 OS bindings. All unsafe OS interaction lives here.
//! FDs become owned Files immediately; pre_exec performs only async-signal-safe calls.
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    mem::MaybeUninit,
    os::{
        fd::{FromRawFd, RawFd},
        unix::{fs::OpenOptionsExt, process::CommandExt},
    },
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(not(any(
    all(target_os = "linux", target_arch = "x86_64"),
    all(
        target_os = "macos",
        any(target_arch = "aarch64", target_arch = "x86_64")
    ),
)))]
compile_error!("Flere supports Linux x86_64 and macOS arm64/x86_64; audit other OS ABIs first");

#[cfg(target_os = "linux")]
use libc::ptsname_r;
use libc::{
    cfmakeraw, fcntl, flock, getuid, grantpt, ioctl, kill, poll, posix_openpt, setsid, signal,
    tcgetattr, tcsetattr, unlockpt,
};
// Darwin exports this reentrant API since macOS 10.13.4; libc does not bind it yet.
// The caller owns the writable buffer and the function never retains it.
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn ptsname_r(fd: libc::c_int, buffer: *mut libc::c_char, len: libc::size_t) -> libc::c_int;
}
type Termios = libc::termios;
pub type PollFd = libc::pollfd;
pub const READ: i16 = libc::POLLIN;
pub const WRITE: i16 = libc::POLLOUT;
pub const HUP: i16 = libc::POLLHUP;
static STOP: AtomicBool = AtomicBool::new(false);
extern "C" fn stop_signal(_: i32) {
    STOP.store(true, Ordering::Relaxed);
}
pub fn signals() {
    // The signal handler only stores an atomic boolean on both supported Unix ABIs.
    unsafe {
        signal(libc::SIGHUP, stop_signal as *const () as usize);
        signal(libc::SIGTERM, stop_signal as *const () as usize);
        signal(libc::SIGINT, stop_signal as *const () as usize);
    }
}
pub fn stopping() -> bool {
    STOP.load(Ordering::Relaxed)
}
pub fn uid() -> u32 {
    unsafe { getuid() }
}
#[cfg(target_os = "linux")]
pub fn same_user(fd: RawFd) -> io::Result<bool> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    cvt(unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    })?;
    Ok(cred.uid == uid())
}
#[cfg(target_os = "macos")]
pub fn same_user(fd: RawFd) -> io::Result<bool> {
    let (mut peer_uid, mut peer_gid) = (0, 0);
    // getpeereid authenticates the connected Unix socket using kernel credentials.
    // Output pointers are caller-owned and remain valid for this synchronous call.
    cvt(unsafe { libc::getpeereid(fd, &mut peer_uid, &mut peer_gid) })?;
    Ok(peer_uid == uid())
}
fn cvt(n: i32) -> io::Result<i32> {
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n)
    }
}
pub fn nonblock(fd: RawFd) -> io::Result<()> {
    let flags = cvt(unsafe { fcntl(fd, libc::F_GETFL) })?;
    cvt(unsafe { fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) })?;
    Ok(())
}
pub fn wait(fds: &mut [PollFd], timeout: i32) -> io::Result<()> {
    let r = unsafe { poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
    if r < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
pub fn dimensions(fd: RawFd) -> (u16, u16) {
    let mut w = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { ioctl(fd, libc::TIOCGWINSZ, &mut w) } < 0 || w.ws_col == 0 || w.ws_row == 0 {
        (100, 30)
    } else {
        (w.ws_col, w.ws_row)
    }
}
/// Pixel size of an outer terminal cell, when the kernel exposes it.
pub fn cell_dimensions(fd: RawFd) -> Option<(usize, usize)> {
    let mut w: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { ioctl(fd, libc::TIOCGWINSZ, &mut w) } < 0 || w.ws_col == 0 || w.ws_row == 0 {
        return None;
    }
    let cell = (
        (w.ws_xpixel / w.ws_col) as usize,
        (w.ws_ypixel / w.ws_row) as usize,
    );
    crate::avatar::cell_bytes(cell).ok().map(|_| cell)
}

// zlib's C ABI uses unsigned long lengths on both supported Unix targets.
// Only owned, bounded byte buffers cross this synchronous boundary.
#[link(name = "z")]
unsafe extern "C" {
    fn compressBound(source_len: libc::c_ulong) -> libc::c_ulong;
    fn compress2(
        dest: *mut u8,
        dest_len: *mut libc::c_ulong,
        source: *const u8,
        source_len: libc::c_ulong,
        level: libc::c_int,
    ) -> libc::c_int;
}
pub fn compress_pixels(bytes: &[u8]) -> io::Result<Vec<u8>> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(io::Error::other("pixel buffer exceeds bound"));
    }
    let mut len = unsafe { compressBound(bytes.len() as libc::c_ulong) };
    let mut data = vec![0; len as usize];
    let status = unsafe {
        compress2(
            data.as_mut_ptr(),
            &mut len,
            bytes.as_ptr(),
            bytes.len() as libc::c_ulong,
            1,
        )
    };
    if status != 0 || len as usize > data.len() {
        return Err(io::Error::other("pixel compression failed"));
    }
    data.truncate(len as usize);
    Ok(data)
}

pub fn resize(fd: RawFd, cols: u16, rows: u16) -> io::Result<()> {
    let w = libc::winsize {
        ws_col: cols,
        ws_row: rows,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    cvt(unsafe { ioctl(fd, libc::TIOCSWINSZ, &w) })?;
    Ok(())
}
pub struct RawTerminal {
    fd: RawFd,
    original: Termios,
}
impl RawTerminal {
    pub fn enter(fd: RawFd) -> io::Result<Self> {
        let mut term = MaybeUninit::<Termios>::uninit();
        cvt(unsafe { tcgetattr(fd, term.as_mut_ptr()) })?;
        let original = unsafe { term.assume_init() };
        let mut raw = original;
        unsafe { cfmakeraw(&mut raw) };
        cvt(unsafe { tcsetattr(fd, libc::TCSANOW, &raw) })?;
        Ok(Self { fd, original })
    }
}
impl Drop for RawTerminal {
    fn drop(&mut self) {
        unsafe { tcsetattr(self.fd, libc::TCSANOW, &self.original) };
    }
}
pub fn detach(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            cvt(setsid())?;
            Ok(())
        });
    }
}
pub fn clean_environment(cmd: &mut Command) {
    for (key, _) in std::env::vars_os() {
        let k = key.to_string_lossy();
        if k.starts_with("SWITCHYARD_")
            || k.starts_with("ORCA_")
            || k.starts_with("FLERE_")
            || matches!(
                k.as_ref(),
                "CODEX_THREAD_ID" | "CODEX_SESSION_ID" | "CODEX_TURN_ID" | "CLAUDECODE"
            )
            || k == "TMUX"
            || k == "TMUX_PANE"
        {
            cmd.env_remove(key);
        }
    }
}
pub fn shell() -> String {
    if let Some(s) = std::env::var_os("SHELL") {
        let p = std::path::Path::new(&s);
        if p.is_absolute() && p.is_file() {
            return s.to_string_lossy().into_owned();
        }
    }
    if let Ok(path) = login_shell()
        && path.is_absolute()
        && path.is_file()
    {
        return path.to_string_lossy().into_owned();
    }
    "/bin/sh".into()
}
pub fn spawn_pty(
    cwd: &std::path::Path,
    shell: &str,
    cols: u16,
    rows: u16,
) -> io::Result<(File, Process)> {
    let mut cmd = Command::new(shell);
    cmd.arg("-i");
    spawn_command_pty(cwd, &mut cmd, cols, rows)
}
/// Spawn exactly this argv under a new controlling PTY (also used by interactive tests).
pub fn spawn_command_pty(
    cwd: &std::path::Path,
    cmd: &mut Command,
    cols: u16,
    rows: u16,
) -> io::Result<(File, Process)> {
    let _timing = crate::diagnostics::measure("pty-spawn");
    // O_RDWR | O_NOCTTY | O_CLOEXEC. The master is not the supervisor's controlling TTY.
    let fd = cvt(unsafe { posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) })?;
    let master = unsafe { File::from_raw_fd(fd) };
    cvt(unsafe { grantpt(fd) })?;
    cvt(unsafe { unlockpt(fd) })?;
    let mut name = [0u8; 256];
    let e = unsafe { ptsname_r(fd, name.as_mut_ptr().cast(), name.len()) };
    if e != 0 {
        // Linux returns an errno value; Darwin returns -1 and sets errno.
        return Err(if e < 0 {
            io::Error::last_os_error()
        } else {
            io::Error::from_raw_os_error(e)
        });
    }
    let end = name
        .iter()
        .position(|b| *b == 0)
        .ok_or_else(|| io::Error::other("PTY name too long"))?;
    let path = std::str::from_utf8(&name[..end]).map_err(io::Error::other)?;
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(path)?;
    resize(fd, cols, rows)?;
    nonblock(fd)?;
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| std::path::PathBuf::from(p).join(".cache")))
        .ok_or_else(|| {
            io::Error::other("HOME or XDG_CACHE_HOME is required for private temporary files")
        })?
        .join("flere/tmp");
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&cache)?;
    cmd.env("TMPDIR", &cache)
        .env("TMP", &cache)
        .env("TEMP", &cache);
    cmd.current_dir(cwd)
        .env("TERM", "xterm-256color")
        .env("COLORTERM", "truecolor")
        .env("FLERE", "1");
    clean_environment(cmd);
    cmd.stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    unsafe {
        cmd.pre_exec(|| {
            cvt(setsid())?;
            cvt(ioctl(0, libc::TIOCSCTTY as _, 0))?;
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok((master, Process::new(child)))
}
/// The launcher must belong to the foreground group of this owned PTY.
pub fn foreground_process(master: RawFd, pid: u32) -> io::Result<bool> {
    // POSIX process groups and tcgetpgrp use pid_t; no descriptors are adopted.
    let foreground = cvt(unsafe { libc::tcgetpgrp(master) })?;
    let group = cvt(unsafe { libc::getpgid(pid as libc::pid_t) })?;
    Ok(foreground > 0 && foreground == group)
}
pub fn hangup(master: RawFd, child: &mut Process) {
    // Target only the foreground group of this owned PTY, and its unreaped direct child.
    let mut group = 0i32;
    if unsafe { ioctl(master, libc::TIOCGPGRP, &mut group) } >= 0 && group > 0 {
        unsafe { kill(-group, libc::SIGHUP) };
    }
    if matches!(child.try_wait(), Ok(None)) {
        unsafe { kill(child.id() as i32, libc::SIGHUP) };
    }
}
/// The FIFO belongs to one private run directory; no descriptor is inherited.
pub(crate) fn private_fifo(path: &std::path::Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    // POSIX mkfifo takes a NUL-terminated path and mode_t on Linux/macOS.
    cvt(unsafe { libc::mkfifo(name.as_ptr(), 0o600) })?;
    Ok(())
}
pub fn nonce() -> io::Result<String> {
    let mut b = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|b| format!("{b:02x}")).collect())
}

pub fn lock(fd: RawFd) -> io::Result<()> {
    cvt(unsafe { flock(fd, libc::LOCK_EX | libc::LOCK_NB) })?;
    Ok(())
}

/// A native child receives Ctrl-C; the host survives to return to the user's shell.
/// Exec resets this caught handler for each external program; HUP/TERM retain defaults.
pub fn host_signals() {
    extern "C" fn interrupt(_: i32) {}
    unsafe {
        libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t);
    }
}

pub fn executable_path() -> io::Result<std::path::PathBuf> {
    let path = std::env::current_exe()?;
    let value = path.to_string_lossy();
    Ok(std::path::PathBuf::from(
        value.strip_suffix(" (deleted)").unwrap_or(&value),
    ))
}

/// Child ownership survives supervisor exec; unreaped child PIDs cannot be reused.
pub struct Process {
    child: Option<Child>,
    pid: u32,
    status: Option<ExitStatus>,
}
impl Process {
    fn new(child: Child) -> Self {
        let pid = child.id();
        Self {
            child: Some(child),
            pid,
            status: None,
        }
    }
    pub fn id(&self) -> u32 {
        self.pid
    }
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        if self.status.is_some() {
            return Ok(self.status);
        }
        if let Some(c) = &mut self.child {
            self.status = c.try_wait()?;
            return Ok(self.status);
        }
        let mut status = 0;
        let result = unsafe { libc::waitpid(self.pid as i32, &mut status, libc::WNOHANG) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if result > 0 {
            use std::os::unix::process::ExitStatusExt;
            self.status = Some(ExitStatus::from_raw(status));
        }
        Ok(self.status)
    }
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    pub fn kill(&mut self) -> io::Result<()> {
        if self.try_wait()?.is_none() {
            cvt(unsafe { libc::kill(self.pid as i32, libc::SIGKILL) })?;
        }
        Ok(())
    }
    pub fn raw_status(&self) -> Option<i32> {
        use std::os::unix::process::ExitStatusExt;
        self.status.map(|s| s.into_raw())
    }
    pub fn restore(pid: u32, status: Option<i32>, start: &str) -> io::Result<Self> {
        use std::os::unix::process::ExitStatusExt;
        if status.is_none() {
            let (parent, current) = child_identity(pid)?;
            if parent != std::process::id() || current != start {
                return Err(io::Error::other("refresh child identity changed"));
            }
        }
        Ok(Self {
            child: None,
            pid,
            status: status.map(ExitStatus::from_raw),
        })
    }
}
#[cfg(target_os = "linux")]
pub fn child_identity(pid: u32) -> io::Result<(u32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("invalid process stat"))?
        .1
        .split_whitespace()
        .collect();
    if fields.len() < 20 {
        return Err(io::Error::other("short process stat"));
    }
    Ok((
        fields[1].parse().map_err(io::Error::other)?,
        fields[19].into(),
    ))
}
pub fn inherit_fd(fd: RawFd, inherit: bool) -> io::Result<()> {
    let flags = cvt(unsafe { libc::fcntl(fd, libc::F_GETFD) })?;
    cvt(unsafe {
        libc::fcntl(
            fd,
            libc::F_SETFD,
            if inherit {
                flags & !libc::FD_CLOEXEC
            } else {
                flags | libc::FD_CLOEXEC
            },
        )
    })?;
    Ok(())
}
/// Adopt only descriptors preserved by our validated same-PID refresh image, each exactly once.
pub fn adopt_file(fd: RawFd) -> io::Result<File> {
    if fd < 3 {
        return Err(io::Error::other("invalid refresh descriptor"));
    }
    cvt(unsafe { libc::fcntl(fd, libc::F_GETFD) })?;
    inherit_fd(fd, false)?;
    Ok(unsafe { File::from_raw_fd(fd) })
}
pub fn adopt_listener(fd: RawFd) -> io::Result<std::os::unix::net::UnixListener> {
    use std::os::fd::IntoRawFd;
    let f = adopt_file(fd)?;
    Ok(unsafe { std::os::unix::net::UnixListener::from_raw_fd(f.into_raw_fd()) })
}
pub fn adopt_stream(fd: RawFd) -> io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::IntoRawFd;
    let f = adopt_file(fd)?;
    Ok(unsafe { std::os::unix::net::UnixStream::from_raw_fd(f.into_raw_fd()) })
}
pub fn parent_pid() -> u32 {
    unsafe { libc::getppid() as u32 }
}

/// Resolve the current account through the system directory service, not /etc/passwd.
fn login_shell() -> io::Result<std::path::PathBuf> {
    use std::{ffi::CStr, os::unix::ffi::OsStrExt};
    let mut buffer = vec![0u8; 65536];
    let mut entry = MaybeUninit::<libc::passwd>::uninit();
    let mut result = std::ptr::null_mut();
    // getpwuid_r initializes entry and points its fields into our owned buffer.
    let status = unsafe {
        libc::getpwuid_r(
            uid(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status));
    }
    if result.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "login account unavailable",
        ));
    }
    let entry = unsafe { entry.assume_init() };
    if entry.pw_shell.is_null() {
        return Err(io::Error::other("login shell unavailable"));
    }
    let bytes = unsafe { CStr::from_ptr(entry.pw_shell) }.to_bytes();
    Ok(std::ffi::OsStr::from_bytes(bytes).into())
}

#[cfg(target_os = "linux")]
fn bounded_process_file(path: impl AsRef<std::path::Path>, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("process metadata exceeds bound"));
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
pub fn process_children(pid: u32, limit: usize) -> io::Result<Vec<u32>> {
    let data = match bounded_process_file(format!("/proc/{pid}/task/{pid}/children"), 65536) {
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                && std::path::Path::new(&format!("/proc/{pid}")).exists() =>
        {
            // A missing main-thread record need not mean the process has exited.
            // Preserve fail-closed delivery when the descendant proof is incomplete.
            return Err(io::Error::other("native child list unavailable"));
        }
        result => result?,
    };
    let children: Vec<u32> = std::str::from_utf8(&data)
        .map_err(io::Error::other)?
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(io::Error::other)?;
    if children.len() > limit {
        return Err(io::Error::other("native descendant proof exceeds bound"));
    }
    Ok(children)
}
#[cfg(target_os = "linux")]
pub fn process_executable(pid: u32) -> io::Result<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
}
#[cfg(target_os = "linux")]
pub fn process_cwd(pid: u32) -> io::Result<std::path::PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/cwd"))
}
#[cfg(target_os = "linux")]
pub fn process_arguments(pid: u32) -> io::Result<Vec<u8>> {
    bounded_process_file(format!("/proc/{pid}/cmdline"), 1024 * 1024)
}
#[cfg(target_os = "linux")]
pub fn process_environment(pid: u32) -> io::Result<Vec<u8>> {
    bounded_process_file(format!("/proc/{pid}/environ"), 1024 * 1024)
}

/// A reference to an observed open descriptor, never an arbitrary transcript search.
pub struct ProcessFile {
    pub path: std::path::PathBuf,
    pid: u32,
    fd: i32,
    #[cfg(target_os = "macos")]
    identity: (u64, u64),
}
#[cfg(target_os = "linux")]
pub fn process_files(pid: u32, limit: usize) -> io::Result<Vec<ProcessFile>> {
    let mut files = Vec::new();
    for (index, entry) in std::fs::read_dir(format!("/proc/{pid}/fd"))?.enumerate() {
        if index >= limit {
            return Err(io::Error::other("native descriptor proof exceeds bound"));
        }
        let entry = entry?;
        let Ok(path) = std::fs::read_link(entry.path()) else {
            continue;
        };
        let fd = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::other("invalid process descriptor"))?;
        files.push(ProcessFile { path, pid, fd });
    }
    Ok(files)
}
impl ProcessFile {
    #[cfg(target_os = "linux")]
    pub fn open(&self) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(format!("/proc/{}/fd/{}", self.pid, self.fd))
    }
    #[cfg(target_os = "macos")]
    pub fn open(&self) -> io::Result<File> {
        use std::os::unix::fs::MetadataExt;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)?;
        let metadata = file.metadata()?;
        let current = macos::vnode_file(self.pid, self.fd)?;
        // Darwin cannot duplicate another process's FD. Match the opened inode to
        // the kernel descriptor before returning it; reject rename/replacement races.
        if !metadata.is_file()
            || (metadata.dev(), metadata.ino()) != self.identity
            || current.identity != self.identity
            || current.path != self.path
        {
            return Err(io::Error::other(
                "native descriptor changed during inspection",
            ));
        }
        Ok(file)
    }
}

#[cfg(target_os = "macos")]
pub use macos::{
    child_identity, process_arguments, process_children, process_cwd, process_environment,
    process_executable, process_files,
};

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt, path::PathBuf};

    // These two repr(C) layouts match sys/proc_info.h on Darwin arm64/x86_64.
    // libc supplies the nested vnode layout; all buffers are owned by this caller.
    #[repr(C)]
    struct ProcFileInfo {
        flags: u32,
        status: u32,
        offset: i64,
        kind: i32,
        guard: u32,
    }
    #[repr(C)]
    struct VnodeFdInfo {
        file: ProcFileInfo,
        vnode: libc::vnode_info_path,
    }

    fn pid_value(pid: u32) -> io::Result<i32> {
        if pid == 0 || pid > i32::MAX as u32 {
            return Err(io::Error::other("invalid process ID"));
        }
        Ok(pid as i32)
    }
    fn bsd_info(pid: u32) -> io::Result<libc::proc_bsdinfo> {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        // proc_pidinfo writes this concrete SDK layout; partial results are never read.
        let n = unsafe {
            libc::proc_pidinfo(
                pid_value(pid)?,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if n != size {
            return Err(proc_error(n));
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_pid != pid || info.pbi_uid != uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "process ownership differs",
            ));
        }
        Ok(info)
    }
    fn proc_error(n: i32) -> io::Error {
        if n <= 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::ESRCH) {
                return io::Error::new(io::ErrorKind::NotFound, "process exited");
            }
            if e.raw_os_error().is_some_and(|n| n != 0) {
                return e;
            }
        }
        io::Error::other("incomplete process information")
    }
    fn path_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
        let end = bytes
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| io::Error::other("unterminated process path"))?;
        let path = std::path::Path::new(OsStr::from_bytes(&bytes[..end]));
        if !path.is_absolute() {
            return Err(io::Error::other("invalid process path"));
        }
        Ok(path.into())
    }
    fn vnode_path(info: &libc::vnode_info_path) -> io::Result<PathBuf> {
        let bytes: Vec<u8> = info.vip_path.iter().flatten().map(|b| *b as u8).collect();
        path_bytes(&bytes)
    }
    pub fn child_identity(pid: u32) -> io::Result<(u32, String)> {
        let info = bsd_info(pid)?;
        Ok((
            info.pbi_ppid,
            format!("{}:{}", info.pbi_start_tvsec, info.pbi_start_tvusec),
        ))
    }
    pub fn process_children(pid: u32, limit: usize) -> io::Result<Vec<u32>> {
        let before = child_identity(pid)?;
        let mut pids = vec![0i32; limit.min(128) + 1];
        // The kernel filters by the exact parent; this does not scan all user processes.
        // libproc returns zero for both an empty list and a syscall failure.
        // Reset this thread's errno so a failed uniqueness proof is never empty-success.
        unsafe {
            *libc::__error() = 0;
        }
        let n = unsafe {
            libc::proc_listchildpids(
                pid_value(pid)?,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(pids.as_slice()) as i32,
            )
        };
        if n < 0
            || (n == 0
                && io::Error::last_os_error()
                    .raw_os_error()
                    .is_some_and(|e| e != 0))
        {
            return Err(proc_error(n));
        }
        if n as usize >= pids.len() {
            return Err(io::Error::other("native descendant proof exceeds bound"));
        }
        pids.truncate(n as usize);
        if child_identity(pid)? != before {
            return Err(io::Error::other("native parent changed"));
        }
        let mut children = Vec::new();
        for child in pids.into_iter().filter(|p| *p > 0) {
            match child_identity(child as u32) {
                Ok((parent, _)) if parent == pid => children.push(child as u32),
                Ok(_) => return Err(io::Error::other("native descendant changed parent")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        Ok(children)
    }
    pub fn process_executable(pid: u32) -> io::Result<PathBuf> {
        bsd_info(pid)?;
        let mut bytes = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let n = unsafe {
            libc::proc_pidpath(
                pid_value(pid)?,
                bytes.as_mut_ptr().cast(),
                bytes.len() as u32,
            )
        };
        if n <= 0 {
            return Err(proc_error(n));
        }
        path_bytes(&bytes)
    }
    pub fn process_cwd(pid: u32) -> io::Result<PathBuf> {
        bsd_info(pid)?;
        let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::uninit();
        let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as i32;
        let n = unsafe {
            libc::proc_pidinfo(
                pid_value(pid)?,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if n != size {
            return Err(proc_error(n));
        }
        vnode_path(&unsafe { info.assume_init() }.pvi_cdir)
    }
    pub(super) fn vnode_file(pid: u32, fd: i32) -> io::Result<ProcessFile> {
        let mut info = MaybeUninit::<VnodeFdInfo>::uninit();
        let size = std::mem::size_of::<VnodeFdInfo>() as i32;
        // PROC_PIDFDVNODEPATHINFO = 2 in the Darwin SDK. Exact-size check protects the ABI.
        let n =
            unsafe { libc::proc_pidfdinfo(pid_value(pid)?, fd, 2, info.as_mut_ptr().cast(), size) };
        if n != size {
            return Err(proc_error(n));
        }
        let info = unsafe { info.assume_init() };
        let path = vnode_path(&info.vnode)?;
        let stat = info.vnode.vip_vi.vi_stat;
        Ok(ProcessFile {
            path,
            pid,
            fd,
            identity: (u64::from(stat.vst_dev), stat.vst_ino),
        })
    }
    pub fn process_files(pid: u32, limit: usize) -> io::Result<Vec<ProcessFile>> {
        bsd_info(pid)?;
        let mut fds = vec![
            libc::proc_fdinfo {
                proc_fd: 0,
                proc_fdtype: 0
            };
            limit.min(256) + 1
        ];
        let size = std::mem::size_of_val(fds.as_slice()) as i32;
        let n = unsafe {
            libc::proc_pidinfo(
                pid_value(pid)?,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                size,
            )
        };
        if n <= 0 {
            return Err(proc_error(n));
        }
        if n >= size || !(n as usize).is_multiple_of(std::mem::size_of::<libc::proc_fdinfo>()) {
            return Err(io::Error::other("native descriptor proof exceeds bound"));
        }
        fds.truncate(n as usize / std::mem::size_of::<libc::proc_fdinfo>());
        let mut files = Vec::new();
        for fd in fds
            .into_iter()
            .filter(|f| f.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
        {
            match vnode_file(pid, fd.proc_fd) {
                Ok(file) => files.push(file),
                Err(e) if matches!(e.raw_os_error(), Some(libc::EBADF | libc::ENOENT)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(files)
    }
    struct ProcArgs {
        args: Vec<u8>,
        env: Vec<u8>,
    }
    fn arguments(pid: u32) -> io::Result<ProcArgs> {
        let before = child_identity(pid)?;
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid_value(pid)?];
        let mut bytes = vec![0u8; 1024 * 1024];
        let mut size = bytes.len();
        // Reads only the requested owned process. The bounded buffer is temporary;
        // callers retain only explicit config-location/session fields, never credentials.
        cvt(unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                bytes.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        })?;
        bytes.truncate(size);
        if child_identity(pid)? != before {
            return Err(io::Error::other("native process changed"));
        }
        parse_arguments(&bytes)
    }
    fn parse_arguments(bytes: &[u8]) -> io::Result<ProcArgs> {
        let count = i32::from_ne_bytes(
            bytes
                .get(..4)
                .ok_or_else(|| io::Error::other("short process arguments"))?
                .try_into()
                .unwrap(),
        );
        if count < 0 || count as usize > bytes.len() {
            return Err(io::Error::other("invalid process argument count"));
        }
        let mut offset = 4;
        fn string_end(bytes: &[u8], start: usize) -> io::Result<usize> {
            bytes
                .get(start..)
                .and_then(|s| s.iter().position(|b| *b == 0))
                .map(|n| start + n)
                .ok_or_else(|| io::Error::other("unterminated process arguments"))
        }
        offset = string_end(bytes, offset)? + 1; // executable path, then padding
        while bytes.get(offset) == Some(&0) {
            offset += 1;
        }
        let start = offset;
        for _ in 0..count {
            offset = string_end(bytes, offset)? + 1;
        }
        Ok(ProcArgs {
            args: bytes[start..offset].to_vec(),
            env: bytes[offset..].to_vec(),
        })
    }
    pub fn process_arguments(pid: u32) -> io::Result<Vec<u8>> {
        Ok(arguments(pid)?.args)
    }
    pub fn process_environment(pid: u32) -> io::Result<Vec<u8>> {
        Ok(arguments(pid)?.env)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn procargs_preserves_empty_arguments_and_environment_boundaries() {
            let mut bytes = 3i32.to_ne_bytes().to_vec();
            bytes.extend_from_slice(
                b"/owned/codex\0\0\0codex\0\0argument with spaces\0HOME=/owned/home\0VALUE=a=b\0\0",
            );
            let parsed = parse_arguments(&bytes).unwrap();
            assert_eq!(parsed.args, b"codex\0\0argument with spaces\0");
            assert_eq!(parsed.env, b"HOME=/owned/home\0VALUE=a=b\0\0");
        }
        #[test]
        fn procargs_rejects_truncated_and_invalid_kernel_records() {
            for bytes in [vec![], vec![1, 0], (-1i32).to_ne_bytes().to_vec(), {
                let mut bytes = 2i32.to_ne_bytes().to_vec();
                bytes.extend_from_slice(b"/owned/codex\0codex\0unterminated");
                bytes
            }] {
                assert!(parse_arguments(&bytes).is_err());
            }
        }
    }
}

/// Decode bounded PNG/JPEG data using macOS ImageIO on an image worker.
/// CG/CF create-rule objects are released on every return; no callbacks or retained
/// Rust pointers. Darwin CGFloat is f64 on the supported 64-bit architectures.
#[cfg(target_os = "macos")]
pub fn image_pixels(bytes: &[u8]) -> io::Result<(usize, usize, Vec<u8>)> {
    use std::ffi::c_void;
    type Ref = *const c_void;
    #[repr(C)]
    struct Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
        fn CFRelease(value: Ref);
    }
    #[link(name = "ImageIO", kind = "framework")]
    unsafe extern "C" {
        fn CGImageSourceCreateWithData(data: Ref, options: Ref) -> Ref;
        fn CGImageSourceCreateImageAtIndex(source: Ref, index: usize, options: Ref) -> Ref;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGImageGetWidth(image: Ref) -> usize;
        fn CGImageGetHeight(image: Ref) -> usize;
        fn CGColorSpaceCreateDeviceRGB() -> Ref;
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits: usize,
            stride: usize,
            space: Ref,
            bitmap_info: u32,
        ) -> Ref;
        fn CGContextSetInterpolationQuality(context: Ref, quality: i32);
        fn CGContextDrawImage(context: Ref, rect: Rect, image: Ref);
    }
    struct Owned(Ref);
    impl Owned {
        fn new(value: Ref) -> io::Result<Self> {
            if value.is_null() {
                Err(io::Error::other("ImageIO could not decode image"))
            } else {
                Ok(Self(value))
            }
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) };
        }
    }
    crate::image_preview::dimensions(bytes)?;
    let null = std::ptr::null();
    unsafe {
        let data = Owned::new(CFDataCreate(null, bytes.as_ptr(), bytes.len() as isize))?;
        let source = Owned::new(CGImageSourceCreateWithData(data.0, null))?;
        let image = Owned::new(CGImageSourceCreateImageAtIndex(source.0, 0, null))?;
        let (w, h) = (CGImageGetWidth(image.0), CGImageGetHeight(image.0));
        if w == 0 || h == 0 || w.checked_mul(h).is_none_or(|n| n > 20_000_000) {
            return Err(io::Error::other("decoded image dimensions exceed bound"));
        }
        let scale = (1600.0 / w as f64).min(1200.0 / h as f64).min(1.0);
        let (width, height) = (
            (w as f64 * scale).round().max(1.0) as usize,
            (h as f64 * scale).round().max(1.0) as usize,
        );
        let mut rgba = vec![0; width * height * 4];
        let space = Owned::new(CGColorSpaceCreateDeviceRGB())?;
        // Big-endian 32-bit byte order + premultiplied-last = R,G,B,A bytes.
        let context = Owned::new(CGBitmapContextCreate(
            rgba.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            space.0,
            0x4001,
        ))?;
        CGContextSetInterpolationQuality(context.0, 3);
        CGContextDrawImage(
            context.0,
            Rect {
                x: 0.0,
                y: 0.0,
                width: width as f64,
                height: height as f64,
            },
            image.0,
        );
        // Kitty expects straight alpha. Never divide by a transparent pixel.
        for pixel in rgba.as_chunks_mut::<4>().0 {
            let alpha = pixel[3] as u32;
            if alpha > 0 && alpha < 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
        Ok((width, height, rgba))
    }
}

pub(crate) fn copy_png(bytes: &[u8], path: &std::path::Path) -> io::Result<()> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() > 20 * 1024 * 1024 {
        return Err(io::Error::other(
            "clipboard requires a PNG no larger than 20 MiB",
        ));
    }
    crate::image_preview::dimensions(bytes)?;
    let text = path
        .to_str()
        .filter(|text| path.is_absolute() && !text.chars().any(char::is_control))
        .ok_or_else(|| io::Error::other("screenshot path must be absolute printable UTF-8"))?;
    #[cfg(target_os = "macos")]
    {
        clipboard_image::copy(bytes, text)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = text;
        crate::clipboard_owner::publish(path)
    }
}

#[cfg(target_os = "macos")]
mod clipboard_image {
    use super::*;
    use std::{ffi::c_void, ptr};
    type Ref = *const c_void;
    // macOS arm64/x86_64: CFIndex is signed pointer-sized and OSStatus is Int32.
    // Create/Copy references are owned and released; CFData copies borrowed bytes.
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: Ref);
        fn CFStringCreateWithBytes(
            allocator: Ref,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> Ref;
        fn CFDataCreate(allocator: Ref, bytes: *const u8, len: isize) -> Ref;
        #[cfg(test)]
        fn CFDataGetLength(data: Ref) -> isize;
        #[cfg(test)]
        fn CFDataGetBytePtr(data: Ref) -> *const u8;
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn PasteboardCreate(name: Ref, out: *mut Ref) -> i32;
        fn PasteboardClear(board: Ref) -> i32;
        fn PasteboardPutItemFlavor(
            board: Ref,
            item: *mut c_void,
            kind: Ref,
            data: Ref,
            flags: u32,
        ) -> i32;
        #[cfg(test)]
        fn PasteboardCopyItemFlavorData(
            board: Ref,
            item: *mut c_void,
            kind: Ref,
            data: *mut Ref,
        ) -> i32;
    }
    struct Owned(Ref);
    impl Owned {
        fn new(value: Ref) -> io::Result<Self> {
            if value.is_null() {
                Err(io::Error::other("native image operation failed"))
            } else {
                Ok(Self(value))
            }
        }
        fn string(text: &str) -> io::Result<Self> {
            // CF copies the UTF-8 bytes; the Rust string need not outlive the result.
            Self::new(unsafe {
                CFStringCreateWithBytes(
                    ptr::null(),
                    text.as_ptr(),
                    text.len() as isize,
                    0x08000100,
                    0,
                )
            })
        }
        #[cfg(test)]
        fn bytes(&self) -> io::Result<Vec<u8>> {
            let len = unsafe { CFDataGetLength(self.0) };
            if len <= 0 || len > 20 * 1024 * 1024 {
                return Err(io::Error::other("encoded screenshot exceeds bound"));
            }
            let bytes = unsafe { CFDataGetBytePtr(self.0) };
            if bytes.is_null() {
                return Err(io::Error::other("image data is unavailable"));
            }
            Ok(unsafe { std::slice::from_raw_parts(bytes, len as usize) }.to_vec())
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe {
                CFRelease(self.0);
            }
        }
    }
    fn board(name: Ref) -> io::Result<Owned> {
        let mut out = ptr::null();
        status(unsafe { PasteboardCreate(name, &mut out) })?;
        Owned::new(out)
    }
    fn status(code: i32) -> io::Result<()> {
        if code == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "clipboard operation failed ({code})"
            )))
        }
    }
    fn put(board: &Owned, bytes: &[u8], path: &str) -> io::Result<()> {
        let kind = Owned::string("public.png")?;
        let data =
            Owned::new(unsafe { CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize) })?;
        let text_kind = Owned::string("public.utf8-plain-text")?;
        let text_data =
            Owned::new(unsafe { CFDataCreate(ptr::null(), path.as_ptr(), path.len() as isize) })?;
        // Item 1 is an opaque pasteboard identity, never dereferenced as memory.
        // Both representations are actual data, not promises. The system owns
        // its copies after the local CF references are released. Image-aware
        // readers get PNG; text-only terminal paste gets a retained local path.
        status(unsafe { PasteboardClear(board.0) })?;
        status(unsafe {
            PasteboardPutItemFlavor(board.0, ptr::without_provenance_mut(1), kind.0, data.0, 0)
        })?;
        status(unsafe {
            PasteboardPutItemFlavor(
                board.0,
                ptr::without_provenance_mut(1),
                text_kind.0,
                text_data.0,
                0,
            )
        })
    }
    pub(super) fn copy(bytes: &[u8], path: &str) -> io::Result<()> {
        let name = Owned::string("com.apple.pasteboard.clipboard")?;
        put(&board(name.0)?, bytes, path)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn screenshot_private_pasteboard_supports_native_image_and_terminal_text_readers() {
            let png = include_bytes!("../tests/fixtures/local-image.png").to_vec();
            let (width, height, _) = crate::os::image_pixels(&png).unwrap();
            let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap())
                .join(".cache/flere/tests")
                .join(crate::os::nonce().unwrap());
            crate::server::private_state(&root).unwrap();
            let path = root.join("buffer's screenshot café.png");
            crate::workspace::atomic_write(&path, &png).unwrap();
            let board_name = format!("app.flere.tests.{}", crate::os::nonce().unwrap());
            let name = Owned::string(&board_name).unwrap();
            let pasteboard = board(name.0).unwrap();
            put(&pasteboard, &png, path.to_str().unwrap()).unwrap();
            let second = board(name.0).unwrap();
            drop(pasteboard); // The writing reference is no longer needed for paste.
            let kind = Owned::string("public.png").unwrap();
            let mut read = ptr::null();
            status(unsafe {
                PasteboardCopyItemFlavorData(
                    second.0,
                    ptr::without_provenance_mut(1),
                    kind.0,
                    &mut read,
                )
            })
            .unwrap();
            assert_eq!(Owned::new(read).unwrap().bytes().unwrap(), png);
            // Read through AppKit in a separate process, like a terminal or
            // native image app would. A same-API PNG round trip alone missed
            // the absence of text that made ordinary terminal paste do nothing.
            let output = std::process::Command::new("/usr/bin/osascript")
                .args([
                    "-l",
                    "JavaScript",
                    "-e",
                    r#"
function run(argv) {
    ObjC.import('AppKit');
    const board = $.NSPasteboard.pasteboardWithName($(argv[0]));
    const png = board.dataForType($('public.png'));
    const bitmap = $.NSBitmapImageRep.alloc.initWithData(png);
    const image = $.NSImage.alloc.initWithPasteboard(board);
    return JSON.stringify({
        text: ObjC.unwrap(board.stringForType($.NSPasteboardTypeString)),
        samePNG: Boolean(png.isEqualToData($.NSData.dataWithContentsOfFile($(argv[1])))),
        valid: Boolean(image.isValid),
        width: Number(bitmap.pixelsWide), height: Number(bitmap.pixelsHigh)
    });
}
"#,
                ])
                .arg(&board_name)
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let read: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(
                read,
                serde_json::json!({
                    "text": path.to_str().unwrap(), "samePNG": true,
                    "valid": true, "width": width, "height": height
                })
            );
            std::fs::remove_dir_all(root).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;
    #[test]
    fn socket_credentials_accept_owned_peers_and_reject_non_sockets() {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert!(same_user(a.as_raw_fd()).unwrap());
        assert!(same_user(b.as_raw_fd()).unwrap());
        assert!(same_user(File::open("/dev/null").unwrap().as_raw_fd()).is_err());
    }
}
