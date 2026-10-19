# Run ONLY on a clean, GitHub-hosted ephemeral Windows runner.
# In particular, never execute this script on a developer workstation.
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
  throw 'Windows beta installer smoke is restricted to GitHub-hosted Windows runners'
}
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

$legacyInstall = Join-Path $env:LOCALAPPDATA 'Aquilum'
$legacyData = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app'
$uninstallBase = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$legacyRegistry = Join-Path $uninstallBase 'Aquilum'
$akulaRegistry = Join-Path $uninstallBase 'Akula'
$runRegistry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$akulaInstall = Join-Path $env:LOCALAPPDATA 'Akula'
$akulaData = Join-Path $env:APPDATA 'io.github.lalalendia.akula'

foreach ($path in @($legacyInstall, $legacyData, $legacyRegistry, $akulaRegistry, $akulaInstall, $akulaData)) {
  if (Test-Path $path) { throw "Not a clean runner: $path already exists" }
}
# Model an existing Aquilum installation without downloading or executing it.
# A genuine Akula installer must not overwrite its directories, Run key or uninstall key.
New-Item -ItemType Directory -Force -Path $legacyInstall, $legacyData | Out-Null
$legacyExeMarker = Join-Path $legacyInstall 'legacy-do-not-touch.txt'
$legacyDataMarker = Join-Path $legacyData 'user-data-do-not-touch.txt'
Set-Content -LiteralPath $legacyExeMarker -Value 'AQUILUM-INSTALL'
Set-Content -LiteralPath $legacyDataMarker -Value 'AQUILUM-DATA'
New-Item -Path $legacyRegistry -Force | Out-Null
Set-ItemProperty -Path $legacyRegistry -Name DisplayName -Value 'Aquilum'
Set-ItemProperty -Path $legacyRegistry -Name DisplayVersion -Value '0.2.6'
New-Item -Path $runRegistry -Force | Out-Null
Set-ItemProperty -Path $runRegistry -Name Aquilum -Value 'Aquilum-startup-sentinel'

function Assert-LegacyUntouched {
  if ((Get-Content -LiteralPath $legacyExeMarker -Raw).Trim() -ne 'AQUILUM-INSTALL') { throw 'Upstream install was modified' }
  if ((Get-Content -LiteralPath $legacyDataMarker -Raw).Trim() -ne 'AQUILUM-DATA') { throw 'Upstream app data was modified' }
  if ((Get-ItemProperty -Path $legacyRegistry -Name DisplayName).DisplayName -ne 'Aquilum') { throw 'Upstream uninstall entry was modified' }
  if ((Get-ItemProperty -Path $runRegistry -Name Aquilum).Aquilum -ne 'Aquilum-startup-sentinel') { throw 'Upstream startup key was modified' }
}

$setups = @(Get-ChildItem (Join-Path $env:GITHUB_WORKSPACE 'dist') -Filter 'Akula_*-setup.exe')
if ($setups.Count -ne 1) { throw "Expected a single Akula setup, found $($setups.Count)" }
$installed = Start-Process $setups[0].FullName -ArgumentList '/S' -Wait -PassThru
if ($installed.ExitCode -ne 0) { throw "Akula NSIS exit: $($installed.ExitCode)" }

$exe = Join-Path $akulaInstall 'akula-app.exe'
if (!(Test-Path $exe)) { throw "Akula binary not installed at $exe" }
if (!(Test-Path $akulaRegistry)) { throw 'Akula uninstall registration missing' }
if ((Get-ItemProperty $akulaRegistry -Name DisplayName).DisplayName -ne 'Akula') { throw 'Installer has wrong product name' }
$built = Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\release\akula-app.exe'
if ((Get-FileHash $exe).Hash -ne (Get-FileHash $built).Hash) { throw 'Installed binary does not match built Akula' }
Assert-LegacyUntouched

$vault = Join-Path $env:RUNNER_TEMP 'akula-beta-vault'
New-Item -ItemType Directory -Force -Path $vault | Out-Null
$note = Join-Path $vault 'First note.md'
Set-Content -LiteralPath $note -Value "# Akula Beta`n`nExisting local document"
$original = (Get-FileHash -LiteralPath $note).Hash
$syntheticEdit = 'AKULA_GUI_EDIT_' + [Guid]::NewGuid().ToString('N')

function Start-And-Close {
  param([switch]$EditNote)
  $arguments = "--vault `"$vault`" --data-dir `"$akulaData`" --open `"$note`""
  if ($EditNote) { $arguments += ' --focus' }
  $app = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
  $visible = $false
  for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 1
    $app.Refresh()
    if ($app.HasExited) { throw "Akula exited unexpectedly: $($app.ExitCode)" }
    if ($app.MainWindowHandle -ne 0) { $visible = $true; break }
  }
  if (!$visible) { throw 'Akula window did not appear' }
  if ($EditNote) {
    # This must be a genuine key sequence into the focused installed editor.
    # The script never writes to $note directly after initial fixture creation.
    Send-AkulaBetaText -Process $app -Text $syntheticEdit
    $published = $false
    for ($i = 0; $i -lt 30; $i++) {
      Start-Sleep -Seconds 1
      $app.Refresh()
      if ($app.HasExited) { throw 'Akula exited during the GUI edit' }
      $onDisk = [System.IO.File]::ReadAllText($note)
      if ($onDisk.Contains($syntheticEdit)) { $published = $true; break }
    }
    if (!$published) {
      throw 'OS-injected GUI text was not saved to the Markdown file within 30 seconds'
    }
    if (!$onDisk.Contains('# Akula Beta') -or !$onDisk.Contains('Existing local document')) {
      throw 'GUI edit removed original note content'
    }
  }
  if (!$app.CloseMainWindow()) { throw 'Akula window refused close request' }
  if (!$app.WaitForExit(20000)) { throw 'Akula did not exit within 20s' }
}
Start-And-Close -EditNote
$state = Join-Path $akulaData 'window-state.json'
if (!(Test-Path $state)) { throw 'Akula window state not persisted to its own AppData' }
$savedText = [System.IO.File]::ReadAllText($note)
if (!$savedText.Contains($syntheticEdit) -or !$savedText.Contains('# Akula Beta') -or !$savedText.Contains('Existing local document')) {
  throw 'Installed GUI edit was not preserved after graceful shutdown'
}
if ($savedText.Split([string[]]@($syntheticEdit), [StringSplitOptions]::None).Length -ne 2) {
  throw 'Expected exactly one GUI-injected edit marker'
}
$savedHash = (Get-FileHash -LiteralPath $note).Hash
if ($savedHash -eq $original) { throw 'GUI input had no observable file effect' }
# The second launch must not change bytes, even after reopening that same note.
Start-And-Close
if ((Get-FileHash -LiteralPath $note).Hash -ne $savedHash) { throw 'Restart changed the previously edited note' }
Assert-LegacyUntouched

$uninstall = Join-Path $akulaInstall 'uninstall.exe'
if (!(Test-Path $uninstall)) { throw 'Akula uninstaller missing' }
$removed = Start-Process $uninstall -ArgumentList '/S' -Wait -PassThru
if ($removed.ExitCode -ne 0) { throw "Akula uninstall exit: $($removed.ExitCode)" }
for ($i = 0; $i -lt 30 -and ((Test-Path $exe) -or (Test-Path $akulaRegistry)); $i++) {
  Start-Sleep -Seconds 1
}
if (Test-Path $exe) { throw 'Akula executable remains after uninstall' }
if (Test-Path $akulaRegistry) { throw 'Akula uninstall registry key remains' }
if (!(Test-Path $state)) { throw 'Uninstall removed Akula user state' }
if ((Get-FileHash -LiteralPath $note).Hash -ne $savedHash) { throw 'Uninstall changed the edited local document' }
Assert-LegacyUntouched
Write-Host 'AKULA_WINDOWS_BETA_GUI_EDIT_PASS: NSIS install, real OS keyboard edit, on-disk save, restart, uninstall and upstream sentinels'
