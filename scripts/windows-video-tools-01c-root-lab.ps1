# VIDEO-TOOLS-01c: genuine hosted Windows known-folder and junction rejection.
# Must run before real 01a installer; no executable is staged by this test.
#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01C_HOLD: exact-head hosted Windows pull request required'
}
if ((git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA -or $LASTEXITCODE -ne 0) {
  throw 'VIDEO_TOOLS_01C_HOLD: nonreviewed checkout'
}
$principal = [Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01C_HOLD: protected-root test requires admin token'
}
. (Join-Path $env:GITHUB_WORKSPACE 'scripts/video-tools-01c-os-known-root.ps1')
$known = Get-AkulaVideoToolsOsProgramFiles
$save = [Environment]::GetEnvironmentVariable('ProgramFiles','Process')
try {
  [Environment]::SetEnvironmentVariable('ProgramFiles', (Join-Path $env:RUNNER_TEMP 'SPOOF-PROGRAM-FILES'), 'Process')
  $after = Get-AkulaVideoToolsOsProgramFiles
  if (!$known.Equals($after,[StringComparison]::OrdinalIgnoreCase)) {
    throw 'VIDEO_TOOLS_01C_HOLD: process environment redirected OS known folder'
  }
} finally {
  [Environment]::SetEnvironmentVariable('ProgramFiles', $save, 'Process')
}
$id=[guid]::NewGuid().ToString('N')
$expected = Join-Path $known ('AkulaVideoToolsLab-' + $id)
$outside = Join-Path $env:RUNNER_TEMP ('video-tools-01c-junction-target-' + $id)
if ((Test-Path -LiteralPath $expected) -or (Test-Path -LiteralPath $outside)) {
  throw 'VIDEO_TOOLS_01C_HOLD: disposable path already exists'
}
Assert-AkulaVideoToolsLabRoot -Path $expected -LabId $id
New-Item -ItemType Directory -Path $outside | Out-Null
try {
  New-Item -ItemType Junction -Path $expected -Target $outside | Out-Null
  $meta = Get-Item -LiteralPath $expected -Force
  if (($meta.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
    throw 'VIDEO_TOOLS_01C_HOLD: negative fixture is not a junction'
  }
  $rejected = $false
  try {
    Assert-AkulaVideoToolsLabRoot -Path $expected -LabId $id -RequireExists
  } catch {
    if ($_.Exception.Message.Contains('VIDEO_TOOLS_01C_HOLD: reparse')) {
      $rejected = $true
    } else { throw }
  }
  if (!$rejected) {
    throw 'VIDEO_TOOLS_01C_HOLD: redirecting junction was accepted'
  }
  $notOurRoot = Join-Path $env:RUNNER_TEMP ('AkulaVideoToolsLab-' + $id)
  $rejected = $false
  try {
    Assert-AkulaVideoToolsLabRoot -Path $notOurRoot -LabId $id
  } catch {
    if ($_.Exception.Message.Contains('outside exact OS-known product root')) {
      $rejected = $true
    } else { throw }
  }
  if (!$rejected) {
    throw 'VIDEO_TOOLS_01C_HOLD: arbitrary runner-temp root accepted'
  }
} finally {
  if (Test-Path -LiteralPath $expected) {
    $meta = Get-Item -LiteralPath $expected -Force
    if (($meta.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) {
      throw 'VIDEO_TOOLS_01C_HOLD: deletion would target a regular directory'
    }
    Remove-Item -LiteralPath $expected -Force
  }
  if (Test-Path -LiteralPath $outside) {
    $items = @(Get-ChildItem -LiteralPath $outside -Force)
    if ($items.Count -ne 0) {
      throw 'VIDEO_TOOLS_01C_HOLD: junction test wrote into external directory'
    }
    Remove-Item -LiteralPath $outside
  }
}
Write-Output 'VIDEO_TOOLS_01C_OS_ROOT_PASS os_known_folder=1 env_spoof_denied=1 junction_denied=1 out_of_root_denied=1 release=0'
