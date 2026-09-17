//! Shared bounded default-release selection for core and companion.
use serde::{Deserialize, Deserializer};
use std::io;

pub(crate) const URL: &str =
    "https://raw.githubusercontent.com/robert-cronin/flere/main/packaging/channels/stable.json";
pub(crate) const MAX_BYTES: usize = 8192;
// The two-argument shape is intentional: old Windows launchers reject this
// known stateless prefix before forwarding, so it certifies this binary itself.
pub const CAPABILITY_ARGS: [&str; 2] = ["--build-info", "--default-channel-info"];
pub const CAPABILITY: &[u8] = b"{\"schema_version\":1,\"source_policy\":\"default_channel_v1\"}\n";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}
const LINUX: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-msvc";
const ARM_MAC: &str = "aarch64-apple-darwin";
const INTEL_MAC: &str = "x86_64-apple-darwin";

// An absent field is allowed; an explicit JSON null is not a target or pin.
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Channel {
    schema_version: u64,
    targets: Targets,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Targets {
    #[serde(
        rename = "x86_64-unknown-linux-gnu",
        default,
        deserialize_with = "present"
    )]
    linux: Option<Record>,
    #[serde(
        rename = "x86_64-pc-windows-msvc",
        default,
        deserialize_with = "present"
    )]
    windows: Option<Record>,
    #[serde(rename = "aarch64-apple-darwin", default, deserialize_with = "present")]
    arm_mac: Option<Record>,
    #[serde(rename = "x86_64-apple-darwin", default, deserialize_with = "present")]
    intel_mac: Option<Record>,
}

#[derive(Deserialize)]
#[serde(tag = "policy", rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Current {
        version: String,
        manifests: Manifests,
    },
    LegacyUnsigned {
        version: String,
        manifests: Manifests,
    },
    Unavailable {},
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifests {
    #[serde(rename = "flere", default, deserialize_with = "present")]
    core: Option<Pin>,
    #[serde(rename = "flere-connect", default, deserialize_with = "present")]
    companion: Option<Pin>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Selection {
    pub version: String,
    pub url: String,
    pub pin: Pin,
    pub legacy_unsigned: bool,
    component: String,
    target: String,
}

/// Compare releases numerically; a newer development version is never an upgrade target.
pub(crate) fn newer(candidate: &str, current: &str) -> io::Result<bool> {
    if !version(candidate) || !version(current) {
        return Err(invalid("Cannot compare non-release Flere versions"));
    }
    let parts = |value: &str| {
        value
            .split('.')
            .map(|v| v.parse::<u32>().unwrap())
            .collect::<Vec<_>>()
    };
    Ok(parts(candidate) > parts(current))
}

pub(crate) fn available(
    bytes: &[u8],
    target: &str,
    component: &str,
    current: &str,
) -> io::Result<Option<String>> {
    let selected = select(bytes, target, component)?;
    Ok(
        (!selected.legacy_unsigned && newer(&selected.version, current)?)
            .then_some(selected.version),
    )
}

pub(crate) fn official_source(url: &str, target: &str, component: &str) -> bool {
    let prefix = "https://github.com/robert-cronin/flere/releases/download/v";
    url.strip_prefix(prefix)
        .and_then(|rest| rest.split_once('/'))
        .is_some_and(|(release, _)| {
            manifest_url(release, target, component).is_ok_and(|expected| expected == url)
        })
}

fn version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && !(part.len() > 1 && part.starts_with('0'))
                && part.bytes().all(|b| b.is_ascii_digit())
                && part.parse::<u32>().is_ok_and(|n| n <= i32::MAX as u32)
        })
}

pub(crate) fn select(bytes: &[u8], target: &str, component: &str) -> io::Result<Selection> {
    if !matches!(component, "flere" | "flere-connect") {
        return Err(invalid("Unsupported default release component"));
    }
    if bytes.len() > MAX_BYTES {
        return Err(invalid("Default release channel exceeds its byte bound"));
    }
    let channel: Channel = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if channel.schema_version != 1 {
        return Err(invalid("Unsupported default release channel schema"));
    }
    let mut selected = None;
    // Validate every supplied record, including targets not selected for this
    // operation. Fixed named fields reject unknown and duplicate JSON keys.
    for (name, record) in [
        (LINUX, channel.targets.linux),
        (WINDOWS, channel.targets.windows),
        (ARM_MAC, channel.targets.arm_mac),
        (INTEL_MAC, channel.targets.intel_mac),
    ] {
        let Some(record) = record else { continue };
        let (release, manifests, legacy_unsigned) = match record {
            Record::Current { version, manifests } => (version, manifests, false),
            Record::LegacyUnsigned { version, manifests } => (version, manifests, true),
            Record::Unavailable {} => continue,
        };
        if !version(&release)
            || legacy_unsigned && (name != ARM_MAC || release != "0.3.0")
            || manifests.companion.is_none()
            || manifests.core.is_some() != (name != WINDOWS)
        {
            return Err(invalid(
                "Invalid default release target policy or component inventory",
            ));
        }
        for pin in [&manifests.core, &manifests.companion]
            .into_iter()
            .flatten()
        {
            if pin.bytes == 0
                || pin.bytes > MAX_MANIFEST_BYTES
                || pin.sha256.len() != 64
                || !pin
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid("Invalid default release manifest size/SHA-256"));
            }
        }
        let pin = if component == "flere" {
            manifests.core
        } else {
            manifests.companion
        };
        if name == target
            && let Some(pin) = pin
        {
            selected = Some(Selection {
                url: manifest_url(&release, name, component)?,
                version: release,
                pin,
                legacy_unsigned,
                component: component.into(),
                target: target.into(),
            });
        }
    }
    selected.ok_or_else(|| invalid(
        "No default prebuilt Flere package is available for this component and target; use a reviewed explicit package or the source Homebrew route on macOS",
    ))
}

/// The immutable installed release location is derived from the retained build;
/// DefaultChannel stores intent only, so rollback cannot leave a stale URL.
pub(crate) fn manifest_url(release: &str, target: &str, component: &str) -> io::Result<String> {
    if !version(release)
        || !matches!(target, LINUX | WINDOWS | ARM_MAC | INTEL_MAC)
        || !matches!(component, "flere" | "flere-connect")
        || (target == WINDOWS && component == "flere")
    {
        return Err(invalid(
            "Default source needs a supported canonical release identity",
        ));
    }
    Ok(format!(
        "https://github.com/robert-cronin/flere/releases/download/v{release}/{component}-{target}.manifest.json"
    ))
}

impl Selection {
    pub(crate) fn verify_identity(
        &self,
        version: &str,
        target: &str,
        component: &str,
    ) -> io::Result<()> {
        if version != self.version || target != self.target || component != self.component {
            return Err(invalid(
                "Default release manifest version/target/component differs from channel",
            ));
        }
        Ok(())
    }
}

impl Pin {
    pub(crate) fn verify(&self, bytes: &[u8], sha256: &str) -> io::Result<()> {
        if bytes.len() as u64 != self.bytes || sha256 != self.sha256 {
            return Err(invalid(
                "Default release manifest size/SHA-256 differs from channel",
            ));
        }
        Ok(())
    }
}

/// Updates may remain on the same release, but must not turn a saved current
/// policy into the legacy unsigned route or silently downgrade it.
pub(crate) fn select_update(
    bytes: &[u8],
    target: &str,
    component: &str,
    current: Option<&str>,
) -> io::Result<Selection> {
    let selected = select(bytes, target, component)?;
    let numbers = |v: &str| {
        v.split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
    };
    if selected.legacy_unsigned
        || current.is_some_and(|v| {
            !version(v) || numbers(v).unwrap() > numbers(&selected.version).unwrap()
        })
    {
        return Err(invalid(
            "Default channel is legacy or older than the installed release; choose an explicit reviewed package for a downgrade",
        ));
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn document() -> Value {
        let pin = json!({"bytes":42,"sha256":"a".repeat(64)});
        json!({"schema_version":1,"targets":{
            LINUX:{"policy":"current","version":"0.3.6","manifests":{"flere":pin,"flere-connect":pin}},
            WINDOWS:{"policy":"current","version":"0.3.6","manifests":{"flere-connect":pin}},
            ARM_MAC:{"policy":"legacy_unsigned","version":"0.3.0","manifests":{"flere":pin,"flere-connect":pin}},
            INTEL_MAC:{"policy":"unavailable"}
        }})
    }

    #[test]
    fn discovery_reports_only_newer_supported_releases() {
        assert!(newer("0.3.10", "0.3.9").unwrap());
        assert!(!newer("0.3.9", "0.3.10").unwrap());
        assert!(!newer("0.3.9", "0.3.9").unwrap());
        assert!(newer("0.3.9", "development").is_err());
        let bytes = serde_json::to_vec(&document()).unwrap();
        assert_eq!(
            available(&bytes, LINUX, "flere", "0.3.5")
                .unwrap()
                .as_deref(),
            Some("0.3.6")
        );
        assert!(
            available(&bytes, LINUX, "flere", "0.3.6")
                .unwrap()
                .is_none()
        );
        assert!(
            available(&bytes, LINUX, "flere", "0.3.10")
                .unwrap()
                .is_none()
        );
        assert!(
            available(&bytes, ARM_MAC, "flere", "0.2.0")
                .unwrap()
                .is_none()
        );
        assert!(available(&bytes, INTEL_MAC, "flere", "0.2.0").is_err());
        let official = manifest_url("0.3.8", LINUX, "flere").unwrap();
        assert!(official_source(&official, LINUX, "flere"));
        assert!(!official_source(&official, WINDOWS, "flere-connect"));
        assert!(!official_source(
            &(official + "?redirect=elsewhere"),
            LINUX,
            "flere"
        ));
    }

    #[test]
    fn fixed_targets_select_immutable_core_only_and_explicit_legacy() {
        let bytes = serde_json::to_vec(&document()).unwrap();
        let linux = select(&bytes, LINUX, "flere").unwrap();
        assert_eq!(
            linux.url,
            "https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere-x86_64-unknown-linux-gnu.manifest.json"
        );
        assert_eq!(linux.version, "0.3.6");
        assert_eq!(linux.pin.bytes, 42);
        assert!(!linux.legacy_unsigned);
        let mac = select(&bytes, ARM_MAC, "flere").unwrap();
        assert_eq!(mac.version, "0.3.0");
        assert!(mac.legacy_unsigned);
        for unavailable in [INTEL_MAC, WINDOWS, "foreign-target"] {
            assert!(select(&bytes, unavailable, "flere").is_err());
        }
        assert!(select(br#"{"schema_version":1,"targets":{}}"#, LINUX, "flere").is_err());
    }

    #[test]
    fn companion_selection_uses_its_own_pin_on_linux_windows_and_legacy_mac() {
        let mut value = document();
        for target in [LINUX, WINDOWS, ARM_MAC] {
            value["targets"][target]["manifests"]["flere-connect"] =
                json!({"bytes": 73, "sha256": "b".repeat(64)});
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        for target in [LINUX, WINDOWS, ARM_MAC] {
            let selected = select(&bytes, target, "flere-connect").unwrap();
            assert_eq!(selected.pin.bytes, 73);
            assert_eq!(selected.pin.sha256, "b".repeat(64));
            assert_eq!(selected.legacy_unsigned, target == ARM_MAC);
            assert_eq!(
                selected.url,
                format!(
                    "https://github.com/robert-cronin/flere/releases/download/v{}/flere-connect-{target}.manifest.json",
                    selected.version
                )
            );
        }
        assert_eq!(select(&bytes, LINUX, "flere").unwrap().pin.bytes, 42);
        assert!(select(&bytes, WINDOWS, "flere").is_err());
        assert!(select(&bytes, LINUX, "foreign").is_err());
        assert!(select(&bytes, INTEL_MAC, "flere-connect").is_err());
    }

    #[test]
    fn all_target_records_require_exact_inventory_policy_version_and_pin_types() {
        let original = document();
        let mut cases = Vec::new();
        for bad in [
            json!("01.3.6"),
            json!("0.3"),
            json!("0.3.6.0"),
            json!("+0.3.6"),
            json!("0.3.2147483648"),
            json!("0.3.６"),
            Value::Null,
        ] {
            let mut value = original.clone();
            value["targets"][LINUX]["version"] = bad;
            cases.push(value);
        }
        for bad in [
            json!(0),
            json!(65537),
            json!(true),
            json!(1.5),
            json!("42"),
            Value::Null,
        ] {
            let mut value = original.clone();
            value["targets"][LINUX]["manifests"]["flere"]["bytes"] = bad;
            cases.push(value);
        }
        for bad in [
            json!("A".repeat(64)),
            json!("a".repeat(63)),
            json!("g".repeat(64)),
            json!(1),
        ] {
            let mut value = original.clone();
            value["targets"][LINUX]["manifests"]["flere"]["sha256"] = bad;
            cases.push(value);
        }
        for target in [LINUX, WINDOWS, ARM_MAC, INTEL_MAC] {
            let mut value = original.clone();
            value["targets"][target] = Value::Null;
            cases.push(value);
        }
        let mut value = original.clone();
        value["targets"][ARM_MAC]["version"] = json!("0.3.1");
        cases.push(value);
        let mut value = original.clone();
        value["targets"][LINUX]["policy"] = json!("legacy_unsigned");
        cases.push(value);
        let mut value = original.clone();
        value["targets"][WINDOWS]["manifests"]["flere"] =
            original["targets"][LINUX]["manifests"]["flere"].clone();
        cases.push(value);
        let mut value = original.clone();
        value["targets"][LINUX]["manifests"]
            .as_object_mut()
            .unwrap()
            .remove("flere-connect");
        cases.push(value);
        let mut value = original.clone();
        value["targets"][LINUX]["manifests"]["flere"] = Value::Null;
        cases.push(value);
        let mut value = original.clone();
        value["targets"][INTEL_MAC]["version"] = json!("0.3.6");
        cases.push(value);
        let mut value = original;
        value["schema_version"] = json!(2);
        cases.push(value);
        for value in cases {
            assert!(
                select(&serde_json::to_vec(&value).unwrap(), LINUX, "flere").is_err(),
                "accepted {value}"
            );
        }
        let mut largest = document();
        largest["targets"][LINUX]["version"] = json!("2147483647.0.2147483647");
        assert!(select(&serde_json::to_vec(&largest).unwrap(), LINUX, "flere").is_ok());
    }

    #[test]
    fn duplicate_and_unknown_fields_are_rejected_at_each_level() {
        let raw = serde_json::to_string(&document()).unwrap();
        for (needle, replacement) in [
            (
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1",
            ),
            ("\"targets\":{", "\"targets\":{},\"targets\":{"),
            (
                "\"x86_64-unknown-linux-gnu\":{",
                "\"x86_64-unknown-linux-gnu\":{\"policy\":\"unavailable\"},\"x86_64-unknown-linux-gnu\":{",
            ),
            (
                "\"policy\":\"current\"",
                "\"policy\":\"current\",\"policy\":\"current\"",
            ),
            (
                "\"version\":\"0.3.6\"",
                "\"version\":\"0.3.6\",\"version\":\"0.3.6\"",
            ),
            ("\"manifests\":{", "\"manifests\":{},\"manifests\":{"),
            ("\"flere\":{", "\"flere\":{},\"flere\":{"),
            ("\"bytes\":42", "\"bytes\":42,\"bytes\":42"),
            ("\"sha256\":", "\"sha256\":\"ignored\",\"sha256\":"),
            (
                "\"schema_version\":1",
                "\"schema_version\":1,\"url\":\"https://foreign\"",
            ),
            (
                "\"targets\":{",
                "\"targets\":{\"foreign\":{\"policy\":\"unavailable\"},",
            ),
            (
                "\"policy\":\"current\"",
                "\"policy\":\"current\",\"command\":\"run\"",
            ),
            ("\"manifests\":{", "\"manifests\":{\"other\":{},"),
            ("\"bytes\":42", "\"bytes\":42,\"url\":\"https://foreign\""),
        ] {
            assert!(raw.contains(needle));
            assert!(
                select(
                    raw.replacen(needle, replacement, 1).as_bytes(),
                    LINUX,
                    "flere"
                )
                .is_err(),
                "accepted {replacement}"
            );
        }
        assert!(select(&vec![b' '; MAX_BYTES + 1], LINUX, "flere").is_err());
    }
    #[test]
    fn explicit_update_selects_new_channel_once_and_rejects_legacy_or_downgrade() {
        let mut value = document();
        let old = serde_json::to_vec(&value).unwrap();
        let frozen = select_update(&old, LINUX, "flere", Some("0.3.5")).unwrap();
        value["targets"][LINUX]["version"] = json!("0.3.7");
        let new = serde_json::to_vec(&value).unwrap();
        let next = select_update(&new, LINUX, "flere", Some("0.3.6")).unwrap();
        assert_eq!(frozen.version, "0.3.6");
        assert_eq!(next.version, "0.3.7");
        assert_ne!(frozen.url, next.url);
        assert!(select_update(&old, LINUX, "flere", Some("0.3.7")).is_err());
        assert!(select_update(&old, LINUX, "flere", Some("bad")).is_err());
        assert!(select_update(&old, ARM_MAC, "flere", None).is_err());
        assert!(select(&old, ARM_MAC, "flere").unwrap().legacy_unsigned);
        assert!(frozen.pin.verify(&[0; 42], &"a".repeat(64)).is_ok());
        assert!(frozen.pin.verify(&[0; 41], &"a".repeat(64)).is_err());
        assert!(frozen.pin.verify(&[0; 42], &"b".repeat(64)).is_err());
        assert!(
            frozen
                .verify_identity("0.3.6", LINUX, "flere-connect")
                .is_err()
        );
        assert!(frozen.verify_identity("0.3.6", WINDOWS, "flere").is_err());
        assert!(frozen.verify_identity("0.3.7", LINUX, "flere").is_err());
    }
}
