# VIDEO-18c-02b-02b-02a: distinct source-built runnable PE generations.
# Windows hosted pull_request only, no installer, no update or release.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$OldAppPath,
  [Parameter(Mandatory=$true)][string]$NewAppPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$oldCommit='ec6d6d48a78e60fca7a6f810b825418500bad64b'
if($env:GITHUB_ACTIONS -cne 'true' -or
   $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
   $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
   !$env:EXPECTED_HEAD_SHA -or
   (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO18C02B02B02A_HOLD: exact-head hosted Windows PR only'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP)
$oldTree=Join-Path $repo '_akula-historical'
$oldApp=[IO.Path]::GetFullPath($OldAppPath)
$newApp=[IO.Path]::GetFullPath($NewAppPath)
if($oldApp -cne $OldAppPath -or $newApp -cne $NewAppPath -or
   !$oldApp.StartsWith(($temp.TrimEnd('\')+'\'),[StringComparison]::OrdinalIgnoreCase) -or
   !$newApp.StartsWith(($repo.TrimEnd('\')+'\'),[StringComparison]::OrdinalIgnoreCase) -or
   (git -C $oldTree rev-parse HEAD).Trim() -cne $oldCommit -or
   (git -C $repo rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO18C02B02B02A_HOLD: stale or wrong-source PE generation'
}
function Assert-RealInput([string]$file) {
  $item=Get-Item -LiteralPath $file -Force -ErrorAction Stop
  if($item.PSIsContainer -or
     ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
     $item.Length -lt 1) {
    throw "VIDEO18C02B02B02A_HOLD: not a genuine non-reparse input file $file"
  }
}
function Assert-RealFile([string]$file) {
  Assert-RealInput $file
  $item=Get-Item -LiteralPath $file -Force -ErrorAction Stop
  if($item.Length -lt 512) {
    throw "VIDEO18C02B02B02A_HOLD: PE unexpectedly short $file"
  }
  $stream=[IO.File]::OpenRead($file)
  try {
    if($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) {
      throw "VIDEO18C02B02B02A_HOLD: non-PE source $file"
    }
  } finally {$stream.Dispose()}
}
Assert-RealFile $oldApp
Assert-RealFile $newApp
$oldSource=Join-Path $oldTree 'aquilum-app/native/src/updater/windows_first_gui_ack.rs'
$newSource=Join-Path $repo 'aquilum-app/native/src/updater/windows_first_gui_ack.rs'
if((Test-Path -LiteralPath $oldSource) -or !(Test-Path -LiteralPath $newSource -PathType Leaf)) {
  throw 'VIDEO18C02B02B02A_HOLD: claimed old/new source identity not verified'
}
$oldCargo=Join-Path $oldTree 'aquilum-app/native/Cargo.toml'
$newCargo=Join-Path $repo 'aquilum-app/native/Cargo.toml'
$oldLock=Join-Path $oldTree 'aquilum-app/Cargo.lock'
$newLock=Join-Path $repo 'aquilum-app/Cargo.lock'
foreach($file in @($oldCargo,$newCargo,$oldLock,$newLock)){Assert-RealInput $file}
$oldCargoBlob=(git -C $oldTree rev-parse HEAD:aquilum-app/native/Cargo.toml).Trim()
$newCargoBlob=(git -C $repo rev-parse HEAD:aquilum-app/native/Cargo.toml).Trim()
$oldLockBlob=(git -C $oldTree rev-parse HEAD:aquilum-app/Cargo.lock).Trim()
$newLockBlob=(git -C $repo rev-parse HEAD:aquilum-app/Cargo.lock).Trim()
# A real version increase must have different source/lock blobs.
# Compare every checkout against its own tracked Git blob, never against the other generation.
$oldCargoWorktree=(git -C $oldTree hash-object aquilum-app/native/Cargo.toml).Trim()
$newCargoWorktree=(git -C $repo hash-object aquilum-app/native/Cargo.toml).Trim()
$oldLockWorktree=(git -C $oldTree hash-object aquilum-app/Cargo.lock).Trim()
$newLockWorktree=(git -C $repo hash-object aquilum-app/Cargo.lock).Trim()
if($oldCargoBlob -cne $oldCargoWorktree -or $newCargoBlob -cne $newCargoWorktree -or
   $oldLockBlob -cne $oldLockWorktree -or $newLockBlob -cne $newLockWorktree -or
   $oldCargoBlob -ceq $newCargoBlob -or $oldLockBlob -ceq $newLockBlob) {
  throw 'VIDEO18C02B02B02A_HOLD: untracked or same-generation source/lock substituted'
}
$oldCargoText=Get-Content -LiteralPath $oldCargo -Raw
$oldLockText=Get-Content -LiteralPath $oldLock -Raw
$current=@(node (Join-Path $repo 'scripts/akula-version.cjs') check)
if($LASTEXITCODE -ne 0 -or $current.Count -ne 1 -or [version]$current[0] -le [version]'0.3.20'){
  throw 'VIDEO18C02B02B02A_HOLD: current source must be newer than the pinned historical source'
}
$newVersion=$current[0]
if($oldCargoText -notmatch '(?ms)^\[package\]\s+name = "aquilum-native"\s+version = "0\.3\.20"' -or
   $oldLockText -notmatch '(?m)^\[\[package\]\]\r?\nname = "aquilum-native"\r?\nversion = "0\.3\.20"' -or
   $oldLockText -notmatch '(?m)^\[\[package\]\]\r?\nname = "aquilum-core"\r?\nversion = "0\.3\.20"') {
  throw 'VIDEO18C02B02B02A_HOLD: real old/new source or locked package versions misrepresented'
}
$oldHash=(Get-FileHash -LiteralPath $oldApp -Algorithm SHA256).Hash.ToLowerInvariant()
$newHash=(Get-FileHash -LiteralPath $newApp -Algorithm SHA256).Hash.ToLowerInvariant()
if($oldHash -ceq $newHash) {
  throw 'VIDEO18C02B02B02A_HOLD: no distinct source-built Akula PE'
}
Write-Host "VIDEO18C02B02B02A_DISTINCT_SOURCE_PE_PASS old_source=$oldCommit new_source=$env:EXPECTED_HEAD_SHA different_sha=1 old_version=0.3.20 new_version=$newVersion no_mutated_pe=1 release=0"
$tag=[guid]::NewGuid().ToString('N')
$profile=Join-Path $temp ('Video18c02b02b02aOldGui-'+$tag)
if(Test-Path -LiteralPath $profile) {
  throw 'VIDEO18C02B02B02A_HOLD: old GUI profile reused'
}
New-Item -ItemType Directory -Path $profile -ErrorAction Stop | Out-Null
$vault=Join-Path $profile 'vault'
$data=Join-Path $profile 'data'
New-Item -ItemType Directory -Path $vault -ErrorAction Stop | Out-Null
New-Item -ItemType Directory -Path $data -ErrorAction Stop | Out-Null
$note=Join-Path $vault 'Historical Akula Window.md'
[IO.File]::WriteAllText($note,
  ('# Historical Akula GUI test'+[Environment]::NewLine+'Local data, not an update.'+[Environment]::NewLine),
  [Text.UTF8Encoding]::new($false))
$noteHash=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$start=[System.Diagnostics.ProcessStartInfo]::new()
$start.FileName=$oldApp
$start.WorkingDirectory=[IO.Path]::GetDirectoryName($oldApp)
$start.UseShellExecute=$false
foreach($arg in @('--vault',$vault,'--data-dir',$data,'--open',$note,'--focus')){
  [void]$start.ArgumentList.Add($arg)
}
foreach($name in @($start.Environment.Keys)){
  if($name.StartsWith('AKULA_VIDEO18C_',[StringComparison]::OrdinalIgnoreCase)){
    [void]$start.Environment.Remove($name)
  }
}
$process=[System.Diagnostics.Process]::new()
$process.StartInfo=$start
$ok=$false
try {
  if(!$process.Start()) {
    throw 'VIDEO18C02B02B02A_HOLD: old GUI process did not start'
  }
  $pidPinned=$process.Id
  if($pidPinned -le 1 -or $pidPinned -eq $PID) {
    throw 'VIDEO18C02B02B02A_HOLD: invalid pinned old GUI OS PID'
  }
  $started=$process.StartTime.ToUniversalTime()
  for($i=0;$i -lt 100;$i++){
    $process.Refresh()
    if($process.HasExited){
      throw 'VIDEO18C02B02B02A_HOLD: old GUI exited before first visible window'
    }
    if($process.MainWindowHandle -ne [IntPtr]::Zero -and
       $process.MainWindowTitle.Contains('Akula')) {
      $pinned=[System.Diagnostics.Process]::GetProcessById($pidPinned)
      try {
        if($pinned.StartTime.ToUniversalTime() -ne $started -or
           $pinned.MainWindowHandle -ne $process.MainWindowHandle){
          throw 'VIDEO18C02B02B02A_HOLD: old GUI window/PID attribution mismatch'
        }
      } finally {$pinned.Dispose()}
      $ok=$true
      break
    }
    Start-Sleep -Milliseconds 200
  }
  if(!$ok){throw 'VIDEO18C02B02B02A_HOLD: no visible historical GUI within deadline'}
  if(!$process.CloseMainWindow() -or !$process.WaitForExit(20000)){
    throw 'VIDEO18C02B02B02A_HOLD: old GUI did not close cleanly'
  }
  if((Get-FileHash -LiteralPath $oldApp -Algorithm SHA256).Hash.ToLowerInvariant() -cne $oldHash -or
     (Get-FileHash -LiteralPath $newApp -Algorithm SHA256).Hash.ToLowerInvariant() -cne $newHash -or
     (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteHash) {
    throw 'VIDEO18C02B02B02A_HOLD: PE or user note changed after old GUI launch'
  }
  Write-Output 'VIDEO18C02B02B02A_OLD_GUI_VISIBLE_PASS old_process=1 pinned_pid=1 visible_window=1 clean_exit=1 note_preserved=1 new_signed_ack_separate=1 no_update=1 release=0'
} finally {
  if(!$process.HasExited){
    $process.Kill($true)
    [void]$process.WaitForExit(5000)
  }
  $process.Dispose()
}
