//! Local named SSH configurations. Authentication stays in OpenSSH; a reconnect
//! stores connection arguments only, never an image, terminal input or a command.
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::PathBuf,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub remote: String,
    pub remote_explicit: bool,
    pub state: Option<String>,
    pub ssh: String,
}
fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
fn printable(value: &str) -> bool {
    !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-@[]:".contains(&b))
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
impl Connection {
    pub fn parse(args: &[String]) -> io::Result<Self> {
        let host = args
            .first()
            .filter(|v| valid_host(v))
            .ok_or_else(|| invalid("use an SSH host alias or user@host"))?
            .clone();
        let mut result = Self {
            host,
            remote: "flere".into(),
            remote_explicit: false,
            state: None,
            ssh: "ssh".into(),
        };
        let mut seen = std::collections::HashSet::new();
        for pair in args[1..].chunks(2) {
            if pair.len() != 2 || !seen.insert(pair[0].as_str()) || !printable(&pair[1]) {
                return Err(invalid(
                    "connection options require unique names and printable values",
                ));
            }
            match pair[0].as_str() {
                "--remote" => {
                    result.remote = pair[1].clone();
                    result.remote_explicit = true;
                }
                "--state" => result.state = Some(pair[1].clone()),
                "--ssh" => result.ssh = pair[1].clone(),
                "--image" => {} // Explicit one-shot paste is deliberately never saved.
                _ => return Err(invalid("unknown connection option; use --help")),
            }
        }
        Ok(result)
    }
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![self.host.clone(), "--ssh".into(), self.ssh.clone()];
        if self.remote_explicit {
            args.extend(["--remote".into(), self.remote.clone()]);
        }
        if let Some(state) = &self.state {
            args.extend(["--state".into(), state.clone()]);
        }
        args
    }
    fn json(&self) -> Value {
        json!({"host":self.host,"remote":self.remote,"remote_explicit":self.remote_explicit,"state":self.state,"ssh":self.ssh})
    }
    fn decode(value: &Value) -> io::Result<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| invalid("invalid saved SSH connection"))?;
        if object.len() != 5 {
            return Err(invalid("unknown saved connection fields"));
        }
        let string = |key| {
            object
                .get(key)
                .and_then(Value::as_str)
                .filter(|s| printable(s))
                .map(String::from)
                .ok_or_else(|| invalid("invalid saved connection value"))
        };
        let result = Self {
            host: string("host")?,
            remote: string("remote")?,
            remote_explicit: object
                .get("remote_explicit")
                .and_then(Value::as_bool)
                .ok_or_else(|| invalid("missing remote selection mode"))?,
            ssh: string("ssh")?,
            state: match object.get("state") {
                Some(Value::Null) => None,
                Some(Value::String(s)) if printable(s) => Some(s.clone()),
                _ => return Err(invalid("invalid saved remote state directory")),
            },
        };
        if !valid_host(&result.host) || (!result.remote_explicit && result.remote != "flere") {
            return Err(invalid("invalid saved SSH host"));
        }
        Ok(result)
    }
}

#[derive(Default)]
struct Saved {
    profiles: BTreeMap<String, Connection>,
    last: Option<Connection>,
}
struct Store {
    root: PathBuf,
}
fn options() -> OpenOptions {
    let options = OpenOptions::new();
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::OpenOptionsExt;
        let mut options = options;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        options
    };
    options
}
fn owned(meta: &fs::Metadata, directory: bool) -> bool {
    let valid = !meta.file_type().is_symlink()
        && if directory {
            meta.is_dir()
        } else {
            meta.is_file()
        };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        valid
            && meta.uid() == crate::os::uid()
            && meta.mode() & 0o077 == 0
            && (directory || meta.nlink() == 1)
    }
    #[cfg(not(unix))]
    {
        valid
    }
}
impl Store {
    fn standard() -> io::Result<Self> {
        #[cfg(windows)]
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(unix)]
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")));
        let root = base
            .filter(|p| p.is_absolute())
            .ok_or_else(|| invalid("local configuration directory is unavailable"))?
            .join("flere-connect");
        Ok(Self { root })
    }
    fn directory(&self) -> io::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.root)?;
        if !owned(&fs::symlink_metadata(&self.root)?, true) {
            return Err(invalid("connection directory must be owned and private"));
        }
        Ok(())
    }
    fn read(&self) -> io::Result<Saved> {
        let path = self.root.join("connections.json");
        let meta = match fs::symlink_metadata(&self.root) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Saved::default()),
            result => result?,
        };
        if !owned(&meta, true) {
            return Err(invalid("connection directory must be owned and private"));
        }
        let mut file = match options().read(true).open(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Saved::default()),
            result => result?,
        };
        if !owned(&file.metadata()?, false) {
            return Err(invalid("connections file must be owned and private"));
        }
        let mut bytes = Vec::new();
        (&mut file).take(65537).read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(invalid("saved connections exceed 64 KiB"));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if value.get("schema_version").and_then(Value::as_u64) != Some(1)
            || value.as_object().is_none_or(|v| v.len() != 3)
        {
            return Err(invalid("unsupported connections schema"));
        }
        let profiles = value
            .get("connections")
            .and_then(Value::as_object)
            .filter(|p| p.len() <= 64)
            .ok_or_else(|| invalid("invalid saved connections"))?;
        let mut saved = Saved::default();
        for (name, profile) in profiles {
            if !valid_name(name) {
                return Err(invalid("invalid connection name"));
            }
            saved
                .profiles
                .insert(name.clone(), Connection::decode(profile)?);
        }
        saved.last = match value.get("last") {
            Some(Value::Null) => None,
            Some(value) => Some(Connection::decode(value)?),
            None => return Err(invalid("missing last connection")),
        };
        Ok(saved)
    }
    fn update(&self, update: impl FnOnce(&mut Saved) -> io::Result<()>) -> io::Result<()> {
        self.directory()?;
        let lock = options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".lock"))?;
        if !owned(&lock.metadata()?, false) {
            return Err(invalid("invalid connection lock"));
        }
        lock.try_lock().map_err(io::Error::other)?;
        let mut saved = self.read()?;
        update(&mut saved)?;
        if saved.profiles.len() > 64 {
            return Err(invalid("at most 64 saved connections are supported"));
        }
        let profiles: serde_json::Map<String, Value> = saved
            .profiles
            .iter()
            .map(|(n, p)| (n.clone(), p.json()))
            .collect();
        let bytes = serde_json::to_vec_pretty(&json!({"schema_version":1,"last":saved.last.as_ref().map(Connection::json),"connections":profiles})).map_err(io::Error::other)?;
        if bytes.len() > 65536 {
            return Err(invalid("saved connections exceed 64 KiB"));
        }
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        let partial = self
            .root
            .join(format!(".connections-{}-{stamp}.part", std::process::id()));
        let result = (|| {
            let mut file = options().write(true).create_new(true).open(&partial)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&partial, self.root.join("connections.json"))?;
            #[cfg(unix)]
            fs::File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        let _ = fs::remove_file(&partial);
        result
    }
}

/// Resolve before opening SSH. Administrative commands do not connect anywhere.
pub fn prepare(args: &[String]) -> io::Result<Option<Vec<String>>> {
    if args.first().is_some_and(|s| s == "connections") {
        let store = Store::standard()?;
        match args.get(1).map(String::as_str) {
            Some("list") if args.len() == 2 => {
                for (name, profile) in store.read()?.profiles {
                    println!("{name}\t{}\t{}", profile.host, profile.remote);
                }
            }
            Some("save") if args.len() >= 4 && valid_name(&args[2]) => {
                let profile = Connection::parse(&args[3..])?;
                store.update(|saved| {
                    saved.profiles.insert(args[2].clone(), profile);
                    Ok(())
                })?;
                println!("Saved connection {}", args[2]);
            }
            Some("remove") if args.len() == 3 && valid_name(&args[2]) => {
                store.update(|saved| {
                    saved
                        .profiles
                        .remove(&args[2])
                        .ok_or_else(|| invalid("unknown saved connection"))?;
                    Ok(())
                })?;
                println!("Removed connection {}", args[2]);
            }
            _ => {
                return Err(invalid(
                    "use connections list | save NAME HOST [options] | remove NAME",
                ));
            }
        }
        return Ok(None);
    }
    if args
        .first()
        .is_some_and(|s| s == "--connection" || s == "reconnect")
    {
        let saved = Store::standard()?.read()?;
        let profile = if args[0] == "reconnect" && args.len() == 1 {
            saved
                .last
                .ok_or_else(|| invalid("no previous successful connection"))?
        } else if args.len() == 2 {
            saved
                .profiles
                .get(&args[1])
                .cloned()
                .ok_or_else(|| invalid("unknown saved connection"))?
        } else {
            return Err(invalid("use --connection NAME or reconnect [NAME]"));
        };
        return Ok(Some(profile.args()));
    }
    Connection::parse(args)?;
    Ok(Some(args.to_vec()))
}
pub fn remember(args: &[String]) -> io::Result<()> {
    let profile = Connection::parse(args)?;
    Store::standard()?.update(|saved| {
        saved.last = Some(profile);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn reconnect_records_only_connection_identity_never_input_or_one_shot_image() {
        let automatic = Connection::parse(&args(&["dev-alias"])).unwrap();
        assert!(!automatic.remote_explicit);
        assert!(!automatic.args().iter().any(|arg| arg == "--remote"));
        assert_eq!(Connection::decode(&automatic.json()).unwrap(), automatic);
        let profile = Connection::parse(&args(&[
            "dev-alias",
            "--remote",
            "/home/me/bin/rail hand",
            "--state",
            "/home/me/private state",
            "--image",
            "/home/me/one-shot.png",
        ]))
        .unwrap();
        assert!(
            !profile
                .args()
                .iter()
                .any(|s| s.contains("image") || s.contains("one-shot"))
        );
        assert_eq!(Connection::decode(&profile.json()).unwrap(), profile);
        for bad in ["", "-oProxyCommand=bad", "host;command", "host\nother"] {
            assert!(!valid_host(bad));
        }
        assert!(Connection::parse(&args(&["host", "--ssh", "ssh", "--ssh", "other"])).is_err());
    }
    #[test]
    fn saved_profiles_are_private_atomic_and_reject_unrecognized_schemas() {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("LOCALAPPDATA"))
            .unwrap();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let store = Store {
            root: PathBuf::from(&home)
                .join(".cache/flere/tests")
                .join(format!("connections-{}-{stamp}", std::process::id())),
        };
        assert!(store.read().unwrap().profiles.is_empty());
        assert!(!store.root.exists());
        let connection =
            Connection::parse(&args(&["example", "--state", "/home/u/state"])).unwrap();
        store
            .update(|s| {
                s.profiles.insert("work".into(), connection.clone());
                s.last = Some(connection.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(store.read().unwrap().last, Some(connection.clone()));
        assert_eq!(store.read().unwrap().profiles["work"], connection);
        assert!(
            fs::read_dir(&store.root).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
        );
        fs::write(
            store.root.join("connections.json"),
            br#"{"schema_version":2,"last":null,"connections":{}}"#,
        )
        .unwrap();
        assert!(store.read().is_err());
        fs::remove_dir_all(store.root).unwrap();
    }
}
