# VIDEO-TOOLS-01e: hosted-Windows-only native Rust ACL witness.
# Native checker is compiled from its actual shipped source, not reimplemented in PowerShell.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$Root,
  [Parameter(Mandatory=$true)][string]$LabId,
  [Parameter(Mandatory=$true)][string]$Work
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01E_HOLD: exact-head hosted Windows PR required'
}
$head = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $head -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO_TOOLS_01E_HOLD: nonreviewed native witness source'
}
$principal = [Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01E_HOLD: only elevated ephemeral lab may modify ACEs'
}
$repo = [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists
$source = Join-Path $repo 'aquilum-app/native/src/video/tools_package_windows.rs'
$tests = Join-Path $Work 'native-video-tools-01e-tests.exe'
$witness = Join-Path $Work 'native-video-tools-01e-witness.exe'
& rustc --edition=2024 --test $source -o $tests
if ($LASTEXITCODE -ne 0) {
  throw 'VIDEO_TOOLS_01E_HOLD: native Win32 Rust tests did not compile'
}
& rustc --edition=2024 --cfg akula_video_tools_acl_witness $source -o $witness
if ($LASTEXITCODE -ne 0) {
  throw 'VIDEO_TOOLS_01E_HOLD: native Win32 ACL admission did not compile'
}
$priorLabRoot = [Environment]::GetEnvironmentVariable('AKULA_VIDEO_TOOLS_01E_LAB_ROOT','Process')
$priorProgramFiles = [Environment]::GetEnvironmentVariable('ProgramFiles','Process')
try {
  $env:AKULA_VIDEO_TOOLS_01E_LAB_ROOT = $Root
  # Spoofing ambient ProgramFiles must not alter the GUID-anchored Win32 API.
  [Environment]::SetEnvironmentVariable('ProgramFiles',(Join-Path $env:RUNNER_TEMP 'SPOOF-01E-PROGRAMFILES'),'Process')
  & $tests --nocapture --test-threads=1 | Out-Host
  if ($LASTEXITCODE -ne 0) {
    throw 'VIDEO_TOOLS_01E_HOLD: native Windows unit test failed'
  }
  function Native-Witness-Exit {
    $process = Start-Process -FilePath $witness -Wait -PassThru -NoNewWindow
    return [int]$process.ExitCode
  }
  if ((Native-Witness-Exit) -ne 0) {
    throw 'VIDEO_TOOLS_01E_HOLD: protected package was denied before mutation'
  }
  function Require-NativeDeny {
    param([string]$Target,[string]$Sid,[Security.AccessControl.FileSystemRights]$Right)
    Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists
    $before = Get-Acl -LiteralPath $Target
    $modified = Get-Acl -LiteralPath $Target
    $rule = [Security.AccessControl.FileSystemAccessRule]::new(
      [Security.Principal.SecurityIdentifier]::new($Sid),
      $Right,
      [Security.AccessControl.AccessControlType]::Allow)
    $modified.AddAccessRule($rule)
    $changed = $false
    try {
      Set-Acl -LiteralPath $Target -AclObject $modified -ErrorAction Stop
      $changed = $true
      if ((Native-Witness-Exit) -eq 0) {
        throw 'VIDEO_TOOLS_01E_HOLD: native Rust accepted poisoned Windows ACL'
      }
    } finally {
      if ($changed) {
        Set-Acl -LiteralPath $Target -AclObject $before -ErrorAction Stop
        if ((Native-Witness-Exit) -ne 0) {
          throw 'VIDEO_TOOLS_01E_HOLD: native Rust failed after original ACL restored'
        }
      }
    }
  }
  Require-NativeDeny -Target $Root -Sid 'S-1-1-0' -Right WriteData
  Require-NativeDeny -Target (Join-Path $Root 'akula-quickjs.exe') -Sid 'S-1-5-11' -Right ChangePermissions
  if ((Native-Witness-Exit) -ne 0) {
    throw 'VIDEO_TOOLS_01E_HOLD: original package is no longer protected'
  }
  Write-Output 'VIDEO_TOOLS_01E_NATIVE_WIN32_PASS known_folder=1 root_acl=1 exe_acl=1 poisoned_root_denied=1 poisoned_exe_denied=1 restored=1 release=0'
} finally {
  [Environment]::SetEnvironmentVariable('AKULA_VIDEO_TOOLS_01E_LAB_ROOT',$priorLabRoot,'Process')
  [Environment]::SetEnvironmentVariable('ProgramFiles',$priorProgramFiles,'Process')
}
