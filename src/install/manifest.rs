//! Shared package identity and compatibility; no filesystem or platform behavior.
use serde::{Deserialize, Serialize};
use std::io;

pub const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VersionRange {
    pub current: u64,
    pub read_min: u64,
    pub read_max: u64,
}
impl VersionRange {
    pub fn accepts(&self, version: u64) -> bool {
        self.read_min <= version && version <= self.read_max
    }
    fn valid(&self) -> bool {
        self.read_min > 0 && self.accepts(self.current) && self.read_max <= 1024
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolCompatibility {
    pub current: String,
    pub accepts: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<VersionRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_handoff: Option<VersionRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved_state: Option<VersionRange>,
    pub remote_protocol: ProtocolCompatibility,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildMetadata {
    pub schema_version: u64,
    pub identity_kind: String,
    pub component: String,
    pub package_version: String,
    pub build_id: String,
    pub target: String,
    pub profile: String,
    pub rustc: String,
    pub compatibility: Compatibility,
}
impl BuildMetadata {
    pub fn validate(&self) -> io::Result<()> {
        if self.schema_version != 1
            || self.identity_kind != "cargo_generation_stamp"
            || !matches!(self.component.as_str(), "flere" | "flere-connect")
            || [
                &self.package_version,
                &self.build_id,
                &self.target,
                &self.profile,
                &self.rustc,
            ]
            .iter()
            .any(|s| s.is_empty() || s.len() > 2048 || s.contains(['\0', '\x1b']))
        {
            return Err(invalid("invalid or unsupported embedded package identity"));
        }
        let c = &self.compatibility;
        let p = &c.remote_protocol;
        if p.current.is_empty()
            || p.current.len() > 64
            || p.accepts.is_empty()
            || p.accepts.len() > 16
            || !p.accepts.contains(&p.current)
            || p.accepts.iter().any(|s| s.is_empty() || s.len() > 64)
            || [&c.snapshot, &c.refresh_handoff, &c.saved_state]
                .into_iter()
                .flatten()
                .any(|r| !r.valid())
            || (self.component == "flere"
                && (c.snapshot.is_none()
                    || c.refresh_handoff.is_none()
                    || c.saved_state.is_none()
                    || c.control_identity.is_none()))
        {
            return Err(invalid("invalid package compatibility ranges"));
        }
        Ok(())
    }

    /// A manifest is only an early compatibility check. The running supervisor's
    /// candidate handoff validator still decides whether actual state can transfer.
    pub fn accepts_runtime(&self, running: &Self) -> bool {
        if self.component != running.component || self.target != running.target {
            return false;
        }
        match (
            &self.compatibility.refresh_handoff,
            &running.compatibility.refresh_handoff,
            &self.compatibility.saved_state,
            &running.compatibility.saved_state,
        ) {
            (Some(h), Some(old_h), Some(s), Some(old_s)) => {
                h.accepts(old_h.current) && s.accepts(old_s.current)
            }
            (None, None, None, None) => true,
            _ => false,
        }
    }

    pub fn reads_core(&self, core: &Self) -> bool {
        self.component == "flere-connect"
            && core.component == "flere"
            && self
                .compatibility
                .remote_protocol
                .accepts
                .contains(&core.compatibility.remote_protocol.current)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackageSource {
    Local,
    Public {
        manifest_url: String,
    },
    /// Resolve the fixed default channel on explicit preparation, never Apply.
    DefaultChannel {},
    Adopted,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SourceReceipt {
    pub git_commit: String,
    pub dirty: bool,
    pub source_sha256: String,
    /// Producer assertions, explicitly bound to the exact source and payload.
    pub checks: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub file_name: String,
    /// Flat release services need a distinct asset name for each target. The
    /// package's local executable keeps its fixed component basename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_file: Option<String>,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u64,
    pub build: BuildMetadata,
    pub payload: Payload,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceReceipt>,
}
pub(crate) fn digest_valid(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Manifest {
    pub fn validate(&self) -> io::Result<()> {
        self.build.validate()?;
        if self.schema_version != 1
            || self.payload.bytes == 0
            || self.payload.bytes > MAX_PAYLOAD
            || !digest_valid(&self.payload.sha256)
            || self.payload.file_name != self.build.component
            || self.payload.download_file.as_ref().is_some_and(|name| {
                name.len() > 256
                    || name.starts_with('.')
                    || name.is_empty()
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            })
            || self.source.as_ref().is_some_and(|s| {
                !digest_valid(&s.source_sha256)
                    || s.git_commit.len() > 64
                    || s.checks.len() > 32
                    || s.checks.iter().any(|s| s.len() > 256)
            })
        {
            return Err(invalid("invalid or unsupported package manifest"));
        }
        Ok(())
    }
    pub fn id(&self) -> String {
        format!("{}-{}", self.build.component, self.payload.sha256)
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;
    #[test]
    fn default_intent_is_strict_and_does_not_infer_historical_public_sources() {
        for raw in [
            r#"{"kind":"local"}"#,
            r#"{"kind":"adopted"}"#,
            r#"{"kind":"public","manifest_url":"https://github.com/robert-cronin/flere/releases/download/v0.3.7/flere-x86_64-unknown-linux-gnu.manifest.json"}"#,
            r#"{"kind":"default_channel"}"#,
        ] {
            let source: PackageSource = serde_json::from_str(raw).unwrap();
            assert_eq!(
                serde_json::to_value(&source).unwrap(),
                serde_json::from_str::<serde_json::Value>(raw).unwrap()
            );
            assert_eq!(
                matches!(source, PackageSource::DefaultChannel {}),
                raw.contains("default_channel")
            );
        }
        for raw in [
            r#"{"kind":"default_channel","manifest_url":"https://example.invalid/manifest.json"}"#,
            r#"{"kind":"default_channel","follow":true}"#,
            r#"{"kind":"public","manifest_url":"https://example.invalid/manifest.json","follow_default":true}"#,
        ] {
            assert!(serde_json::from_str::<PackageSource>(raw).is_err());
        }
    }
}
