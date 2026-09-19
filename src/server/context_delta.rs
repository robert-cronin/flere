//! Optional exact-field deltas over compact context. This cache is disposable:
//! missing bases always return a full view, never a partial reconstruction.
use super::*;
use serde_json::{Value, json};

const MAX_ENTRY_BYTES: usize = 128 * 1024;
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 64;

#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<Entry>,
    bytes: usize,
}
struct Entry {
    session: u64,
    run: String,
    revision: String,
    view: Vec<u8>,
}

pub(super) fn since(args: &Value) -> io::Result<Option<&str>> {
    match args.get("since") {
        None => Ok(None),
        Some(Value::String(value))
            if value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Ok(Some(value))
        }
        _ => Err(invalid(
            "since must be a context revision; omit it for a full view",
        )),
    }
}

impl Cache {
    fn take(&mut self, session: u64, run: &str) -> Option<Entry> {
        let index = self
            .entries
            .iter()
            .position(|e| e.session == session && e.run == run)?;
        let entry = self.entries.remove(index)?;
        self.bytes -= entry.view.len();
        Some(entry)
    }

    pub(super) fn invalidate(&mut self, session: u64, run: &str) {
        let _ = self.take(session, run);
    }

    pub(super) fn project(
        &mut self,
        session: u64,
        run: &str,
        since: Option<&str>,
        mut view: Value,
    ) -> io::Result<Value> {
        let encoded = serde_json::to_vec(&view).map_err(io::Error::other)?;
        let old = self.take(session, run);
        // Large views remain complete and retrievable; do not keep an unbounded
        // second copy just to optimize a subsequent read.
        if encoded.len() > MAX_ENTRY_BYTES {
            view["context_mode"] = json!("full");
            view["context_revision"] = Value::Null;
            return Ok(view);
        }
        let revision = match &old {
            Some(old) if old.view == encoded => old.revision.clone(),
            _ => os::nonce()?,
        };
        let base = old
            .as_ref()
            .filter(|old| since == Some(&old.revision))
            .map(|old| {
                serde_json::from_slice::<Value>(&old.view)
                    .map(|previous| (old, previous))
                    .map_err(io::Error::other)
            })
            .transpose()?
            .filter(|(_, previous)| {
                // A native process can switch conversations without changing
                // its hosted run. That new chat may not have the old base.
                previous["epoch"] == view["epoch"]
                    && previous["workspace"]["id"] == view["workspace"]["id"]
                    && previous["messaging"]["conversation"] == view["messaging"]["conversation"]
            });
        let response = if let Some((old, previous)) = base {
            // Exact comparison, not a hash used to suppress potentially binding
            // content. Keep source records and complete changed fields verbatim.
            let current = view.as_object().expect("context object");
            let previous = previous.as_object().expect("cached context object");
            let changed: serde_json::Map<_, _> = current
                .iter()
                .filter(|(key, value)| previous.get(*key) != Some(*value))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let removed: Vec<_> = previous
                .keys()
                .filter(|key| !current.contains_key(*key))
                .collect();
            json!({"context_mode":"delta","base_revision":old.revision,
                "context_revision":revision,
                "identity":{"epoch":view["epoch"],"workspace":view["workspace"]["id"],
                    "session":session,"run":run,"conversation":view["messaging"]["conversation"]},
                "changes":changed,"removed":removed})
        } else {
            view["context_mode"] = json!("full");
            view["context_revision"] = json!(revision);
            view
        };
        while self.entries.len() >= MAX_ENTRIES || self.bytes + encoded.len() > MAX_CACHE_BYTES {
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            self.bytes -= entry.view.len();
        }
        self.bytes += encoded.len();
        self.entries.push_back(Entry {
            session,
            run: run.into(),
            revision,
            view: encoded,
        });
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deltas_are_exact_and_lost_foreign_or_evicted_bases_reset() {
        let mut cache = Cache::default();
        let original = json!({"epoch":"fixture","workspace":{"id":1},
            "assignment":"Do not publish. 界","messages":[{"id":"m1","body":"original"}],
            "decisions":[{"answer":null}],"obsolete":"removed later"});
        let full = cache.project(1, "first", None, original.clone()).unwrap();
        let revision = full["context_revision"].as_str().unwrap();
        let repeated = cache
            .project(1, "first", Some(revision), original.clone())
            .unwrap();
        assert_eq!(repeated["context_revision"], revision);
        assert_eq!(repeated["changes"], json!({}));
        let mut next = original.clone();
        next["decisions"][0]["answer"] = json!("Keep it local; approval unchanged.");
        next["messages"] = json!([]);
        next.as_object_mut().unwrap().remove("obsolete");
        let delta = cache
            .project(1, "first", Some(revision), next.clone())
            .unwrap();
        assert_eq!(delta["changes"]["messages"], json!([]));
        let mut rebuilt = original;
        for key in delta["removed"].as_array().unwrap() {
            rebuilt
                .as_object_mut()
                .unwrap()
                .remove(key.as_str().unwrap());
        }
        for (key, value) in delta["changes"].as_object().unwrap() {
            rebuilt[key] = value.clone();
        }
        assert_eq!(rebuilt, next);
        let current = delta["context_revision"].as_str().unwrap();
        assert_eq!(
            cache
                .project(2, "other", Some(current), next.clone())
                .unwrap()["context_mode"],
            "full"
        );
        assert_eq!(
            cache
                .project(1, "different-run", Some(current), next.clone())
                .unwrap()["context_mode"],
            "full"
        );
        for id in 3..70 {
            cache.project(id, "fixture", None, next.clone()).unwrap();
        }
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.bytes <= MAX_CACHE_BYTES);
        assert_eq!(
            cache
                .project(1, "first", Some(current), next.clone())
                .unwrap()["context_mode"],
            "full"
        );
        assert_eq!(
            Cache::default()
                .project(1, "first", Some(current), next)
                .unwrap()["context_mode"],
            "full"
        );
    }

    #[test]
    fn changed_context_identity_requires_a_complete_base() {
        let original = json!({"epoch":"original","workspace":{"id":1},
            "messaging":{"conversation":"first"},"assignment":"Preserve this exact constraint.",
            "decisions":[{"answer":"Keep all work local."}]});
        for change in [
            json!({"epoch":"new"}),
            json!({"workspace":{"id":2}}),
            json!({"messaging":{"conversation":"second"}}),
            json!({"messaging":{"conversation":null}}),
        ] {
            let mut cache = Cache::default();
            let first = cache
                .project(1, "same-run", None, original.clone())
                .unwrap();
            let mut next = original.clone();
            for (key, value) in change.as_object().unwrap() {
                next[key] = value.clone();
            }
            let mut reset = cache
                .project(
                    1,
                    "same-run",
                    first["context_revision"].as_str(),
                    next.clone(),
                )
                .unwrap();
            assert_eq!(reset["context_mode"], "full");
            assert_ne!(reset["context_revision"], first["context_revision"]);
            reset.as_object_mut().unwrap().remove("context_mode");
            reset.as_object_mut().unwrap().remove("context_revision");
            assert_eq!(reset, next);
        }
    }

    #[test]
    fn cache_bounds_never_truncate_the_returned_evidence() {
        let mut cache = Cache::default();
        let huge = json!({"assignment":"x".repeat(MAX_ENTRY_BYTES + 1)});
        let full = cache.project(1, "fixture", None, huge.clone()).unwrap();
        assert_eq!(full["assignment"], huge["assignment"]);
        assert!(full["context_revision"].is_null());
        assert_eq!(cache.bytes, 0);
        for id in 0..40 {
            cache
                .project(id, "fixture", None, json!({"body":"x".repeat(100_000)}))
                .unwrap();
            assert!(cache.bytes <= MAX_CACHE_BYTES);
        }
        assert_eq!(
            cache.bytes,
            cache.entries.iter().map(|e| e.view.len()).sum::<usize>()
        );
        assert!(cache.entries.len() < MAX_ENTRIES);
        let bytes = cache.bytes;
        cache.invalidate(39, "wrong-run");
        assert_eq!(cache.bytes, bytes);
        cache.invalidate(39, "fixture");
        assert!(cache.bytes < bytes);
        assert_eq!(
            cache.bytes,
            cache.entries.iter().map(|e| e.view.len()).sum::<usize>()
        );
        let bytes = cache.bytes;
        cache.invalidate(39, "fixture");
        assert_eq!(cache.bytes, bytes);
        for value in [
            json!({"since":null}),
            json!({"since":"bad"}),
            json!({"since":2}),
        ] {
            assert!(since(&value).is_err());
        }
    }
}
