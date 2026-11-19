# VIDEO-TOOLS-01o: real signed, distinct PE generations. CI-only, NO release.
#requires -Version 7.2
param([Parameter(Mandatory=$true)][string]$MuxPath,
      [Parameter(Mandatory=$true)][string]$YtDlpPath,
      [Parameter(Mandatory=$true)][string]$QuickJsPath,
      [Parameter(Mandatory=$true)][string]$ProofFile)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
   $env:GITHUB_EVENT_NAME -cne 'pull_request' -or
   $env:EXPECTED_HEAD_SHA -notmatch '^[0-9a-f]{40}$'){
  throw 'VIDEO_TOOLS_01O_HOLD: hosted exact-HEAD Windows PR only'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$head=(git -C $repo rev-parse HEAD).Trim()
if($LASTEXITCODE -ne 0 -or $head -cne $env:EXPECTED_HEAD_SHA){
  throw 'VIDEO_TOOLS_01O_HOLD: checkout differs from PR head'
}
$principal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){
  throw 'VIDEO_TOOLS_01O_HOLD: real elevated Windows installation required'
}
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
$requiredProof=Join-Path $temp ('akula-video-tools-01o-'+$env:EXPECTED_HEAD_SHA+'.proof')
if(![IO.Path]::GetFullPath($ProofFile).Equals($requiredProof,[StringComparison]::OrdinalIgnoreCase) -or
   (Test-Path -LiteralPath $ProofFile)){
  throw 'VIDEO_TOOLS_01O_HOLD: invalid proof file'
}
$signer=[IO.Path]::GetFullPath($env:VIDEO_TOOLS_01J_MINISIGN)
if(!$signer.StartsWith($temp,[StringComparison]::OrdinalIgnoreCase) -or
   !(Test-Path -LiteralPath $signer -PathType Leaf)){
  throw 'VIDEO_TOOLS_01O_HOLD: sha256-pinned CI signer missing'
}
. (Join-Path $repo 'scripts/video-tools-01c-os-known-root.ps1')
$root=Join-Path (Get-AkulaVideoToolsOsProgramFiles) 'Akula Video Tools'
$oldBuild=[IO.Path]::GetFullPath((Join-Path $repo 'labs/video-lite/target/release/dash.exe'))
$native=Join-Path $repo 'labs/video-tools-01j/target/release/akula-video-tools-01j-signed-ci-witness.exe'
if((Test-Path -LiteralPath $root) -or
   ![IO.Path]::GetFullPath($MuxPath).Equals($oldBuild,[StringComparison]::OrdinalIgnoreCase) -or
   !(Test-Path -LiteralPath $native -PathType Leaf)){
  throw 'VIDEO_TOOLS_01O_HOLD: unexpected protected product or unreviewed binaries'
}
$work=Join-Path $temp ('video-tools-01o-'+[guid]::NewGuid().ToString('N'))
$created=$false
$installed=$false
$key=Join-Path $work 'CI-ONLY-NOT-PUBLISHER.key'
$pub=Join-Path $work 'CI-ONLY-NOT-PUBLISHER.pub'
$three=@{};$unsigned=@{};$five=@{};$installer=@{}
function Assert-Identity([string]$binary,[string]$version) {
  $output=@(& $binary '--video-tools-lab-build-id' 2>&1)
  if($LASTEXITCODE -ne 0 -or ($output|Out-String).Trim() -cne ('AKULA_VIDEO_TOOLS_LAB_MUX_VERSION='+$version)){
    throw 'VIDEO_TOOLS_01O_HOLD: binary compilation does not report expected identity'
  }
}
function Assert-Native([string]$mode,[string]$version) {
  Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
  if(@(Get-ChildItem -LiteralPath $root -Force).Count -ne 6){
    throw 'VIDEO_TOOLS_01O_HOLD: protected install does not contain six files'
  }
  $output=@(& $native $mode $root $pub 2>&1)
  $expected=if($mode -ceq 'valid'){'VIDEO_TOOLS_01J_NATIVE_SIGNED_INSTALLED_PASS'}else{
    'VIDEO_TOOLS_01M_NATIVE_STATE_PASS state=incompatible-version'}
  if($LASTEXITCODE -ne 0 -or !(($output|Out-String).Contains($expected))){
    throw ('VIDEO_TOOLS_01O_HOLD: native protected verifier did not authenticate '+$mode)
  }
  Assert-Identity (Join-Path $root 'akula-video-mux.exe') $version
}
function Uninstall-Lab {
  $exe=Join-Path $root 'uninstall.exe'
  $result=Start-Process -FilePath $exe -ArgumentList '/S' -PassThru -Wait
  if($result.ExitCode -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: protected uninstall failed'}
  if(Test-Path -LiteralPath $root){
    if(@(Get-ChildItem -LiteralPath $root -Force).Count -ne 0){
      throw 'VIDEO_TOOLS_01O_HOLD: uninstall left protected objects'
    }
    Remove-Item -LiteralPath $root
  }
}
try {
  New-Item -ItemType Directory -Path $work -ErrorAction Stop | Out-Null
  $created=$true
  $pinned=Get-Content -LiteralPath (Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json') -Raw | ConvertFrom-Json
  if($pinned.release_admitted -ne $false -or $pinned.installer_signing_authorized -ne $false){
    throw 'VIDEO_TOOLS_01O_HOLD: distribution remains unapproved'
  }
  foreach($v in @('old','new')){
    $three[$v]=Join-Path $work ($v+'-three')
    $unsigned[$v]=Join-Path $work ($v+'-unsigned')
    $five[$v]=Join-Path $work ($v+'-five')
    $installer[$v]=Join-Path $work ($v+'-UNSIGNED-NSIS.exe')
    New-Item -ItemType Directory -Path $three[$v] -ErrorAction Stop | Out-Null
  }
  Copy-Item -LiteralPath $oldBuild -Destination (Join-Path $three['old'] 'akula-video-mux.exe')
  Assert-Identity (Join-Path $three['old'] 'akula-video-mux.exe') '0.1.0'
  & cargo build --locked --release --manifest-path (Join-Path $repo 'labs/video-lite/Cargo.toml') --features lab_mux_020
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: second mux generation failed compilation'}
  Copy-Item -LiteralPath $oldBuild -Destination (Join-Path $three['new'] 'akula-video-mux.exe')
  Assert-Identity (Join-Path $three['new'] 'akula-video-mux.exe') '0.2.0'
  $oldSha=(Get-FileHash -LiteralPath (Join-Path $three['old'] 'akula-video-mux.exe') -Algorithm SHA256).Hash
  $newSha=(Get-FileHash -LiteralPath (Join-Path $three['new'] 'akula-video-mux.exe') -Algorithm SHA256).Hash
  if($oldSha -ceq $newSha){throw 'VIDEO_TOOLS_01O_HOLD: two real PE SHA256 digests are identical'}
  foreach($v in @('old','new')){
    foreach($input in @{'akula-yt-dlp.exe'=$YtDlpPath;'akula-quickjs.exe'=$QuickJsPath}.GetEnumerator()){
      $source=[IO.Path]::GetFullPath($input.Value)
      $ref=@($pinned.components | Where-Object {$_.name -ceq $input.Key})
      if($ref.Count -ne 1 -or
         (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ref[0].sha256){
        throw 'VIDEO_TOOLS_01O_HOLD: upstream PE does not match pinned release'
      }
      Copy-Item -LiteralPath $source -Destination (Join-Path $three[$v] $input.Key)
    }
    & node (Join-Path $repo 'scripts/video-tools-01i-unsigned-custody.cjs') $three[$v] $unsigned[$v] | Out-Host
    if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: real PE unsigned custody failure'}
  }
  $newManifest=Join-Path $unsigned['new'] 'manifest.json'
  $raw=[IO.File]::ReadAllText($newManifest)
  if($raw.Split([string[]]@('"version":"0.1.0"'),[StringSplitOptions]::None).Count -ne 2){
    throw 'VIDEO_TOOLS_01O_HOLD: expected exactly one lab version field'
  }
  [IO.File]::WriteAllText($newManifest,$raw.Replace('"version":"0.1.0"','"version":"0.2.0"'),[Text.UTF8Encoding]::new($false))
  & $signer -G -W -s $key -p $pub | Out-Host
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: temporary CI signing key generation failed'}
  $make=Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS/makensis.exe'
  if(!(Test-Path -LiteralPath $make -PathType Leaf)){throw 'VIDEO_TOOLS_01O_HOLD: pinned NSIS missing'}
  foreach($v in @('old','new')){
    $manifest=Join-Path $unsigned[$v] 'manifest.json'
    $sig=Join-Path $unsigned[$v] 'manifest.minisig'
    & $signer -S -q -s $key -m $manifest -x $sig
    if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: manifest signing failed'}
    & $signer -Vm $manifest -x $sig -p $pub -Q | Out-Host
    if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: signed manifest not authenticated'}
    New-Item -ItemType Directory -Path $five[$v] -ErrorAction Stop | Out-Null
    foreach($name in @('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe')){
      Copy-Item -LiteralPath (Join-Path $three[$v] $name) -Destination (Join-Path $five[$v] $name)
    }
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $five[$v] 'manifest.json')
    Copy-Item -LiteralPath $sig -Destination (Join-Path $five[$v] 'manifest.minisig')
    & $make ("/DLAB_STAGE="+$five[$v]) ("/DLAB_OUTPUT="+$installer[$v]) (Join-Path $repo 'scripts/video-tools-01j-six-file-lab.nsi') | Out-Host
    if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: protected NSIS build failed'}
  }
  $a=Start-Process -FilePath $installer['old'] -ArgumentList '/S' -PassThru -Wait
  $installed=Test-Path -LiteralPath (Join-Path $root 'uninstall.exe') -PathType Leaf
  if($a.ExitCode -ne 0 -or !$installed){throw 'VIDEO_TOOLS_01O_HOLD: old signed install failed'}
  Assert-Native 'valid' '0.1.0'
  & node (Join-Path $repo 'scripts/video-tools-01o-signed-floor.cjs') $five['new'] $root $pub $signer 'ALLOW' | Out-Host
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: signed newer candidate was not admitted'}
  Uninstall-Lab
  $installed=$false
  $b=Start-Process -FilePath $installer['new'] -ArgumentList '/S' -PassThru -Wait
  $installed=Test-Path -LiteralPath (Join-Path $root 'uninstall.exe') -PathType Leaf
  if($b.ExitCode -ne 0 -or !$installed){throw 'VIDEO_TOOLS_01O_HOLD: new signed install failed'}
  Assert-Native 'status-incompatible' '0.2.0'
  $hashes=@{}
  foreach($name in @('akula-video-mux.exe','akula-yt-dlp.exe','akula-quickjs.exe',
                     'manifest.json','manifest.minisig','uninstall.exe')){
    $hashes[$name]=(Get-FileHash -LiteralPath (Join-Path $root $name) -Algorithm SHA256).Hash
  }
  & node (Join-Path $repo 'scripts/video-tools-01o-signed-floor.cjs') $five['old'] $root $pub $signer 'DENY' | Out-Host
  if($LASTEXITCODE -ne 0){throw 'VIDEO_TOOLS_01O_HOLD: signed downgrade candidate was not refused'}
  $direct=Start-Process -FilePath $installer['old'] -ArgumentList '/S' -PassThru -Wait
  if($direct.ExitCode -ne 77){throw 'VIDEO_TOOLS_01O_HOLD: older NSIS overwrote newer package'}
  foreach($name in @($hashes.Keys)){
    if((Get-FileHash -LiteralPath (Join-Path $root $name) -Algorithm SHA256).Hash -cne $hashes[$name]){
      throw ('VIDEO_TOOLS_01O_HOLD: denied downgrade modified protected file '+$name)
    }
  }
  Assert-Native 'status-incompatible' '0.2.0'
  Uninstall-Lab
  $installed=$false
  Write-Output 'VIDEO_TOOLS_01O_REAL_MUX_FLOOR_PASS distinct_real_pe=1 signed_versions=2 native_installs=2 downgrade_denied=1 bytes_unchanged=6 release=0'
  [IO.File]::WriteAllText($ProofFile,('VIDEO_TOOLS_01O_REAL_MUX_FLOOR_PASS'+[Environment]::NewLine),[Text.UTF8Encoding]::new($false))
} finally {
  if($installed -and (Test-Path -LiteralPath (Join-Path $root 'uninstall.exe') -PathType Leaf)){
    Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
    Start-Process -FilePath (Join-Path $root 'uninstall.exe') -ArgumentList '/S' -Wait | Out-Null
  }
  if(Test-Path -LiteralPath $root){
    Assert-AkulaVideoToolsNoReparseParents -Path $root -RequireExists
    if(@(Get-ChildItem -LiteralPath $root -Force).Count -eq 0){Remove-Item -LiteralPath $root}
  }
  if($created -and (Test-Path -LiteralPath $work)){
    if(![IO.Path]::GetFullPath($work).StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)){
      throw 'VIDEO_TOOLS_01O_HOLD: isolated secret cleanup escaped runner temp'
    }
    Remove-Item -LiteralPath $work -Recurse -Force
  }
}
