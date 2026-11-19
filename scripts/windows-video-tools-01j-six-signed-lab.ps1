# VIDEO-TOOLS-01j: real six-file signed TEST installation and Akula native verifier.
# Exact-head disposable Windows PR only. TEST key never authorizes production.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$MuxPath,
  [Parameter(Mandatory=$true)][string]$YtDlpPath,
  [Parameter(Mandatory=$true)][string]$QuickJsPath,
  [Parameter(Mandatory=$true)][string]$ProofFile
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01J_HOLD: only hosted exact-HEAD Windows PR supported'
}
$head=(git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $head -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO_TOOLS_01J_HOLD: checkout is not reviewed HEAD'
}
$principal=[Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01J_HOLD: protected separate install needs real administrator token'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$expectedProof=Join-Path ([IO.Path]::GetFullPath($env:RUNNER_TEMP)) ('akula-video-tools-01j-'+$env:EXPECTED_HEAD_SHA+'.proof')
if(![IO.Path]::GetFullPath($ProofFile).Equals(
    [IO.Path]::GetFullPath($expectedProof),[StringComparison]::OrdinalIgnoreCase) -or
    (Test-Path -LiteralPath $ProofFile)){
  throw 'VIDEO_TOOLS_01J_HOLD: unsafe or existing final acceptance proof path'
}
$signer=$env:VIDEO_TOOLS_01J_MINISIGN
if([string]::IsNullOrWhiteSpace($signer)){
  throw 'VIDEO_TOOLS_01J_HOLD: exact official test signer missing'
}
$signer=[IO.Path]::GetFullPath($signer)
$runnerTemp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
if(!$signer.StartsWith($runnerTemp,[StringComparison]::OrdinalIgnoreCase) -or
   !(Test-Path -LiteralPath $signer -PathType Leaf) -or
   ((Get-Item -LiteralPath $signer -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){
  throw 'VIDEO_TOOLS_01J_HOLD: test Minisign executable escaped verified runner temp'
}
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
$program=Get-AkulaVideoToolsOsProgramFiles
$root=Join-Path $program 'Akula Video Tools'
$work=Join-Path ([IO.Path]::GetFullPath($env:RUNNER_TEMP)) ('video-tools-01j-'+[guid]::NewGuid().ToString('N'))
$stageThree=Join-Path $work 'source-pinned-three'
$unsigned=Join-Path $work 'unsigned-custody'
$stageSix=Join-Path $work 'six-file-stage'
$keyDir=Join-Path $work 'ephemeral-ci-key-only'
$installer=Join-Path $work 'six-file-lab-UNSIGNED-installer.exe'
$names=@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe',
         'manifest.json','manifest.minisig','uninstall.exe')
if ((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $work)) {
  throw 'VIDEO_TOOLS_01J_HOLD: protected product already exists; cannot overwrite'
}
$expectedMux=[IO.Path]::GetFullPath((Join-Path $repo 'labs/video-lite/target/release/dash.exe'))
if (![IO.Path]::GetFullPath($MuxPath).Equals(
    $expectedMux,[StringComparison]::OrdinalIgnoreCase)) {
  throw 'VIDEO_TOOLS_01J_HOLD: Rust mux must be source-built at exact checkout'
}
$inputs=[ordered]@{
  'akula-video-mux.exe'=$MuxPath
  'akula-yt-dlp.exe'=$YtDlpPath
  'akula-quickjs.exe'=$QuickJsPath
}
$createdWork=$false
$installed=$false
$noteCreated=$false
$privateNote=Join-Path $root '01j-unrelated-user-note.txt'
function Assert-Exact-Six {
  if (!(Test-Path -LiteralPath $root -PathType Container)) {
    throw 'VIDEO_TOOLS_01J_HOLD: separate product missing after install'
  }
  Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
  $items=@(Get-ChildItem -LiteralPath $root -Force -ErrorAction Stop)
  if ($items.Count -ne $names.Count -or
      @($items | Where-Object { $_.Name -cnotin $names }).Count -ne 0) {
    throw 'VIDEO_TOOLS_01J_HOLD: protected product must contain exactly six files'
  }
  foreach ($item in $items) {
    if ($item.PSIsContainer -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
      throw 'VIDEO_TOOLS_01J_HOLD: protected package contains alias or directory'
    }
  }
}
function Run-Native {
  param([Parameter(Mandatory=$true)][string]$Mode,
        [Parameter(Mandatory=$true)][string]$Public)
  $command=Join-Path $repo 'labs/video-tools-01j/target/release/akula-video-tools-01j-signed-ci-witness.exe'
  if (!(Test-Path -LiteralPath $command -PathType Leaf)) {
    throw 'VIDEO_TOOLS_01J_HOLD: native Akula Rust verification binary missing'
  }
  $messages=@(& $command $Mode $root $Public 2>&1)
  $code=$LASTEXITCODE
  $messages | Out-Host
  if ($code -ne 0) {
    throw ('VIDEO_TOOLS_01J_HOLD: native '+$Mode+' failed: '+($messages | Out-String))
  }
  $required=if ($Mode -ceq 'valid') {
    'VIDEO_TOOLS_01J_NATIVE_SIGNED_INSTALLED_PASS'
  } elseif ($Mode -ceq 'missing') {
    'VIDEO_TOOLS_01J_MISSING_PASS'
  } else {
    'VIDEO_TOOLS_01J_NATIVE_REJECT_PASS case='+$Mode
  }
  if (!(($messages | Out-String).Contains($required))) {
    throw ('VIDEO_TOOLS_01J_HOLD: native witness omitted '+$required)
  }
}
try {
  New-Item -ItemType Directory -Path $stageThree -Force | Out-Null
  $createdWork=$true
  $pinned=Get-Content -LiteralPath (Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json') -Raw | ConvertFrom-Json
  if ($pinned.release_admitted -ne $false -or
      $pinned.installer_signing_authorized -ne $false -or
      $pinned.stage_binary_distribution_authorized -ne $false) {
    throw 'VIDEO_TOOLS_01J_HOLD: source/license allowlist became distributable'
  }
  foreach($name in @($inputs.Keys)) {
    $source=[IO.Path]::GetFullPath($inputs[$name])
    $entry=@($pinned.components | Where-Object { $_.name -ceq $name })
    if ($entry.Count -ne 1 -or !(Test-Path -LiteralPath $source -PathType Leaf)) {
      throw 'VIDEO_TOOLS_01J_HOLD: source executable or provenance absent'
    }
    if ($name -ne 'akula-video-mux.exe' -and (
        (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry[0].sha256 -or
        (Get-Item -LiteralPath $source).Length -ne $entry[0].size_bytes)) {
      throw 'VIDEO_TOOLS_01J_HOLD: pinned external PE hash/size differs'
    }
    Copy-Item -LiteralPath $source -Destination (Join-Path $stageThree $name) -ErrorAction Stop
  }
  & cargo build --release --manifest-path (Join-Path $repo 'labs/video-tools-01j/Cargo.toml')
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO_TOOLS_01J_HOLD: native Akula verifier failed to compile' }
  Run-Native -Mode 'missing' -Public '-'
  $output=@(& node (Join-Path $repo 'scripts/video-tools-01i-unsigned-custody.cjs') $stageThree $unsigned 2>&1)
  $output | Out-Host
  if ($LASTEXITCODE -ne 0 -or
      !(($output | Out-String).Contains('VIDEO_TOOLS_01I_UNSIGNED_CUSTODY_PASS'))) {
    throw 'VIDEO_TOOLS_01J_HOLD: source-pinned three-file manifest not generated'
  }
  $manifest=Join-Path $unsigned 'manifest.json'
  $signature=Join-Path $unsigned 'manifest.minisig'
  $data=Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json
  $lock=(Get-FileHash -LiteralPath (Join-Path $repo 'labs/video-lite/Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($data.source_commit -cne $env:EXPECTED_HEAD_SHA -or
      $data.cargo_lock_sha256 -cne $lock -or
      $data.binaries.Count -ne 3) {
    throw 'VIDEO_TOOLS_01J_HOLD: unsigned manifest has wrong source provenance'
  }
  New-Item -ItemType Directory -Path $keyDir | Out-Null
  $private=Join-Path $keyDir 'CI-ONLY-DO-NOT-PUBLISH.key'
  $public=Join-Path $keyDir 'CI-ONLY.pub'
  $otherPrivate=Join-Path $keyDir 'CI-ONLY-OTHER.key'
  $otherPublic=Join-Path $keyDir 'CI-ONLY-OTHER.pub'
  & $signer -G -W -s $private -p $public | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO_TOOLS_01J_HOLD: ephemeral key generation failed' }
  & $signer -G -W -s $otherPrivate -p $otherPublic | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO_TOOLS_01J_HOLD: independent wrong key generation failed' }
  & $signer -S -q -s $private -m $manifest -x $signature
  if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $signature)) {
    throw 'VIDEO_TOOLS_01J_HOLD: test-only Minisign signature creation failed'
  }
  & $signer -Vm $manifest -x $signature -p $public -Q | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO_TOOLS_01J_HOLD: real Minisign self-verification failed' }
  # A manifest/metadata copy is installed; the private and public test key
  # remain OUTSIDE Program Files and are never passed to production Akula.
  New-Item -ItemType Directory -Path $stageSix | Out-Null
  foreach($name in @($inputs.Keys)) {
    Copy-Item -LiteralPath (Join-Path $stageThree $name) -Destination (Join-Path $stageSix $name)
  }
  Copy-Item -LiteralPath $manifest -Destination (Join-Path $stageSix 'manifest.json')
  Copy-Item -LiteralPath $signature -Destination (Join-Path $stageSix 'manifest.minisig')
  if (@(Get-ChildItem -LiteralPath $stageSix).Count -ne 5) {
    throw 'VIDEO_TOOLS_01J_HOLD: installer stage contains more than five input files'
  }
  $make=Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS/makensis.exe'
  if (!(Test-Path -LiteralPath $make -PathType Leaf)) {
    throw 'VIDEO_TOOLS_01J_HOLD: protected NSIS compiler missing'
  }
  & $make ("/DLAB_STAGE="+$stageSix) ("/DLAB_OUTPUT="+$installer) (Join-Path $repo 'scripts/video-tools-01j-six-file-lab.nsi') | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO_TOOLS_01J_HOLD: six-file NSIS compilation failed' }
  if (Test-Path -LiteralPath $root) { throw 'VIDEO_TOOLS_01J_HOLD: product root appeared before install' }
  $install=Start-Process -FilePath $installer -ArgumentList '/S' -PassThru -Wait
  if ($install.ExitCode -ne 0) { throw 'VIDEO_TOOLS_01J_HOLD: separate signed fixture installation failed' }
  $installed=$true
  Assert-Exact-Six
  Run-Native -Mode 'valid' -Public $public
  Run-Native -Mode 'wrong-key' -Public $otherPublic
  # Negative mutation: exact signed on-disk manifest changed, no signature change.
  $installedManifest=Join-Path $root 'manifest.json'
  $savedManifest=[IO.File]::ReadAllBytes($installedManifest)
  try {
    $changed=[byte[]]$savedManifest.Clone()
    $changed[5] = $changed[5] -bxor 1
    [IO.File]::WriteAllBytes($installedManifest,$changed)
    Run-Native -Mode 'tamper-manifest' -Public $public
  } finally { [IO.File]::WriteAllBytes($installedManifest,$savedManifest) }
  $installedSig=Join-Path $root 'manifest.minisig'
  $savedSig=[IO.File]::ReadAllBytes($installedSig)
  try {
    $changed=[byte[]]$savedSig.Clone()
    # Modify the signed base64 payload, never the unauthenticated comment.
    $firstLineBreak=[Array]::IndexOf($changed,[byte]10)
    if($firstLineBreak -lt 0 -or $firstLineBreak+1 -ge $changed.Length){
      throw 'VIDEO_TOOLS_01J_HOLD: Minisign signature has no signed payload'
    }
    $changed[$firstLineBreak+1] = $changed[$firstLineBreak+1] -bxor 1
    [IO.File]::WriteAllBytes($installedSig,$changed)
    Run-Native -Mode 'tamper-signature' -Public $public
  } finally { [IO.File]::WriteAllBytes($installedSig,$savedSig) }
  $installedPe=Join-Path $root 'akula-video-mux.exe'
  $savedPe=[IO.File]::ReadAllBytes($installedPe)
  try {
    $changed=[byte[]]$savedPe.Clone()
    $changed[200] = $changed[200] -bxor 1
    [IO.File]::WriteAllBytes($installedPe,$changed)
    Run-Native -Mode 'tamper-pe' -Public $public
  } finally { [IO.File]::WriteAllBytes($installedPe,$savedPe) }
  $seventh=Join-Path $root 'unexpected-extra.exe'
  try {
    [IO.File]::WriteAllBytes($seventh,[byte[]](77,90,0,0))
    Run-Native -Mode 'seventh-file' -Public $public
  } finally { if(Test-Path -LiteralPath $seventh){Remove-Item -LiteralPath $seventh -Force} }
  Assert-Exact-Six
  Run-Native -Mode 'valid' -Public $public
  # An unrelated user note must NOT be deleted by the dedicated uninstaller.
  Set-Content -LiteralPath $privateNote -Value 'preserve-foreign-note' -NoNewline
  $noteCreated=$true
  $uninstall=Start-Process -FilePath (Join-Path $root 'uninstall.exe') -ArgumentList '/S' -PassThru -Wait
  if($uninstall.ExitCode -ne 0){throw 'VIDEO_TOOLS_01J_HOLD: six-file uninstall failed'}
  $installed=$false
  if(!(Test-Path -LiteralPath $privateNote -PathType Leaf)) {
    throw 'VIDEO_TOOLS_01J_HOLD: separate package removed unrelated user note'
  }
  foreach($name in $names) {
    if(Test-Path -LiteralPath (Join-Path $root $name)) {
      throw 'VIDEO_TOOLS_01J_HOLD: uninstaller left a package component behind'
    }
  }
  if ((Get-Content -LiteralPath $privateNote -Raw) -cne 'preserve-foreign-note') {
    throw 'VIDEO_TOOLS_01J_HOLD: unrelated content changed'
  }
  Remove-Item -LiteralPath $privateNote -Force
  $noteCreated=$false
  $remain=@(Get-ChildItem -LiteralPath $root -Force)
  if($remain.Count -ne 0){throw 'VIDEO_TOOLS_01J_HOLD: unexpected leftover protected content'}
  Remove-Item -LiteralPath $root
  Run-Native -Mode 'missing' -Public '-'
  Write-Output 'VIDEO_TOOLS_01J_SIX_SIGNED_PASS fixed_root=1 real_exes=3 files=6 ephemeral_minisign=1 native_discovery=1 hold=1 negative=5 uninstall=1 private_notes_preserved=1 release=0'
  # This file is the authoritative CI handoff. It is created ONLY after
  # both positive native Minisign verdicts, all five negative cases, and
  # uninstall + user data preservation have completed successfully.
  $proofContent="VIDEO_TOOLS_01J_NATIVE_SIGNED_INSTALLED_PASS`nVIDEO_TOOLS_01J_SIX_SIGNED_PASS`n"
  [IO.File]::WriteAllText($ProofFile,$proofContent,[Text.UTF8Encoding]::new($false))
} finally {
  if($installed -and (Test-Path -LiteralPath (Join-Path $root 'uninstall.exe') -PathType Leaf)) {
    Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
    Start-Process -FilePath (Join-Path $root 'uninstall.exe') -ArgumentList '/S' -Wait | Out-Null
  }
  if($noteCreated -and (Test-Path -LiteralPath $privateNote)) {
    Remove-Item -LiteralPath $privateNote -Force -ErrorAction Stop
  }
  if(Test-Path -LiteralPath $root) {
    Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
    if(@(Get-ChildItem -LiteralPath $root -Force).Count -eq 0) {
      Remove-Item -LiteralPath $root
    }
  }
  if($createdWork -and (Test-Path -LiteralPath $work)) {
    $temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
    if(![IO.Path]::GetFullPath($work).StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)) {
      throw 'VIDEO_TOOLS_01J_HOLD: refusing unsafe ephemeral secret cleanup'
    }
    Remove-Item -LiteralPath $work -Recurse -Force
  }
}
