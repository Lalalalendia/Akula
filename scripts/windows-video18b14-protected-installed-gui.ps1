# VIDEO-18b-14: genuine protected five-PE NSIS + physical Windows GUI acceptance.
# No product installer, registry identity, signed release or user-owned data.
#requires -Version 7.2
param(
    [Parameter(Mandatory=$true)][string]$AppPath,
    [Parameter(Mandatory=$true)][string]$ShivaPath,
    [Parameter(Mandatory=$true)][string]$MuxPath,
    [Parameter(Mandatory=$true)][string]$YtDlpPath,
    [Parameter(Mandatory=$true)][string]$QuickJsPath,
    [Parameter(Mandatory=$true)][string]$FixtureDir
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:GITHUB_ACTIONS -cne 'true' -or
    (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
    throw 'VIDEO18B14_HOLD: requires exact-head hosted Windows PR'
}
$principal = [Security.Principal.WindowsPrincipal]::new(
    [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'VIDEO18B14_HOLD: true protected NSIS installation requires elevated runner'
}
function Assert-Regular([string]$file) {
    $item=Get-Item -LiteralPath $file -ErrorAction Stop
    if (!$item.PSIsContainer -and
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {return}
    throw "VIDEO18B14_HOLD: unsafe path: $file"
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$repoPrefix=$repo.TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP)
$tempPrefix=$temp.TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
$fixture=[IO.Path]::GetFullPath($FixtureDir)
$tag=[guid]::NewGuid().ToString('N')
$stage=Join-Path $temp ('Video18b14Stage-'+$tag)
$setup=Join-Path $stage 'isolated-protected-unsigned-setup.exe'
$protected=[IO.Path]::GetFullPath((Join-Path $env:ProgramFiles ('AkulaVideo18bRealNsisLab-'+$tag)))
$expectedRoot=[IO.Path]::GetFullPath($env:ProgramFiles).TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
if (!$fixture.StartsWith($repoPrefix,[StringComparison]::OrdinalIgnoreCase) -or
    !(Test-Path -LiteralPath (Join-Path $fixture 'manifest.mpd') -PathType Leaf) -or
    !$protected.StartsWith($expectedRoot,[StringComparison]::OrdinalIgnoreCase) -or
    ([IO.Path]::GetFileName($protected) -cne ('AkulaVideo18bRealNsisLab-'+$tag)) -or
    (Test-Path -LiteralPath $stage) -or
    (Test-Path -LiteralPath $protected)) {
    throw 'VIDEO18B14_HOLD: incorrect or reused disposable roots'
}
$names=@('akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs')
$sources=[ordered]@{
    'akula-app'=$AppPath
    'shiva-pdf-preview'=$ShivaPath
    'akula-video-mux'=$MuxPath
    'akula-yt-dlp'=$YtDlpPath
    'akula-quickjs'=$QuickJsPath
}
$allow=Get-Content -LiteralPath (Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json') -Raw | ConvertFrom-Json
if ($allow.release_admitted -ne $false -or
    @($allow.components).Count -ne 5 -or
    (@($allow.components | ForEach-Object { $_.name }) -join ',') -cne
       (($names | ForEach-Object {$_+'.exe'}) -join ',')) {
    throw 'VIDEO18B14_HOLD: experimental five-PE inventory no longer isolated'
}
$mk=Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS\makensis.exe'
$nsi=Join-Path $repo 'scripts/video18b-protected-five-real.nsi'
if (!(Test-Path -LiteralPath $mk -PathType Leaf)) {throw 'VIDEO18B14 pinned makensis unavailable'}
$installed=$false
$created=$false
$guiPassed=$false
$outputDir=Join-Path $repo 'video18b14-output'
$output=Join-Path $outputDir 'protected-installed.mp4'
$vault=Join-Path $temp ('video18b14-vault-'+$tag)
$data=Join-Path $temp ('video18b14-data-'+$tag)
$probe=Join-Path $temp ('video18b14-events-'+$tag+'.log')
$note=Join-Path $vault 'VIDEO16 Media.md'
$media=Join-Path $data 'local-videos/dQw4w9WgXcQ.mp4'
$expectedHashes=@{}
try {
    New-Item -ItemType Directory -Path $stage -ErrorAction Stop | Out-Null
    $created=$true
    foreach($name in $names){
        $source=[IO.Path]::GetFullPath($sources[$name])
        if (!$source.StartsWith($repoPrefix,[StringComparison]::OrdinalIgnoreCase) -and
            !$source.StartsWith($tempPrefix,[StringComparison]::OrdinalIgnoreCase)) {
            throw "VIDEO18B14_HOLD: source outside hosted checkout/temp: $name"
        }
        Assert-Regular $source
        $check=[IO.File]::OpenRead($source)
        try {
            if ($check.Length -lt 512 -or $check.ReadByte() -ne 77 -or $check.ReadByte() -ne 90) {
                throw "VIDEO18B14_HOLD: source not genuine PE/MZ: $name"
            }
        } finally { $check.Dispose() }
        $dest=Join-Path $stage ($name+'.exe')
        Copy-Item -LiteralPath $source -Destination $dest -ErrorAction Stop
        Assert-Regular $dest
        $sha=(Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant()
        if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $sha) {
            throw "VIDEO18B14_HOLD: source/stage hash mismatch: $name"
        }
        $expectedHashes[$name]=$sha
        if($name -ceq 'akula-video-mux' -and $sha -cne $env:AKULA_BUNDLED_MUX_SHA256){
            throw 'VIDEO18B14_HOLD: unpinned Rust mux'
        }
        if($name -in @('akula-yt-dlp','akula-quickjs')){
            $record=@($allow.components | Where-Object { $_.name -ceq ($name+'.exe') })
            if($record.Count -ne 1 -or $sha -cne $record[0].sha256 -or
               (Get-Item -LiteralPath $dest).Length -ne $record[0].size_bytes){
                throw "VIDEO18B14_HOLD: untrusted upstream helper $name"
            }
        }
    }
    if (@(Get-ChildItem -LiteralPath $stage -File).Count -ne 5) {
        throw 'VIDEO18B14_HOLD: stage requires exactly five real PE executables'
    }
    & $mk "/DLAB_ID=$tag" "/DLAB_STAGE=$stage" "/DLAB_OUTPUT=$setup" $nsi | Out-Host
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $setup -PathType Leaf)) {
        throw 'VIDEO18B14 protected NSIS compile failed'
    }
    $proc=Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
    $installed=Test-Path -LiteralPath $protected -PathType Container
    if($proc.ExitCode -ne 0 -or !$installed) {
        throw 'VIDEO18B14 protected NSIS install failed'
    }
    $rootItem=Get-Item -LiteralPath $protected
    if (($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'VIDEO18B14 protected NSIS root is aliased'
    }
    foreach($name in $names) {
        $file=Join-Path $protected ($name+'.exe')
        Assert-Regular $file
        if((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedHashes[$name]){
            throw "VIDEO18B14 protected NSIS altered real executable: $name"
        }
    }
    Assert-Regular (Join-Path $protected 'uninstall.exe')
    Write-Host 'VIDEO18B14_PROTECTED_REAL_INSTALLED_PASS five_verified_sha256=5'
    New-Item -ItemType Directory -Path $outputDir -ErrorAction Stop | Out-Null
    $env:AKULA_VIDEO18B14_PROTECTED_GUI='1'
    $args=@('labs/video-lite/video18b14/gui_acceptance.py',
        '--repo',$repo,'--manifest-dir',$fixture,
        '--exe',(Join-Path $protected 'akula-app.exe'),
        '--vault',$vault,'--data',$data,'--probe',$probe,'--output',$output)
    & python @args
    if ($LASTEXITCODE -ne 0){throw 'VIDEO18B14 real protected GUI physical-pointer test failed'}
    if (!(Test-Path -LiteralPath $output -PathType Leaf) -or
        (Get-Item -LiteralPath $output).Length -lt 1000 -or
        !(Test-Path -LiteralPath $note -PathType Leaf) -or
        !(Test-Path -LiteralPath $media -PathType Leaf)) {
        throw 'VIDEO18B14 GUI note/video/output was not persisted'
    }
    $noteSha=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
    $mediaSha=(Get-FileHash -LiteralPath $media -Algorithm SHA256).Hash
    foreach($name in $names) {
        if ((Get-FileHash -LiteralPath (Join-Path $protected ($name+'.exe')) -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expectedHashes[$name]) {
            throw 'VIDEO18B14 local GUI altered a protected installed EXE'
        }
    }
    if (@(Get-ChildItem -LiteralPath $vault -Recurse -File |
          Where-Object {$_.Extension -in @('.mp4','.m4a','.m4s','.webm')}).Count -ne 0) {
        throw 'VIDEO18B14 local media leaked to note vault'
    }
    $guiPassed=$true
    Write-Host 'VIDEO18B14_PROTECTED_GUI_RESTART_PLAY_PASS real_pointer=1 decoded_frame=1'
} finally {
    if($installed){
        $uninstaller=Join-Path $protected 'uninstall.exe'
        Assert-Regular $uninstaller
        $gone=Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
        if($gone.ExitCode -ne 0 -or (Test-Path -LiteralPath $protected)){
            throw 'VIDEO18B14 protected NSIS uninstall failed'
        }
    }
    if($created){ Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction Stop }
}
if (!$guiPassed -or !(Test-Path -LiteralPath $note -PathType Leaf) -or
    !(Test-Path -LiteralPath $media -PathType Leaf) -or
    (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteSha -or
    (Get-FileHash -LiteralPath $media -Algorithm SHA256).Hash -cne $mediaSha) {
    throw 'VIDEO18B14 installed uninstall lost or altered private local media'
}
Write-Host 'VIDEO18B14_PROTECTED_INSTALL_GUI_UNINSTALL_PASS install=1 click=1 restart=1 play=1 five_exe_removed=1 private_media_preserved=1 signed=0 release=0'
