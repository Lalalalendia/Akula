# RELEASE-WINDOWS-NSIS-UPGRADE-01. Run on disposable GitHub Windows only.
# Two genuinely compiled Akula installers; no upstream Aquilum migration.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:RUNNER_OS -ne 'Windows' -or
    $env:GITHUB_ACTIONS -ne 'true' -or
    $env:AKULA_WINDOWS_DISPOSABLE_UPGRADE_LAB -ne '1') {
  throw 'AKULA_WINDOWS_UPDATE_HOLD: non-ephemeral Windows update test refused'
}
$dist = Join-Path $env:GITHUB_WORKSPACE 'dist'
$v1s = @(Get-ChildItem -LiteralPath $dist -Filter 'Akula_0.3.9_*-setup.exe' -File)
$v2s = @(Get-ChildItem -LiteralPath (Join-Path $dist 'e2e-v2') -Filter 'Akula_0.3.10_*-setup.exe' -File)
if ($v1s.Count -ne 1 -or $v2s.Count -ne 1) {
  throw "Expected exactly one genuine Akula NSIS v1 and v2: $($v1s.Count)/$($v2s.Count)"
}
$v1 = $v1s[0].FullName
$v2 = $v2s[0].FullName
if ((Get-FileHash $v1).Hash -eq (Get-FileHash $v2).Hash) {
  throw 'Two Windows installers have identical bytes'
}
$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$state = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$registry = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$oldInstall = Join-Path $env:LOCALAPPDATA 'Aquilum\legacy-do-not-touch.txt'
$oldData = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app\user-data-do-not-touch.txt'
$oldReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Aquilum'
$runReg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$sentinels = @{}
foreach ($path in @($oldInstall, $oldData)) {
  if (!(Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing Aquilum witness: $path" }
  $sentinels[$path] = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
}
if (!(Test-Path $oldReg)) { throw 'Aquilum uninstall witness absent' }
$oldDisplay = (Get-ItemProperty -Path $oldReg -Name DisplayName).DisplayName
$oldRun = (Get-ItemProperty -Path $runReg -Name Aquilum).Aquilum
$expectedV1 = (Get-Content -LiteralPath (Join-Path $dist 'e2e-v1-executable-sha256.txt') -Raw).Trim().ToUpperInvariant()
$expectedV2 = (Get-Content -LiteralPath (Join-Path $dist 'e2e-v2/executable-sha256.txt') -Raw).Trim().ToUpperInvariant()
if ($expectedV1 -notmatch '^[A-F0-9]{64}$' -or
    $expectedV2 -notmatch '^[A-F0-9]{64}$' -or
    $expectedV1 -eq $expectedV2) {
  throw 'Different real native v1/v2 executable SHA-256 values are required'
}
$vault = Join-Path $env:RUNNER_TEMP 'akula-nsis-two-version-vault'
New-Item -ItemType Directory -Force -Path $vault | Out-Null
$note = Join-Path $vault 'Upgrade.md'
Set-Content -LiteralPath $note -Encoding UTF8 -Value '# Akula upgrade acceptance - preserve this document'
$noteSha = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash

function Assert-Witnesses {
  if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteSha) {
    throw 'Upgrade changed the Markdown document'
  }
  foreach ($path in $sentinels.Keys) {
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $sentinels[$path]) {
      throw "Akula update modified upstream Aquilum file $path"
    }
  }
  if ((Get-ItemProperty -Path $oldReg -Name DisplayName).DisplayName -ne $oldDisplay -or
      (Get-ItemProperty -Path $runReg -Name Aquilum).Aquilum -ne $oldRun) {
    throw 'Akula update modified upstream Aquilum registry'
  }
}

function Invoke-NSIS([string]$installer, [string[]]$flags) {
  $process = Start-Process -FilePath $installer -ArgumentList $flags -PassThru
  if (!$process.WaitForExit(120000)) {
    Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
    throw "NSIS update timed out: $installer"
  }
  if ($process.ExitCode -ne 0) { throw "NSIS returned $($process.ExitCode): $installer" }
}

function Assert-Installed([string]$version, [string]$digest) {
  if (!(Test-Path -LiteralPath $exe -PathType Leaf) -or !(Test-Path $registry)) {
    throw "Installed Akula missing for $version"
  }
  $actual = (Get-ItemProperty -Path $registry -Name DisplayVersion).DisplayVersion
  if ($actual -ne $version) { throw "Installed $actual; expected $version" }
  if ((Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash -ne $digest) {
    throw "Native installed binary SHA mismatch for $version"
  }
  $registered = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' |
    Where-Object { (Get-ItemProperty -Path $_.PSPath -ErrorAction SilentlyContinue).DisplayName -eq 'Akula' })
  if ($registered.Count -ne 1) { throw 'Duplicate or missing Akula uninstall identity' }
}

function Wait-AkulaGUI([int]$excludedPid = -1) {
  for ($i = 0; $i -lt 55; $i++) {
    $candidate = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue |
      Where-Object { $_.MainWindowHandle -ne 0 -and $_.Id -ne $excludedPid })
    if ($candidate.Count -gt 0) { return $candidate[0] }
    Start-Sleep -Seconds 1
  }
  throw 'Akula GUI did not open after NSIS update'
}

function Close-AkulaGUI {
  $all = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue)
  foreach ($process in $all) {
    $process.Refresh()
    if ($process.MainWindowHandle -ne 0) { $null = $process.CloseMainWindow() }
    if (!$process.WaitForExit(25000)) {
      Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
      throw 'Akula GUI did not exit cleanly'
    }
  }
}

# Existing Windows beta smoke completed and uninstalled Akula, keeping user data.
if (Test-Path -LiteralPath $exe) { throw 'Earlier Windows beta smoke left Akula installed' }
Invoke-NSIS -installer $v1 -flags @('/S')
Assert-Installed '0.3.9' $expectedV1
$guiArgs = '--vault "' + $vault + '" --open "' + $note + '"'
Start-Process -FilePath $exe -ArgumentList $guiArgs | Out-Null
$old = Wait-AkulaGUI
Write-Host "AKULA_WINDOWS_REAL_V1_GUI_PASS pid=$($old.Id)"

# The real upstream Windows auto-updater used these NSIS upgrade flags.
Invoke-NSIS -installer $v2 -flags @('/S','/R','/UPDATE','/ARGS')
Assert-Installed '0.3.10' $expectedV2
$new = Wait-AkulaGUI -excludedPid $old.Id
Assert-Witnesses
Write-Host 'AKULA_WINDOWS_REAL_UPGRADE_PASS v1=0.3.9 v2=0.3.10 nsis=1 sha256=1 gui=1 upstream_intact=1'

# Kill v2 and run an explicit v1 installer recovery. This is NOT an
# automatic rollback or durable version-floor test: those remain P0.
Stop-Process -Id $new.Id -Force
for ($i = 0; $i -lt 35 -and (Get-Process -Id $new.Id -ErrorAction SilentlyContinue); $i++) {
  Start-Sleep -Milliseconds 500
}
Invoke-NSIS -installer $v1 -flags @('/S','/R','/UPDATE','/ARGS')
Assert-Installed '0.3.9' $expectedV1
$restored = Wait-AkulaGUI -excludedPid $new.Id
Assert-Witnesses
Write-Host 'AKULA_WINDOWS_REAL_RECOVERY_PASS old_version=0.3.9 exact_binary=1 gui=1 note=1 upstream_intact=1'
Close-AkulaGUI

if (!(Test-Path -LiteralPath $state)) { throw 'Akula AppData absent after upgrade/recovery' }
$uninstaller = Join-Path $install 'uninstall.exe'
if (!(Test-Path -LiteralPath $uninstaller -PathType Leaf)) { throw 'NSIS uninstaller missing' }
Invoke-NSIS -installer $uninstaller -flags @('/S')
for ($i = 0; $i -lt 60 -and ((Test-Path $exe) -or (Test-Path $registry)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path $exe) -or (Test-Path $registry)) { throw 'NSIS uninstall left Akula behind' }
if (!(Test-Path -LiteralPath $state)) { throw 'Uninstall erased Akula user state' }
Assert-Witnesses
Write-Host 'AKULA_WINDOWS_NSIS_TWO_VERSION_PASS install=1 v1_to_v2=1 recovery_v1=1 real_gui=1 data_untouched=1'
