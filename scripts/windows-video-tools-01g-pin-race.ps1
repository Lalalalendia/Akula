# VIDEO-TOOLS-01g. Actual Win32 sharing race on real protected NSIS files.
# Hosted Windows PR-only, no publisher sign, no release, no execution of video PEs.
#requires -Version 7.2
param([Parameter(Mandatory=$true)][string]$Root,
      [Parameter(Mandatory=$true)][string]$LabId,
      [Parameter(Mandatory=$true)][string]$Stage,
      [Parameter(Mandatory=$true)][string]$Work)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
   $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
   $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01G_HOLD: exact-head hosted Windows PR required'
}
$head=(git rev-parse HEAD).Trim()
if($LASTEXITCODE -ne 0 -or $head -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO_TOOLS_01G_HOLD: checkout not at reviewed HEAD'
}
$principal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01G_HOLD: elevated token required for real race witness'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$runner=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
if(![IO.Path]::GetFullPath($Work).StartsWith($runner,[StringComparison]::OrdinalIgnoreCase) -or
   ![IO.Path]::GetFullPath($Stage).StartsWith($runner,[StringComparison]::OrdinalIgnoreCase)) {
  throw 'VIDEO_TOOLS_01G_HOLD: ephemeral test directories escaped runner temp'
}
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
. (Join-Path $repo 'scripts/video-tools-01d-acl.ps1')
Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists
Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
$names=@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe')
$hashes=@{}
foreach($name in $names){
  $hashes[$name]=(Get-FileHash -LiteralPath (Join-Path $Root $name) -Algorithm SHA256).Hash
  if($hashes[$name] -cne (Get-FileHash -LiteralPath (Join-Path $Stage $name) -Algorithm SHA256).Hash){
    throw 'VIDEO_TOOLS_01G_HOLD: installed PE differs from staged source'
  }
}
$uninstallHash=(Get-FileHash -LiteralPath (Join-Path $Root 'uninstall.exe') -Algorithm SHA256).Hash
$source=Join-Path $repo 'aquilum-app/native/src/video/tools_package_pin_windows.rs'
$tests=Join-Path $Work 'video-tools-01g-tests.exe'
$witness=Join-Path $Work 'video-tools-01g-witness.exe'
& rustc --edition=2024 --test $source -o $tests
if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01G_HOLD: native tests did not compile'}
& $tests --nocapture --test-threads=1 | Out-Host
if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01G_HOLD: native handle tests failed'}
& rustc --edition=2024 --cfg akula_video_tools_01g_witness $source -o $witness
if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01G_HOLD: native held witness did not compile'}
$prior=@{}
foreach($key in @('AKULA_VIDEO_TOOLS_01G_ROOT','AKULA_VIDEO_TOOLS_01G_SOURCE','AKULA_VIDEO_TOOLS_01G_WORK')){
  $prior[$key]=[Environment]::GetEnvironmentVariable($key,'Process')
}
$ready=Join-Path $Work 'video-tools-01g-ready'
$release=Join-Path $Work 'video-tools-01g-release'
$stdout=Join-Path $Work 'video-tools-01g-out.txt'
$stderr=Join-Path $Work 'video-tools-01g-err.txt'
if((Test-Path -LiteralPath $ready) -or (Test-Path -LiteralPath $release)){
  throw 'VIDEO_TOOLS_01G_HOLD: race barrier already present'
}
$process=$null
try {
  $env:AKULA_VIDEO_TOOLS_01G_ROOT=$Root
  $env:AKULA_VIDEO_TOOLS_01G_SOURCE=$Stage
  $env:AKULA_VIDEO_TOOLS_01G_WORK=$Work
  $process=Start-Process -FilePath $witness -PassThru -NoNewWindow -RedirectStandardOutput $stdout -RedirectStandardError $stderr -ErrorAction Stop
  for($i=0;$i -lt 200 -and !(Test-Path -LiteralPath $ready);$i++){
    if($process.HasExited){break}
    Start-Sleep -Milliseconds 100
  }
  if(!(Test-Path -LiteralPath $ready) -or $process.HasExited){
    $details=if(Test-Path -LiteralPath $stderr){Get-Content -LiteralPath $stderr -Raw}else{'no stderr'}
    throw ('VIDEO_TOOLS_01G_HOLD: Rust did not acquire read-only handles: '+$details)
  }
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class AkulaVideoToolsPinnedRights {
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, EntryPoint="CreateFileW", SetLastError=true)]
  public static extern SafeFileHandle Open(string path,uint access,uint share,IntPtr security,uint disposition,uint flags,IntPtr template);
}
'@
  function Assert-ShareViolation {
    param([string]$Target,[uint32]$Access,[uint32]$Flags)
    Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists
    $handle=[AkulaVideoToolsPinnedRights]::Open($Target,$Access,7,[IntPtr]::Zero,3,$Flags,[IntPtr]::Zero)
    $errorCode=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if(!$handle.IsInvalid){
      $handle.Dispose()
      throw ('VIDEO_TOOLS_01G_HOLD: mutation opened while held: '+$Target)
    }
    $handle.Dispose()
    if($errorCode -ne 32){
      throw ('VIDEO_TOOLS_01G_HOLD: expected ERROR_SHARING_VIOLATION=32, got '+$errorCode+' at '+$Target)
    }
  }
  Assert-ShareViolation -Target $Root -Access ([uint32]0x00010000) -Flags ([uint32]0x02000000)
  foreach($name in @($names + 'uninstall.exe')){
    $file=Join-Path $Root $name
    Assert-ShareViolation -Target $file -Access ([uint32]0x40000000) -Flags ([uint32]0)
    Assert-ShareViolation -Target $file -Access ([uint32]0x00010000) -Flags ([uint32]0)
  }
  if($process.HasExited){throw 'VIDEO_TOOLS_01G_HOLD: Rust released handles before test'}
  Set-Content -LiteralPath $release -Value 'release' -NoNewline
  if(!$process.WaitForExit(15000) -or $process.ExitCode -ne 0){
    throw 'VIDEO_TOOLS_01G_HOLD: Rust held-handle witness failed to exit'
  }
  $proof=Get-Content -LiteralPath $stdout -Raw
  if(!$proof.Contains('VIDEO_TOOLS_01G_NATIVE_PIN_PASS source_bytes=3 root_held=1 files_held=4 release=0')){
    throw 'VIDEO_TOOLS_01G_HOLD: native Rust pin proof missing'
  }
  # Positive control: write access succeeds only AFTER all handles close.
  $handle=[AkulaVideoToolsPinnedRights]::Open((Join-Path $Root $names[0]),[uint32]0x40000000,7,[IntPtr]::Zero,3,0,[IntPtr]::Zero)
  if($handle.IsInvalid){
    $handle.Dispose()
    throw 'VIDEO_TOOLS_01G_HOLD: write-right positive control failed after release'
  }
  $handle.Dispose()
  foreach($name in $names){
    if((Get-FileHash -LiteralPath (Join-Path $Root $name) -Algorithm SHA256).Hash -cne $hashes[$name]){
      throw 'VIDEO_TOOLS_01G_HOLD: installed bytes changed during race'
    }
  }
  if((Get-FileHash -LiteralPath (Join-Path $Root 'uninstall.exe') -Algorithm SHA256).Hash -cne $uninstallHash){
    throw 'VIDEO_TOOLS_01G_HOLD: uninstaller bytes changed'
  }
  Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
  Write-Output 'VIDEO_TOOLS_01G_PIN_RACE_PASS held_root=1 held_files=4 write_denied=4 delete_denied=5 sharing_violation=32 release_confirmed=1 bytes_unchanged=1 release=0'
} finally {
  if($null -ne $process -and !$process.HasExited){
    if(!(Test-Path -LiteralPath $release)){Set-Content -LiteralPath $release -Value 'release' -NoNewline}
    if(!$process.WaitForExit(10000)){$process.Kill($true);$process.WaitForExit()}
  }
  foreach($key in $prior.Keys){[Environment]::SetEnvironmentVariable($key,$prior[$key],'Process')}
}
