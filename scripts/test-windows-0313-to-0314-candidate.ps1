# AKULA 0313-0314: disposable Windows upgrade from exact published signed bytes.
# This test never runs on a workstation or touches real Dropbox credentials.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_REPOSITORY -cne 'Lalalalendia/Akula' -or
    $env:GITHUB_REF -cne 'refs/heads/main' -or
    $env:AKULA_0313_CANDIDATE_DISPOSABLE -cne '1') {
  throw 'AKULA_0313_CANDIDATE_UPGRADE_HOLD: disposable main-only Windows required'
}
$oldSource = '23383c52a6a63fb2e2f6b71d567e41e9018cda22'
$oldInstallerHash = '1526b83305707204e57e2937d9db8499e539c37fa6a3117854514454349c1fc8'
$oldName = 'Akula_0.3.13_x64-setup.exe'
$newName = 'Akula_0.3.14_x64-setup.exe'
$base = Join-Path $env:RUNNER_TEMP 'akula-0313-to-0314-candidate'
if (Test-Path -LiteralPath $base) { throw 'Upgrade workspace preexists' }
New-Item -ItemType Directory -Path $base -Force | Out-Null
$dist = Join-Path $env:GITHUB_WORKSPACE 'dist'
$newSetup = Join-Path $dist $newName
$manifestPath = Join-Path $dist 'latest.json'
$builtExe = Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\release\akula-app.exe'
foreach ($p in @($newSetup, ($newSetup + '.sig'), $manifestPath, ($manifestPath + '.sig'), $builtExe)) {
  if (!(Test-Path -LiteralPath $p -PathType Leaf)) { throw "Candidate file missing: $p" }
}
$manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
$win = $manifest.artifacts.'windows-x86_64'
$newDigest = (Get-FileHash $newSetup -Algorithm SHA256).Hash.ToLowerInvariant()
if ($manifest.product -cne 'Akula' -or $manifest.version -cne '0.3.14' -or
    $manifest.source_commit -cne $env:GITHUB_SHA -or $win.filename -cne $newName -or
    $win.sha256 -cne $newDigest -or $win.size -ne (Get-Item $newSetup).Length) {
  throw 'New candidate manifest source, version, hash or size mismatch'
}

# Pin the public source, GitHub release digest and downloaded exact bytes.
$public = Invoke-RestMethod -Uri 'https://api.github.com/repos/Lalalalendia/Akula/releases/tags/v0.3.13' -Headers @{ Accept = 'application/vnd.github+json'; 'User-Agent' = 'Akula-Candidate-Upgrade' }
$oldAssets = @($public.assets | Where-Object { $_.name -ceq $oldName })
if ($public.tag_name -cne 'v0.3.13' -or $public.draft -or $public.prerelease -or
    $public.target_commitish -cne $oldSource -or $oldAssets.Count -ne 1 -or
    $oldAssets[0].digest -cne ('sha256:' + $oldInstallerHash)) {
  throw 'Pinned published v0.3.13 release identity changed'
}
$oldSetup = Join-Path $base $oldName
Invoke-WebRequest -Uri ('https://github.com/Lalalalendia/Akula/releases/download/v0.3.13/' + $oldName) -OutFile $oldSetup -MaximumRetryCount 3 -RetryIntervalSec 3
if ((Get-FileHash $oldSetup -Algorithm SHA256).Hash.ToLowerInvariant() -cne $oldInstallerHash -or
    (Get-Item $oldSetup).Length -ne $oldAssets[0].size) {
  throw 'Downloaded v0.3.13 installer differs from published SHA-256'
}
$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$state = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
if ((Test-Path $exe) -or (Test-Path $reg) -or !(Test-Path $state -PathType Container)) {
  throw 'Previous installed Windows smoke did not cleanly uninstall while preserving state'
}
$stateWitness = Join-Path $state 'candidate-upgrade-preserve.txt'
if (Test-Path $stateWitness) { throw 'Test witness preexists' }
[IO.File]::WriteAllText($stateWitness, 'AKULA-USER-STATE')
$stateHash = (Get-FileHash $stateWitness -Algorithm SHA256).Hash
$legacyInstall = Join-Path $env:LOCALAPPDATA 'Aquilum\legacy-do-not-touch.txt'
$legacyData = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app\user-data-do-not-touch.txt'
$legacyReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aquilum'
$runReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
foreach ($p in @($legacyInstall, $legacyData)) {
  if (!(Test-Path -LiteralPath $p -PathType Leaf)) { throw "Aquilum witness missing: $p" }
}
$legacyHashes = @((Get-FileHash $legacyInstall -Algorithm SHA256).Hash, (Get-FileHash $legacyData -Algorithm SHA256).Hash)
$vault = Join-Path $base 'vault'
New-Item -ItemType Directory -Force -Path $vault | Out-Null
$note = Join-Path $vault 'Keep.md'
[IO.File]::WriteAllText($note, '# Preserve user Markdown')
$noteHash = (Get-FileHash $note -Algorithm SHA256).Hash
function Assert-Preserved {
  if (!(Test-Path $stateWitness) -or
      (Get-FileHash $stateWitness -Algorithm SHA256).Hash -ne $stateHash -or
      (Get-FileHash $note -Algorithm SHA256).Hash -ne $noteHash -or
      (Get-FileHash $legacyInstall -Algorithm SHA256).Hash -ne $legacyHashes[0] -or
      (Get-FileHash $legacyData -Algorithm SHA256).Hash -ne $legacyHashes[1] -or
      (Get-ItemProperty $legacyReg -Name DisplayName).DisplayName -cne 'Aquilum' -or
      (Get-ItemProperty $runReg -Name Aquilum).Aquilum -cne 'Aquilum-startup-sentinel') {
    throw 'Upgrade or uninstall modified user state, note or Aquilum'
  }
}
function Install-NSIS([string]$path, [string[]]$flags) {
  $p = Start-Process -FilePath $path -ArgumentList $flags -PassThru
  if (!$p.WaitForExit(180000)) {
    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    throw "NSIS timed out: $path"
  }
  if ($p.ExitCode -ne 0) { throw "NSIS exit code $($p.ExitCode): $path" }
}
function Assert-Installed([string]$version) {
  if (!(Test-Path $exe -PathType Leaf) -or !(Test-Path $reg)) { throw 'Akula install missing' }
  if ((Get-ItemProperty $reg -Name DisplayVersion).DisplayVersion -cne $version) {
    throw "Expected installed Akula $version"
  }
  $entries = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' |
    Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName -ceq 'Akula' })
  if ($entries.Count -ne 1) { throw 'Duplicate or missing Akula uninstall identity' }
}
function Wait-Gui([int]$excluded = -1) {
  for ($i = 0; $i -lt 70; $i++) {
    $apps = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue |
      Where-Object { $_.MainWindowHandle -ne 0 -and $_.Id -ne $excluded })
    if ($apps.Count -gt 0) { return $apps[0] }
    Start-Sleep -Seconds 1
  }
  throw 'Installed Akula GUI did not appear'
}

Install-NSIS $oldSetup @('/S')
Assert-Installed '0.3.13'
$oldHash = (Get-FileHash $exe -Algorithm SHA256).Hash
$argsForApp = '--vault "' + $vault + '" --open "' + $note + '"'
$old = Start-Process -FilePath $exe -ArgumentList $argsForApp -PassThru
$oldGui = Wait-Gui
if ($old.Id -ne $oldGui.Id -or !$oldGui.CloseMainWindow() -or !$oldGui.WaitForExit(25000)) {
  throw 'Published v0.3.13 GUI did not close correctly before update'
}
Assert-Preserved
Write-Host 'AKULA_0313_UPGRADE_BASELINE_PASS signed_source=1 gui=1 notes=1'

# Reproduce the installed updater NSIS flags without publishing any new release.
Install-NSIS $newSetup @('/S','/R','/UPDATE','/ARGS')
Assert-Installed '0.3.14'
$newHash = (Get-FileHash $exe -Algorithm SHA256).Hash
if ($newHash -eq $oldHash -or $newHash -ne (Get-FileHash $builtExe -Algorithm SHA256).Hash) {
  throw '0.3.14 update did not install exact candidate native executable'
}
$newGui = Wait-Gui
Assert-Preserved
Write-Host 'AKULA_0313_TO_0314_SIGNED_CANDIDATE_UPGRADE_PASS versions=1 nsis=1 gui=1 notes=1 appdata=1 aquilum=1'

if (!$newGui.CloseMainWindow() -or !$newGui.WaitForExit(25000)) {
  throw 'Candidate GUI did not shut down'
}
Install-NSIS (Join-Path $install 'uninstall.exe') @('/S')
for ($i = 0; $i -lt 40 -and ((Test-Path $exe) -or (Test-Path $reg)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path $exe) -or (Test-Path $reg) -or !(Test-Path $state)) {
  throw 'Candidate uninstall did not preserve Akula AppData'
}
Assert-Preserved
Write-Host 'AKULA_0313_TO_0314_CANDIDATE_CLEANUP_PASS one_app=1 state=1 upstream=1'
