# VIDEO-TOOLS-01f: actual non-admin Windows access-rights test, PR-only.
# No publisher signing, no Video Tools launches, no release.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$Root,
  [Parameter(Mandatory=$true)][string]$LabId,
  [Parameter(Mandatory=$true)][string]$Work,
  [switch]$SignedSix
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$') {
  throw 'VIDEO_TOOLS_01F_HOLD: exact-head hosted Windows PR required'
}
$head=(git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $head -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO_TOOLS_01F_HOLD: checkout does not match reviewed head'
}
$principal=[Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01F_HOLD: the ephemeral user setup requires admin'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
. (Join-Path $repo 'scripts/video-tools-01d-acl.ps1')
if ($SignedSix) {
  $expectedRoot=Join-Path (Get-AkulaVideoToolsOsProgramFiles) 'Akula Video Tools'
  if (![IO.Path]::GetFullPath($Root).Equals(
      [IO.Path]::GetFullPath($expectedRoot),[StringComparison]::OrdinalIgnoreCase)) {
    throw 'VIDEO_TOOLS_01P_HOLD: signed package root is not OS-known'
  }
  Assert-AkulaVideoToolsNoReparseParents -Path $Root -RequireExists
  $items=@(Get-ChildItem -LiteralPath $Root -Force -ErrorAction Stop)
  $expectedNames=@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe',
                   'manifest.json','manifest.minisig','uninstall.exe')
  if ($items.Count -ne 6 -or @($items | Where-Object {
      $_.Name -cnotin $expectedNames -or $_.PSIsContainer -or
      ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0
    }).Count -ne 0) {
    throw 'VIDEO_TOOLS_01P_HOLD: non-exact signed six-file package'
  }
} else {
  Assert-AkulaVideoToolsLabRoot -Path $Root -LabId $LabId -RequireExists
  Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
}
$tempBase=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
if (![IO.Path]::GetFullPath($Work).StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'VIDEO_TOOLS_01F_HOLD: test output escaped runner temp'
}
$names=if($SignedSix){$expectedNames}else{@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe','uninstall.exe')}
$before=@{}
foreach($name in $names) {
  $before[$name]=(Get-FileHash -LiteralPath (Join-Path $Root $name) -Algorithm SHA256).Hash
}
$user='akv01f' + $LabId.Substring(0,10)
if (Get-LocalUser -Name $user -ErrorAction SilentlyContinue) {
  throw 'VIDEO_TOOLS_01F_HOLD: temporary account collision'
}
# Unique strong test credential stays in memory, never output or stored in artifacts.
$plain='Xk9!' + [Convert]::ToHexString(
  [Security.Cryptography.RandomNumberGenerator]::GetBytes(24)) + 'aB'
$secret=ConvertTo-SecureString -String $plain -AsPlainText -Force
$plain=$null
$created=$false
$probeCreated=$false
$probeDir=$null
$childSource=@'
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$root=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('__ROOT_B64__'))
$expected='__EXPECTED_SID__'
$signedSix=[bool]::Parse('__SIGNED_SIX__')
$id=[Security.Principal.WindowsIdentity]::GetCurrent()
$principal=[Security.Principal.WindowsPrincipal]::new($id)
if ($id.User.Value -cne $expected -or
    $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO_TOOLS_01F_HOLD: child is not the intended standard account'
}
$exe=Join-Path $root 'akula-video-mux.exe'
$stream=[IO.File]::OpenRead($exe)
try {
  if ($stream.ReadByte() -lt 0) {
    throw 'VIDEO_TOOLS_01F_HOLD: installed executable unreadable'
  }
} finally {
  $stream.Dispose()
}
# Request real Windows access rights WITHOUT actually editing protected files.
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class AkulaVideoToolsAccessProbe {
  [DllImport("kernel32.dll", EntryPoint = "CreateFileW",
    CharSet = CharSet.Unicode, SetLastError = true)]
  public static extern SafeFileHandle Open(
    string path, uint access, uint share, IntPtr security,
    uint disposition, uint flags, IntPtr templateFile);
}
"@
$cases=@(
  @{ Name='root_create'; Path=$root; Access=[uint32]0x2; Flags=[uint32]0x02000000 },
  @{ Name='root_deletechild'; Path=$root; Access=[uint32]0x40; Flags=[uint32]0x02000000 },
  @{ Name='exe_write'; Path=$exe; Access=[uint32]0x2; Flags=[uint32]0 },
  @{ Name='exe_attributes'; Path=$exe; Access=[uint32]0x100; Flags=[uint32]0 },
  @{ Name='exe_delete'; Path=$exe; Access=[uint32]0x10000; Flags=[uint32]0 },
  @{ Name='exe_change_acl'; Path=$exe; Access=[uint32]0x40000; Flags=[uint32]0 },
  @{ Name='exe_take_ownership'; Path=$exe; Access=[uint32]0x80000; Flags=[uint32]0 }
)
foreach($case in $cases) {
  $handle=[AkulaVideoToolsAccessProbe]::Open(
    $case.Path,$case.Access,7,[IntPtr]::Zero,3,$case.Flags,[IntPtr]::Zero)
  $code=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
  if (!$handle.IsInvalid) {
    $handle.Dispose()
    throw ('VIDEO_TOOLS_01F_HOLD: forbidden right granted ' + $case.Name)
  }
  $handle.Dispose()
  if ($code -ne 5) {
    throw ('VIDEO_TOOLS_01F_HOLD: expected ERROR_ACCESS_DENIED=5 for ' +
      $case.Name + ', got ' + $code)
  }
}
if ($signedSix) {
  # Every authenticated metadata/PE/uninstaller file is readable, but
  # none grants write/delete/ownership/ACL-change to a genuine local user.
  $six=@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe',
         'manifest.json','manifest.minisig','uninstall.exe')
  $denied=0
  foreach($name in $six) {
    $filePath=Join-Path $root $name
    $stream=[IO.File]::OpenRead($filePath)
    try {
      if($stream.ReadByte() -lt 0){throw 'VIDEO_TOOLS_01P_HOLD: unreadable '+$name}
    } finally {$stream.Dispose()}
    foreach($access in @([uint32]0x2,[uint32]0x10000,[uint32]0x40000,[uint32]0x80000)) {
      $handle=[AkulaVideoToolsAccessProbe]::Open($filePath,$access,7,[IntPtr]::Zero,3,0,[IntPtr]::Zero)
      $code=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
      if(!$handle.IsInvalid) {
        $handle.Dispose()
        throw 'VIDEO_TOOLS_01P_HOLD: signed component unexpectedly writable '+$name
      }
      $handle.Dispose()
      if($code -ne 5){throw ('VIDEO_TOOLS_01P_HOLD: expected ERROR_ACCESS_DENIED for '+$name+', got '+$code)}
      $denied++
    }
  }
  if($denied -ne 24){throw 'VIDEO_TOOLS_01P_HOLD: incomplete signed component access denial'}
  Write-Output 'VIDEO_TOOLS_01P_CHILD_PASS standard_token=1 signed_six_read=6 denied_rights=24'
}
Write-Output 'VIDEO_TOOLS_01F_CHILD_PASS standard_token=1 read=1 denied_rights=7'
'@
try {
  $account=New-LocalUser -Name $user -Password $secret -AccountNeverExpires -PasswordNeverExpires -UserMayNotChangePassword -Description 'Akula PR-only temporary ACL test' -ErrorAction Stop
  $created=$true
  if ($account.SID.Value -notmatch '^S-1-5-21-') {
    throw 'VIDEO_TOOLS_01F_HOLD: temporary user SID invalid'
  }
  $group=Get-LocalGroup -SID ([Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'))
  $members=@(Get-LocalGroupMember -Group $group -ErrorAction Stop)
  if (@($members | Where-Object { $_.SID.Value -eq $account.SID.Value }).Count -eq 0) {
    Add-LocalGroupMember -Group $group -Member $account -ErrorAction Stop
  }
  $credential=[PSCredential]::new("$env:COMPUTERNAME\$user",$secret)
  $rootB64=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Root))
  $child=$childSource.Replace('__ROOT_B64__',$rootB64).Replace('__EXPECTED_SID__',$account.SID.Value).Replace('__SIGNED_SIX__',$SignedSix.ToString())
  # CreateProcessWithLogonW accepts at most 1024 command-line characters.
  # A full -EncodedCommand exceeded this limit and failed with ERROR_INVALID_PARAMETER.
  # Put non-secret fixture code in a unique admin-owned ProgramData directory instead.
  $common=[Environment]::GetFolderPath('CommonApplicationData')
  $probeDir=Join-Path $common ('AkulaVideoTools01f-' + $LabId)
  if (Test-Path -LiteralPath $probeDir) {
    throw 'VIDEO_TOOLS_01F_HOLD: disposable child script path already exists'
  }
  New-Item -ItemType Directory -Path $probeDir -ErrorAction Stop | Out-Null
  $probeCreated=$true
  $probeFile=Join-Path $probeDir 'probe.ps1'
  [IO.File]::WriteAllText($probeFile,$child,[Text.UTF8Encoding]::new($false))
  $probeHash=(Get-FileHash -LiteralPath $probeFile -Algorithm SHA256).Hash
  # Verify no broad trustee may edit the source executed by the standard user.
  $probeAcl=Get-Acl -LiteralPath $probeFile -ErrorAction Stop
  $mutating=[int][Security.AccessControl.FileSystemRights]::WriteData -bor
            [int][Security.AccessControl.FileSystemRights]::ChangePermissions -bor
            [int][Security.AccessControl.FileSystemRights]::Delete
  foreach ($ace in $probeAcl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])) {
    if ($ace.AccessControlType -eq [Security.AccessControl.AccessControlType]::Allow -and
        $ace.IdentityReference.Value -in @('S-1-1-0','S-1-5-11','S-1-5-32-545') -and
        ([int]$ace.FileSystemRights -band $mutating) -ne 0) {
      throw 'VIDEO_TOOLS_01F_HOLD: child script is writable by ordinary users'
    }
  }
  $stdout=Join-Path $Work 'video-tools-01f-stdout.txt'
  $stderr=Join-Path $Work 'video-tools-01f-stderr.txt'
  $pwsh=Join-Path $PSHOME 'pwsh.exe'
  if (!(Test-Path -LiteralPath $pwsh -PathType Leaf)) {
    throw 'VIDEO_TOOLS_01F_HOLD: Windows PowerShell binary missing'
  }
  # Start a genuine separate local-user logon with a short -File argument.
  # It must not inherit the elevated runner's security token.
  $args=@('-NoProfile','-NonInteractive','-File',('"' + $probeFile + '"'))
  $process=Start-Process -FilePath $pwsh -Credential $credential -LoadUserProfile -WorkingDirectory $common -ArgumentList $args -RedirectStandardOutput $stdout -RedirectStandardError $stderr -Wait -PassThru -ErrorAction Stop
  if ((Get-FileHash -LiteralPath $probeFile -Algorithm SHA256).Hash -cne $probeHash) {
    throw 'VIDEO_TOOLS_01F_HOLD: standard user changed the child script'
  }
  $output=Get-Content -LiteralPath $stdout -Raw -ErrorAction Stop
  if ($process.ExitCode -ne 0 -or
      !$output.Contains('VIDEO_TOOLS_01F_CHILD_PASS standard_token=1 read=1 denied_rights=7')) {
    $diagnostic=if (Test-Path -LiteralPath $stderr) {
      (Get-Content -LiteralPath $stderr -Raw -ErrorAction Stop)
    } else { 'no stderr' }
    throw ('VIDEO_TOOLS_01F_HOLD: actual non-admin OS test failed exit=' +
      $process.ExitCode + ': ' + $diagnostic.Substring(0,[Math]::Min(2048,$diagnostic.Length)))
  }
  if($SignedSix) {
    if(!$output.Contains('VIDEO_TOOLS_01P_CHILD_PASS standard_token=1 signed_six_read=6 denied_rights=24')){
      throw 'VIDEO_TOOLS_01P_HOLD: actual six-file Windows standard-user witness missing'
    }
    Assert-AkulaVideoToolsNoReparseParents -Path $Root -RequireExists
  } else {
    Assert-AkulaVideoToolsInstalledAcl -Path $Root -LabId $LabId
  }
  foreach($name in $names) {
    if ((Get-FileHash -LiteralPath (Join-Path $Root $name) -Algorithm SHA256).Hash -cne $before[$name]) {
      throw 'VIDEO_TOOLS_01F_HOLD: installed bytes changed during standard-user test'
    }
  }
  if($SignedSix) {
    Write-Output 'VIDEO_TOOLS_01P_SIGNED_NONADMIN_PASS token=standard_user signed_six_read=6 denied_rights=24 six_sha_unchanged=1 release=0'
  } else {
    Write-Output 'VIDEO_TOOLS_01F_NONADMIN_PASS token=standard_user read=1 create=0 write=0 delete=0 acl=0 ownership=0 restored=1 release=0'
  }
} finally {
  try {
    if ($created) {
      Remove-LocalUser -Name $user -ErrorAction Stop
      if (Get-LocalUser -Name $user -ErrorAction SilentlyContinue) {
        throw 'VIDEO_TOOLS_01F_HOLD: disposable Windows user still exists'
      }
    }
  } finally {
    if ($probeCreated) {
      $expectedDir=Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) ('AkulaVideoTools01f-' + $LabId)
      if (![IO.Path]::GetFullPath($probeDir).Equals(
          [IO.Path]::GetFullPath($expectedDir),[StringComparison]::OrdinalIgnoreCase)) {
        throw 'VIDEO_TOOLS_01F_HOLD: unsafe child script cleanup target'
      }
      $dirInfo=Get-Item -LiteralPath $probeDir -Force -ErrorAction Stop
      $contents=@(Get-ChildItem -LiteralPath $probeDir -Force -ErrorAction Stop)
      if (($dirInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
          $contents.Count -ne 1 -or $contents[0].Name -cne 'probe.ps1' -or
          ($contents[0].Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw 'VIDEO_TOOLS_01F_HOLD: unexpected disposable child script contents'
      }
      Remove-Item -LiteralPath (Join-Path $probeDir 'probe.ps1') -Force -ErrorAction Stop
      Remove-Item -LiteralPath $probeDir -ErrorAction Stop
    }
  }
}
