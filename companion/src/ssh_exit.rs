//! Remote stdout can close just before OpenSSH reports its final exit status.
use std::{
    io,
    process::{Child, ExitStatus},
    time::{Duration, Instant},
};

pub fn remote_closed(child: &mut Child) -> io::Result<ExitStatus> {
    wait_until(child, Instant::now() + Duration::from_secs(2))
}

fn wait_until(child: &mut Child, deadline: Instant) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(status)
            } else {
                Err(io::Error::other(format!(
                    "SSH connection failed ({status}); the remote UI disconnected"
                )))
            };
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Remote output closed before SSH reported an exit status; detached without replaying input",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    struct Fixture(Child);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn fixture(mode: &str) -> Fixture {
        Fixture(
            Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "ssh_exit::tests::exit_fixture"])
                .env("FLERE_SSH_EXIT_TEST", mode)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    #[test]
    fn exit_fixture() {
        let Ok(mode) = std::env::var("FLERE_SSH_EXIT_TEST") else {
            return;
        };
        if mode == "pending" {
            std::thread::sleep(Duration::from_secs(10));
        } else {
            std::thread::sleep(Duration::from_millis(80));
        }
        std::process::exit(if mode == "failure" { 23 } else { 0 });
    }

    #[test]
    fn delayed_ssh_failure_is_reported_instead_of_a_normal_detach() {
        let mut child = fixture("failure");
        let error = remote_closed(&mut child.0).unwrap_err();
        assert!(error.to_string().contains("SSH connection failed"));
        assert!(error.to_string().contains("23"));
    }

    #[test]
    fn deliberate_remote_detach_keeps_its_success_status() {
        let mut child = fixture("success");
        assert!(remote_closed(&mut child.0).unwrap().success());
    }

    #[test]
    fn remote_close_does_not_wait_forever_for_a_stuck_ssh_process() {
        let mut child = fixture("pending");
        let error = wait_until(&mut child.0, Instant::now()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(child.0.try_wait().unwrap().is_none());
    }
}
