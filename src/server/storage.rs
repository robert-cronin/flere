//! Guarded activation of lossless record storage beyond the legacy JSON bound.
//! New storage remains private; old JSON bytes are retained inside the database.
use super::*;
use crate::record_store::{CommitOutcome, Resolution, Store};
mod views;
use serde_json::{Value, json};

pub(super) struct Storage {
    store: Option<Store>,
    directory: String,
    unresolved: Option<String>,
    before: Value,
    checkpoint_ids: Vec<String>,
    active_view: bool,
    legacy_view: bool,
    delivery_after: Option<String>,
    context_fields: Value,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    records: String,
}
fn failure(error: impl std::fmt::Debug) -> io::Error {
    io::Error::other(format!("record storage: {error:?}"))
}
fn path(state: &Path, name: &str) -> io::Result<PathBuf> {
    let nonce = name
        .strip_prefix("records-")
        .ok_or_else(|| invalid("invalid record-store directory"))?;
    if nonce.len() != 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid record-store directory"));
    }
    let root = state.join(name);
    let metadata = fs::symlink_metadata(&root)?;
    if !metadata.is_dir() || metadata.uid() != os::uid() || metadata.mode() & 0o077 != 0 {
        return Err(invalid("record store must be an owned private directory"));
    }
    let database = fs::symlink_metadata(root.join("state.sqlite"))?;
    if !database.is_file() || database.uid() != os::uid() || database.mode() & 0o077 != 0 {
        return Err(invalid("record database must be an owned private file"));
    }
    Ok(root)
}
pub(crate) fn read_layout(state: &Path, value: Value) -> io::Result<Value> {
    let manifest: Manifest = serde_json::from_value(value).map_err(failure)?;
    if manifest.version != 11 {
        return Err(invalid("unsupported record manifest"));
    }
    Store::read_layout(&path(state, &manifest.records)?).map_err(failure)
}
pub(super) fn load(state: &Path, bytes: &[u8]) -> io::Result<Option<(Saved, Storage)>> {
    let value: Value = serde_json::from_slice(bytes).map_err(failure)?;
    if value["version"] != 11 {
        return Ok(None);
    }
    let manifest: Manifest = serde_json::from_value(value).map_err(failure)?;
    if manifest.version != 11 {
        return Err(invalid("unsupported record manifest"));
    }
    let store = Store::open(&path(state, &manifest.records)?).map_err(failure)?;
    let value = store.load_layout().map_err(failure)?;
    let before = value["coordination"].clone();
    let saved = serde_json::from_value(value).map_err(failure)?;
    Ok(Some((
        saved,
        Storage {
            store: Some(store),
            directory: manifest.records,
            unresolved: None,
            before,
            checkpoint_ids: vec![],
            active_view: false,
            legacy_view: false,
            delivery_after: None,
            context_fields: Value::Null,
        },
    )))
}
pub(super) fn read_legacy(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != os::uid() || meta.len() > 8 * 1024 * 1024 {
        return Err(invalid("invalid or oversized saved state file"));
    }
    let mut bytes = Vec::new();
    file.take(8 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(invalid("saved state exceeds 8 MiB"));
    }
    Ok(Some(bytes))
}
fn fence_marker(state: &Path, marker: &[u8]) -> io::Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(state.join("workspaces.v2.json"))?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.uid() != os::uid() || meta.len() != marker.len() as u64 {
        return Err(invalid("activation marker differs"));
    }
    let mut bytes = Vec::new();
    (&file)
        .take(marker.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes != marker {
        return Err(invalid("activation marker differs"));
    }
    file.sync_all()?;
    File::open(state)?.sync_all()
}
// Publication can report failure after rename. Return success only when either
// the writer or an independent exact-file plus directory fence proves durability.
fn publish_marker(
    state: &Path,
    marker: &[u8],
    write: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    write().or_else(|error| fence_marker(state, marker).map_err(|_| error))
}
impl Server {
    pub(super) fn record_unavailable(&self) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|s| s.unresolved.is_some() || s.store.is_none())
    }
    pub(super) fn record_mode(&self) -> bool {
        self.storage
            .as_ref()
            .is_some_and(|s| !s.active_view && !s.legacy_view)
    }
    pub(super) fn release_legacy_view(&mut self) {
        if let Some(storage) = &mut self.storage
            && storage.legacy_view
        {
            self.coordination.messages.clear();
            self.coordination.decisions.clear();
            self.coordination.checkpoints.clear();
            self.coordination.dispatches.clear();
            storage.before = json!(self.coordination);
            storage.legacy_view = false;
        }
    }
    pub(super) fn persist_records(&mut self, bytes: &[u8]) -> io::Result<()> {
        let root: Value = serde_json::from_slice(bytes).map_err(failure)?;
        if let Some(mut storage) = self.storage.take() {
            let result = storage.persist(self, &root);
            self.storage = Some(storage);
            return result;
        }
        // Preserve the exact accepted source. No separate reread may stand in
        // for validation of the bytes imported into the new store.
        let source = read_legacy(&self.state.join("workspaces.v2.json"))?;
        let original = source
            .clone()
            .unwrap_or_else(|| br#"{"version":10,"active":0,"workspaces":[]}"#.to_vec());
        let fallback =
            if serde_json::from_slice::<Value>(&original).map_err(failure)?["coordination"]
                .is_null()
            {
                read_legacy(&self.state.join("coordination.json"))?
            } else {
                None
            };
        drop(Server::load_source(
            &self.state,
            Some((&original, fallback.as_deref())),
        )?);
        let directory = format!("records-{}", os::nonce()?);
        let dbroot = self.state.join(&directory);
        fs::DirBuilder::new().mode(0o700).create(&dbroot)?;
        // Unreferenced preparations are retained, never automatically adopted
        // or deleted. Only the durable manifest selects an active database.
        let store = self.with_durable_job(|| {
            let mut store = Store::open(&dbroot).map_err(failure)?;
            store
                .import_legacy(&original, fallback.as_deref())
                .map_err(failure)?;
            Ok(store)
        })?;
        let mut storage = Storage {
            store: Some(store),
            directory,
            unresolved: None,
            before: root["coordination"].clone(),
            checkpoint_ids: vec![],
            active_view: false,
            legacy_view: true,
            delivery_after: None,
            context_fields: Value::Null,
        };
        storage.persist(self, &root)?;
        let marker = serde_json::to_vec(&json!({"version":11,"records":storage.directory}))
            .map_err(failure)?;
        let destination = self.state.join("workspaces.v2.json");
        // A cooperating supervisor is single-writer. Detect outside edits
        // before replacing its source; keep the prepared database for inspection.
        if read_legacy(&destination)? != source {
            return Err(invalid("saved source changed during record activation"));
        }
        let state = self.state.clone();
        if let Err(error) = publish_marker(&state, &marker, || {
            self.write_durable(&destination, &marker)
        }) {
            // Even if the old bytes are still visible, a failed durability
            // fence cannot prove which source will survive a crash.
            storage.unresolved = Some(format!("record activation outcome unresolved: {error}"));
            self.storage = Some(storage);
            return Err(invalid(
                "record activation outcome unresolved; dependent operations are unavailable",
            ));
        }
        self.storage = Some(storage);
        Ok(())
    }
}
impl Storage {
    fn persist(&mut self, server: &mut Server, root: &Value) -> io::Result<()> {
        if self.unresolved.is_some() {
            return Err(invalid("record storage unavailable pending recovery"));
        }
        let operation = os::nonce()?;
        let next_ids = if self.legacy_view {
            Vec::new()
        } else {
            let count = root["coordination"]["checkpoints"]
                .as_array()
                .map_or(0, Vec::len)
                .saturating_sub(self.checkpoint_ids.len());
            self.ready()?
                .checkpoint_append_ids(count)
                .map_err(failure)?
        };
        let mut store = self
            .store
            .take()
            .ok_or_else(|| invalid("record storage unavailable pending recovery"))?;
        self.unresolved = Some("record job did not return a verified outcome".into());
        let before = &self.before;
        let checkpoint_ids = &self.checkpoint_ids;
        let legacy = self.legacy_view;
        let (returned, result) = server.with_durable_job(move || {
            let outcome = if legacy {
                store.sync_snapshot(&operation, root)
            } else {
                store.sync_working_view(&operation, before, root, checkpoint_ids)
            };
            Ok(match outcome {
                CommitOutcome::Committed(()) => (Some(store), Ok(())),
                CommitOutcome::Unchanged(error) => (Some(store), Err(failure(error))),
                CommitOutcome::AlreadyCommitted => (Some(store), Ok(())),
                CommitOutcome::Uncertain { ticket, .. } => match store.reconcile(ticket) {
                    Ok(recovered) => {
                        let result = if recovered.resolution == Resolution::Committed {
                            Ok(())
                        } else {
                            Err(invalid("record transaction did not commit"))
                        };
                        (Some(recovered.store), result)
                    }
                    Err(error) => (None, Err(failure(error))),
                },
                CommitOutcome::Unavailable => (
                    None,
                    Err(invalid("record storage unavailable pending recovery")),
                ),
            })
        })?;
        self.store = returned;
        self.unresolved = if self.store.is_none() {
            Some(
                result
                    .as_ref()
                    .err()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        if result.is_ok() {
            self.checkpoint_ids.extend(next_ids);
            self.before = root["coordination"].clone();
        }
        result
    }
    fn ready(&self) -> io::Result<&Store> {
        if self.unresolved.is_some() {
            return Err(invalid("record storage unavailable pending recovery"));
        }
        self.store
            .as_ref()
            .ok_or_else(|| invalid("record storage unavailable pending recovery"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (PathBuf, PathBuf) {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(&os::nonce().unwrap()[..12]);
        let state = root.join("s");
        private_state(&state).unwrap();
        (root, state)
    }
    fn fill(server: &mut Server) {
        for n in 0..520 {
            server.coordination.messages.push(coordination::Message {
                id: format!("{n:032x}"),
                from: 0,
                to: 0,
                body: "x".repeat(16384),
                intent: "quiet".into(),
                saved: 1,
                surfaced: Some(1),
                native_surfaced: Some(1),
                acknowledged: Some(1),
                delivery: None,
                chat: None,
            });
        }
    }
    #[test]
    fn activation_can_start_without_an_old_file_and_keeps_unreferenced_preparations_unused() {
        let (root, state) = fixture();
        let orphan = state.join(format!("records-{}", os::nonce().unwrap()));
        fs::DirBuilder::new().mode(0o700).create(&orphan).unwrap();
        fs::write(orphan.join("incomplete"), b"synthetic preparation").unwrap();
        let mut server = Server::load(&state).unwrap();
        assert!(server.storage.is_none());
        fill(&mut server);
        server.persist().unwrap();
        server.release_legacy_view();
        assert!(server.coordination.messages.is_empty());
        drop(server);
        let server = Server::load(&state).unwrap();
        assert!(server.storage.is_some());
        assert!(orphan.join("incomplete").exists());
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn activation_validates_captured_source_before_publication_and_preserves_unknown_original_fields()
     {
        let (root, state) = fixture();
        let mut server = Server::load(&state).unwrap();
        server.persist().unwrap();
        let file = state.join("workspaces.v2.json");
        let original = fs::read(&file).unwrap();
        fs::write(&file,br#"{"version":10,"active":0,"workspaces":[{"id":0,"name":"invalid","cwd":"/","meta":{},"tabs":[],"selected":0}]}"#).unwrap();
        let invalid_source = fs::read(&file).unwrap();
        fill(&mut server);
        assert!(server.persist().is_err());
        assert_eq!(fs::read(&file).unwrap(), invalid_source);
        assert!(server.storage.is_none());
        fs::write(&file, &original).unwrap();
        server.coordination.messages.truncate(1);
        server.persist().unwrap();
        let mut source: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        source["coordination"]["messages"][0]["extra_legacy_evidence"] =
            json!("keep exact synthetic field");
        source["coordination"]["messages"][0]
            .as_object_mut()
            .unwrap()
            .remove("native_surfaced");
        let source = serde_json::to_vec(&source).unwrap();
        fs::write(&file, &source).unwrap();
        drop(server);
        let mut server = Server::load(&state).unwrap();
        server.coordination.messages.clear();
        fill(&mut server);
        server.coordination.messages[0].native_surfaced = None;
        server.persist().unwrap();
        let store = server.storage.as_ref().unwrap().ready().unwrap();
        assert_eq!(store.original_sources().unwrap().0, source);
        let message = store
            .get(
                &format!("{:032x}", 0),
                crate::record_store::Scope {
                    workspace: 0,
                    conversation: None,
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            message["extra_legacy_evidence"],
            "keep exact synthetic field"
        );
        assert!(message.get("native_surfaced").is_none());
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn activation_error_after_rename_is_reconciled_but_missing_or_foreign_marker_is_not() {
        let (root, state) = fixture();
        let file = state.join("workspaces.v2.json");
        let marker = br#"{"version":11,"records":"records-0123456789abcdef0123456789abcdef"}"#;
        fs::write(&file, b"old source").unwrap();
        assert!(
            publish_marker(&state, marker, || Err(io::Error::other(
                "synthetic before rename"
            )))
            .is_err()
        );
        assert_eq!(fs::read(&file).unwrap(), b"old source");
        publish_marker(&state, marker, || {
            fs::write(&file, marker)?;
            Err(io::Error::other("synthetic after rename"))
        })
        .unwrap();
        fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(root.join("unknown"), &file).unwrap();
        assert!(
            publish_marker(&state, marker, || Err(io::Error::other(
                "synthetic ambiguous publication"
            )))
            .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unresolved_store_blocks_dependent_commands_without_blocking_terminal_observation() {
        let (root, state) = fixture();
        let mut server = Server::load(&state).unwrap();
        fill(&mut server);
        server.persist().unwrap();
        server.release_legacy_view();
        server.storage.as_mut().unwrap().unresolved = Some("synthetic uncertain outcome".into());
        assert!(server.command("ping").is_ok());
        assert!(server.command("snapshot").is_ok());
        assert!(server.command("coordinate\t1\tcontext\t7b7d").is_err());
        assert!(server.command("new-stopped\t78\t2f").is_err());
        assert!(server.persist().is_err());
        drop(server);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_api_crosses_legacy_limit_and_cold_reopens_exact_ack_and_layout() {
        let root = PathBuf::from(std::env::var_os("HOME").unwrap())
            .join(".cache/flere/tests")
            .join(&os::nonce().unwrap()[..12]);
        let state = root.join("s");
        private_state(&state).unwrap();
        let mut server = Server::load(&state).unwrap();
        server.workspaces.push(Workspace {
            id: 1,
            name: "synthetic".into(),
            cwd: root.clone(),
            tabs: vec![],
            selected: 0,
            meta: CardMeta::default(),
            split: None,
        });
        server.active = 1;
        server.next = 2;
        server.persist().unwrap();
        // Seed bounded accepted records just below the legacy capacity boundary.
        for n in 0..495 {
            server.coordination.messages.push(coordination::Message {
                id: format!("{n:032x}"),
                from: 0,
                to: 1,
                body: "x".repeat(16384),
                intent: "quiet".into(),
                saved: 1,
                surfaced: Some(1),
                native_surfaced: Some(1),
                acknowledged: Some(1),
                delivery: None,
                chat: None,
            });
        }
        server.persist().unwrap();
        assert!(server.storage.is_none());
        for n in 495..520 {
            server.coordination.messages.push(coordination::Message {
                id: format!("{n:032x}"),
                from: 0,
                to: 1,
                body: "y".repeat(16384),
                intent: "quiet".into(),
                saved: 2,
                surfaced: None,
                native_surfaced: None,
                acknowledged: None,
                delivery: None,
                chat: None,
            });
        }
        server.persist().unwrap();
        assert!(server.storage.is_some());
        server.release_legacy_view();
        let id = format!("{:032x}", 519);
        let reply = server
            .coordination_inbox(
                1,
                Some((999, "synthetic-unverified-native")),
                &json!({"ack_ids":[id]}),
            )
            .unwrap();
        assert_eq!(reply["acknowledged"], json!([id]));
        let expected = server
            .storage
            .as_ref()
            .unwrap()
            .ready()
            .unwrap()
            .export_legacy_canonical()
            .unwrap()["coordination"]
            .clone();
        drop(server);
        let mut reopened = Server::load(&state).unwrap();
        assert!(reopened.workspaces[0].tabs.is_empty());
        assert!(reopened.coordination.messages.is_empty());
        let context = reopened.coordinate(1, None, "context", &json!({})).unwrap();
        assert_eq!(context["messages_total"], 520);
        assert_eq!(context["pending_messages"], 24);
        let mut records = Vec::new();
        let mut after = Value::Null;
        loop {
            let mut args = json!({"include_acknowledged":true,"limit":32});
            if !after.is_null() {
                args["after"] = after;
            }
            let page = reopened.coordinate(1, None, "inbox", &args).unwrap();
            records.extend(page["messages"].as_array().unwrap().clone());
            after = page["next_after"].clone();
            if after.is_null() {
                break;
            }
        }
        assert!(
            json!(records) == expected["messages"],
            "cold pages must preserve every exact record"
        );
        assert_eq!(reopened.workspaces[0].name, "synthetic");
        assert!(!records[519]["acknowledged"].is_null());
        assert!(reopened.coordination.messages.is_empty());
        drop(reopened);
        fs::remove_dir_all(root).unwrap();
    }
}
