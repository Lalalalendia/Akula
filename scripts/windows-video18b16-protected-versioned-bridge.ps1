# VIDEO-18b-16: real protected NSIS legacy-flat -> current/ lab bridge.
# Hosted pull-request-only. Never run against a real Akula installation.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$AppPath,
  [Parameter(Mandatory=$true)][string]$ShivaPath,
  [Parameter(Mandatory=$true)][string]$MuxPath,
  [Parameter(Mandatory=$true)][string]$YtDlpPath,
  [Parameter(Mandatory=$true)][string]$QuickJsPath
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
    !$env:EXPECTED_HEAD_SHA -or
    (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO18B16_HOLD: only exact-head hosted Windows PR'
}
$principal=[Security.Principal.WindowsPrincipal]::new(
  [Security.Principal.WindowsIdentity]::GetCurrent())
if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO18B16_HOLD: protected Windows fixture must be elevated'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP)
$programFiles=[IO.Path]::GetFullPath($env:ProgramFiles)
$tag=[guid]::NewGuid().ToString('N')
$root=[IO.Path]::GetFullPath((Join-Path $programFiles ('AkulaVideo18bRealNsisLab-'+$tag)))
$stage=[IO.Path]::GetFullPath((Join-Path $temp ('Video18b16Stage-'+$tag)))
$media=[IO.Path]::GetFullPath((Join-Path $temp ('Video18b16Media-'+$tag)))
$note=Join-Path $media 'private-note.md'
$v1=Join-Path $stage 'lab-flat-v1-unsigned.exe'
$v2=Join-Path $stage 'lab-versioned-v2-unsigned.exe'
$current=Join-Path $root 'current'
$names=@('akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs')
$src=[ordered]@{
  'akula-app'=$AppPath
  'shiva-pdf-preview'=$ShivaPath
  'akula-video-mux'=$MuxPath
  'akula-yt-dlp'=$YtDlpPath
  'akula-quickjs'=$QuickJsPath
}
$expected=@{}
$accountName='av16'+$tag.Substring(0,11)
$password='Av16!'+[guid]::NewGuid().ToString('N')+'P7'
$createdUser=$false
$createdStage=$false
$createdMedia=$false
$installed=$false
$success=$false
$mk=Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS\makensis.exe'
$flatNsi=Join-Path $repo 'scripts/video18b-protected-five-real.nsi'
$bridgeNsi=Join-Path $repo 'scripts/video18b16-protected-versioned-bridge.nsi'
$prefixRepo=$repo.TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
$prefixTemp=$temp.TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar
if ($root -cne (Join-Path $programFiles ('AkulaVideo18bRealNsisLab-'+$tag)) -or
    !$root.StartsWith($programFiles.TrimEnd([IO.Path]::DirectorySeparatorChar)+[IO.Path]::DirectorySeparatorChar,
      [StringComparison]::OrdinalIgnoreCase) -or
    (Test-Path -LiteralPath $root) -or
    (Test-Path -LiteralPath $stage) -or
    (Test-Path -LiteralPath $media) -or
    !(Test-Path -LiteralPath $mk -PathType Leaf) -or
    !(Test-Path -LiteralPath $flatNsi -PathType Leaf) -or
    !(Test-Path -LiteralPath $bridgeNsi -PathType Leaf)) {
  throw 'VIDEO18B16_HOLD: fixture or NSIS compiler not isolated'
}
function Assert-RealFile([string]$path) {
  $metadata=Get-Item -LiteralPath $path -Force -ErrorAction Stop
  if ($metadata.PSIsContainer -or
      ($metadata.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "VIDEO18B16_HOLD: symlink or not regular file $path"
  }
}
function Assert-RealDir([string]$path) {
  $metadata=Get-Item -LiteralPath $path -Force -ErrorAction Stop
  if (!$metadata.PSIsContainer -or
      ($metadata.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "VIDEO18B16_HOLD: junction or not real directory $path"
  }
}
function Assert-Five([string]$dir) {
  Assert-RealDir $dir
  $entries=@(Get-ChildItem -LiteralPath $dir -Force)
  if ($entries.Count -ne 5) { throw "VIDEO18B16_HOLD: wrong five-member generation $dir" }
  foreach($name in $names) {
    $file=Join-Path $dir ($name+'.exe')
    Assert-RealFile $file
    $actual=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $expected[$name]) {
      throw "VIDEO18B16_HOLD: generation SHA mismatch $name"
    }
  }
}
function Assert-Flat([string]$dir) {
  Assert-RealDir $dir
  foreach($name in $names) {
    $file=Join-Path $dir ($name+'.exe')
    Assert-RealFile $file
    if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected[$name]) {
      throw "VIDEO18B16_HOLD: legacy flat version changed $name"
    }
  }
}
function Compile-LabNsis([string]$source,[string]$output) {
  & $mk "/DLAB_ID=$tag" "/DLAB_STAGE=$stage" "/DLAB_OUTPUT=$output" $source | Out-Host
  if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $output -PathType Leaf)) {
    throw 'VIDEO18B16_HOLD: real NSIS compiler failed'
  }
}
function Invoke-LabSetup([string]$file) {
  Assert-RealFile $file
  $process=Start-Process -FilePath $file -ArgumentList '/S' -Wait -PassThru
  if ($process.ExitCode -ne 0) { throw 'VIDEO18B16_HOLD: NSIS setup returned failure' }
}
$native=@'
using System;
using System.IO;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Security.Principal;
using Microsoft.Win32.SafeHandles;
public static class AkulaVideo18b16Acl {
  [DllImport("advapi32.dll", CharSet=CharSet.Unicode, SetLastError=true, EntryPoint="LogonUserW")]
  static extern bool LogonUser(string username, string domain, string password,
      int type, int provider, out SafeAccessTokenHandle token);
  static void Denied(string label, Action operation) {
    bool denied=false;
    try {operation();}
    catch (UnauthorizedAccessException) {denied=true;}
    catch (IOException error) when (error.HResult == unchecked((int)0x80070005)) {denied=true;}
    if(!denied) throw new InvalidOperationException("VIDEO18B16_HOLD: unprivileged modification allowed: "+label);
    Console.WriteLine("VIDEO18B16_DENIED "+label);
  }
  public static string Check(string user,string password,string root,string mediaRoot) {
    SafeAccessTokenHandle token;
    if(!LogonUser(user,".",password,2,0,out token))
      throw new Win32Exception(Marshal.GetLastWin32Error(),"VIDEO18B16_HOLD: no standard-user token");
    using(token) {
      return WindowsIdentity.RunImpersonated<string>(token,()=>{
        using(var identity=WindowsIdentity.GetCurrent()){
          if(new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))
            throw new InvalidOperationException("VIDEO18B16_HOLD: test user is admin");
        }
        string[] files={"akula-app.exe","shiva-pdf-preview.exe","akula-video-mux.exe",
          "akula-yt-dlp.exe","akula-quickjs.exe"};
        string current=Path.Combine(root,"current");
        int count=0;
        foreach(string name in files){
          string file=Path.Combine(current,name);
          using(var stream=File.OpenRead(file)){
            if(stream.Length<512 || stream.ReadByte()!=77 || stream.ReadByte()!=90)
              throw new InvalidOperationException("VIDEO18B16_HOLD: standard user cannot read real PE");
          }
          Denied("replace-current:"+name,()=>File.WriteAllText(file,"ATTACK"));count++;
        }
        Denied("inject-current",()=>File.WriteAllText(Path.Combine(current,"fake.exe"),"ATTACK"));count++;
        Denied("rename-current",()=>Directory.Move(current,current+"-hijack"));count++;
        Denied("overwrite-uninstaller",()=>File.WriteAllText(Path.Combine(root,"uninstall.exe"),"ATTACK"));count++;
        Denied("overwrite-legacy",()=>File.WriteAllText(Path.Combine(root,"akula-app.exe"),"ATTACK"));count++;
        Denied("inject-stage",()=>Directory.CreateDirectory(Path.Combine(root,".staging")));count++;
        string independent=Path.Combine(mediaRoot,"nonadmin-local-media.txt");
        File.WriteAllText(independent,"LOCAL-ONLY-MEDIA");
        if(File.ReadAllText(independent)!="LOCAL-ONLY-MEDIA")
          throw new InvalidOperationException("VIDEO18B16_HOLD: independent local media not writable");
        return "VIDEO18B16_PROTECTED_STANDARD_USER_PASS denied="+count+" readable_current_pe=5 outside_media_writable=1";
      });
    }
  }
}
'@
try {
  New-Item -ItemType Directory -Path $stage -ErrorAction Stop | Out-Null
  $createdStage=$true
  New-Item -ItemType Directory -Path $media -ErrorAction Stop | Out-Null
  $createdMedia=$true
  [IO.File]::WriteAllText($note,"VIDEO18B16 private data must survive\`n",
    [Text.UTF8Encoding]::new($false))
  $noteHash=(Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
  $allow=Get-Content -LiteralPath (Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json') -Raw | ConvertFrom-Json
  if ($allow.release_admitted -ne $false -or
      $allow.installer_signing_authorized -ne $false -or
      $allow.stage_binary_distribution_authorized -ne $false -or
      @($allow.components).Count -ne 5 -or
      (@($allow.components | ForEach-Object {$_.name}) -join ',') -cne (($names | ForEach-Object {$_+'.exe'}) -join ',')) {
    throw 'VIDEO18B16_HOLD: non-release five-PE source custody changed'
  }
  foreach ($name in $names) {
    $source=[IO.Path]::GetFullPath($src[$name])
    if (!$source.StartsWith($prefixRepo,[StringComparison]::OrdinalIgnoreCase) -and
        !$source.StartsWith($prefixTemp,[StringComparison]::OrdinalIgnoreCase)) {
      throw "VIDEO18B16_HOLD: input source outside hosted ephemeral roots: $name"
    }
    Assert-RealFile $source
    $file=Join-Path $stage ($name+'.exe')
    Copy-Item -LiteralPath $source -Destination $file -ErrorAction Stop
    Assert-RealFile $file
    $check=[IO.File]::OpenRead($file)
    try {
      if ($check.Length -lt 512 -or $check.ReadByte() -ne 77 -or $check.ReadByte() -ne 90) {
        throw "VIDEO18B16_HOLD: expected genuine PE/MZ $name"
      }
    } finally { $check.Dispose() }
    $sha=(Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLowerInvariant()
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $sha) {
      throw 'VIDEO18B16_HOLD: staged PE provenance mismatch'
    }
    $expected[$name]=$sha
    if ($name -ceq 'akula-video-mux' -and
        (!$env:AKULA_BUNDLED_MUX_SHA256 -or $sha -cne $env:AKULA_BUNDLED_MUX_SHA256)) {
      throw 'VIDEO18B16_HOLD: compiled Rust mux pin mismatch'
    }
    if ($name -in @('akula-yt-dlp','akula-quickjs')) {
      $entry=@($allow.components | Where-Object {$_.name -ceq ($name+'.exe')})
      if ($entry.Count -ne 1 -or $sha -cne $entry[0].sha256 -or
          (Get-Item -LiteralPath $file).Length -ne $entry[0].size_bytes) {
        throw 'VIDEO18B16_HOLD: upstream helper SHA/size differs from pinned source'
      }
    }
  }
  Assert-Five $stage
  Compile-LabNsis $flatNsi $v1
  Invoke-LabSetup $v1
  $installed=Test-Path -LiteralPath $root -PathType Container
  if (!$installed) { throw 'VIDEO18B16_HOLD: no real flat protected NSIS installation' }
  Assert-Flat $root
  if (@(Get-ChildItem -LiteralPath $root -Force).Count -ne 6 -or
      (Test-Path -LiteralPath $current) -or
      !(Test-Path -LiteralPath (Join-Path $root 'uninstall.exe') -PathType Leaf)) {
    throw 'VIDEO18B16_HOLD: wrong initial flat NSIS layout'
  }
  $oldUninstaller=(Get-FileHash -LiteralPath (Join-Path $root 'uninstall.exe') -Algorithm SHA256).Hash
  Write-Host 'VIDEO18B16_GENUINE_LEGACY_FLAT_NSIS_PASS pe_sha256=5 protected=1'
  Compile-LabNsis $bridgeNsi $v2
  Invoke-LabSetup $v2
  Assert-Flat $root
  Assert-Five $current
  if (@(Get-ChildItem -LiteralPath $root -Force).Count -ne 7) {
    throw 'VIDEO18B16_HOLD: unexpected root entries during two-generation bridge'
  }
  $uninstall=Join-Path $root 'uninstall.exe'
  Assert-RealFile $uninstall
  if ((Get-FileHash -LiteralPath $uninstall -Algorithm SHA256).Hash -ceq $oldUninstaller) {
    throw 'VIDEO18B16_HOLD: legacy uninstaller not replaced by dual-layout cleanup'
  }
  Write-Host 'VIDEO18B16_REAL_PROTECTED_DUAL_LAYOUT_PASS old_flat=5 new_current=5 hashes_identical=1 fallback_retained=1'
  # Deliberate partial-current corruption: no verified bridge acceptance may follow.
  Remove-Item -LiteralPath (Join-Path $current 'akula-quickjs.exe') -Force -ErrorAction Stop
  $rejected=$false
  try { Assert-Five $current } catch { $rejected=$true }
  if (!$rejected) { throw 'VIDEO18B16_HOLD: partial current generation accepted' }
  Assert-Flat $root
  Copy-Item -LiteralPath (Join-Path $stage 'akula-quickjs.exe') -Destination (Join-Path $current 'akula-quickjs.exe') -ErrorAction Stop
  Assert-Five $current
  Write-Host 'VIDEO18B16_TORN_CURRENT_REJECTED_PASS old_fallback=5 restored_current=5'
  & icacls.exe $media '/inheritance:r' '/grant:r' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)M' | Out-Host
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO18B16_HOLD: cannot provision isolated local media ACL' }
  $secure=ConvertTo-SecureString -String $password -AsPlainText -Force
  New-LocalUser -Name $accountName -Password $secure -PasswordNeverExpires -ErrorAction Stop | Out-Null
  $createdUser=$true
  $sidUsers=[Security.Principal.SecurityIdentifier]::new('S-1-5-32-545')
  $members=@(Get-LocalGroupMember -SID $sidUsers -ErrorAction Stop)
  if (@($members | Where-Object { $_.Name -match [regex]::Escape($accountName) }).Count -eq 0) {
    Add-LocalGroupMember -SID $sidUsers -Member $accountName -ErrorAction Stop
  }
  if (@(Get-LocalGroupMember -Group 'Administrators' -ErrorAction Stop |
        Where-Object { $_.Name -match [regex]::Escape($accountName) }).Count -ne 0) {
    throw 'VIDEO18B16_HOLD: fixture account has administrator role'
  }
  Add-Type -TypeDefinition $native -ErrorAction Stop
  $proof=[AkulaVideo18b16Acl]::Check($accountName,$password,$root,$media)
  Write-Host $proof
  Assert-Flat $root
  Assert-Five $current
  if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteHash) {
    throw 'VIDEO18B16_HOLD: private local note changed'
  }
  $gone=Start-Process -FilePath $uninstall -ArgumentList '/S' -Wait -PassThru
  if ($gone.ExitCode -ne 0 -or (Test-Path -LiteralPath $root)) {
    throw 'VIDEO18B16_HOLD: dual-layout uninstaller leaked real PE or protected root'
  }
  $installed=$false
  if (!(Test-Path -LiteralPath $note -PathType Leaf) -or
      (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteHash -or
      (Get-Content -LiteralPath (Join-Path $media 'nonadmin-local-media.txt') -Raw).Trim() -cne 'LOCAL-ONLY-MEDIA') {
    throw 'VIDEO18B16_HOLD: user media or note destroyed by uninstall'
  }
  $success=$true
  Write-Host 'VIDEO18B16_PROTECTED_VERSIONED_BRIDGE_PASS real_exe=5 current=5 legacy_retained_until_uninstall=1 nonadmin_denied=10 complete_uninstall=1 private_data_preserved=1 release=0'
} finally {
  if ($createdUser) {
    Remove-LocalUser -Name $accountName -ErrorAction Stop
  }
  # Never recursively erase a protected installation when a lab fails. A
  # disposable GitHub-hosted image is destroyed after the failed job.
  if ($installed -and (Test-Path -LiteralPath $root)) {
    Write-Warning 'VIDEO18B16_HOLD: failed lab preserved protected root for ephemeral runner teardown'
  }
  if ($createdStage) {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction Stop
  }
  if ($createdMedia) {
    Remove-Item -LiteralPath $media -Recurse -Force -ErrorAction Stop
  }
}
if (!$success) { throw 'VIDEO18B16_HOLD: incomplete lab' }
