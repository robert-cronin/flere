//! Optional companion-owned update flow. Requests carry no executable sources;
//! those are chosen with fresh local input and retained in a local plan.
pub const CAPABILITY: &[u8] = b"coordinated-update-v1";
pub const REQUEST: u8 = 41;
pub const ACTIVATE: u8 = 42;
pub const ACK: u8 = 43;
pub const RESULT: u8 = 44;
pub const CALL: u8 = 45;
pub const BEGIN: u8 = 46;
pub const DATA: u8 = 47;
pub const END: u8 = 48;
pub const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
pub fn token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

// Passive NOTICE probe is safe on older v6 peers, which reject unknown
// CAPABILITIES. Support is reset for every bridge handshake.
pub const OWNERSHIP_PROBE: &[u8] = b"coordinated-update-owner-v1?";
pub const OWNERSHIP_CAPABILITY: &[u8] = b"coordinated-update-owner-v1";
pub const CANDIDATE_CAPABILITY: &[u8] = b"flere-coordinated-ownership-v1\n";
pub const OWNERSHIP_LIMIT: usize = 32 * 1024;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnerKind {
    Managed,
    Manual,
    Manager,
    Unknown,
}
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallationOwner {
    pub kind: OwnerKind,
    pub executable: String,
    pub sha256: String,
    pub attempt: Option<String>,
    pub guidance: String,
}
impl InstallationOwner {
    pub fn validate(&self) -> std::io::Result<()> {
        if [&self.executable, &self.guidance].iter().any(|s| {
            s.len() > 4096
                || s.chars().any(|c| {
                    c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
                })
        }) || self.sha256.len() != 64
            || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.attempt.as_ref().is_some_and(|s| !token(s))
            || (self.kind == OwnerKind::Managed) != self.attempt.is_some()
        {
            return Err(std::io::Error::other(
                "invalid installation ownership evidence",
            ));
        }
        Ok(())
    }
    pub fn require_update(&self) -> std::io::Result<()> {
        self.validate()?;
        if matches!(self.kind, OwnerKind::Manager | OwnerKind::Unknown) {
            return Err(std::io::Error::other(self.guidance.clone()));
        }
        Ok(())
    }
    pub fn manual(&self) -> bool {
        self.kind == OwnerKind::Manual
    }
}
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoreOwners {
    pub schema_version: u64,
    pub epoch: String,
    pub pid: u32,
    pub frontend: InstallationOwner,
    pub supervisor: InstallationOwner,
}
impl CoreOwners {
    pub fn require_update(&self) -> std::io::Result<()> {
        if self.schema_version != 1 || !token(&self.epoch) || self.pid == 0 {
            return Err(std::io::Error::other(
                "remote ownership report is unsupported; upgrade the remote endpoint manually first",
            ));
        }
        self.frontend.require_update()?;
        self.supervisor.require_update()
    }
    pub fn manual(&self) -> bool {
        self.frontend.manual() || self.supervisor.manual()
    }
}
