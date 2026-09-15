//! Current-token Windows profile data, independent of inherited home variables.
// Used only for the observed default per-user manager layouts.
pub(crate) fn literal_path(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 4
        && bytes.len() <= 4096
        && !value.chars().any(char::is_control)
        && bytes[0].is_ascii_alphabetic()
        && &bytes[1..3] == b":\\"
        && value[3..].split('\\').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with('.')
                && !part.ends_with(' ')
                && !part
                    .chars()
                    .any(|c| matches!(c, ':' | '/' | '%' | '<' | '>' | '"' | '|' | '?' | '*'))
        })
}

// The caller adds only fixed script text, bounds the child, and validates output.
// No record chooses a key or expands environment variables. .NET only: ordinary
// Windows PowerShell can inherit PowerShell 7's incompatible module lookup path.
#[cfg(windows)]
pub(crate) const FUNCTION: &str = r#"function GetFlereProfile {
    $identity=[Security.Principal.WindowsIdentity]::GetCurrent();
    try { $sid=$identity.User.Value } finally { $identity.Dispose() }
    if (!$sid) { throw 'current user SID missing' }
    $profiles=[Microsoft.Win32.RegistryKey]::OpenBaseKey([Microsoft.Win32.RegistryHive]::LocalMachine,[Microsoft.Win32.RegistryView]::Registry64);
    try {
        $profileKey=$profiles.OpenSubKey('SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList\'+$sid,$false);
        if ($null -eq $profileKey) { throw 'current user profile missing' }
        try {
            $kind=$profileKey.GetValueKind('ProfileImagePath');
            if ($kind -ne [Microsoft.Win32.RegistryValueKind]::String -and $kind -ne [Microsoft.Win32.RegistryValueKind]::ExpandString) { throw 'invalid profile field type' }
            $profilePath=$profileKey.GetValue('ProfileImagePath',$null,[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames);
            if ($profilePath -isnot [string] -or !$profilePath) { throw 'invalid profile path' }
            return $profilePath;
        } finally { $profileKey.Dispose() }
    } finally { $profiles.Dispose() }
}
"#;
