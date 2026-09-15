use super::*;
use crate::bootstrap::{Cancellation, Selection};
use serde_json::{Value, json};

const TARGET: &str = "x86_64-unknown-linux-gnu";
const MANIFEST_URL: &str = "https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere-x86_64-unknown-linux-gnu.manifest.json";

struct Fixture {
    root: PathBuf,
    request: Request,
}
impl Fixture {
    fn new() -> Self {
        let mut request = Request::new(
            crate::connections::Connection {
                host: "fixture".into(),
                remote: COMPONENT.into(),
                remote_explicit: false,
                state: None,
                ssh: "unused".into(),
            },
            Selection::Automatic,
            Source::DefaultChannel,
        );
        request.names.cache_directory = format!("flere-channel-test-{}", update::nonce().unwrap());
        let directory = cache(&request.names).unwrap();
        Self {
            root: directory.parent().unwrap().parent().unwrap().into(),
            request,
        }
    }
    fn hash(&self, bytes: &[u8]) -> String {
        let path = self.root.join("hash-input");
        fs::write(&path, bytes).unwrap();
        digest(&path).unwrap()
    }
    fn manifest(&self) -> (Vec<u8>, Vec<u8>) {
        // These bytes are never executed; only normal package hashing is used.
        let payload = b"default-channel fixture payload".to_vec();
        let mut build: Value = serde_json::from_str(crate::build_info::json()).unwrap();
        build["component"] = json!(COMPONENT);
        build["target"] = json!(TARGET);
        build["package_version"] = json!("0.3.6");
        build["compatibility"] = json!({
            "control_identity":"fixture-control-v1",
            "snapshot":{"current":5,"read_min":1,"read_max":5},
            "refresh_handoff":{"current":6,"read_min":1,"read_max":6},
            "saved_state":{"current":7,"read_min":2,"read_max":7},
            "remote_protocol":{"current":"flere-remote-v6","accepts":["flere-remote-v6"]}
        });
        let value = json!({"schema_version":1,"build":build,"payload":{"file_name":COMPONENT,"bytes":payload.len(),"sha256":self.hash(&payload)}});
        (serde_json::to_vec(&value).unwrap(), payload)
    }
    fn index(&self, manifest: &[u8]) -> Vec<u8> {
        let pin = json!({"bytes":manifest.len(),"sha256":self.hash(manifest)});
        serde_json::to_vec(&json!({"schema_version":1,"targets":{TARGET:{"policy":"current","version":"0.3.6","manifests":{"flere":pin,"flere-connect":pin}}}})).unwrap()
    }
    fn empty_packages(&self) {
        assert_eq!(
            fs::read_dir(cache(&self.request.names).unwrap())
                .unwrap()
                .count(),
            0
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("Channel fixture retained at {}", self.root.display());
        } else {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
}

#[test]
fn default_fetches_one_index_and_only_pinned_core_then_revalidates_cache() {
    let f = Fixture::new();
    let (manifest, payload) = f.manifest();
    let index = f.index(&manifest);
    let mut calls = Vec::new();
    let package = obtain_with(&f.request, TARGET, |url, maximum| {
        calls.push((url.to_owned(), maximum));
        match url {
            channel::URL => Ok(index.clone()),
            MANIFEST_URL => Ok(manifest.clone()),
            "https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere" => {
                Ok(payload.clone())
            }
            _ => panic!("unexpected fetch {url}"),
        }
    })
    .unwrap();
    assert_eq!(
        calls,
        [
            (channel::URL.into(), 8192),
            (MANIFEST_URL.into(), MAX_JSON),
            (
                "https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere".into(),
                payload.len()
            )
        ]
    );
    assert_eq!(package.source_url.as_deref(), Some(MANIFEST_URL));
    assert_eq!(
        fs::read(package.directory.join(COMPONENT)).unwrap(),
        payload
    );
    let mut repeated = Vec::new();
    let cached = obtain_with(&f.request, TARGET, |url, _| {
        repeated.push(url.to_owned());
        match url {
            channel::URL => Ok(index.clone()),
            MANIFEST_URL => Ok(manifest.clone()),
            _ => panic!("cache fetched payload"),
        }
    })
    .unwrap();
    assert_eq!(cached.manifest, package.manifest);
    assert_eq!(repeated, [channel::URL, MANIFEST_URL]);
    // A cached package does not authorize an offline/default-index fallback.
    assert!(
        obtain_with(&f.request, TARGET, |url, _| {
            assert_eq!(url, channel::URL);
            Err(io::ErrorKind::NotConnected.into())
        })
        .is_err()
    );
}

#[test]
fn explicit_and_local_sources_bypass_channel_unchanged() {
    let mut f = Fixture::new();
    let (manifest, payload) = f.manifest();
    f.request.source = Source::HttpsManifest("https://example.test/core.manifest.json".into());
    let mut calls = Vec::new();
    let package = obtain_with(&f.request, TARGET, |url, _| {
        calls.push(url.to_owned());
        match url {
            "https://example.test/core.manifest.json" => Ok(manifest.clone()),
            "https://example.test/flere" => Ok(payload.clone()),
            _ => panic!("explicit source queried channel"),
        }
    })
    .unwrap();
    assert_eq!(
        calls,
        [
            "https://example.test/core.manifest.json",
            "https://example.test/flere"
        ]
    );
    f.request.source = Source::LocalPackage(package.directory.clone());
    let local = obtain_with(&f.request, TARGET, |_, _| {
        panic!("local package fetched anything")
    })
    .unwrap();
    assert_eq!(local.manifest, package.manifest);
    assert!(local.source_url.is_none());
}

#[test]
fn invalid_index_and_raw_manifest_pin_stop_before_payload_or_cache_install() {
    let f = Fixture::new();
    let (manifest, _) = f.manifest();
    let index = f.index(&manifest);
    for bad_index in [b"{}".to_vec(), vec![b' '; channel::MAX_BYTES + 1]] {
        let mut calls = 0;
        assert!(
            obtain_with(&f.request, TARGET, |url, _| {
                calls += 1;
                assert_eq!(url, channel::URL);
                Ok(bad_index.clone())
            })
            .is_err()
        );
        assert_eq!(calls, 1);
        f.empty_packages();
    }
    for bytes in [vec![b'!'; manifest.len()], vec![b'!'; manifest.len() + 1]] {
        let mut calls = 0;
        let error = obtain_with(&f.request, TARGET, |url, _| {
            calls += 1;
            match url {
                channel::URL => Ok(index.clone()),
                MANIFEST_URL => Ok(bytes.clone()),
                _ => panic!("unverified manifest fetched payload"),
            }
        })
        .err()
        .unwrap();
        assert!(
            error
                .to_string()
                .contains(if bytes.len() == manifest.len() {
                    "SHA-256 differs"
                } else {
                    "size differs"
                })
        );
        assert_eq!(calls, 2);
        f.empty_packages();
    }
}

#[test]
fn pinned_manifest_must_still_match_selected_version_target_and_component() {
    let f = Fixture::new();
    let (manifest, _) = f.manifest();
    for (field, wrong) in [
        ("package_version", "0.3.7"),
        ("target", "aarch64-apple-darwin"),
        ("component", "flere-connect"),
    ] {
        let mut value: Value = serde_json::from_slice(&manifest).unwrap();
        value["build"][field] = json!(wrong);
        let bytes = serde_json::to_vec(&value).unwrap();
        let index = f.index(&bytes);
        let mut calls = 0;
        assert!(
            obtain_with(&f.request, TARGET, |url, _| {
                calls += 1;
                match url {
                    channel::URL => Ok(index.clone()),
                    MANIFEST_URL => Ok(bytes.clone()),
                    _ => panic!("wrong metadata fetched payload"),
                }
            })
            .is_err()
        );
        assert_eq!(calls, 2);
        f.empty_packages();
    }
}

#[test]
fn cancellation_at_channel_and_manifest_boundaries_selects_no_payload() {
    let mut f = Fixture::new();
    let (manifest, _) = f.manifest();
    let index = f.index(&manifest);
    f.request.cancellation.cancel();
    assert!(
        obtain_with(&f.request, TARGET, |_, _| panic!(
            "cancelled operation fetched"
        ))
        .is_err()
    );
    for cancel_at in [channel::URL, MANIFEST_URL] {
        f.request.cancellation = Cancellation::default();
        let cancelled = f.request.cancellation.clone();
        let error = obtain_with(&f.request, TARGET, |url, _| {
            if url == cancel_at {
                cancelled.cancel();
            }
            match url {
                channel::URL => Ok(index.clone()),
                MANIFEST_URL => Ok(manifest.clone()),
                _ => panic!("cancelled operation fetched payload"),
            }
        })
        .err()
        .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        f.empty_packages();
    }
}
