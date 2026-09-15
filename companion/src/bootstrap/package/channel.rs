use super::{COMPONENT, MAX_JSON, invalid};
use serde::{Deserialize, Deserializer};
use std::io;

pub(super) const URL: &str =
    "https://raw.githubusercontent.com/robert-cronin/flere/main/packaging/channels/stable.json";
pub(super) const MAX_BYTES: usize = 8192;
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
pub(super) struct Pin {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug)]
pub(super) struct Selection {
    pub version: String,
    pub url: String,
    pub pin: Pin,
    pub legacy_unsigned: bool,
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

pub(super) fn select(bytes: &[u8], target: &str) -> io::Result<Selection> {
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
                || pin.bytes > MAX_JSON as u64
                || pin.sha256.len() != 64
                || !pin
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(invalid("Invalid default release manifest size/SHA-256"));
            }
        }
        if name == target
            && let Some(pin) = manifests.core
        {
            selected = Some(Selection {
                url: format!(
                    "https://github.com/robert-cronin/flere/releases/download/v{release}/{COMPONENT}-{name}.manifest.json"
                ),
                version: release,
                pin,
                legacy_unsigned,
            });
        }
    }
    selected.ok_or_else(|| invalid(
        "No default prebuilt Flere core is available for this target; use a reviewed explicit package or the source Homebrew route on macOS",
    ))
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
    fn fixed_targets_select_immutable_core_only_and_explicit_legacy() {
        let bytes = serde_json::to_vec(&document()).unwrap();
        let linux = select(&bytes, LINUX).unwrap();
        assert_eq!(
            linux.url,
            "https://github.com/robert-cronin/flere/releases/download/v0.3.6/flere-x86_64-unknown-linux-gnu.manifest.json"
        );
        assert_eq!(linux.version, "0.3.6");
        assert_eq!(linux.pin.bytes, 42);
        assert!(!linux.legacy_unsigned);
        let mac = select(&bytes, ARM_MAC).unwrap();
        assert_eq!(mac.version, "0.3.0");
        assert!(mac.legacy_unsigned);
        for unavailable in [INTEL_MAC, WINDOWS, "foreign-target"] {
            assert!(select(&bytes, unavailable).is_err());
        }
        assert!(select(br#"{"schema_version":1,"targets":{}}"#, LINUX).is_err());
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
                select(&serde_json::to_vec(&value).unwrap(), LINUX).is_err(),
                "accepted {value}"
            );
        }
        let mut largest = document();
        largest["targets"][LINUX]["version"] = json!("2147483647.0.2147483647");
        assert!(select(&serde_json::to_vec(&largest).unwrap(), LINUX).is_ok());
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
                select(raw.replacen(needle, replacement, 1).as_bytes(), LINUX).is_err(),
                "accepted {replacement}"
            );
        }
        assert!(select(&vec![b' '; MAX_BYTES + 1], LINUX).is_err());
    }
}
