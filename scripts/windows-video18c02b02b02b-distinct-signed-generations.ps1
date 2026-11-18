# VIDEO-18c-02b-02b-02b: two DIFFERENT source-built installed five-PE generations.
# Ephemeral GitHub-hosted Windows PR fixture only. No updater, release or commit.
#requires -Version 7.2
param(
  [Parameter(Mandatory=$true)][string]$OldAppPath,
  [Parameter(Mandatory=$true)][string]$NewAppPath,
  [Parameter(Mandatory=$true)][string]$ShivaPath,
  [Parameter(Mandatory=$true)][string]$MuxPath,
  [Parameter(Mandatory=$true)][string]$YtDlpPath,
  [Parameter(Mandatory=$true)][string]$QuickJsPath,
  [switch]$Rar2Positive
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$oldCommit='ec6d6d48a78e60fca7a6f810b825418500bad64b'
if($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
   $env:GITHUB_EVENT_NAME -cne 'pull_request' -or !$env:EXPECTED_HEAD_SHA -or
   (git rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA) {
  throw 'VIDEO18C02B02B02B_HOLD: exact-head hosted Windows PR only'
}
$principal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  throw 'VIDEO18C02B02B02B_HOLD: protected Program Files fixture needs admin'
}
$repo=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP)
$historical=Join-Path $repo '_akula-historical'
if((git -C $historical rev-parse HEAD).Trim() -cne $oldCommit -or
   (git -C $repo rev-parse HEAD).Trim() -cne $env:EXPECTED_HEAD_SHA -or
   (git -C $historical rev-parse HEAD:aquilum-app/Cargo.lock).Trim() -cne
   (git -C $repo rev-parse HEAD:aquilum-app/Cargo.lock).Trim()) {
  throw 'VIDEO18C02B02B02B_HOLD: old or new source provenance missing'
}
$oldExpected=[IO.Path]::GetFullPath((Join-Path $temp 'video18c02b02b02a-old-native/release/aquilum-native.exe'))
$newExpected=[IO.Path]::GetFullPath((Join-Path $repo '.artifacts/cargo-native/release/aquilum-native.exe'))
if(![string]::Equals([IO.Path]::GetFullPath($OldAppPath),$oldExpected,[StringComparison]::OrdinalIgnoreCase) -or
   ![string]::Equals([IO.Path]::GetFullPath($NewAppPath),$newExpected,[StringComparison]::OrdinalIgnoreCase)) {
  throw 'VIDEO18C02B02B02B_HOLD: executable outside source-pinned build roots'
}
$names=@('akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs')
$helpers=[ordered]@{
  'shiva-pdf-preview'=$ShivaPath
  'akula-video-mux'=$MuxPath
  'akula-yt-dlp'=$YtDlpPath
  'akula-quickjs'=$QuickJsPath
}
function Assert-RealFile([string]$path) {
  $f=Get-Item -LiteralPath $path -Force -ErrorAction Stop
  if($f.PSIsContainer -or ($f.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
     $f.Length -lt 512) {throw "VIDEO18C02B02B02B_HOLD: not a genuine PE $path"}
  $stream=[IO.File]::OpenRead($path)
  try {
    if($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) {
      throw "VIDEO18C02B02B02B_HOLD: not MZ/PE $path"
    }
  } finally {$stream.Dispose()}
}
function File-Sha([string]$path) {
  return (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Assert-Set([string]$dir,[hashtable]$expected,[bool]$isFlat) {
  $d=Get-Item -LiteralPath $dir -Force -ErrorAction Stop
  if(!$d.PSIsContainer -or ($d.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw 'VIDEO18C02B02B02B_HOLD: generation directory is reparse or missing'
  }
  $actual=@(Get-ChildItem -LiteralPath $dir -Force)
  $expectedCount=5
  if($isFlat){
    # A genuine flat NSIS install contains five PE plus its uninstaller.
    # After the bridge it also contains the single protected current/ directory.
    $expectedCount=6
    if(Test-Path -LiteralPath (Join-Path $dir 'current') -PathType Container){
      $expectedCount=7
    }
    $uninstaller=Join-Path $dir 'uninstall.exe'
    Assert-RealFile $uninstaller
  }
  if($actual.Count -ne $expectedCount) {
    throw 'VIDEO18C02B02B02B_HOLD: incorrect installed generation member count'
  }
  foreach($name in $names) {
    $path=Join-Path $dir ($name+'.exe')
    Assert-RealFile $path
    if((File-Sha $path) -cne $expected[$name]) {
      throw "VIDEO18C02B02B02B_HOLD: changed generation PE $name"
    }
  }
}
function Copy-Generation([string]$dir,[string]$app,[hashtable]$hashes) {
  New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
  $source=@{'akula-app'=$app}
  foreach($name in $helpers.Keys){$source[$name]=$helpers[$name]}
  foreach($name in $names){
    $file=$source[$name]
    Assert-RealFile $file
    $destination=Join-Path $dir ($name+'.exe')
    Copy-Item -LiteralPath $file -Destination $destination -ErrorAction Stop
    Assert-RealFile $destination
    $hashes[$name]=File-Sha $destination
    if((File-Sha $file) -cne $hashes[$name]){
      throw 'VIDEO18C02B02B02B_HOLD: staged PE provenance mismatch'
    }
  }
}
function Run-OldProtectedGui([string]$app,[string]$vault,[string]$data,[string]$note) {
  Assert-RealFile $app
  $start=[System.Diagnostics.ProcessStartInfo]::new()
  $start.FileName=$app
  $start.WorkingDirectory=[IO.Path]::GetDirectoryName($app)
  $start.UseShellExecute=$false
  foreach($arg in @('--vault',$vault,'--data-dir',$data,'--open',$note,'--focus')) {
    [void]$start.ArgumentList.Add($arg)
  }
  $p=[System.Diagnostics.Process]::new()
  $p.StartInfo=$start
  $seen=$false
  try {
    if(!$p.Start()){throw 'VIDEO18C02B02B02B_HOLD: historical installed GUI did not start'}
    $pidPinned=$p.Id
    $started=$p.StartTime.ToUniversalTime()
    for($i=0;$i -lt 110;$i++){
      $p.Refresh()
      if($p.HasExited){throw 'VIDEO18C02B02B02B_HOLD: old GUI exited before visible window'}
      if($p.MainWindowHandle -ne [IntPtr]::Zero -and $p.MainWindowTitle.Contains('Akula')) {
        $same=[System.Diagnostics.Process]::GetProcessById($pidPinned)
        try {
          if($same.StartTime.ToUniversalTime() -ne $started -or
             $same.MainWindowHandle -ne $p.MainWindowHandle) {
            throw 'VIDEO18C02B02B02B_HOLD: old GUI window not tied to pinned PID'
          }
        } finally {$same.Dispose()}
        $seen=$true
        break
      }
      Start-Sleep -Milliseconds 200
    }
    if(!$seen -or !$p.CloseMainWindow() -or !$p.WaitForExit(20000)){
      throw 'VIDEO18C02B02B02B_HOLD: old protected GUI not cleanly visible/closed'
    }
    Write-Host 'VIDEO18C02B02B02B_OLD_PROTECTED_GUI_FALLBACK_PASS visible=1 pinned_pid=1 clean_exit=1'
  } finally {
    if(!$p.HasExited){$p.Kill($true);[void]$p.WaitForExit(5000)}
    $p.Dispose()
  }
}
$tag=[guid]::NewGuid().ToString('N')
$programFiles=[IO.Path]::GetFullPath($env:ProgramFiles)
$root=Join-Path $programFiles ('AkulaVideo18bRealNsisLab-'+$tag)
$work=Join-Path $temp ('Video18c02b02b02b-'+$tag)
$oldStage=Join-Path $work 'legacy'
$newStage=Join-Path $work 'candidate'
$trustOld=Join-Path $work 'old-trust'
$trustNew=Join-Path $work 'new-trust'
$flatNsi=Join-Path $repo 'scripts/video18b-protected-five-real.nsi'
$bridgeNsi=Join-Path $repo 'scripts/video18b16-protected-versioned-bridge.nsi'
$makensis=Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'NSIS/makensis.exe'
$oldInstaller=Join-Path $work 'legacy-unsigned.exe'
$newInstaller=Join-Path $work 'candidate-unsigned.exe'
$manifestCrate=Join-Path $repo 'labs/video-lite/video18c02b02b-signed-gui/Cargo.toml'
$manifestLock=Join-Path $repo 'labs/video-lite/video18c02b02b-signed-gui/Cargo.lock'
if((Test-Path -LiteralPath $root) -or (Test-Path -LiteralPath $work) -or
   (Test-Path -LiteralPath $manifestLock) -or !(Test-Path -LiteralPath $makensis -PathType Leaf) -or
   !(Test-Path -LiteralPath $flatNsi -PathType Leaf) -or
   !(Test-Path -LiteralPath $bridgeNsi -PathType Leaf) -or
   !(Test-Path -LiteralPath $manifestCrate -PathType Leaf)) {
  throw 'VIDEO18C02B02B02B_HOLD: non-isolated lab or missing pinned tool'
}
$allow=Get-Content -LiteralPath (Join-Path $repo 'docs/release/video18b04-five-exe-allowlist.json') -Raw | ConvertFrom-Json
if($allow.release_admitted -ne $false -or $allow.installer_signing_authorized -ne $false -or
   $allow.stage_binary_distribution_authorized -ne $false) {
  throw 'VIDEO18C02B02B02B_HOLD: release permission not held'
}
$oldHashes=@{}
$newHashes=@{}
$installed=$false
$priorTarget=$env:CARGO_TARGET_DIR
try {
  New-Item -ItemType Directory -Path $work -ErrorAction Stop | Out-Null
  Copy-Generation $oldStage $OldAppPath $oldHashes
  Copy-Generation $newStage $NewAppPath $newHashes
  if($oldHashes['akula-app'] -ceq $newHashes['akula-app']) {
    throw 'VIDEO18C02B02B02B_HOLD: actual old/new installed GUI PE identical'
  }
  foreach($name in $helpers.Keys){
    if($oldHashes[$name] -cne $newHashes[$name]) {
      throw 'VIDEO18C02B02B02B_HOLD: shared four pinned helper inputs diverged'
    }
  }
  foreach($name in @('akula-yt-dlp','akula-quickjs')){
    $component=@($allow.components | Where-Object {$_.name -ceq ($name+'.exe')})
    if($component.Count -ne 1 -or $newHashes[$name] -cne $component[0].sha256 -or
       (Get-Item -LiteralPath (Join-Path $newStage ($name+'.exe'))).Length -ne $component[0].size_bytes) {
      throw 'VIDEO18C02B02B02B_HOLD: upstream helpers not approved pinned bytes'
    }
  }
  if(!$env:AKULA_BUNDLED_MUX_SHA256 -or
     $newHashes['akula-video-mux'] -cne $env:AKULA_BUNDLED_MUX_SHA256) {
    throw 'VIDEO18C02B02B02B_HOLD: compiled mux pin mismatch'
  }
  foreach($pair in @(@($oldStage,$oldInstaller,$flatNsi),@($newStage,$newInstaller,$bridgeNsi))){
    & $makensis ("/DLAB_ID="+$tag) ("/DLAB_STAGE="+$pair[0]) ("/DLAB_OUTPUT="+$pair[1]) $pair[2] | Out-Host
    if($LASTEXITCODE -ne 0){throw 'VIDEO18C02B02B02B_HOLD: NSIS compile failed'}
    Assert-RealFile $pair[1]
  }
  foreach($installer in @($oldInstaller,$newInstaller)){
    $process=Start-Process -FilePath $installer -ArgumentList '/S' -Wait -PassThru
    if($process.ExitCode -ne 0){throw 'VIDEO18C02B02B02B_HOLD: NSIS installer failed'}
    $installed=$true
    if($installer -ceq $oldInstaller){Assert-Set $root $oldHashes $true}
  }
  $current=Join-Path $root 'current'
  Assert-Set $root $oldHashes $true
  Assert-Set $current $newHashes $false
  Write-Host 'VIDEO18C02B02B02B_DISTINCT_INSTALLED_FIVE_PASS old=5 current=5 app_sha_distinct=1 pinned_sources=1 nonrelease=1'
  New-Item -ItemType Directory -Path $trustOld -ErrorAction Stop | Out-Null
  New-Item -ItemType Directory -Path $trustNew -ErrorAction Stop | Out-Null
  $lockSha=File-Sha (Join-Path $repo 'aquilum-app/Cargo.lock')
  $mini=(Get-Command minisign.exe -CommandType Application -ErrorAction Stop).Source
  Assert-RealFile $mini
  $key=Join-Path $trustNew 'test-publisher.key'
  $pub=Join-Path $trustNew 'test-publisher.pub'
  & $mini -G -W -s $key -p $pub
  if($LASTEXITCODE -ne 0){throw 'VIDEO18C02B02B02B_HOLD: ephemeral Minisign key generation failed'}
  Copy-Item -LiteralPath $pub -Destination (Join-Path $trustOld 'test-publisher.pub') -ErrorAction Stop
  $manifests=@{}
  foreach($gen in @(@('old',$root,$oldHashes,$oldCommit,'0.3.20',$trustOld),
                    @('new',$current,$newHashes,$env:EXPECTED_HEAD_SHA,'0.3.21',$trustNew))){
    $records=@()
    foreach($name in $names){
      $file=Join-Path $gen[1] ($name+'.exe')
      Assert-RealFile $file
      $records+=([ordered]@{filename=($name+'.exe');sha256=(File-Sha $file);size=(Get-Item $file).Length})
    }
    $signed=[ordered]@{
      schema=1;product='Akula';platform='windows-x86_64'
      purpose='protected-video-five-exe';version=$gen[4]
      source_commit=$gen[3];cargo_lock_sha256=$lockSha;components=$records
    }
    $manifest=Join-Path $gen[5] 'five-manifest.json'
    $signature=Join-Path $gen[5] 'five-manifest.minisig'
    [IO.File]::WriteAllText($manifest,($signed|ConvertTo-Json -Depth 8 -Compress)+[Environment]::NewLine,
      [Text.UTF8Encoding]::new($false))
    & $mini -S -s $key -m $manifest -x $signature -c 'VIDEO18C DISTINCT NONRELEASE' -t 'Ephemeral test only'
    if($LASTEXITCODE -ne 0){throw 'VIDEO18C02B02B02B_HOLD: signing failed'}
    & $mini -Vm $manifest -x $signature -p $pub -Q
    if($LASTEXITCODE -ne 0){throw 'VIDEO18C02B02B02B_HOLD: Minisign verification failed'}
    $manifests[$gen[0]]=@{manifest=$manifest;signature=$signature;hash=File-Sha $manifest}
  }
  if($manifests.old.hash -ceq $manifests.new.hash) {
    throw 'VIDEO18C02B02B02B_HOLD: two signed manifest generations identical'
  }
  & $mini -Vm $manifests.new.manifest -x $manifests.old.signature -p $pub -Q 2>$null | Out-Null
  if($LASTEXITCODE -eq 0){throw 'VIDEO18C02B02B02B_HOLD: wrong-generation signature accepted'}
  Assert-Set $root $oldHashes $true
  Assert-Set $current $newHashes $false
  Write-Host 'VIDEO18C02B02B02B_TWO_MINISIGN_MANIFESTS_PASS distinct_manifests=1 signed_old=1 signed_new=1 cross_signature_denied=1 release=0'
  $vault=Join-Path $work 'vault'
  $data=Join-Path $work 'data'
  New-Item -ItemType Directory -Path $vault -ErrorAction Stop | Out-Null
  New-Item -ItemType Directory -Path $data -ErrorAction Stop | Out-Null
  $note=Join-Path $vault 'Protected historical note.md'
  [IO.File]::WriteAllText($note,'Private note survives candidate stage.',[Text.UTF8Encoding]::new($false))
  $noteHash=File-Sha $note
  Run-OldProtectedGui (Join-Path $root 'akula-app.exe') $vault $data $note
  if((File-Sha $note) -cne $noteHash) {
    throw 'VIDEO18C02B02B02B_HOLD: historical GUI modified private data'
  }
  Remove-Item -LiteralPath (Join-Path $current 'akula-quickjs.exe') -Force -ErrorAction Stop
  $rejected=$false
  try{Assert-Set $current $newHashes $false}catch{$rejected=$true}
  if(!$rejected){throw 'VIDEO18C02B02B02B_HOLD: torn current generation accepted'}
  Assert-Set $root $oldHashes $true
  Copy-Item -LiteralPath (Join-Path $newStage 'akula-quickjs.exe') -Destination (Join-Path $current 'akula-quickjs.exe') -ErrorAction Stop
  Assert-Set $current $newHashes $false
  Write-Host 'VIDEO18C02B02B02B_TORN_NEW_REJECTED_PASS old_fallback_intact=1 restored_current=1'
  if(!$Rar2Positive){
  $target=Join-Path $work 'signed-gui-build'
  $env:CARGO_TARGET_DIR=$target
  try{
    cargo generate-lockfile --manifest-path $manifestCrate
    if($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $manifestLock -PathType Leaf)){
      throw 'VIDEO18C02B02B02B_HOLD: disposable Rust lab lock failed'
    }
    cargo build --release --locked --manifest-path $manifestCrate
    if($LASTEXITCODE -ne 0){throw 'VIDEO18C02B02B02B_HOLD: signed GUI Rust lab build failed'}
    $bin=Join-Path $target 'release/akula-video18c02b02b-signed-gui-lab.exe'
    Assert-RealFile $bin
    $receipt=@(& $bin (Join-Path $current 'akula-app.exe') $manifests.new.manifest $manifests.new.signature $pub 2>&1)
    $exit=$LASTEXITCODE
    $receipt | Out-Host
    if($exit -ne 0 -or !(($receipt|Out-String).Contains('VIDEO18C02B02B_SIGNED_RUNNABLE_GUI_ACK_PASS'))){
      throw 'VIDEO18C02B02B02B_HOLD: protected current signed GUI ACK rejected'
    }
  } finally {
    if(Test-Path -LiteralPath $manifestLock -PathType Leaf){
      Remove-Item -LiteralPath $manifestLock -Force -ErrorAction Stop
    }
    if($priorTarget){$env:CARGO_TARGET_DIR=$priorTarget}
    else{Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue}
  }
  Assert-Set $root $oldHashes $true
  Assert-Set $current $newHashes $false
  if((File-Sha $note) -cne $noteHash){
    throw 'VIDEO18C02B02B02B_HOLD: private data changed after signed GUI ACK'
  }
  Write-Host 'VIDEO18C02B02B02B_NEW_SIGNED_GUI_ACK_PASS distinct_old_new=1 current_signed=1 old_fallback_intact=1 no_rar2_commit=1'
  } else {
    # The original distinct-generation scenario proved signed GUI delivery
    # without rerunning rar2. This separately gated PR-only mode starts with
    # an OLD current/, then executes real signed rar2 activation and ACK commit.
    $pinned='32a803e67244836d2794f9bbbdf9b24deab63d7e'
    $engineTree=Join-Path $repo '_rar2-pinned'
    if(!(Test-Path -LiteralPath $engineTree -PathType Container) -or
       (git -C $engineTree rev-parse HEAD).Trim() -cne $pinned){
      throw 'VIDEO18C02B03_HOLD: original pinned rar2 source not checked out'
    }
    $positiveManifest=Join-Path $repo 'labs/video-lite/video18c02b03-rar2-gui/Cargo.toml'
    $positiveLock=Join-Path $repo 'labs/video-lite/video18c02b03-rar2-gui/Cargo.lock'
    if(!(Test-Path -LiteralPath $positiveManifest -PathType Leaf) -or
       (Test-Path -LiteralPath $positiveLock)){
      throw 'VIDEO18C02B03_HOLD: test Rust crate missing or stale lock'
    }
    Assert-Set $current $newHashes $false
    foreach($name in $names){
      $oldInstalled=Join-Path $root ($name+'.exe')
      Assert-RealFile $oldInstalled
      if((File-Sha $oldInstalled) -cne $oldHashes[$name]){
        throw 'VIDEO18C02B03_HOLD: old protected fallback altered before rar2'
      }
      Copy-Item -LiteralPath $oldInstalled -Destination (Join-Path $current ($name+'.exe')) -Force -ErrorAction Stop
    }
    Assert-Set $root $oldHashes $true
    Assert-Set $current $oldHashes $false
    $target=Join-Path $work 'rar2-ack-positive-build'
    $env:CARGO_TARGET_DIR=$target
    try {
      cargo generate-lockfile --manifest-path $positiveManifest
      if($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $positiveLock -PathType Leaf)){
        throw 'VIDEO18C02B03_HOLD: isolated Rust manifest lock failed'
      }
      cargo build --release --locked --manifest-path $positiveManifest
      if($LASTEXITCODE -ne 0){
        throw 'VIDEO18C02B03_HOLD: signed rar2 GUI lab build failed'
      }
      $lab=Join-Path $target 'release/akula-video18c02b03-rar2-positive-lab.exe'
      Assert-RealFile $lab
      $proof=@(& $lab $root $newStage $manifests.new.manifest $manifests.new.signature $pub 2>&1)
      $code=$LASTEXITCODE
      $proof | Out-Host
      if($code -ne 0 -or !(($proof|Out-String).Contains('VIDEO18C02B03_RAR2_REAL_POSITIVE_PASS'))){
        throw 'VIDEO18C02B03_HOLD: distinct signed GUI ACK did not authorize rar2 positive commit'
      }
    } finally {
      if(Test-Path -LiteralPath $positiveLock -PathType Leaf){
        Remove-Item -LiteralPath $positiveLock -Force -ErrorAction Stop
      }
      if($priorTarget){$env:CARGO_TARGET_DIR=$priorTarget}
      else {Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue}
    }
    Assert-Set $root $oldHashes $true
    Assert-Set $current $newHashes $false
    Write-Host 'VIDEO18C02B03_POSITIVE_INSTALLED_FIVE_PASS old_flat=5 new_current=5 release=0'
  }
  $uninstaller=Join-Path $root 'uninstall.exe'
  Assert-RealFile $uninstaller
  $gone=Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
  if($gone.ExitCode -ne 0 -or (Test-Path -LiteralPath $root)){
    throw 'VIDEO18C02B02B02B_HOLD: dual-layout uninstall incomplete'
  }
  $installed=$false
  if(!(Test-Path -LiteralPath $note -PathType Leaf) -or (File-Sha $note) -cne $noteHash){
    throw 'VIDEO18C02B02B02B_HOLD: uninstall destroyed independent user note'
  }
  Write-Output 'VIDEO18C02B02B02B_DISTINCT_SIGNED_GENERATIONS_PASS five_old=5 five_new=5 signed_both=1 gui_old=1 ack_new=1 rollback_fallback=1 uninstall=1 release=0'
} finally {
  if($priorTarget){$env:CARGO_TARGET_DIR=$priorTarget}
  else{Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue}
  if($installed -and (Test-Path -LiteralPath $root)){
    Write-Warning 'VIDEO18C02B02B02B_HOLD: protected failed lab retained until ephemeral runner teardown'
  }
  if(Test-Path -LiteralPath $work){Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction Stop}
}
