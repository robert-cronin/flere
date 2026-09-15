//! Passive user-scope WinGet portable records; no record selects a command.
use std::collections::HashMap;

pub(crate) const PRODUCT_CODE: &str = "RobertCronin.FlereConnect__DefaultSource";
const COMMUNITY_PRODUCT_CODE: &str =
    "RobertCronin.FlereConnect_Microsoft.Winget.Source_8wekyb3d8bbwe";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    LocalManifest,
    Community,
}

impl Origin {
    pub(crate) fn for_package(name: &str) -> Option<Self> {
        [Self::LocalManifest, Self::Community]
            .into_iter()
            .find(|origin| name.eq_ignore_ascii_case(origin.product_code()))
    }

    pub(crate) fn product_code(self) -> &'static str {
        match self {
            Self::LocalManifest => PRODUCT_CODE,
            Self::Community => COMMUNITY_PRODUCT_CODE,
        }
    }

    fn source_identifier(self) -> &'static str {
        match self {
            Self::LocalManifest => "*DefaultSource",
            Self::Community => "Microsoft.Winget.Source_8wekyb3d8bbwe",
        }
    }

    fn uninstall_string(self) -> &'static str {
        match self {
            Self::LocalManifest => {
                "winget uninstall --product-code RobertCronin.FlereConnect__DefaultSource"
            }
            Self::Community => {
                "winget uninstall --product-code RobertCronin.FlereConnect_Microsoft.Winget.Source_8wekyb3d8bbwe"
            }
        }
    }
}
const FIELDS: &[&str] = &[
    "schema",
    "local_appdata",
    "DisplayName",
    "DisplayVersion",
    "WinGetPackageIdentifier",
    "WinGetSourceIdentifier",
    "WinGetInstallerType",
    "InstallLocation",
    "UninstallString",
    "InstallDirectoryCreated",
    "InstallDirectoryAddedToPath",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Record<'a> {
    pub local_appdata: &'a str,
    pub install_location: &'a str,
}

pub(crate) fn record<'a>(
    output: &'a str,
    package_version: &str,
    origin: Origin,
) -> Option<Record<'a>> {
    if output.len() > 65536 {
        return None;
    }
    let mut fields = HashMap::new();
    for line in output.trim_start_matches('\u{feff}').lines() {
        let (name, value) = line.split_once('\t')?;
        if !FIELDS.contains(&name)
            || value.is_empty()
            || value.len() > 4096
            || value.chars().any(char::is_control)
            || fields.insert(name, value).is_some()
        {
            return None;
        }
    }
    if fields.len() != FIELDS.len()
        || fields["schema"] != "flere-winget-owner-v1"
        || !default_local_appdata(fields["local_appdata"])
        || fields["DisplayName"] != "Flere Connect"
        || fields["DisplayVersion"] != package_version
        || fields["WinGetPackageIdentifier"] != "RobertCronin.FlereConnect"
        || fields["WinGetSourceIdentifier"] != origin.source_identifier()
        || fields["WinGetInstallerType"] != "portable"
        || fields["InstallDirectoryCreated"] != "1"
        || fields["InstallDirectoryAddedToPath"] != "0"
        || fields["UninstallString"] != origin.uninstall_string()
    {
        return None;
    }
    Some(Record {
        local_appdata: fields["local_appdata"],
        install_location: fields["InstallLocation"],
    })
}

// This detector supports the normal local profile default only. Validate the
// unexpanded ProfileImagePath plus fixed suffix as data, on every platform, so
// process environment variables cannot redirect the ownership root.
fn default_local_appdata(value: &str) -> bool {
    value
        .strip_suffix(r"\AppData\Local")
        .is_some_and(super::windows_profile::literal_path)
}

// Query only the current-token SID's HKLM/64 ProfileList record and one fixed active
// HKCU/64 package key selected by a fixed origin, never record text. Read ProfileImagePath without expanding inherited
// variables, then append the default AppData\Local suffix; redirected package
// roots are unsupported. The parser checks a bounded literal local profile path.
// No record selects a hive, program or script. The caller bounds time/output.
// Use only .NET APIs: Windows PowerShell may inherit PowerShell 7's module path.
#[cfg(windows)]
const QUERY: &str = r#"$ErrorActionPreference='Stop';
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);
function Emit([string]$name,[string]$value) {
    if (!$value -or [Text.Encoding]::UTF8.GetByteCount($value) -gt 4096) { throw 'invalid owner field length' }
    foreach ($character in $value.ToCharArray()) { if ([char]::IsControl($character)) { throw 'invalid owner field control' } }
    [Console]::Out.WriteLine($name + "`t" + $value);
}
$root=[Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::CurrentUser,[Microsoft.Win32.RegistryView]::Registry64);
try {
    $key=$root.OpenSubKey('Software\Microsoft\Windows\CurrentVersion\Uninstall\RobertCronin.FlereConnect__DefaultSource',$false);
    if ($null -eq $key) { throw 'active WinGet record missing' }
    try {
        Emit 'schema' 'flere-winget-owner-v1';
        Emit 'local_appdata' ((GetFlereProfile)+'\AppData\Local');
        foreach ($name in @('DisplayName','DisplayVersion','WinGetPackageIdentifier','WinGetSourceIdentifier','WinGetInstallerType','InstallLocation','UninstallString')) {
            if ($key.GetValueKind($name) -ne [Microsoft.Win32.RegistryValueKind]::String) { throw 'invalid owner field type' }
            Emit $name ($key.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames));
        }
        foreach ($name in @('InstallDirectoryCreated','InstallDirectoryAddedToPath')) {
            $value=$key.GetValue($name,$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames);
            if ($null -eq $value -and $name -eq 'InstallDirectoryAddedToPath') { $value=0 }
            elseif ($null -eq $value -or $key.GetValueKind($name) -ne [Microsoft.Win32.RegistryValueKind]::DWord) { throw 'invalid owner flag type' }
            Emit $name ([Convert]::ToString($value,[Globalization.CultureInfo]::InvariantCulture));
        }
    } finally { $key.Dispose() }
} finally { $root.Dispose() }
"#;

#[cfg(windows)]
pub(crate) fn query(origin: Origin) -> String {
    // Replacement contains only one of the two fixed product-code constants.
    [
        super::windows_profile::FUNCTION,
        &QUERY.replace(PRODUCT_CODE, origin.product_code()),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values from native WinGet 1.11.510 run 34929525551, HKCU/64. The
    // LocalAppData prefix is synthetic; native query obtains it from Windows.
    fn captured() -> String {
        concat!(
            "schema\tflere-winget-owner-v1\r\n",
            "local_appdata\tC:\\Users\\fixture\\AppData\\Local\r\n",
            "DisplayName\tFlere Connect\r\n",
            "DisplayVersion\t0.3.4\r\n",
            "WinGetPackageIdentifier\tRobertCronin.FlereConnect\r\n",
            "WinGetSourceIdentifier\t*DefaultSource\r\n",
            "WinGetInstallerType\tportable\r\n",
            "InstallLocation\tC:\\Users\\fixture\\AppData\\Local\\Microsoft\\WinGet\\Packages\\RobertCronin.FlereConnect__DefaultSource\r\n",
            "UninstallString\twinget uninstall --product-code RobertCronin.FlereConnect__DefaultSource\r\n",
            "InstallDirectoryCreated\t1\r\n",
            "InstallDirectoryAddedToPath\t0\r\n",
        ).into()
    }

    #[test]
    fn captured_active_record_retains_exact_paths_and_local_source() {
        let text = captured();
        let actual = record(&text, "0.3.4", Origin::LocalManifest).unwrap();
        assert_eq!(actual.local_appdata, r"C:\Users\fixture\AppData\Local");
        assert_eq!(
            actual.install_location,
            format!(
                r"{}\Microsoft\WinGet\Packages\{PRODUCT_CODE}",
                actual.local_appdata
            )
        );
        assert!(record(&text, "0.3.5", Origin::LocalManifest).is_none());
    }

    #[test]
    fn only_literal_drive_absolute_default_profile_paths_are_accepted() {
        let old = r"C:\Users\fixture\AppData\Local";
        for path in [
            old,
            r"D:\Profiles\User Name\AppData\Local",
            r"C:\Users\Zoë\AppData\Local",
        ] {
            let text = captured().replace(old, path);
            assert_eq!(
                record(&text, "0.3.4", Origin::LocalManifest)
                    .unwrap()
                    .local_appdata,
                path
            );
        }
        for path in [
            r"%USERPROFILE%\AppData\Local",
            r"C:\Users\%USERNAME%\AppData\Local",
            r"Users\fixture\AppData\Local",
            r"\Users\fixture\AppData\Local",
            r"\\host\share\fixture\AppData\Local",
            r"\\?\C:\Users\fixture\AppData\Local",
            r"C:Users\fixture\AppData\Local",
            r"C:/Users/fixture/AppData/Local",
            r"C:\Users\..\fixture\AppData\Local",
            r"C:\Users\.\fixture\AppData\Local",
            r"C:\Users\\fixture\AppData\Local",
            r"C:\Users\fixture.\AppData\Local",
            r"C:\Users\fixture \AppData\Local",
            r"C:\Users\fixture:stream\AppData\Local",
            r"C:\Users\fixt*re\AppData\Local",
            r"C:\Users\fixture\RedirectedLocal",
        ] {
            assert!(
                record(
                    &captured().replace(old, path),
                    "0.3.4",
                    Origin::LocalManifest
                )
                .is_none(),
                "accepted {path:?}"
            );
        }
    }

    #[test]
    fn stale_foreign_incomplete_ambiguous_and_unbounded_records_stay_unknown() {
        let text = captured();
        for (old, new) in [
            (
                "schema\tflere-winget-owner-v1",
                "schema\tflere-winget-owner-v2",
            ),
            ("DisplayName\tFlere Connect", "DisplayName\tOther"),
            ("DisplayVersion\t0.3.4", "DisplayVersion\t0.3.3"),
            (
                "WinGetPackageIdentifier\tRobertCronin.FlereConnect",
                "WinGetPackageIdentifier\tOther",
            ),
            (
                "WinGetSourceIdentifier\t*DefaultSource",
                "WinGetSourceIdentifier\tMicrosoft.Winget.Source_8wekyb3d8bbwe",
            ),
            ("WinGetInstallerType\tportable", "WinGetInstallerType\texe"),
            ("InstallDirectoryCreated\t1", "InstallDirectoryCreated\t0"),
            (
                "InstallDirectoryAddedToPath\t0",
                "InstallDirectoryAddedToPath\t1",
            ),
            (
                "UninstallString\twinget uninstall --product-code RobertCronin.FlereConnect__DefaultSource",
                "UninstallString\tother.exe",
            ),
            ("local_appdata\tC:", "local_appdata\tC:\u{1b}"),
        ] {
            assert_ne!(text.replace(old, new), text);
            assert!(
                record(&text.replace(old, new), "0.3.4", Origin::LocalManifest).is_none(),
                "accepted {new:?}"
            );
        }
        for name in FIELDS {
            let missing = text
                .lines()
                .filter(|line| !line.starts_with(&format!("{name}\t")))
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                record(&missing, "0.3.4", Origin::LocalManifest).is_none(),
                "accepted missing {name}"
            );
        }
        for extra in [
            "DisplayVersion\t0.3.4\n",
            "unknown\tx\n",
            "broken\n",
            "local_appdata\t\n",
        ] {
            assert!(record(&(text.clone() + extra), "0.3.4", Origin::LocalManifest).is_none());
        }
        assert!(
            record(
                &text.replace("C:\\Users\\fixture", &"x".repeat(4097)),
                "0.3.4",
                Origin::LocalManifest
            )
            .is_none()
        );
        assert!(record(&"x".repeat(65537), "0.3.4", Origin::LocalManifest).is_none());
        assert!(record("", "0.3.4", Origin::LocalManifest).is_none());
    }

    #[test]
    fn exact_local_and_official_community_package_leaves_select_fixed_origins() {
        for origin in [Origin::LocalManifest, Origin::Community] {
            assert_eq!(Origin::for_package(origin.product_code()), Some(origin));
            assert_eq!(
                Origin::for_package(&origin.product_code().to_ascii_uppercase()),
                Some(origin)
            );
        }
        for name in [
            "",
            "RobertCronin.FlereConnect",
            "RobertCronin.FlereConnect_CustomSource",
            "RobertCronin.FlereConnect_Microsoft.Winget.Platform.Source_8wekyb3d8bbwe",
            "RobertCronin.FlereConnect_Microsoft.Winget.Source_8wekyb3d8bbwe_extra",
            "RobertCronin.FlereConnect__DefaultSource.old",
            r"other\RobertCronin.FlereConnect__DefaultSource",
            "RobertCronin.FlereConnect__DefaultSource/flere.exe",
        ] {
            assert_eq!(Origin::for_package(name), None, "accepted {name:?}");
        }
    }

    #[test]
    fn upstream_derived_community_record_requires_matching_origin_and_product_code() {
        // WinGet 1.11.510 PortableFlow derives package/source identities from
        // the source family in SourceList. This is not native catalogue proof.
        let text = captured()
            .replace(PRODUCT_CODE, COMMUNITY_PRODUCT_CODE)
            .replace("*DefaultSource", Origin::Community.source_identifier());
        let actual = record(&text, "0.3.4", Origin::Community).unwrap();
        assert_eq!(
            actual.install_location,
            format!(
                r"{}\Microsoft\WinGet\Packages\{COMMUNITY_PRODUCT_CODE}",
                actual.local_appdata
            )
        );
        assert!(record(&text, "0.3.4", Origin::LocalManifest).is_none());
        assert!(record(&captured(), "0.3.4", Origin::Community).is_none());
        assert!(record(&text, "0.3.5", Origin::Community).is_none());
        for (from, to) in [
            (Origin::Community.source_identifier(), "*DefaultSource"),
            (Origin::Community.source_identifier(), "CustomSource"),
            (
                Origin::Community.uninstall_string(),
                Origin::LocalManifest.uninstall_string(),
            ),
            (
                "WinGetPackageIdentifier\tRobertCronin.FlereConnect",
                "WinGetPackageIdentifier\tOther",
            ),
        ] {
            assert!(record(&text.replace(from, to), "0.3.4", Origin::Community).is_none());
        }
    }
}
