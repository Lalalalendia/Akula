# VIDEO-TOOLS-01d: genuine Windows ACL mutation witness on disposable NSIS files only.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$Root,
  [Parameter(Mandatory=$true)][string]$LabId
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01D_HOLD: exact-head hosted Windows PR only'
}
$actualHead = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualHead -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO_TOOLS_01D_HOLD: checkout not at reviewed PR HEAD'
}
$principal = [Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01D_HOLD: ACL mutation laboratory needs elevated admin token'
}
$repo = [IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
. (Join-Path $repo 'scripts/video-tools-01d-acl.ps1')
Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
function Assert-UnsafeGrantRejected {
  param(
    [Parameter(Mandatory=$true)][string]$Target,
    [Parameter(Mandatory=$true)][string]$UntrustedSid,
    [Parameter(Mandatory=$true)][Security.AccessControl.FileSystemRights]$GrantedRights
  )
  Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists
  $saved = Get-Acl -LiteralPath $Target -ErrorAction Stop
  $poisoned = Get-Acl -LiteralPath $Target -ErrorAction Stop
  $sid = [Security.Principal.SecurityIdentifier]::new($UntrustedSid)
  $rule = [Security.AccessControl.FileSystemAccessRule]::new(
    $sid, $GrantedRights, [Security.AccessControl.AccessControlType]::Allow)
  $poisoned.AddAccessRule($rule)
  $changed = $false
  try {
    Set-Acl -LiteralPath $Target -AclObject $poisoned -ErrorAction Stop
    $changed = $true
    $rejected = $false
    try {
      Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
    } catch {
      if ($_.Exception.Message.Contains('VIDEO_TOOLS_01D_HOLD: unsafe writable ACL')) {
        $rejected = $true
      } else { throw }
    }
    if (!$rejected) {
      throw ('VIDEO_TOOLS_01D_HOLD: ACL mutation was accepted: ' + $Target)
    }
  } finally {
    if ($changed) {
      # Always restore the exact previous DACL before any uninstall/cleanup.
      Set-Acl -LiteralPath $Target -AclObject $saved -ErrorAction Stop
      Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
    }
  }
}
Assert-UnsafeGrantRejected -Target $Root -UntrustedSid 'S-1-1-0' -GrantedRights WriteData
Assert-UnsafeGrantRejected -Target $Root -UntrustedSid 'S-1-5-32-545' -GrantedRights Delete
Assert-UnsafeGrantRejected -Target (Join-Path $Root 'akula-yt-dlp.exe') -UntrustedSid 'S-1-5-11' -GrantedRights WriteAttributes
Assert-UnsafeGrantRejected -Target (Join-Path $Root 'akula-video-mux.exe') -UntrustedSid 'S-1-5-32-545' -GrantedRights ChangePermissions
Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
Write-Output 'VIDEO_TOOLS_01D_ACL_MUTATION_PASS root_write=0 root_delete=0 exe_attribute_write=0 exe_dacl_write=0 restored=1 release=0'
