# VIDEO-17 genuine NSIS installed GUI, uninstall and preservation acceptance.
# Never run this on a user's machine, and never publish this unsigned installer.
param(
  [Parameter(Mandatory=$true)][string]$SetupPath,
  [Parameter(Mandatory=$true)][string]$StageDir,
  [Parameter(Mandatory=$true)][string]$FixtureDir
)
$ErrorActionPreference = 'Stop'
if($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
   $env:RUNNER_ENVIRONMENT -cne 'github-hosted') {
  throw 'VIDEO17 accepts only ephemeral GitHub-hosted Windows'
}
$root=[IO.Path]::GetFullPath($env:GITHUB_WORKSPACE)
$temp=[IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')+'\'
$setup=[IO.Path]::GetFullPath($SetupPath)
$stage=[IO.Path]::GetFullPath($StageDir)
$fixture=[IO.Path]::GetFullPath($FixtureDir)
foreach($p in @($setup,$stage,$fixture)){
  if(!$p.StartsWith(($root.TrimEnd('\')+'\'),[StringComparison]::OrdinalIgnoreCase)){
    throw 'VIDEO17 refuses paths outside CI checkout'
  }
}
$cfg=Get-Content -LiteralPath (Join-Path $root 'aquilum-app/native/packaging/akula-windows-video17-lab.json') -Raw | ConvertFrom-Json
$release=Get-Content -LiteralPath (Join-Path $root 'aquilum-app/native/packaging/akula-windows.json') -Raw | ConvertFrom-Json
$names=@('akula-app','shiva-pdf-preview','akula-video-mux','akula-yt-dlp','akula-quickjs')
if(!(Test-Path -LiteralPath $setup -PathType Leaf) -or
   ([IO.Path]::GetFileName($setup) -cne "Akula_$($cfg.version)_x64-setup.exe") -or
   $cfg.productName -cne 'Akula' -or $cfg.identifier -cne $release.identifier -or
   $cfg.version -cne $release.version -or @($cfg.binaries).Count -ne 5 -or
   (@($cfg.binaries | ForEach-Object {$_.path}) -join ',') -cne ($names -join ',') -or
   (@($release.binaries | ForEach-Object {$_.path}) -join ',') -cne 'akula-app,shiva-pdf-preview') {
  throw 'VIDEO17 must be an isolated five-binary Akula NSIS candidate'
}
$install=Join-Path $env:LOCALAPPDATA 'Akula'
$exe=Join-Path $install 'akula-app.exe'
$key='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$upstream=Join-Path $env:LOCALAPPDATA 'Aquilum'
$updata=Join-Path $env:APPDATA 'com.dmitriy.aquilum-app'
$upkey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aquilum'
foreach($p in @($install,$key,$upstream,$updata,$upkey)){
  if(Test-Path -LiteralPath $p){throw "VIDEO17 runner not clean: $p"}
}
New-Item -ItemType Directory -Force -Path $upstream,$updata | Out-Null
[IO.File]::WriteAllText((Join-Path $upstream 'sentinel.txt'),'AQUILUM-V17')
[IO.File]::WriteAllText((Join-Path $updata 'sentinel.txt'),'AQUILUM-DATA-V17')
New-Item -Path $upkey -Force | Out-Null
Set-ItemProperty -Path $upkey -Name DisplayName -Value 'Aquilum'
$runkey='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
New-Item -Path $runkey -Force | Out-Null
Set-ItemProperty -Path $runkey -Name Aquilum -Value 'Aquilum-video17-sentinel'
function Assert-Upstream {
  if([IO.File]::ReadAllText((Join-Path $upstream 'sentinel.txt')) -cne 'AQUILUM-V17' -or
     [IO.File]::ReadAllText((Join-Path $updata 'sentinel.txt')) -cne 'AQUILUM-DATA-V17' -or
     (Get-ItemProperty $upkey).DisplayName -cne 'Aquilum' -or
     (Get-ItemProperty $runkey).Aquilum -cne 'Aquilum-video17-sentinel'){
    throw 'VIDEO17 NSIS modified synthetic upstream Aquilum'
  }
}
function Hash([string]$Path) {
  $sha=[Security.Cryptography.SHA256]::Create()
  $file=[IO.File]::OpenRead($Path)
  try {return [BitConverter]::ToString($sha.ComputeHash($file)).Replace('-','').ToLowerInvariant()}
  finally {$file.Dispose();$sha.Dispose()}
}
$expected=@{}
foreach($name in $names){
  $file=Join-Path $stage ($name+'.exe')
  if(!(Test-Path -LiteralPath $file -PathType Leaf) -or
     ((Get-Item $file).Attributes -band [IO.FileAttributes]::ReparsePoint)){
    throw "VIDEO17 stage missing trustworthy $name"
  }
  $expected[$name]=Hash $file
}
if($expected['akula-video-mux'] -cne $env:AKULA_BUNDLED_MUX_SHA256 -or
   $expected['akula-yt-dlp'] -cne 'f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d' -or
   $expected['akula-quickjs'] -cne 'ff03398d74ea6e6bd3a8e58058f7003225abcac229d7d57775b7aaae812a49c7' -or
   (Get-Item (Join-Path $stage 'akula-yt-dlp.exe')).Length -ne 23575744 -or
   (Get-Item (Join-Path $stage 'akula-quickjs.exe')).Length -ne 1873152){
  throw 'VIDEO17 installed candidates do not match pinned helper provenance'
}
$output=Join-Path $root 'video17-output/video17-installed.mp4'
New-Item -ItemType Directory -Force -Path (Split-Path $output -Parent) | Out-Null
$vault=Join-Path $env:RUNNER_TEMP 'video17-vault'
$data=Join-Path $env:RUNNER_TEMP 'video17-data'
$probe=Join-Path $env:RUNNER_TEMP 'video16-events.log'
foreach($p in @($vault,$data,$probe)){
  if(!([IO.Path]::GetFullPath($p)).StartsWith($temp,[StringComparison]::OrdinalIgnoreCase)){
    throw 'VIDEO17 refuses user data paths'
  }
}
$installed=$false
try{
  $proc=Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
  if($proc.ExitCode -ne 0){throw "NSIS installer exited $($proc.ExitCode)"}
  $installed=$true
  if(!(Test-Path -LiteralPath $key) -or
     (Get-ItemProperty $key).DisplayName -cne 'Akula' -or
     (Get-ItemProperty $key).DisplayVersion -cne $cfg.version){
    throw 'VIDEO17 installed wrong Akula identity/version'
  }
  foreach($name in $names){
    $dest=Join-Path $install ($name+'.exe')
    if(!(Test-Path -LiteralPath $dest -PathType Leaf) -or
       ((Get-Item $dest).Attributes -band [IO.FileAttributes]::ReparsePoint) -or
       (Hash $dest) -cne $expected[$name]){
      throw "VIDEO17 real NSIS installed missing or altered $name"
    }
  }
  Assert-Upstream
  Write-Host 'VIDEO17_GENUINE_NSIS_FIVE_BINARIES_PASS installed=1 identical_hashes=5 signed=0'
  # Reuse the exact real-Windows-pointer VIDEO16 acceptance on a genuine
  # NSIS-installed Akula binary instead of manually staging that binary.
  $env:AKULA_VIDEO17_REAL_NSIS='1'
  $args=@('labs/video-lite/video16/gui_acceptance.py','--repo',$root,'--manifest-dir',$fixture,
          '--exe',$exe,'--vault',$vault,'--data',$data,'--probe',$probe,'--output',$output)
  & python @args
  if($LASTEXITCODE -ne 0){throw 'VIDEO17 installed GUI click/restart/playback failed'}
  if(!(Test-Path -LiteralPath $output -PathType Leaf) -or
     (Get-Item $output).Length -lt 1000){throw 'VIDEO17 installed GUI downloaded no media'}
  if(@(Get-ChildItem -LiteralPath $vault -File -Recurse |
       Where-Object {$_.Extension -in '.mp4','.m4a','.m4s','.webm'}).Count -ne 0){
    throw 'VIDEO17 leaked local video into synced vault'
  }
  Assert-Upstream
  Write-Host 'VIDEO17_INSTALLED_GUI_RESTART_PLAY_PASS real_mouse=1 local_video=1 decoded_frame=1'
} finally {
  if($installed){
    $uninstaller=Join-Path $install 'uninstall.exe'
    if(!(Test-Path -LiteralPath $uninstaller -PathType Leaf)){throw 'Missing NSIS uninstaller'}
    $removed=Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
    if($removed.ExitCode -ne 0){throw "NSIS uninstall exited $($removed.ExitCode)"}
    for($i=0;$i -lt 60 -and (Test-Path -LiteralPath $key);$i++){Start-Sleep -Milliseconds 500}
    foreach($name in $names){
      if(Test-Path -LiteralPath (Join-Path $install ($name+'.exe'))){
        throw "VIDEO17 uninstaller retained executable $name"
      }
    }
    if(Test-Path -LiteralPath $key){throw 'NSIS uninstall registry entry remained'}
    Assert-Upstream
    Write-Host 'VIDEO17_NSIS_UNINSTALL_PASS removed_five_exe=1 upstream_untouched=1'
  }
}
$note=Join-Path $vault 'VIDEO16 Media.md'
$media=Join-Path $data 'local-videos/dQw4w9WgXcQ.mp4'
if(!(Test-Path -LiteralPath $output -PathType Leaf) -or
   !(Test-Path -LiteralPath $note -PathType Leaf) -or
   !(Test-Path -LiteralPath $media -PathType Leaf)){
  throw 'VIDEO17 uninstall deleted private media/note or acceptance output'
}
Write-Host 'VIDEO17_INSTALLED_FULL_CYCLE_PASS install=1 download=1 restart=1 play=1 uninstall=1 data_preserved=1'
