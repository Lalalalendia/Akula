# VIDEO-TOOLS-01h: isolated benign Windows CreateProcessW under 01g held handles.
# No video media executable is launched. GitHub-hosted Windows PR only.
#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
   $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
   $env:EXPECTED_HEAD_SHA -notmatch '^[a-f0-9]{40}$'){
  throw 'VIDEO_TOOLS_01H_HOLD: exact-head hosted Windows PR only'
}
$head=(git rev-parse HEAD).Trim()
if($LASTEXITCODE -ne 0 -or $head -cne $env:EXPECTED_HEAD_SHA){
  throw 'VIDEO_TOOLS_01H_HOLD: nonreviewed checkout'
}
$token=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(!$token.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){
  throw 'VIDEO_TOOLS_01H_HOLD: elevated ephemeral Windows runner required'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
. (Join-Path $repo 'scripts/video-tools-01d-acl.ps1')
$tag=[guid]::NewGuid().ToString('N')
$root=Join-Path (Get-AkulaVideoToolsOsProgramFiles) ('AkulaVideoToolsLab-'+$tag)
$work=Join-Path ([IO.Path]::GetFullPath($env:RUNNER_TEMP)) ('video-tools-01h-'+$tag)
$source=Join-Path $work 'benign-child.exe'
$witness=Join-Path $work 'native-boundary-witness.exe'
$stdout=Join-Path $work 'witness-out.txt'
$stderr=Join-Path $work 'witness-err.txt'
$names=@('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe','uninstall.exe')
Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag
if((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $work)){
  throw 'VIDEO_TOOLS_01H_HOLD: disposable lab already exists'
}
function Set-PrivateLabAcl {
  param([string]$Path,[bool]$Directory)
  $admins=[Security.Principal.SecurityIdentifier]::new('S-1-5-32-544')
  $system=[Security.Principal.SecurityIdentifier]::new('S-1-5-18')
  $users=[Security.Principal.SecurityIdentifier]::new('S-1-5-32-545')
  $acl=Get-Acl -LiteralPath $Path -ErrorAction Stop
  $acl.SetAccessRuleProtection($true,$false)
  foreach($rule in @($acl.Access)){$acl.RemoveAccessRuleSpecific($rule)}
  $acl.SetOwner($admins)
  $inherit=if($Directory){[Security.AccessControl.InheritanceFlags]::ContainerInherit -bor [Security.AccessControl.InheritanceFlags]::ObjectInherit}else{[Security.AccessControl.InheritanceFlags]::None}
  foreach($item in @(
    @{Sid=$admins;Rights=[Security.AccessControl.FileSystemRights]::FullControl},
    @{Sid=$system;Rights=[Security.AccessControl.FileSystemRights]::FullControl},
    @{Sid=$users;Rights=[Security.AccessControl.FileSystemRights]::ReadAndExecute}
  )){
    $ace=[Security.AccessControl.FileSystemAccessRule]::new($item.Sid,$item.Rights,$inherit,[Security.AccessControl.PropagationFlags]::None,[Security.AccessControl.AccessControlType]::Allow)
    $acl.AddAccessRule($ace)
  }
  Set-Acl -LiteralPath $Path -AclObject $acl -ErrorAction Stop
}
$createdRoot=$false
$createdWork=$false
$proc=$null
try {
  New-Item -ItemType Directory -Path $work -ErrorAction Stop | Out-Null
  $createdWork=$true
  & rustc --edition=2024 (Join-Path $repo 'scripts/video-tools-01h-benign-child.rs') -o $source
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01H_HOLD: benign source fixture build failed'}
  $native=Join-Path $repo 'aquilum-app/native/src/video/tools_package_01h_process_boundary_windows.rs'
  $tests=Join-Path $work 'native-unit-tests.exe'
  & rustc --edition=2024 --test $native -o $tests
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01H_HOLD: native boundary unit compilation failed'}
  & $tests --nocapture --test-threads=1 | Out-Host
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01H_HOLD: native boundary unit tests failed'}
  & rustc --edition=2024 --cfg akula_video_tools_01h_witness $native -o $witness
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01H_HOLD: CreateProcessW witness build failed'}
  New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
  $createdRoot=$true
  Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists
  Set-PrivateLabAcl -Path $root -Directory $true
  $digest=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
  foreach($name in $names){
    $dest=Join-Path $root $name
    Copy-Item -LiteralPath $source -Destination $dest -ErrorAction Stop
    Set-PrivateLabAcl -Path $dest -Directory $false
    if((Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash -cne $digest){
      throw 'VIDEO_TOOLS_01H_HOLD: installed benign PE source differs'
    }
  }
  Assert-AkulaVideoToolsInstalledAcl -Path $root -LabId $tag
  Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class AkulaVideoTools01hMutation {
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,EntryPoint="CreateFileW",SetLastError=true)]
  public static extern SafeFileHandle Open(string path,uint access,uint share,IntPtr sec,uint disposition,uint flags,IntPtr templateFile);
  [DllImport("kernel32.dll",CharSet=CharSet.Unicode,EntryPoint="MoveFileExW",SetLastError=true)]
  public static extern bool Rename(string source,string target,uint flags);
}
'@
  function Assert-WriteAvailable {
    $h=[AkulaVideoTools01hMutation]::Open((Join-Path $root $names[0]),[uint32]0x40000000,7,[IntPtr]::Zero,3,0,[IntPtr]::Zero)
    if($h.IsInvalid){$h.Dispose();throw 'VIDEO_TOOLS_01H_HOLD: elevated baseline write was unavailable'}
    $h.Dispose()
  }
  Assert-WriteAvailable
  $saved=@{}
  foreach($key in @('AKULA_VIDEO_TOOLS_01H_ROOT','AKULA_VIDEO_TOOLS_01H_SOURCE','AKULA_VIDEO_TOOLS_01H_WORK')){
    $saved[$key]=[Environment]::GetEnvironmentVariable($key,'Process')
  }
  try {
    $env:AKULA_VIDEO_TOOLS_01H_ROOT=$root
    $env:AKULA_VIDEO_TOOLS_01H_SOURCE=$source
    $env:AKULA_VIDEO_TOOLS_01H_WORK=$work
    $proc=Start-Process -FilePath $witness -PassThru -NoNewWindow -RedirectStandardOutput $stdout -RedirectStandardError $stderr -ErrorAction Stop
    function Await-Witness {
      param([string]$Marker)
      $path=Join-Path $work $Marker
      for($i=0;$i -lt 300 -and !(Test-Path -LiteralPath $path);$i++){
        if($proc.HasExited){break}
        Start-Sleep -Milliseconds 100
      }
      if(!(Test-Path -LiteralPath $path) -or $proc.HasExited){
        $details=if(Test-Path -LiteralPath $stderr){Get-Content -LiteralPath $stderr -Raw}else{'no stderr'}
        throw ('VIDEO_TOOLS_01H_HOLD: native handoff barrier failed '+$Marker+' '+$details)
      }
    }
    function Assert-SharingViolation {
      param([string]$Target,[uint32]$Access,[uint32]$Flags)
      $h=[AkulaVideoTools01hMutation]::Open($Target,$Access,7,[IntPtr]::Zero,3,$Flags,[IntPtr]::Zero)
      $code=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
      if(!$h.IsInvalid){$h.Dispose();throw 'VIDEO_TOOLS_01H_HOLD: concurrent mutation handle was granted'}
      $h.Dispose()
      if($code -ne 32){throw ('VIDEO_TOOLS_01H_HOLD: expected ERROR_SHARING_VIOLATION=32, got '+$code)}
    }
    function Attempt-Rename {
      $dest=Join-Path $root 'unexpected-rename.exe'
      $ok=[AkulaVideoTools01hMutation]::Rename((Join-Path $root $names[0]),$dest,0)
      $code=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
      if($ok -or $code -ne 32 -or (Test-Path -LiteralPath $dest)){
        throw ('VIDEO_TOOLS_01H_HOLD: unexpected rename result '+$code)
      }
    }
    function Check-WhilePinned {
      Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists
      Assert-SharingViolation -Target $root -Access ([uint32]0x10000) -Flags ([uint32]0x02000000)
      foreach($name in $names){
        $target=Join-Path $root $name
        Assert-SharingViolation -Target $target -Access ([uint32]0x40000000) -Flags ([uint32]0)
        Assert-SharingViolation -Target $target -Access ([uint32]0x10000) -Flags ([uint32]0)
      }
      Attempt-Rename
    }
    Await-Witness -Marker '01h-pinned'
    Check-WhilePinned
    Set-Content -LiteralPath (Join-Path $work '01h-start') -Value 'start' -NoNewline
    Await-Witness -Marker '01h-launched'
    if(!(Test-Path -LiteralPath (Join-Path $work '01h-child-started'))){
      throw 'VIDEO_TOOLS_01H_HOLD: child image did not start'
    }
    Check-WhilePinned
    Set-Content -LiteralPath (Join-Path $work '01h-finish') -Value 'finish' -NoNewline
    if(!$proc.WaitForExit(30000) -or $proc.ExitCode -ne 0){
      $details=if(Test-Path -LiteralPath $stderr){Get-Content -LiteralPath $stderr -Raw}else{'no stderr'}
      throw ('VIDEO_TOOLS_01H_HOLD: native process handoff failed '+$details)
    }
    $output=Get-Content -LiteralPath $stdout -Raw -ErrorAction Stop
    if(!$output.Contains('VIDEO_TOOLS_01H_CREATEPROCESS_PASS benign=1 signed_tools_executed=0') -or
       !(Test-Path -LiteralPath (Join-Path $work '01h-child-exited'))){
      throw 'VIDEO_TOOLS_01H_HOLD: genuine benign CreateProcessW result missing'
    }
    Assert-WriteAvailable
    foreach($name in $names){
      if((Get-FileHash -LiteralPath (Join-Path $root $name) -Algorithm SHA256).Hash -cne $digest){
        throw 'VIDEO_TOOLS_01H_HOLD: installed benign bytes changed during race'
      }
    }
    Assert-AkulaVideoToolsInstalledAcl -Path $root -LabId $tag
    Write-Output 'VIDEO_TOOLS_01H_HANDOFF_PASS benign=1 before_and_after_mutations_denied=1 createprocess_while_held=1 child_started=1 write_after_drop=1 hashes=4 release=0'
  } finally {
    foreach($key in $saved.Keys){[Environment]::SetEnvironmentVariable($key,$saved[$key],'Process')}
  }
} finally {
  if($null -ne $proc -and !$proc.HasExited){
    if($createdWork){
      Set-Content -LiteralPath (Join-Path $work '01h-start') -Value 'stop' -NoNewline -ErrorAction SilentlyContinue
      Set-Content -LiteralPath (Join-Path $work '01h-finish') -Value 'stop' -NoNewline -ErrorAction SilentlyContinue
    }
    if(!$proc.WaitForExit(10000)){$proc.Kill($true);$proc.WaitForExit()}
  }
  if($createdRoot){
    Assert-AkulaVideoToolsLabRoot -Path $root -LabId $tag -RequireExists
    $items=@(Get-ChildItem -LiteralPath $root -Force)
    if($items.Count -ne $names.Count -or @($items|Where-Object{$_.Name -cnotin $names}).Count -ne 0){
      throw 'VIDEO_TOOLS_01H_HOLD: refusing unsafe cleanup of unknown protected files'
    }
    foreach($item in $items){
      if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){
        throw 'VIDEO_TOOLS_01H_HOLD: refusing removal of aliased protected file'
      }
      Remove-Item -LiteralPath $item.FullName -Force -ErrorAction Stop
    }
    Remove-Item -LiteralPath $root -ErrorAction Stop
  }
  if($createdWork){Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction Stop}
}
