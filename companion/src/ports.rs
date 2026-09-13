//! Explicit loopback forwards owned by this companion. Detach stops only these
//! SSH children; it never stops the remote service or changes a Flere session.
use crate::{connections::Connection, remote_services as service, transfers};
use serde_json::{Value, json};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
struct Forward {
    local: u16,
    remote: u16,
    workspace: String,
    child: crate::Ssh,
    started: Instant,
    ready: bool,
}
#[derive(Default)]
pub struct Ports {
    forwards: Vec<Forward>,
}
impl Ports {
    pub fn request(&mut self, connection: &Connection, data: &[u8]) -> Vec<u8> {
        let result = self.action(connection, data);
        let (success, message) = match result {
            Ok(message) => (true, message),
            Err(error) => (false, error.to_string()),
        };
        serde_json::to_vec(&json!({"success":success,"message":message,"ports":self.rows()}))
            .unwrap()
    }
    fn action(&mut self, connection: &Connection, data: &[u8]) -> io::Result<String> {
        let value = service::decode(data)?;
        let op = value
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| service::invalid("missing port action"))?;
        if op == "list" {
            return Ok("Only explicitly forwarded ports appear here".into());
        }
        let local = value
            .get("local")
            .and_then(Value::as_u64)
            .filter(|p| (1..=65535).contains(p))
            .ok_or_else(|| service::invalid("local port must be 1–65535"))?
            as u16;
        match op {
            "forward" => {
                let remote = value
                    .get("remote")
                    .and_then(Value::as_u64)
                    .filter(|p| (1..=65535).contains(p))
                    .ok_or_else(|| service::invalid("remote port must be 1–65535"))?
                    as u16;
                if self.forwards.len() >= 16 || self.forwards.iter().any(|p| p.local == local) {
                    return Err(service::invalid(
                        "local port already forwarded or sixteen-forward limit reached",
                    ));
                }
                let workspace = value
                    .get("workspace")
                    .and_then(Value::as_str)
                    .filter(|s| service::printable(s))
                    .ok_or_else(|| service::invalid("missing port workspace label"))?
                    .chars()
                    .take(128)
                    .collect();
                let address = format!("127.0.0.1:{local}:127.0.0.1:{remote}");
                // Fail before spawning if another local listener already owns this
                // endpoint. SSH still performs the authoritative bind with
                // ExitOnForwardFailure after this short preflight is released.
                drop(TcpListener::bind((Ipv4Addr::LOCALHOST, local))?);
                let child = crate::Ssh(
                    Command::new(&connection.ssh)
                        .args([
                            "-N",
                            "-T",
                            "-o",
                            "BatchMode=yes",
                            "-o",
                            "ExitOnForwardFailure=yes",
                            "-o",
                            "ServerAliveInterval=15",
                            "-o",
                            "ServerAliveCountMax=2",
                            "-L",
                            &address,
                            "--",
                            &connection.host,
                        ])
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::inherit())
                        .spawn()?,
                );
                self.forwards.push(Forward {
                    local,
                    remote,
                    workspace,
                    child,
                    started: Instant::now(),
                    ready: false,
                });
                Ok(format!("Starting SSH forward 127.0.0.1:{local}"))
            }
            "stop" => {
                let index = self
                    .forwards
                    .iter()
                    .position(|p| p.local == local)
                    .ok_or_else(|| service::invalid("unknown owned forward"))?;
                self.forwards.remove(index);
                Ok(format!(
                    "Stopped local forward {local}; remote service kept running"
                ))
            }
            "open" => {
                let forward = self
                    .forwards
                    .iter_mut()
                    .find(|p| p.local == local)
                    .ok_or_else(|| service::invalid("unknown owned forward"))?;
                if !forward.ready || forward.child.0.try_wait()?.is_some() {
                    return Err(service::invalid("forward is not listening yet"));
                }
                transfers::launch(&format!("http://127.0.0.1:{local}/"))?;
                Ok(format!("Opened http://127.0.0.1:{local}/"))
            }
            _ => Err(service::invalid("unknown port action")),
        }
    }
    fn rows(&self) -> Vec<Value> {
        self.forwards.iter().map(|p| json!({"local":p.local,"remote":p.remote,"workspace":p.workspace,"status":if p.ready {"listening"} else {"starting"}})).collect()
    }
    pub fn tick(&mut self) -> Option<Vec<u8>> {
        let mut changed = false;
        let mut failure = String::new();
        self.forwards.retain_mut(|p| {
            if let Ok(Some(status)) = p.child.0.try_wait() {
                changed = true;
                failure = format!(
                    "SSH forward {} ended ({status}); remote service kept running",
                    p.local
                );
                return false;
            }
            if !p.ready && p.started.elapsed() > Duration::from_millis(250) {
                let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), p.local);
                if TcpStream::connect_timeout(&address, Duration::from_millis(5)).is_ok() {
                    p.ready = true;
                    changed = true;
                }
            }
            true
        });
        changed.then(|| {
            serde_json::to_vec(
                &json!({"success":failure.is_empty(),"message":failure,"ports":self.rows()}),
            )
            .unwrap()
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    #[test]
    fn forwards_use_loopback_ssh_argv_and_stop_only_the_owned_child() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(format!("ports-{}-{stamp}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let script = root.join("ssh");
        let log = root.join("argv.json");
        fs::write(&script, format!("#!/usr/bin/python3\nimport sys,socket,json,pathlib\nargs=sys.argv[1:]\npathlib.Path({:?}).write_text(json.dumps(args))\naddress=args[args.index('-L')+1].split(':')\nassert address[0]==address[2]=='127.0.0.1'\ns=socket.socket();s.bind((address[0],int(address[1])));s.listen(8)\nwhile True:\n c,_=s.accept();c.close()\n",log.to_str().unwrap())).unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let connection = Connection {
            host: "fixture-host".into(),
            remote: "unused-remote".into(),
            remote_explicit: true,
            state: None,
            ssh: script.to_str().unwrap().into(),
        };
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let local = listener.local_addr().unwrap().port();
        let mut ports = Ports::default();
        let request =
            json!({"op":"forward","local":local,"remote":4317,"workspace":"Exact workspace"});
        assert!(
            ports
                .action(&connection, &serde_json::to_vec(&request).unwrap())
                .is_err()
        );
        assert!(
            !log.exists(),
            "an occupied local endpoint must not start SSH"
        );
        drop(listener);
        ports
            .action(&connection, &serde_json::to_vec(&request).unwrap())
            .unwrap();
        assert!(
            ports
                .action(&connection, &serde_json::to_vec(&request).unwrap())
                .is_err()
        );
        assert!(
            ports
                .action(
                    &connection,
                    &serde_json::to_vec(&json!({"op":"open","local":local})).unwrap()
                )
                .is_err(),
            "not ready: cannot open a browser"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while !ports.forwards[0].ready {
            ports.tick();
            assert!(
                !ports.forwards.is_empty(),
                "fixture SSH exited before listening"
            );
            assert!(Instant::now() < deadline, "fixture SSH readiness timeout");
            std::thread::sleep(Duration::from_millis(10));
        }
        let argv: Vec<String> = serde_json::from_slice(&fs::read(&log).unwrap()).unwrap();
        assert_eq!(&argv[..2], ["-N", "-T"]);
        assert!(
            argv.windows(2)
                .any(|p| p == ["-o", "ExitOnForwardFailure=yes"])
        );
        assert!(
            argv.windows(2)
                .any(|p| p == ["-L", &format!("127.0.0.1:{local}:127.0.0.1:4317")])
        );
        assert_eq!(&argv[argv.len() - 2..], ["--", "fixture-host"]);
        assert!(!argv.iter().any(|v| v.contains("StrictHostKeyChecking")
            || v.contains("serve")
            || v.contains("kill")));
        let rows = ports.rows();
        assert_eq!(rows[0]["workspace"], "Exact workspace");
        assert_eq!(rows[0]["status"], "listening");
        let unrelated = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        ports
            .action(
                &connection,
                &serde_json::to_vec(&json!({"op":"stop","local":local})).unwrap(),
            )
            .unwrap();
        assert!(ports.forwards.is_empty());
        assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, local)).is_err());
        assert!(TcpStream::connect(unrelated.local_addr().unwrap()).is_ok());
        assert!(
            ports
                .action(
                    &connection,
                    &serde_json::to_vec(
                        &json!({"op":"stop","local":unrelated.local_addr().unwrap().port()})
                    )
                    .unwrap()
                )
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
