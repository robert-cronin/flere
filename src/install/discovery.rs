//! Metadata-only background checks. No download or installation of packages.
use std::{
    io,
    sync::mpsc,
    time::{Duration, Instant},
};

pub struct Discovery {
    next: Instant,
    job: Option<mpsc::Receiver<io::Result<Option<String>>>>,
}
impl Default for Discovery {
    fn default() -> Self {
        Self {
            next: Instant::now() + Duration::from_secs(5),
            job: None,
        }
    }
}
impl Discovery {
    pub fn tick(
        &mut self,
        check: impl FnOnce() -> io::Result<Option<String>> + Send + 'static,
    ) -> Option<io::Result<Option<String>>> {
        if let Some(job) = &self.job {
            let result = match job.try_recv() {
                Ok(result) => result,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Err(io::Error::other("Update check stopped"))
                }
            };
            self.job = None;
            return Some(result);
        }
        if Instant::now() >= self.next {
            self.next = Instant::now() + Duration::from_secs(4 * 60 * 60);
            let (tx, rx) = mpsc::channel();
            self.job = Some(rx);
            std::thread::spawn(move || {
                let _ = tx.send(check());
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_check_never_blocks_or_duplicates_and_rechecks_after_interval() {
        let mut discovery = Discovery {
            next: Instant::now(),
            job: None,
        };
        let (release, wait) = mpsc::channel();
        assert!(
            discovery
                .tick(move || {
                    wait.recv().unwrap();
                    Ok(Some("0.3.10".into()))
                })
                .is_none()
        );
        assert!(discovery.tick(|| panic!("duplicate check")).is_none());
        release.send(()).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(result) = discovery.tick(|| panic!("duplicate check")) {
                assert_eq!(result.unwrap().as_deref(), Some("0.3.10"));
                break;
            }
            assert!(Instant::now() < until);
            std::thread::yield_now();
        }
        assert!(
            discovery
                .tick(|| panic!("check before next interval"))
                .is_none()
        );
        discovery.next = Instant::now();
        assert!(discovery.tick(|| Ok(None)).is_none());
        assert!(discovery.job.is_some());
    }
}
