//! Explicit, bounded GitHub reads. Hover never calls request().
use super::*;
use std::{
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Link {
    pub url: String,
    pub kind: &'static str,
    pub number: u64,
}
impl Link {
    pub fn parse(value: &str) -> Option<Self> {
        if value.len() > 2048 || value.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        let path = value.strip_prefix("https://github.com/")?;
        let path = path.split(['?', '#']).next()?;
        let parts: Vec<_> = path.split('/').collect();
        if parts.len() != 4
            || !parts[..2].iter().all(|s| {
                !s.is_empty()
                    && *s != "."
                    && *s != ".."
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
        {
            return None;
        }
        let kind = match parts[2] {
            "issues" => "issue",
            "pull" => "pr",
            _ => return None,
        };
        if !parts[3].bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let number = parts[3].parse::<u64>().ok().filter(|n| *n > 0)?;
        Some(Self {
            url: format!(
                "https://github.com/{}/{}/{}/{number}",
                parts[0], parts[1], parts[2]
            ),
            kind,
            number,
        })
    }
}
#[derive(Clone)]
struct Entry {
    url: String,
    lines: Vec<String>,
    at: Instant,
    ok: bool,
}
struct Job {
    rx: mpsc::Receiver<Vec<Entry>>,
    generation: u64,
}
#[derive(Default)]
pub(super) struct Github {
    cache: Vec<Entry>,
    job: Option<Job>,
    pending: Vec<Link>,
    serial: Arc<AtomicU64>,
}
impl Github {
    pub fn cancel(&mut self) {
        self.serial.fetch_add(1, Ordering::Relaxed);
        self.pending.clear();
    }
    pub fn request(&mut self, links: Vec<Link>, force: bool) {
        self.cancel();
        self.pending = links
            .into_iter()
            .filter(|link| {
                force
                    || !self.cache.iter().any(|e| {
                        e.url == link.url
                            && e.at.elapsed() < Duration::from_secs(if e.ok { 60 } else { 10 })
                    })
            })
            .take(2)
            .collect();
        self.start();
    }
    fn start(&mut self) {
        if self.job.is_some() || self.pending.is_empty() {
            return;
        }
        let links = std::mem::take(&mut self.pending);
        let serial = self.serial.clone();
        let generation = serial.load(Ordering::Relaxed);
        let (tx, rx) = mpsc::sync_channel(1);
        self.job = Some(Job { rx, generation });
        std::thread::spawn(move || {
            let mut entries = Vec::new();
            for link in links {
                if serial.load(Ordering::Relaxed) != generation {
                    break;
                }
                let value = fetch(&link, &serial, generation);
                let ok = value.is_ok();
                entries.push(Entry {
                    url: link.url,
                    lines: value.unwrap_or_else(|e| vec![e]),
                    at: Instant::now(),
                    ok,
                });
            }
            let _ = tx.send(entries);
        });
    }
    pub fn tick(&mut self) -> bool {
        let Some(job) = &self.job else {
            return false;
        };
        let result = match job.rx.try_recv() {
            Ok(v) => v,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(_) => Vec::new(),
        };
        if job.generation == self.serial.load(Ordering::Relaxed) {
            for entry in result {
                self.cache.retain(|e| e.url != entry.url);
                self.cache.push(entry);
                if self.cache.len() > 64 {
                    self.cache.remove(0);
                }
            }
        }
        self.job = None;
        self.start();
        true
    }
    pub fn lines(&self, link: &Link, focused: bool) -> Vec<String> {
        if let Some(e) = self.cache.iter().find(|e| e.url == link.url) {
            let mut lines = e.lines.clone();
            if e.at.elapsed() >= Duration::from_secs(60) {
                lines.push("Cached · r refresh".into());
            }
            return lines;
        }
        vec![
            if focused && (self.job.is_some() || !self.pending.is_empty()) {
                "Loading GitHub details…"
            } else {
                "Not fetched · open details to load"
            }
            .into(),
        ]
    }
}
impl Drop for Github {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn command(link: &Link) -> Command {
    let mut c = Command::new("gh");
    c.args([
        link.kind,
        "view",
        &link.url,
        "--json",
        if link.kind == "pr" {
            "title,state,isDraft,headRefName,baseRefName,reviewDecision,statusCheckRollup"
        } else {
            "title,state"
        },
    ])
    .env("GH_PROMPT_DISABLED", "1")
    .env("GH_PAGER", "cat")
    .env("NO_COLOR", "1")
    .env_remove("GH_DEBUG")
    .env_remove("GH_FORCE_TTY");
    c
}
fn fetch(link: &Link, serial: &AtomicU64, generation: u64) -> Result<Vec<String>, String> {
    let data = run(
        &mut command(link),
        serial,
        generation,
        Duration::from_secs(5),
    )?;
    decode(&data, link.kind)
}
fn short(v: &serde_json::Value, key: &str) -> String {
    wire::passive(v[key].as_str().unwrap_or(""))
        .chars()
        .take(2048)
        .collect()
}
fn decode(data: &[u8], kind: &str) -> Result<Vec<String>, String> {
    let v: serde_json::Value =
        serde_json::from_slice(data).map_err(|_| "GitHub returned invalid metadata")?;
    let title = short(&v, "title");
    if title.is_empty() {
        return Err("GitHub returned no title".into());
    }
    let mut lines = vec![
        title,
        format!(
            "State: {}{}",
            match v["state"].as_str() {
                Some("OPEN") => "Open",
                Some("CLOSED") => "Closed",
                Some("MERGED") => "Merged",
                _ => "Unknown",
            },
            if v["isDraft"].as_bool() == Some(true) {
                " · draft"
            } else {
                ""
            }
        ),
    ];
    if kind == "pr" {
        let base = short(&v, "baseRefName");
        let head = short(&v, "headRefName");
        if !base.is_empty() {
            lines.push(format!("{head} → {base}"));
        }
        let checks = v["statusCheckRollup"].as_array();
        let mut failed = 0;
        let mut pending = 0;
        let mut passed = 0;
        let mut unknown = 0;
        for check in checks.into_iter().flatten() {
            match check["conclusion"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| check["state"].as_str())
            {
                Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => passed += 1,
                Some(
                    "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED"
                    | "STARTUP_FAILURE" | "STALE",
                ) => failed += 1,
                Some("PENDING" | "EXPECTED") => pending += 1,
                _ if matches!(
                    check["status"].as_str(),
                    Some("IN_PROGRESS" | "QUEUED" | "WAITING" | "REQUESTED" | "PENDING")
                ) =>
                {
                    pending += 1
                }
                _ => unknown += 1,
            }
        }
        lines.push(if passed + failed + pending + unknown == 0 {
            "No checks reported".into()
        } else {
            format!(
                "Checks: {passed} passed · {failed} failed · {pending} pending · {unknown} unknown"
            )
        });
        lines.push(format!(
            "Review: {}",
            match v["reviewDecision"].as_str() {
                Some("APPROVED") => "approved",
                Some("CHANGES_REQUESTED") => "changes requested",
                Some("REVIEW_REQUIRED") => "required",
                _ => "no decision reported",
            }
        ));
    }
    Ok(lines)
}
fn run(
    c: &mut Command,
    serial: &AtomicU64,
    generation: u64,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut child = c
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "Install gh to load GitHub details"
            } else {
                "Unable to start gh"
            }
        })?;
    let result = (|| {
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        os::nonblock(stdout.as_raw_fd()).map_err(|_| "Unable to read gh output")?;
        os::nonblock(stderr.as_raw_fd()).map_err(|_| "Unable to read gh output")?;
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut eof = [false; 2];
        let mut status = None;
        let until = Instant::now() + timeout;
        loop {
            if serial.load(Ordering::Relaxed) != generation {
                return Err("GitHub request cancelled".into());
            }
            if Instant::now() >= until {
                return Err("GitHub timed out · r retry".into());
            }
            for (i, (reader, bytes)) in [
                (&mut stdout as &mut dyn Read, &mut out),
                (&mut stderr as &mut dyn Read, &mut err),
            ]
            .into_iter()
            .enumerate()
            {
                if eof[i] {
                    continue;
                }
                let mut buf = [0; 8192];
                for _ in 0..8 {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            eof[i] = true;
                            break;
                        }
                        Ok(n) => {
                            if bytes.len() + n > 256 * 1024 {
                                return Err("GitHub response too large".into());
                            }
                            bytes.extend_from_slice(&buf[..n]);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(_) => return Err("Unable to read GitHub response".into()),
                    }
                }
            }
            if status.is_none() {
                status = child.try_wait().map_err(|_| "Unable to wait for gh")?;
            }
            if let Some(status) = status
                && eof.iter().all(|v| *v)
            {
                if status.success() {
                    return Ok(out);
                }
                let diagnostic = String::from_utf8_lossy(&err).to_lowercase();
                return Err(
                    if diagnostic.contains("auth login") || diagnostic.contains("401") {
                        "GitHub sign-in needed · gh auth login"
                    } else if diagnostic.contains("rate limit") {
                        "GitHub rate limit reached · retry later"
                    } else {
                        "GitHub unavailable · links still work · r retry"
                    }
                    .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}
pub(super) fn opener(url: &str) -> Option<Command> {
    let link = Link::parse(url)?;
    #[cfg(target_os = "macos")]
    let mut c = {
        let mut c = Command::new("/usr/bin/open");
        c.arg("-u");
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut c = Command::new("xdg-open");
    c.arg(link.url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Some(c)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn github_targets_are_canonical_exact_and_never_shell_code() {
        for url in [
            "http://github.com/a/b/issues/1",
            "https://evil.test/a/b/pull/1",
            "https://github.com.evil.test/a/b/pull/1",
            "https://github.com@evil.test/a/b/pull/1",
            "https://github.com/a/../pull/1",
            "https://github.com/a/b/pull/-1",
            "https://github.com/a/b/pull/0",
            "https://github.com/a/b/pull/1/extra",
            "https://github.com/a/b/pull/%31",
            "https://github.com/a/b/pull/1\n",
            "https://github.com/a/b/pull/$(touch_owned)",
        ] {
            assert!(Link::parse(url).is_none(), "{url}");
            assert!(opener(url).is_none());
        }
        let link = Link::parse("https://github.com/owner/repo/pull/193?tab=files#note").unwrap();
        assert_eq!(link.url, "https://github.com/owner/repo/pull/193");
        let c = command(&link);
        let args: Vec<_> = c.get_args().map(|s| s.to_str().unwrap()).collect();
        assert_eq!(&args[..4], ["pr", "view", link.url.as_str(), "--json"]);
        assert!(!args.contains(&"--web"));
        let open = opener(&link.url).unwrap();
        assert_eq!(open.get_args().last().unwrap(), link.url.as_str());
        #[cfg(target_os = "macos")]
        assert_eq!(open.get_program(), "/usr/bin/open");
        #[cfg(target_os = "linux")]
        assert_eq!(open.get_program(), "xdg-open");
    }
    #[test]
    fn github_check_states_are_reported_without_guessing_or_escape_replay() {
        let body = br#"{"title":"Demo\u001b]52;bad\u0007 title","state":"OPEN","isDraft":true,"headRefName":"feat","baseRefName":"main","reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[{"conclusion":"SUCCESS"},{"state":"PENDING"},{"conclusion":"FAILURE"},{"status":"IN_PROGRESS"},{"status":"COMPLETED"}]}"#;
        let lines = decode(body, "pr").unwrap().join("\n");
        assert!(!lines.contains('\x1b') && !lines.contains('\x07'));
        assert!(lines.contains("1 passed · 1 failed · 2 pending · 1 unknown"));
        assert!(lines.contains("draft") && lines.contains("changes requested"));
        assert!(
            decode(br#"{"title":"Empty","state":"MERGED"}"#, "pr")
                .unwrap()
                .join("\n")
                .contains("No checks reported")
        );
        assert!(decode(b"not json", "pr").is_err());
        assert!(decode(b"{}", "issue").is_err());
    }
    #[test]
    fn github_process_is_bounded_cancellable_and_does_not_expose_stderr() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(os::nonce().unwrap());
        fs::create_dir_all(&root).unwrap();
        let script = root.join("gh-fixture");
        let serial = AtomicU64::new(1);
        let execute = |body: &str, timeout: Duration| {
            fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
            run(&mut Command::new(&script), &serial, 1, timeout)
        };
        assert_eq!(
            execute("printf '{\"title\":\"Fixture\"}'", Duration::from_secs(1)).unwrap(),
            br#"{"title":"Fixture"}"#
        );
        assert!(
            execute(
                "echo 'secret TOKEN auth login' >&2; exit 1",
                Duration::from_secs(1)
            )
            .unwrap_err()
            .contains("sign-in")
        );
        assert_eq!(
            execute("echo 'secret TOKEN' >&2; exit 1", Duration::from_secs(1)).unwrap_err(),
            "GitHub unavailable · links still work · r retry"
        );
        assert!(
            execute("echo 'rate limit' >&2; exit 1", Duration::from_secs(1))
                .unwrap_err()
                .contains("rate limit")
        );
        assert!(
            execute("head -c 300000 /dev/zero", Duration::from_secs(1))
                .unwrap_err()
                .contains("too large")
        );
        let at = Instant::now();
        assert!(
            execute("exec sleep 2", Duration::from_millis(60))
                .unwrap_err()
                .contains("timed out")
        );
        assert!(at.elapsed() < Duration::from_secs(1));
        serial.store(2, Ordering::Relaxed);
        assert!(
            execute("exec sleep 2", Duration::from_secs(1))
                .unwrap_err()
                .contains("cancelled")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn github_cache_reuses_fresh_data_and_discards_cancelled_results() {
        let link = Link::parse("https://github.com/demo/fixture/issues/1").unwrap();
        let mut gh = Github::default();
        gh.cache.push(Entry {
            url: link.url.clone(),
            lines: vec!["Cached fixture".into()],
            at: Instant::now(),
            ok: true,
        });
        gh.request(vec![link.clone()], false);
        assert!(gh.job.is_none() && gh.pending.is_empty());
        assert_eq!(gh.lines(&link, false), vec!["Cached fixture"]);
        let (tx, rx) = mpsc::channel();
        let generation = gh.serial.load(Ordering::Relaxed);
        gh.job = Some(Job { rx, generation });
        gh.cancel();
        tx.send(vec![Entry {
            url: link.url.clone(),
            lines: vec!["Obsolete".into()],
            at: Instant::now(),
            ok: true,
        }])
        .unwrap();
        assert!(gh.tick());
        assert_eq!(gh.lines(&link, false), vec!["Cached fixture"]);
        gh.cache[0].at -= Duration::from_secs(61);
        assert!(
            gh.lines(&link, false)
                .iter()
                .any(|s| s.contains("Cached ·"))
        );
    }
}
