# RELEASE-AKULA-WIN-NEXT-UPGRADE-01. Ephemeral Windows only.
# Exercise previous published Akula -> new signed candidate without publishing.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_REPOSITORY -cne 'Lalalalendia/Akula' -or
    $env:GITHUB_REF -cne 'refs/heads/main' -or
    $env:AKULA_SIGNED_NEXT_CANDIDATE_DISPOSABLE -cne '1') {
  throw 'AKULA_SIGNED_NEXT_CANDIDATE_HOLD: protected ephemeral main-only Windows required'
}
$requested = $env:AKULA_REQUESTED_VERSION
if ($requested -cnotmatch '^\d+\.\d+\.\d+$') { throw 'Expected exact candidate version' }
$dist = Join-Path $env:GITHUB_WORKSPACE 'dist'
$newName = 'Akula_' + $requested + '_x64-setup.exe'
$newSetup = Join-Path $dist $newName
$newManifestPath = Join-Path $dist 'latest.json'
$builtExe = Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\release\akula-app.exe'
$builtPdf = Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\release\shiva-pdf-preview.exe'
foreach ($file in @($newSetup, ($newSetup + '.sig'), $newManifestPath,
                   ($newManifestPath + '.sig'), $builtExe, $builtPdf)) {
  if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Candidate file missing: $file" }
}
$newManifest = Get-Content -LiteralPath $newManifestPath -Raw | ConvertFrom-Json
$newAsset = $newManifest.artifacts.'windows-x86_64'
$newHash = (Get-FileHash -LiteralPath $newSetup -Algorithm SHA256).Hash.ToLowerInvariant()
if ($newManifest.product -cne 'Akula' -or $newManifest.version -cne $requested -or
    $newManifest.source_commit -cne $env:GITHUB_SHA -or
    $newAsset.filename -cne $newName -or $newAsset.sha256 -cne $newHash -or
    $newAsset.size -ne (Get-Item -LiteralPath $newSetup).Length) {
  throw 'Signed candidate metadata/installer/source mismatch'
}

# Pin baseline to the official latest public release and its SHA-256 records.
$api = 'https://api.github.com/repos/Lalalalendia/Akula/releases/latest'
$release = Invoke-RestMethod -Uri $api -Headers @{ Accept = 'application/vnd.github+json'; 'User-Agent' = 'Akula-Signed-Candidate-Upgrade' }
if ($release.draft -or $release.prerelease -or $release.tag_name -cnotmatch '^v\d+\.\d+\.\d+$' -or
    $release.target_commitish -cnotmatch '^[a-f0-9]{40}$') {
  throw 'Official latest Akula release metadata is not eligible'
}
$oldVersion = $release.tag_name.Substring(1)
& node scripts/akula-version.cjs assert-newer $release.tag_name
if ($LASTEXITCODE -ne 0) { throw 'Candidate must be newer than last published Akula' }
$oldName = 'Akula_' + $oldVersion + '_x64-setup.exe'
$base = Join-Path $env:RUNNER_TEMP ('akula-published-' + $oldVersion + '-to-' + $requested)
if (Test-Path -LiteralPath $base) { throw 'Candidate upgrade workspace already exists' }
New-Item -ItemType Directory -Path $base | Out-Null
function Published-Asset([string]$name) {
  $hits = @($release.assets | Where-Object { $_.name -ceq $name })
  if ($hits.Count -ne 1 -or $hits[0].digest -cnotmatch '^sha256:[a-f0-9]{64}$' -or
      $hits[0].size -le 0 -or
      $hits[0].browser_download_url -cne ('https://github.com/Lalalalendia/Akula/releases/download/' + $release.tag_name + '/' + $name)) {
    throw "Missing/ambiguous published Akula asset or digest: $name"
  }
  $asset = $hits[0]
  $path = Join-Path $base $name
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $path -MaximumRetryCount 3 -RetryIntervalSec 3
  if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $asset.digest.Substring(7) -or
      (Get-Item -LiteralPath $path).Length -ne $asset.size) {
    throw "Published Akula asset differs from GitHub SHA256/size: $name"
  }
  return $path
}
$oldSetup = Published-Asset $oldName
$oldManifestPath = Published-Asset 'latest.json'
$oldSig = Published-Asset ($oldName + '.sig')
$oldManifestSig = Published-Asset 'latest.json.sig'
if ((Get-Item -LiteralPath $oldSig).Length -eq 0 -or (Get-Item -LiteralPath $oldManifestSig).Length -eq 0) {
  throw 'Published Akula release is missing detached signatures'
}
$oldManifest = Get-Content -LiteralPath $oldManifestPath -Raw | ConvertFrom-Json
$oldAsset = $oldManifest.artifacts.'windows-x86_64'
if ($oldManifest.product -cne 'Akula' -or $oldManifest.version -cne $oldVersion -or
    $oldManifest.source_commit -cne $release.target_commitish -or
    $oldAsset.filename -cne $oldName -or
    $oldAsset.sha256 -cne (Get-FileHash -LiteralPath $oldSetup -Algorithm SHA256).Hash.ToLowerInvariant() -or
    $oldAsset.size -ne (Get-Item -LiteralPath $oldSetup).Length) {
  throw 'Published baseline release source, manifest and bytes disagree'
}

$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$sidecar = Join-Path $install 'shiva-pdf-preview.exe'
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$state = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$oldReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aquilum'
$runReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$legacyInstall = Join-Path $env:LOCALAPPDATA 'Aquilum\legacy-do-not-touch.txt'
$legacyData = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app\user-data-do-not-touch.txt'
# The preceding installed Windows smoke must have uninstalled Akula, but kept
# its test profile and its independent Aquilum do-not-touch sentinels.
if ((Test-Path $exe) -or (Test-Path $reg) -or !(Test-Path $state -PathType Container)) {
  throw 'Earlier Windows installed smoke did not preserve required clean state'
}
foreach ($path in @($legacyInstall, $legacyData)) {
  if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Aquilum witness missing: $path" }
}
if (!(Test-Path $oldReg) -or !(Test-Path $runReg)) {
  throw 'Aquilum registry witnesses missing'
}
$oldDisplay = (Get-ItemProperty $oldReg -Name DisplayName).DisplayName
$oldRun = (Get-ItemProperty $runReg -Name Aquilum).Aquilum
$legacyDigests = @((Get-FileHash $legacyInstall -Algorithm SHA256).Hash, (Get-FileHash $legacyData -Algorithm SHA256).Hash)
$stateWitness = Join-Path $state 'signed-upgrade-preserve.txt'
if (Test-Path $stateWitness) { throw 'Candidate profile witness preexists' }
[IO.File]::WriteAllText($stateWitness, 'AKULA-USER-STATE')
$stateDigest = (Get-FileHash $stateWitness -Algorithm SHA256).Hash
$vault = Join-Path $base 'vault'
New-Item -ItemType Directory -Path $vault | Out-Null
$note = Join-Path $vault 'Keep.md'
[IO.File]::WriteAllText($note, '# Preserve Akula user Markdown')
$noteDigest = (Get-FileHash $note -Algorithm SHA256).Hash
function Assert-Preserved {
  if (!(Test-Path $stateWitness) -or
      (Get-FileHash $stateWitness -Algorithm SHA256).Hash -ne $stateDigest -or
      (Get-FileHash $note -Algorithm SHA256).Hash -ne $noteDigest -or
      (Get-FileHash $legacyInstall -Algorithm SHA256).Hash -ne $legacyDigests[0] -or
      (Get-FileHash $legacyData -Algorithm SHA256).Hash -ne $legacyDigests[1] -or
      (Get-ItemProperty $oldReg -Name DisplayName).DisplayName -cne $oldDisplay -or
      (Get-ItemProperty $runReg -Name Aquilum).Aquilum -cne $oldRun) {
    throw 'Akula update modified AppData, Markdown or Aquilum witness'
  }
}
function Invoke-NSIS([string]$installer, [string[]]$flags) {
  $process = Start-Process -FilePath $installer -ArgumentList $flags -PassThru
  if (!$process.WaitForExit(180000)) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    throw "NSIS timed out: $installer"
  }
  if ($process.ExitCode -ne 0) { throw "NSIS exit $($process.ExitCode): $installer" }
}
function Assert-Installed([string]$version) {
  if (!(Test-Path $exe -PathType Leaf) -or !(Test-Path $reg)) { throw "Akula $version missing" }
  if ((Get-ItemProperty $reg -Name DisplayVersion).DisplayVersion -cne $version) {
    throw "Akula installed version differs from $version"
  }
  $entries = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' |
    Where-Object { (Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue).DisplayName -ceq 'Akula' })
  if ($entries.Count -ne 1) { throw 'Duplicate or missing Akula uninstall identity' }
}
function Wait-Gui {
  for ($i = 0; $i -lt 70; $i++) {
    $windows = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue |
      Where-Object { $_.MainWindowHandle -ne 0 })
    if ($windows.Count -gt 0) { return $windows[0] }
    Start-Sleep -Seconds 1
  }
  throw 'Installed Akula GUI did not appear'
}
Invoke-NSIS $oldSetup @('/S')
Assert-Installed $oldVersion
$oldExeDigest = (Get-FileHash $exe -Algorithm SHA256).Hash
$argsForApp = '--vault "' + $vault + '" --open "' + $note + '"'
$oldProcess = Start-Process -FilePath $exe -ArgumentList $argsForApp -PassThru
$oldGui = Wait-Gui
if ($oldProcess.Id -ne $oldGui.Id -or !$oldGui.CloseMainWindow() -or !$oldGui.WaitForExit(25000)) {
  throw 'Published Akula baseline GUI did not close normally'
}
Assert-Preserved
Write-Host "AKULA_PUBLISHED_BASELINE_PASS version=$oldVersion gui=1 notes=1"

# Exercise real NSIS updater arguments against the new signed candidate.
Invoke-NSIS $newSetup @('/S','/R','/UPDATE','/ARGS')
Assert-Installed $requested
if ((Get-FileHash $exe -Algorithm SHA256).Hash -eq $oldExeDigest -or
    (Get-FileHash $exe -Algorithm SHA256).Hash -ne (Get-FileHash $builtExe -Algorithm SHA256).Hash -or
    !(Test-Path $sidecar -PathType Leaf) -or
    (Get-FileHash $sidecar -Algorithm SHA256).Hash -ne (Get-FileHash $builtPdf -Algorithm SHA256).Hash) {
  throw 'Signed upgrade did not install the exact new Akula and Shiva binaries'
}
$newGui = Wait-Gui
Assert-Preserved
Write-Host "AKULA_SIGNED_NEXT_UPGRADE_PASS previous=$oldVersion candidate=$requested gui=1 hashes=1 state=1 aquilum=1"
if (!$newGui.CloseMainWindow() -or !$newGui.WaitForExit(25000)) {
  throw 'Signed candidate GUI did not close'
}
Invoke-NSIS (Join-Path $install 'uninstall.exe') @('/S')
for ($i=0; $i -lt 40 -and ((Test-Path $exe) -or (Test-Path $reg)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path $exe) -or (Test-Path $reg) -or !(Test-Path $state)) {
  throw 'Candidate uninstall lost Akula AppData or left installed Akula'
}
Assert-Preserved
Write-Host "AKULA_SIGNED_NEXT_CLEANUP_PASS previous=$oldVersion candidate=$requested appdata=1 legacy=1"
