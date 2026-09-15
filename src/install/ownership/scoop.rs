//! Passive records for default per-user Scoop installs from local files or buckets.
//! These values never select a process, script, download or profile directory.
use serde_json::Value;

const PRESERVE_MANIFEST: &str = r#"Rename-Item -LiteralPath "$dir\manifest.json" -NewName 'flere-release.manifest.json' -ErrorAction Stop"#;

// A passive single directory name, never opened, interpolated or executed. The
// installed record identifies Scoop ownership, not a trusted bucket publisher.
fn bucket_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !matches!(value, "." | "..")
        && !value.ends_with(['.', ' '])
        && !value.chars().any(|c| {
            c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
        })
}

pub(crate) fn records(install: &Value, manifest: &Value, version: &str) -> bool {
    let Some(info) = install.as_object() else {
        return false;
    };
    // Scoop's save_install_info drops null fields: normal bucket installs and
    // updates retain `bucket`, while the observed local-file path retains `url`.
    let source_matches = match (info.get("url"), info.get("bucket")) {
        (Some(Value::String(path)), None) => {
            super::windows_profile::literal_path(path)
                && path.to_ascii_lowercase().ends_with(".json")
        }
        (None, Some(Value::String(bucket))) => bucket_name(bucket),
        _ => false,
    };
    if info.len() != 2
        || info.get("architecture").and_then(Value::as_str) != Some("64bit")
        || !source_matches
    {
        return false;
    }
    let Some(recipe) = manifest.as_object() else {
        return false;
    };
    if recipe.keys().any(|key| {
        ![
            "version",
            "description",
            "homepage",
            "license",
            "architecture",
            "bin",
            "pre_install",
            "notes",
        ]
        .contains(&key.as_str())
    }) || manifest["version"].as_str() != Some(version)
        || manifest["license"] != "MIT"
        || manifest["pre_install"].as_str() != Some(PRESERVE_MANIFEST)
        || manifest["bin"] != serde_json::json!(["flere.exe", "flere-connect.exe"])
    {
        return false;
    }
    let Some(architectures) = manifest["architecture"].as_object() else {
        return false;
    };
    let Some(payload) = manifest["architecture"]["64bit"].as_object() else {
        return false;
    };
    // The declared ZIP URL/hash are passive package metadata, not live integrity
    // evidence. The preserved release manifest binds the running payload below.
    architectures.len() == 1
        && payload.len() == 2
        && payload
            .get("hash")
            .and_then(Value::as_str)
            .is_some_and(|hash| {
                hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        && payload
            .get("url")
            .and_then(Value::as_str)
            .is_some_and(|url| {
                url.len() <= 4096
                    && !url.chars().any(|c| c.is_control() || c.is_whitespace())
                    && url
                        .strip_prefix("https://")
                        .or_else(|| url.strip_prefix("http://"))
                        .is_some_and(|rest| !rest.is_empty() && !rest.starts_with('/'))
            })
}

// Construct the observed Windows spelling directly, including on non-Windows
// test hosts. PathBuf::join("apps/flere/current") would retain mixed separators.
pub(crate) fn current_alias(profile: &str, alias: &str) -> Option<String> {
    (super::windows_profile::literal_path(profile)
        && (alias.eq_ignore_ascii_case("flere.exe")
            || alias.eq_ignore_ascii_case("flere-connect.exe")))
    .then(|| format!(r"{profile}\scoop\apps\flere\current\{alias}"))
}

pub(crate) fn shim_matches(text: &str, expected: &str) -> bool {
    let line = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    text.len() <= 4096
        && line
            .strip_prefix("path = \"")
            .and_then(|s| s.strip_suffix('"'))
            .is_some_and(|path| {
                super::windows_profile::literal_path(path) && path.eq_ignore_ascii_case(expected)
            })
}

#[cfg(windows)]
pub(crate) fn profile_query() -> String {
    [
        "$ErrorActionPreference='Stop';\n[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);\n",
        super::windows_profile::FUNCTION,
        "[Console]::Out.WriteLine((GetFlereProfile));\n",
    ].concat()
}

#[cfg(any(windows, test))]
pub(crate) fn profile(output: &str) -> Option<&str> {
    let output = output.trim_start_matches('\u{feff}');
    let path = output
        .strip_suffix("\r\n")
        .or_else(|| output.strip_suffix('\n'))
        .unwrap_or(output);
    super::windows_profile::literal_path(path).then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    // Native Scoop run 34935706908's records, with the user's path replaced by a
    // disposable fixture spelling. ZIP metadata is the actual public v0.3.4.
    fn captured() -> (Value, Value) {
        (
            serde_json::json!({"architecture":"64bit", "url":r"C:\Users\fixture\.cache\flere\tmp\work\flere.json"}),
            serde_json::json!({
                "version":"0.3.4", "description":"OpenSSH and clipboard companion for a Linux or macOS Flere workbench",
                "homepage":"https://github.com/robert-cronin/flere", "license":"MIT",
                "architecture":{"64bit":{
                    "url":"https://github.com/robert-cronin/flere/releases/download/v0.3.4/flere-connect-0.3.4-x86_64-pc-windows-msvc.zip",
                    "hash":"71fd96f768894d849c4773add52bd0e3131e032c77640a02aa1c4af36e5001db"}},
                "bin":["flere.exe","flere-connect.exe"], "pre_install":PRESERVE_MANIFEST,
                "notes":["Requires an existing OpenSSH client and a Linux or macOS SSH host."]
            }),
        )
    }

    #[test]
    fn profile_reply_is_one_bounded_literal_path_not_environment_or_extra_output() {
        assert_eq!(profile("C:\\Users\\fixture\r\n"), Some(r"C:\Users\fixture"));
        assert_eq!(
            profile("\u{feff}D:\\Profiles\\Zoë\n"),
            Some(r"D:\Profiles\Zoë")
        );
        for value in [
            "",
            r"%USERPROFILE%",
            r"C:\Users\..\other",
            r"\\host\share",
            "C:\\Users\\fixture\nother",
            "C:\\Users\\fixture\n\n",
        ] {
            assert!(profile(value).is_none(), "accepted {value:?}");
        }
        assert!(profile(&format!("C:\\{}", "x".repeat(4097))).is_none());
    }

    #[test]
    fn captured_local_manifest_and_both_current_alias_shims_match() {
        let (install, manifest) = captured();
        assert!(current_alias(r"%USERPROFILE%", "flere.exe").is_none());
        assert!(current_alias(r"C:\Users\fixture", r"..\flere.exe").is_none());
        assert!(records(&install, &manifest, "0.3.4"));
        for alias in ["flere.exe", "flere-connect.exe"] {
            let expected = current_alias(r"C:\Users\fixture", alias).unwrap();
            assert_eq!(
                expected,
                format!(r"C:\Users\fixture\scoop\apps\flere\current\{alias}")
            );
            assert!(!expected.contains('/'));
            assert!(shim_matches(
                &format!("path = \"{expected}\"\r\n"),
                &expected
            ));
            assert!(shim_matches(
                &format!("path = \"{}\"\n", expected.to_ascii_lowercase()),
                &expected
            ));
        }
        // The source recipe may have been removed; its path is never opened.
        let mut moved_recipe = install.clone();
        moved_recipe["url"] = r"D:\Removed recipe\Zoë\flere.json".into();
        assert!(records(&moved_recipe, &manifest, "0.3.4"));
    }

    #[test]
    fn ordinary_bucket_install_and_update_records_match_the_reviewed_recipe() {
        // Derived from Scoop b588a06e's Get-Manifest/save_install_info and update,
        // not a claim that our local-manifest native run installed from a bucket.
        for version in ["0.3.4", "0.3.5"] {
            let (_, mut manifest) = captured();
            manifest["version"] = version.into();
            manifest["architecture"]["64bit"]["url"] = format!(
                "https://github.com/robert-cronin/flere/releases/download/v{version}/flere-connect-{version}-x86_64-pc-windows-msvc.zip"
            ).into();
            for bucket in ["flere", "main", "team-bucket", "Team.bucket_2", "Scoop Zoë"] {
                let install = serde_json::json!({"architecture":"64bit", "bucket":bucket});
                assert!(records(&install, &manifest, version), "rejected {bucket}");
                assert!(!records(&install, &manifest, "0.0.0"));
            }
        }
    }

    #[test]
    fn malformed_or_ambiguous_bucket_sources_stay_unknown() {
        let (_, manifest) = captured();
        for bucket in [
            serde_json::json!(null),
            serde_json::json!(false),
            serde_json::json!(42),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!(""),
            serde_json::json!("."),
            serde_json::json!(".."),
            serde_json::json!("../flere"),
            serde_json::json!(r"other\flere"),
            serde_json::json!(r"C:\bucket"),
            serde_json::json!("https://example.com/bucket"),
            serde_json::json!("flere\nother"),
            serde_json::json!("flere\u{1b}"),
            serde_json::json!("flere."),
            serde_json::json!("flere "),
            serde_json::json!("*"),
            serde_json::json!("x".repeat(256)),
        ] {
            let install = serde_json::json!({"architecture":"64bit", "bucket":bucket});
            assert!(
                !records(&install, &manifest, "0.3.4"),
                "accepted {bucket:?}"
            );
        }
        for install in [
            serde_json::json!({"architecture":"64bit"}),
            serde_json::json!({"architecture":"32bit", "bucket":"flere"}),
            serde_json::json!({"architecture":"64bit", "bucket":"flere", "global":true}),
            serde_json::json!({"architecture":"64bit", "bucket":"flere", "url":null}),
            serde_json::json!({"architecture":"64bit", "bucket":"flere", "url":r"C:\work\flere.json"}),
        ] {
            assert!(!records(&install, &manifest, "0.3.4"), "accepted {install}");
        }
    }

    #[test]
    fn foreign_install_fields_and_recipe_identity_stay_unknown() {
        let (install, manifest) = captured();
        assert!(!records(&install, &manifest, "0.3.5"));
        for (key, value) in [
            ("architecture", serde_json::json!("32bit")),
            ("bucket", serde_json::json!("main")),
            ("global", serde_json::json!(true)),
            ("url", serde_json::json!("https://example.com/flere.json")),
            ("url", serde_json::json!(r"%USERPROFILE%\flere.json")),
            ("url", serde_json::json!(r"C:\work\..\flere.json")),
        ] {
            let mut changed = install.clone();
            changed[key] = value;
            assert!(!records(&changed, &manifest, "0.3.4"), "accepted {key}");
        }
        for (key, value) in [
            ("version", serde_json::json!("0.3.3")),
            ("license", serde_json::json!("Other")),
            ("bin", serde_json::json!(["flere.exe"])),
            ("pre_install", serde_json::json!("Remove-Item app")),
            ("post_install", serde_json::json!("other.exe")),
            (
                "architecture",
                serde_json::json!({"arm64":{"url":"https://example.com/a.zip", "hash":"a".repeat(64)}}),
            ),
        ] {
            let mut changed = manifest.clone();
            changed[key] = value;
            for source in [
                install.clone(),
                serde_json::json!({"architecture":"64bit", "bucket":"flere"}),
            ] {
                assert!(!records(&source, &changed, "0.3.4"), "accepted {key}");
            }
        }
        for value in [
            serde_json::json!(null),
            serde_json::json!([]),
            serde_json::json!({}),
        ] {
            assert!(!records(&value, &manifest, "0.3.4"));
            assert!(!records(&install, &value, "0.3.4"));
        }
        let mut changed = manifest.clone();
        changed["architecture"]["64bit"]["hash"] = "invalid".into();
        assert!(!records(&install, &changed, "0.3.4"));
    }

    #[test]
    fn shim_cannot_redirect_to_old_version_foreign_alias_or_extra_arguments() {
        let expected = r"C:\Users\fixture\scoop\apps\flere\current\flere.exe";
        let good = format!("path = \"{expected}\"\r\n");
        for text in [
            String::new(),
            good.replace("current", "0.3.3"),
            good.replace("flere.exe", "flere-connect.exe"),
            good.replace("fixture", "other"),
            format!("{good}args = \"--update\"\r\n"),
            format!("{good}{good}"),
            good.replace("path = ", "target = "),
            good.replace("fixture", "fixt\u{1b}ure"),
            good.replace("fixture", &"a".repeat(4097)),
            good.replace("current", r"..\current"),
            good.replace("C:", "%SCOOP%"),
        ] {
            assert!(!shim_matches(&text, expected), "accepted {text:?}");
        }
    }
}
