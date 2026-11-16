# VIDEO-17 is restricted to an ephemeral Github-hosted Windows runner.
# It installs an UNSIGNED, non-release NSIS lab candidate; never user paths.
param(
    [Parameter(Mandatory=$true)][string]$Installer,
    [Parameter(Mandatory=$true)][string]$StageDir,
    [Parameter(Mandatory=$true)][string]$FixtureDir
)
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted') {
    throw 'VIDEO-17 refuses real user computers'
}
$root=(Resolve-Path $env:GITHUB_WORKSPACE).Path
$setup=(Resolve-Path -LiteralPath $Installer).Path
$stage=(Resolve-Path -LiteralPath $StageDir).Path
$fixture=(Resolve-Path -LiteralPath $FixtureDir).Path
$temp=(Resolve-Path $env:RUNNER_TEMP).Path
$install=Join-Path $env:LOCALAPPDATA 'Akula'
$registry='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$legacy=Join-Path $env:LOCALAPPDATA 'Aquilum'
if (!$setup.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) -or
    !$stage.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) -or
    !$fixture.StartsWith($root,[StringComparison]::OrdinalIgnoreCase) -or
    (Test-Path -LiteralPath $install) -or (Test-Path $registry) -or
    (Get-Process -Name akula-app -ErrorAction SilentlyContinue)) {
    throw 'VIDEO-17 requires a clean, correct staged source checkout'
}
# Protect pre-existing upstream Aquilum identities; no overwrite even in CI.
$legacyExisting=Test-Path -LiteralPath $legacy
$names=@('akula-app.exe','shiva-pdf-preview.exe','akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe')
$expected=@{}
foreach($name in $names) {
    $source=Join-Path $stage $name
    if (!(Test-Path -LiteralPath $source -PathType Leaf)) { throw "Missing stage binary $name" }
    $metadata=Get-Item -LiteralPath $source
    if ($metadata.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "No linked stage executable: $name"
    }
    $expected[$name]=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
}
if ($expected['akula-video-mux.exe'] -cne $env:AKULA_BUNDLED_MUX_SHA256) {
    throw 'Staged Rust mux is not the exact build-time pin'
}
if ($expected['akula-yt-dlp.exe'] -cne 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d' -or
    $expected['akula-quickjs.exe'] -cne 'ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7') {
    throw 'Untrusted official tools in lab stage'
}
$process=Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
if ($process.ExitCode -ne 0) { throw "NSIS exit code $($process.ExitCode)" }
if (!(Test-Path -LiteralPath $registry)) { throw 'NSIS uninstall registration missing' }
if ((Get-ItemProperty -LiteralPath $registry -Name DisplayName).DisplayName -cne 'Akula') {
    throw 'NSIS product identity drift'
}
foreach($name in $names) {
    $dest=Join-Path $install $name
    if (!(Test-Path -LiteralPath $dest -PathType Leaf)) { throw "NSIS omitted sidecar $name" }
    $metadata=Get-Item -LiteralPath $dest
    if ($metadata.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "NSIS installed linked executable $name"
    }
    $actual=(Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $expected[$name]) { throw "NSIS changed exact stage bytes $name" }
}
if ((Test-Path -LiteralPath $legacy) -ne $legacyExisting) {
    throw 'NSIS changed legacy Aquilum identity'
}
Write-Host 'VIDEO17_REAL_NSIS_INSTALLED_FIVE_PINNED_EXES_PASS'
$vault=Join-Path $temp 'video17-vault'
$data=Join-Path $temp 'video17-data'
$probe=Join-Path $temp 'video16-events.log'
$out=Join-Path $root 'video17-output/video17-nsis.mp4'
if (Test-Path $vault) { throw 'Pre-existing disposable vault' }
if (Test-Path $data) { throw 'Pre-existing disposable profile' }
if (Test-Path $probe) { throw 'Pre-existing GUI evidence' }
New-Item -ItemType Directory -Path (Split-Path $out -Parent) -Force | Out-Null
try {
    & python (Join-Path $root 'labs/video-lite/video16/gui_acceptance.py') --repo $root --manifest-dir $fixture --exe (Join-Path $install 'akula-app.exe') --vault $vault --data $data --probe $probe --output $out
    if ($LASTEXITCODE -ne 0) { throw 'VIDEO-17 installed real-GUI E2E failed' }
    if (!(Test-Path -LiteralPath $out -PathType Leaf)) { throw 'Installed NSIS produced no local MP4' }
    $note=Join-Path $vault 'VIDEO16 Media.md'
    if (!(Test-Path -LiteralPath $note -PathType Leaf)) { throw 'Real GUI never opened test note' }
    $digest=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($digest.Length -ne 64) { throw 'Test note lost after restart' }
    Write-Host 'VIDEO17_INSTALLED_NSIS_REAL_GUI_PASS download=1 restart=1 playback=1'
} finally {
    $uninstaller=Join-Path $install 'uninstall.exe'
    if (!(Get-Process -Name akula-app -ErrorAction SilentlyContinue) -and
        (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
        $remove=Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
        if ($remove.ExitCode -ne 0) { throw "VIDEO-17 NSIS uninstall failed $($remove.ExitCode)" }
        for($i=0;$i -lt 30 -and
            ((Test-Path -LiteralPath (Join-Path $install 'akula-app.exe')) -or
             (Test-Path $registry));$i++){Start-Sleep -Seconds 1}
        if (Test-Path -LiteralPath (Join-Path $install 'akula-app.exe')) {
            throw 'NSIS failed to remove installed Akula'
        }
        if (Test-Path $registry) {throw 'NSIS uninstall left product registration'}
    }
}
if (!(Test-Path -LiteralPath $out -PathType Leaf)) { throw 'Lost synthetic MP4 after uninstall' }
if (!(Test-Path -LiteralPath $note -PathType Leaf)) { throw 'Uninstall destroyed synced Markdown' }
if ((Test-Path -LiteralPath $legacy) -ne $legacyExisting) {
    throw 'NSIS altered Aquilum during uninstall'
}
Write-Host 'VIDEO17_NSIS_UNINSTALL_DATA_PRESERVED_PASS no_public_release=1 no_signed_claim=1'
