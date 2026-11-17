# VIDEO-18b-18b: real Minisign signed five-PE controller witness under pinned rar2 OS lock.
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
  # VIDEO18b18b: PR-only trust creation and verification of all FIVE real
  # Windows PEs, after protected dual NSIS and the non-admin ACL checks.
  $pin='32a803e67244836d2794f9bbbdf9b24deab63d7e'
  $upstream=Join-Path $repo '_rar2-pinned'
  if ((git -C $upstream rev-parse HEAD).Trim() -cne $pin) {
    throw 'VIDEO18B18B_HOLD: pinned rar2 source changed'
  }
  $manifestPath=Join-Path $repo 'labs/video-lite/video18b18b-controller/Cargo.toml'
  $lockPath=Join-Path $repo 'labs/video-lite/video18b18b-controller/Cargo.lock'
  if (!(Test-Path -LiteralPath $manifestPath -PathType Leaf) -or
      (Test-Path -LiteralPath $lockPath)) {
    throw 'VIDEO18B18B_HOLD: missing standalone lab crate or unexpected lock'
  }
  if ($env:EXPECTED_HEAD_SHA -cnotmatch '^[a-f0-9]{40}$' -or
      (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
    throw 'VIDEO18B18B_HOLD: unsigned/stale PR head'
  }
  $candidate=Join-Path $stage 'video18b18b-candidate'
  $trust=Join-Path $stage 'video18b18b-trust'
  if ((Test-Path -LiteralPath $candidate) -or (Test-Path -LiteralPath $trust)) {
    throw 'VIDEO18B18B_HOLD: fresh candidate and trust directories required'
  }
  New-Item -ItemType Directory -Path $candidate -ErrorAction Stop | Out-Null
  New-Item -ItemType Directory -Path $trust -ErrorAction Stop | Out-Null
  $records=@()
  foreach($name in $names) {
    $dest=Join-Path $candidate ($name+'.exe')
    Copy-Item -LiteralPath (Join-Path $current ($name+'.exe')) -Destination $dest -ErrorAction Stop
    # Distinct test-only bytes invalidating publisher signatures ON PURPOSE:
    # no candidate is executed or distributed.
    $stream=[IO.FileStream]::new($dest,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try {
      $stream.Seek(0,[IO.SeekOrigin]::End) | Out-Null
      $marker=[Text.Encoding]::UTF8.GetBytes("VIDEO18B18B_NONRELEASE_OVERLAY_$name")
      $stream.Write($marker,0,$marker.Length)
      $stream.Flush($true)
    } finally { $stream.Dispose() }
    Assert-RealFile $dest
    $original=(Get-FileHash -LiteralPath (Join-Path $current ($name+'.exe')) -Algorithm SHA256).Hash.ToLowerInvariant()
    $newHash=(Get-FileHash -LiteralPath $dest -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($original -ceq $newHash) { throw "VIDEO18B18B_HOLD: candidate isn't distinct: $name" }
    $records+=([ordered]@{
      filename=($name+'.exe')
      sha256=$newHash
      size=(Get-Item -LiteralPath $dest).Length
    })
  }
  $lockDigest=(Get-FileHash -LiteralPath (Join-Path $repo 'aquilum-app/Cargo.lock') -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($lockDigest -cnotmatch '^[a-f0-9]{64}$' -or $records.Count -ne 5) {
    throw 'VIDEO18B18B_HOLD: source lock / exact five PE records absent'
  }
  $signedManifest=[ordered]@{
    schema=1
    product='Akula'
    platform='windows-x86_64'
    purpose='protected-video-five-exe'
    version='0.3.21'
    source_commit=$env:EXPECTED_HEAD_SHA
    cargo_lock_sha256=$lockDigest
    components=$records
  }
  $manifestFile=Join-Path $trust 'five-manifest.json'
  [IO.File]::WriteAllText($manifestFile,
    ($signedManifest | ConvertTo-Json -Depth 8 -Compress)+[Environment]::NewLine,
    [Text.UTF8Encoding]::new($false))
  $mini=(Get-Command minisign.exe -CommandType Application -ErrorAction Stop).Source
  Assert-RealFile $mini
  $key=Join-Path $trust 'test-publisher.key'
  $pub=Join-Path $trust 'test-publisher.pub'
  $otherKey=Join-Path $trust 'other.key'
  $otherPub=Join-Path $trust 'other.pub'
  & $mini -G -W -s $key -p $pub
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO18B18B_HOLD: first one-time key generation failed' }
  & $mini -G -W -s $otherKey -p $otherPub
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO18B18B_HOLD: alternate key generation failed' }
  $signature=Join-Path $trust 'five-manifest.minisig'
  & $mini -S -s $key -m $manifestFile -x $signature -c 'VIDEO18B18B NONRELEASE' -t 'PR ephemeral test only'
  if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $signature -PathType Leaf)) {
    throw 'VIDEO18B18B_HOLD: Minisign signature missing'
  }
  & $mini -Vm $manifestFile -x $signature -p $pub -Q
  if ($LASTEXITCODE -ne 0) { throw 'VIDEO18B18B_HOLD: independent signature verification failed' }
  Write-Host 'VIDEO18B18B_REAL_MINISIGN_MANIFEST_PASS exact_head=1 current_lock_hash=1 real_pe=5 key_ephemeral=1 release=0'

  $env:AKULA_VIDEO18B18B_INSTALL_ROOT=$root
  $env:AKULA_VIDEO18B18B_STAGE_ROOT=$stage
  $env:AKULA_VIDEO18B18B_CARGO_LOCK_SHA256=$lockDigest
  try {
    cargo generate-lockfile --manifest-path $manifestPath
    if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $lockPath -PathType Leaf)) {
      throw 'VIDEO18B18B_HOLD: disposable lab dependency graph cannot resolve'
    }
    Write-Host "VIDEO18B18B_EPHEMERAL_LOCK_NOT_RELEASE sha256=$((Get-FileHash $lockPath -Algorithm SHA256).Hash.ToLowerInvariant())"
    cargo test --locked --manifest-path $manifestPath --lib -- --nocapture --test-threads=1
    if ($LASTEXITCODE -ne 0) {
      throw 'VIDEO18B18B_HOLD: signed-five and original rar2 transaction did not agree'
    }
  } finally {
    Remove-Item Env:AKULA_VIDEO18B18B_INSTALL_ROOT -ErrorAction SilentlyContinue
    Remove-Item Env:AKULA_VIDEO18B18B_STAGE_ROOT -ErrorAction SilentlyContinue
    Remove-Item Env:AKULA_VIDEO18B18B_CARGO_LOCK_SHA256 -ErrorAction SilentlyContinue
  }
  Assert-Flat $root
  Assert-Five $current
  Write-Host 'VIDEO18B18B_SIGNED_CONTROLLER_POST_RECOVERY_FIVE_PASS'

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
