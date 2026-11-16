# VIDEO-04 non-release experiment. Windows 10+ only.
# Download a pinned upstream executable into a disposable CI directory.
# NEVER copy it into Akula's NSIS bundle, the user's workspace, or release assets.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ScratchRoot
)
$ErrorActionPreference = 'Stop'
$pinVersion = '2026.10.26'
$expectedSha256 = 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d'
$source = "https://github.com/yt-dlp/yt-dlp/releases/download/$pinVersion/yt-dlp.exe"

function Assert-PinnedHash([string]$Path, [string]$Expected) {
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "VIDEO04_MISSING_PINNED_BINARY: $Path"
    }
    $observed = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($observed -cne $Expected.ToLowerInvariant()) {
        throw "VIDEO04_REJECTED_HASH: expected=$Expected actual=$observed"
    }
}

# A failed hash must prevent *all* execution, including version probes.
$trial = Join-Path $ScratchRoot ('video04-pin-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $trial | Out-Null
try {
    $corrupt = Join-Path $trial 'tampered-not-an-executable.exe'
    [IO.File]::WriteAllText($corrupt, 'untrusted bytes')
    $rejected = $false
    try {
        Assert-PinnedHash $corrupt $expectedSha256
    } catch {
        $rejected = $_.Exception.Message.Contains('VIDEO04_REJECTED_HASH')
    }
    if (!$rejected) {
        throw 'VIDEO04_HASH_FAIL_CLOSED_REGRESSION: corrupted input accepted'
    }
    Write-Host 'VIDEO04_NEGATIVE_HASH_TEST_PASS no_binary_execution=1'

    $exe = Join-Path $trial 'yt-dlp.exe'
    Invoke-WebRequest -Uri $source -OutFile $exe -MaximumRedirection 8 -UseBasicParsing
    Assert-PinnedHash $exe $expectedSha256

    # Version-only process: no URL, no media download, no network challenges.
    $versionOutput = @(& $exe --version 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "VIDEO04_PINNED_BINARY_FAILED: exit=$LASTEXITCODE"
    }
    $observedVersion = ($versionOutput | Out-String).Trim()
    if ($observedVersion -cne $pinVersion) {
        throw "VIDEO04_VERSION_MISMATCH: expected=$pinVersion actual=$observedVersion"
    }

    Write-Host "VIDEO04_PINNED_YTDLP_EXE_PASS version=$pinVersion sha256=$expectedSha256"
    Write-Host 'VIDEO04_EXPERIMENT_SCOPE host=windows-latest download_media=0 bundle_release=0'
    Write-Host 'VIDEO04_NOT_ACCEPTED ffmpeg=unchecked js_runtime=unchecked win7=unsupported gui=unchecked'
} finally {
    Remove-Item -LiteralPath $trial -Recurse -Force -ErrorAction SilentlyContinue
}
