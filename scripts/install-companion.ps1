# First-time Windows companion installation; no source checkout/Rust/SSH login.
# Trust comes from the explicitly chosen HTTPS release channel. An adjacent
# SHA-256 detects corruption, not an independent publisher signature. Publishing
# this bootstrap and actual Windows runtime acceptance remain separate actions.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ManifestUrl,
    [switch]$Adopt
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
Add-Type -AssemblyName System.Net.Http

function Assert-Https([string]$Text, [bool]$Source) {
    $uri = [Uri]$Text
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or $uri.UserInfo -or
        $Text.Length -gt 4096 -or $Text -match '[\x00-\x20\x7f-\uffff\\]') {
        throw 'Choose an explicit HTTPS URL without credentials or control characters.'
    }
    if ($Source -and ($uri.Query -or $uri.Fragment -or -not $uri.AbsolutePath.EndsWith('manifest.json'))) {
        throw 'Choose a manifest.json URL without a query or fragment.'
    }
    return $uri
}

function Receive-Bounded([string]$Url, [long]$Limit, [string]$Destination) {
    $handler = New-Object System.Net.Http.HttpClientHandler
    $handler.AllowAutoRedirect = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(30)
    $response = $null
    $inputStream = $null
    $outputStream = $null
    $watch = [Diagnostics.Stopwatch]::StartNew()
    try {
        $uri = Assert-Https $Url $false
        for ($redirect = 0; $redirect -le 5; $redirect++) {
            $task = $client.GetAsync($uri, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead)
            $response = $task.GetAwaiter().GetResult()
            $status = [int]$response.StatusCode
            if ($status -ge 300 -and $status -lt 400) {
                if ($redirect -eq 5 -or -not $response.Headers.Location) { throw 'Release redirect limit reached.' }
                $next = [Uri]::new($uri, $response.Headers.Location)
                $uri = Assert-Https $next.AbsoluteUri $false
                $response.Dispose()
                $response = $null
                continue
            }
            if ($status -ne 200) { throw "Release download returned HTTP $status." }
            break
        }
        if ($response.Content.Headers.ContentLength -and $response.Content.Headers.ContentLength -gt $Limit) {
            throw 'Release download exceeds its declared bound.'
        }
        $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
        if ($Destination) {
            $outputStream = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        } else {
            $outputStream = New-Object IO.MemoryStream
        }
        $buffer = New-Object byte[] 65536
        [long]$length = 0
        while ($true) {
            [int]$remaining = 120000 - $watch.ElapsedMilliseconds
            if ($remaining -le 0) { throw 'Release download timed out.' }
            $read = $inputStream.ReadAsync($buffer, 0, $buffer.Length)
            if (-not $read.Wait($remaining)) { throw 'Release download timed out.' }
            $count = $read.Result
            if ($count -eq 0) { break }
            $length += $count
            if ($length -gt $Limit) { throw 'Release download exceeds its declared bound.' }
            $outputStream.Write($buffer, 0, $count)
        }
        $outputStream.Flush()
        if (-not $Destination) { return ,($outputStream.ToArray()) }
        return $length
    } finally {
        if ($outputStream) { $outputStream.Dispose() }
        if ($inputStream) { $inputStream.Dispose() }
        if ($response) { $response.Dispose() }
        $client.Dispose()
        $handler.Dispose()
    }
}

$source = Assert-Https $ManifestUrl $true
if ($env:OS -ne 'Windows_NT' -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -or [IntPtr]::Size -ne 8) {
    throw 'This bootstrap requires native Windows x86_64 PowerShell; no core package runs on Windows.'
}
[byte[]]$raw = Receive-Bounded $source.AbsoluteUri 65536 ''
$manifest = [Text.Encoding]::UTF8.GetString($raw) | ConvertFrom-Json
$payload = $manifest.payload
if ($manifest.schema_version -ne 1 -or $manifest.build.component -ne 'flere-connect' -or
    $manifest.build.target -notin @('x86_64-pc-windows-gnu', 'x86_64-pc-windows-msvc') -or $payload.file_name -ne 'flere-connect' -or
    ($payload.bytes -isnot [int] -and $payload.bytes -isnot [long]) -or $payload.bytes -le 0 -or $payload.bytes -gt 268435456 -or
    $payload.bytes -ne [long]$payload.bytes -or $payload.sha256 -cnotmatch '^[0-9a-f]{64}$') {
    throw 'Release component, target, payload size or SHA-256 is invalid.'
}
$asset = $payload.file_name
if ($payload.PSObject.Properties.Name -contains 'download_file' -and $payload.download_file) {
    $asset = $payload.download_file
}
if ($asset -cnotmatch '^[A-Za-z0-9][A-Za-z0-9_.-]{0,255}$') { throw 'Invalid release payload basename.' }
if (-not $env:LOCALAPPDATA -or -not [IO.Path]::IsPathRooted($env:LOCALAPPDATA)) {
    throw 'LOCALAPPDATA must name the current user application-data directory.'
}
$downloads = Join-Path $env:LOCALAPPDATA 'Flere\downloads'
[IO.Directory]::CreateDirectory($downloads) | Out-Null
if (([IO.File]::GetAttributes($downloads) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'The download directory must not be a link or reparse point.'
}
$package = Join-Path $downloads ('bootstrap-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($package) | Out-Null
try {
    $binary = Join-Path $package 'flere-connect'
    $payloadUrl = [Uri]::new($source, $asset)
    $length = Receive-Bounded $payloadUrl.AbsoluteUri ([long]$payload.bytes) $binary
    if ($length -ne $payload.bytes -or (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash.ToLowerInvariant() -cne $payload.sha256) {
        throw 'Downloaded companion differs from its declared size or SHA-256.'
    }
    [IO.File]::WriteAllBytes((Join-Path $package 'manifest.json'), $raw)
    # Windows requires .exe to execute; the installer consumes the shared fixed
    # package basename, verifies metadata again, and retains its own immutable copy.
    $bootstrap = Join-Path $package 'bootstrap.exe'
    [IO.File]::Copy($binary, $bootstrap, $false)
    $arguments = @('install', $package, '--source-url', $source.AbsoluteUri)
    if ($Adopt) { $arguments += '--adopt' }
    & $bootstrap @arguments
    if ($LASTEXITCODE -ne 0) { throw "Companion installation returned $LASTEXITCODE." }
    $installed = Join-Path $env:LOCALAPPDATA 'Flere\bin\flere.exe'
    $alias = Join-Path $env:LOCALAPPDATA 'Flere\bin\flere-connect.exe'
    if (-not [IO.File]::Exists($installed) -or -not [IO.File]::Exists($alias)) {
        throw 'The managed Flere primary command or companion alias is missing.'
    }
    $onPath = Get-Command flere -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $onPath -or $onPath.Source -ne $installed) {
        Write-Warning "Use $installed explicitly, or add its directory to PATH. No shell configuration was changed."
    }
    Write-Host 'Installed Flere. Use flere ssh ALIAS; flere-connect remains an equivalent companion launcher.'
} finally {
    if ([IO.Directory]::Exists($package)) { [IO.Directory]::Delete($package, $true) }
}
