# AKULA-INSTALLED-UPDATER-UIA-01
# Exercises the *installed application* "Check for updates" and "Install"
# buttons via genuine Windows UI Automation; no direct NSIS upgrade command.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_OS -ne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or
    $env:GITHUB_REPOSITORY -cne 'Lalalalendia/Akula' -or
    $env:AKULA_DISPOSABLE_UPDATER_UIA -ne '1' -or
    $PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
  throw 'AKULA_UPDATER_UIA_HOLD: disposable GitHub Windows STA only'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop

$repo = $env:GITHUB_REPOSITORY
$oldVersion = '0.3.11'
$newVersion = '0.3.12'
$oldSource = '5cde5d531e5a20744dd2243e954617d216b20911'
$newSource = 'fa6ec577e55c4ace245c38130cb5ddb07d0c61a3'
$oldSha = '37414336ae05dc8ce2df46e42ae871787dce5a8cb37a1da0fddedc4989bd4793'
$newSha = '7bae191d8a9046bee0f10700e3274489ec9da2964d7517fc01dd0adddf335577'
$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$dataDir = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
$base = Join-Path $env:RUNNER_TEMP 'akula-manual-update-uia-0311-0312'
foreach ($p in @($install, $dataDir, $reg, $base)) {
  if (Test-Path -LiteralPath $p) { throw "Not a clean runner: $p" }
}
New-Item -ItemType Directory -Force -Path $base, $dataDir | Out-Null

function Read-PublicRelease([string]$version, [string]$expectedSource, [string]$expectedSha) {
  $tag = 'v' + $version
  $release = gh api "repos/$repo/releases/tags/$tag" | ConvertFrom-Json
  if ($LASTEXITCODE -ne 0 -or $release.tag_name -cne $tag -or $release.draft -or
      $release.target_commitish -cne $expectedSource) {
    throw "Published version/source mismatch: $tag"
  }
  $filename = 'Akula_' + $version + '_x64-setup.exe'
  $assets = @($release.assets | Where-Object { $_.name -ceq $filename })
  if ($assets.Count -ne 1 -or $assets[0].digest -cne "sha256:$expectedSha") {
    throw "Pinned GitHub installer mismatch: $tag"
  }
  return $assets[0]
}
$oldAsset = Read-PublicRelease $oldVersion $oldSource $oldSha
$newAsset = Read-PublicRelease $newVersion $newSource $newSha
$latest = gh api "repos/$repo/releases/latest" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $latest.tag_name -cne 'v0.3.12' -or
    $latest.target_commitish -cne $newSource) {
  throw 'Manual updater must be exercised against published Latest 0.3.12'
}
$oldSetup = Join-Path $base $oldAsset.name
gh release download 'v0.3.11' --repo $repo --dir $base --pattern $oldAsset.name
if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $oldSetup -PathType Leaf)) {
  throw 'Published 0.3.11 installer download failed'
}
if ((Get-FileHash -LiteralPath $oldSetup -Algorithm SHA256).Hash.ToLowerInvariant() -cne $oldSha -or
    (Get-Item -LiteralPath $oldSetup).Length -ne $oldAsset.size) {
  throw 'Downloaded 0.3.11 installer bytes differ from pinned digest'
}
$installed = Start-Process -FilePath $oldSetup -ArgumentList '/S' -Wait -PassThru
if ($installed.ExitCode -ne 0 -or !(Test-Path -LiteralPath $exe -PathType Leaf)) {
  throw 'Published 0.3.11 NSIS installation failed'
}
if ((Get-ItemProperty -Path $reg -Name DisplayVersion).DisplayVersion -cne $oldVersion) {
  throw 'Windows did not register installed Akula 0.3.11'
}
$oldBinaryHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
Write-Host 'AKULA_UIA_SIGNED_0311_INSTALLED_PASS'

# Explicitly disable startup auto-update. Updating must be initiated by actual
# UIA button invocation, not by App::new() calling updater.install().
$settingsPath = Join-Path $dataDir 'settings.json'
$settings = '{"updates":{"auto":false},"ui":{"language":"en"}}'
[System.IO.File]::WriteAllText($settingsPath, $settings, [System.Text.UTF8Encoding]::new($false))
$originalSettingsHash = (Get-FileHash -LiteralPath $settingsPath -Algorithm SHA256).Hash

$vault = Join-Path $base 'vault'
New-Item -ItemType Directory -Force -Path $vault | Out-Null
$note = Join-Path $vault 'UIA-updater.md'
[System.IO.File]::WriteAllText($note, '# UIA signed update preserves this Markdown', [System.Text.UTF8Encoding]::new($false))
$noteHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$legacy = Join-Path $env:APPDATA 'com.dmitriy.aquilum-app'
if (Test-Path -LiteralPath $legacy) { throw 'Upstream Aquilum data already exists' }
New-Item -ItemType Directory -Force -Path $legacy | Out-Null
$legacyNote = Join-Path $legacy 'must-not-touch.txt'
[System.IO.File]::WriteAllText($legacyNote, 'AQUILUM-UNCHANGED')
$legacyHash = (Get-FileHash -LiteralPath $legacyNote -Algorithm SHA256).Hash
function Assert-Data {
  if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $noteHash -or
      (Get-FileHash -LiteralPath $legacyNote -Algorithm SHA256).Hash -ne $legacyHash) {
    throw 'GUI updater altered user Markdown or legacy Aquilum'
  }
}
$localePath = Join-Path $env:GITHUB_WORKSPACE 'aquilum-app\native\assets\locales\en.json'
$locale = [System.IO.File]::ReadAllText($localePath) | ConvertFrom-Json
$labels = @{
  settings = [string]$locale.common.settings
  system = [string]$locale.settings.nav.system
  check = [string]$locale.settings.system.checkUpdates
  install = [string]$locale.settings.system.installUpdate
}
foreach ($label in $labels.Values) {
  if ([string]::IsNullOrWhiteSpace($label)) { throw 'A UIA button lacks an English accessible name' }
}

$argsForApp = '--vault "' + $vault + '" --data-dir "' + $dataDir + '" --open "' + $note + '" --focus'
$oldProcess = Start-Process -FilePath $exe -ArgumentList $argsForApp -PassThru
for ($i = 0; $i -lt 80; $i++) {
  Start-Sleep -Milliseconds 300
  $oldProcess.Refresh()
  if ($oldProcess.HasExited) { throw '0.3.11 exited before user chose any update button' }
  if ($oldProcess.MainWindowHandle -ne [IntPtr]::Zero) { break }
}
if ($oldProcess.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Installed 0.3.11 GUI did not appear' }
$window = [IntPtr]$oldProcess.MainWindowHandle
$buttonCondition = [System.Windows.Automation.PropertyCondition]::new(
  [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
  [System.Windows.Automation.ControlType]::Button
)
function Wait-Button([string]$name, [int]$seconds) {
  $attempts = $seconds * 4
  for ($i = 0; $i -lt $attempts; $i++) {
    $oldProcess.Refresh()
    if ($oldProcess.HasExited) { throw "Application exited while awaiting UIA button '$name'" }
    try {
      $root = [System.Windows.Automation.AutomationElement]::FromHandle($window)
      $nodes = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
      for ($j = 0; $j -lt $nodes.Count; $j++) {
        $button = $nodes.Item($j)
        if ($button.Current.Name -ceq $name -and $button.Current.IsEnabled -and
            !$button.Current.IsOffscreen) {
          return $button
        }
      }
    } catch [System.Windows.Automation.ElementNotAvailableException] {
      # AccessKit may swap a whole dialog during an update status refresh.
    }
    Start-Sleep -Milliseconds 250
  }
  throw "No enabled and visible UI Automation button '$name' in $seconds seconds"
}
function Invoke-Button([string]$name, [int]$seconds) {
  $button = Wait-Button $name $seconds
  $pattern = $button.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
  if ($null -eq $pattern) { throw "No UIA InvokePattern on '$name'" }
  ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
}
# This is the genuine Settings -> System -> Check for updates -> Install path.
Invoke-Button $labels.settings 40
Invoke-Button $labels.system 25
Invoke-Button $labels.check 25
Write-Host 'AKULA_UIA_MANUAL_CHECK_INVOKED_PASS'
Invoke-Button $labels.install 150
Write-Host 'AKULA_UIA_MANUAL_INSTALL_INVOKED_PASS'
# Signed metadata discovery, payload signature verification, staging and
# ShellExecuteW all happen inside installed Akula; the test never runs v0.3.12 NSIS.
if (!$oldProcess.WaitForExit(180000)) {
  throw 'Installed Akula 0.3.11 did not exit after user clicked Install'
}
Write-Host 'AKULA_UIA_INSTALLED_APP_APPLY_EXIT_PASS'

$newProcess = $null
for ($i = 0; $i -lt 180; $i++) {
  Start-Sleep -Seconds 1
  $newCandidates = @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue |
    Where-Object { $_.Id -ne $oldProcess.Id -and $_.MainWindowHandle -ne 0 })
  if ($newCandidates.Count -gt 0 -and (Test-Path -LiteralPath $reg) -and
      (Get-ItemProperty -Path $reg -Name DisplayVersion).DisplayVersion -ceq $newVersion) {
    $newProcess = $newCandidates[0]
    break
  }
}
if ($null -eq $newProcess) {
  throw 'Signed GUI update did not install and launch a visible 0.3.12 process'
}
$newBinaryHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
if ($newBinaryHash -ceq $oldBinaryHash) {
  throw 'Manual GUI update left installed 0.3.11 binary unchanged'
}
Assert-Data
if (!(Test-Path -LiteralPath $settingsPath -PathType Leaf) -or
    (Get-FileHash -LiteralPath $settingsPath -Algorithm SHA256).Hash -ne $originalSettingsHash) {
  throw 'Signed GUI updater changed manual update preference'
}
Write-Host 'AKULA_UIA_SIGNED_IN_APP_UPDATE_PASS from=0.3.11 to=0.3.12 manual_check=1 manual_install=1 signed_apply=1 gui=1 notes=1'

# The update itself is accepted before cleanup. Enforce a clean close and
# uninstall as a second independent acceptance marker.
$newProcess.Refresh()
if (!$newProcess.CloseMainWindow() -or !$newProcess.WaitForExit(25000)) {
  throw 'Installed 0.3.12 failed graceful GUI shutdown after automatic relaunch'
}
if (@(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue).Count -ne 0) {
  throw 'Akula left unexpected processes after manual GUI update'
}
$uninstaller = Join-Path $install 'uninstall.exe'
if (!(Test-Path -LiteralPath $uninstaller -PathType Leaf)) { throw 'Published updater left no NSIS uninstaller' }
$removed = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
if ($removed.ExitCode -ne 0) { throw "Uninstaller exit $($removed.ExitCode)" }
for ($i = 0; $i -lt 45 -and ((Test-Path -LiteralPath $exe) -or (Test-Path -LiteralPath $reg)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path -LiteralPath $exe) -or (Test-Path -LiteralPath $reg) -or
    !(Test-Path -LiteralPath $dataDir -PathType Container)) {
  throw 'GUI updater cleanup failed or deleted user AppData'
}
Assert-Data
Write-Host 'AKULA_UIA_SIGNED_IN_APP_CLEANUP_PASS uninstall=1 state=1 notes=1'
