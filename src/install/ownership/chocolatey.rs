//! Passive Chocolatey ownership evidence; never selects or executes a command.
use serde_json::Value;
use std::collections::HashSet;

pub(crate) fn supported_version(value: &str) -> bool {
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts[0] == "2"
        && parts[1..].iter().all(|part| {
            !part.is_empty() && part.len() <= 8 && part.bytes().all(|b| b.is_ascii_digit())
        })
}

pub(crate) fn installed(output: &str, tool_version: &str, package_version: &str) -> bool {
    if output.len() > 65536 || !supported_version(tool_version) {
        return false;
    }
    let mut ids = HashSet::new();
    let mut tool = false;
    let mut package = false;
    for line in output.trim_start_matches('\u{feff}').lines() {
        let Some((id, version)) = line.split_once('|') else {
            return false;
        };
        if id.is_empty()
            || id.len() > 128
            || !id.as_bytes()[0].is_ascii_alphanumeric()
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || version.is_empty()
            || version.len() > 128
            || !version.as_bytes()[0].is_ascii_digit()
            || !version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".+_-".contains(&b))
            || !ids.insert(id.to_ascii_lowercase())
        {
            return false;
        }
        if id.eq_ignore_ascii_case("chocolatey") {
            if version != tool_version {
                return false;
            }
            tool = true;
        }
        if id.eq_ignore_ascii_case("flere-connect") {
            if version != package_version {
                return false;
            }
            package = true;
        }
    }
    tool && package
}

pub(crate) fn manifest_matches(manifest: &Value, build: &Value, bytes: u64, sha256: &str) -> bool {
    manifest["schema_version"] == 1
        && build["component"] == "flere-connect"
        && build["profile"] == "release"
        && matches!(
            build["target"].as_str(),
            Some("x86_64-pc-windows-msvc" | "x86_64-pc-windows-gnu")
        )
        && manifest["build"] == *build
        && manifest["payload"]["file_name"] == "flere-connect"
        && manifest["payload"]["bytes"].as_u64() == Some(bytes)
        && manifest["payload"]["sha256"].as_str() == Some(sha256)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_requires_exact_payload_and_full_running_build() {
        let build = serde_json::json!({"component":"flere-connect", "profile":"release",
            "target":"x86_64-pc-windows-msvc", "build_id":"current"});
        let manifest = serde_json::json!({"schema_version":1, "build":build,
            "payload":{"file_name":"flere-connect", "bytes":123, "sha256":"expected"}});
        assert!(manifest_matches(&manifest, &build, 123, "expected"));
        assert!(!manifest_matches(&manifest, &build, 124, "expected"));
        assert!(!manifest_matches(&manifest, &build, 123, "different"));
        let mut stale = build.clone();
        stale["build_id"] = "older".into();
        assert!(!manifest_matches(&manifest, &stale, 123, "expected"));
    }
    #[test]
    fn captured_version_and_list_shape_require_unique_current_package() {
        let captured = "chocolatey|2.7.4\r\nflere-connect|0.3.4\r\n";
        assert!(installed(captured, "2.7.4", "0.3.4"));
        for output in [
            "chocolatey|2.7.4\n",
            "chocolatey|2.7.4\nflere-connect|0.3.3\n",
            "chocolatey|2.7.4\nflere-connect|0.3.4\nFLERE-CONNECT|0.3.4\n",
            "Chocolatey v2.7.4\nflere-connect|0.3.4\n",
            "chocolatey|2.7.4\nflere-connect|0.3.4|extra\n",
        ] {
            assert!(!installed(output, "2.7.4", "0.3.4"));
        }
        assert!(!installed(captured, "1.4.0", "0.3.4"));
        assert!(!installed(captured, "3.0.0", "0.3.4"));
        assert!(!installed(captured, "2.7.3", "0.3.4"));
    }
}
