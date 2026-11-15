# Disposable Windows QA: test profile handoff and the real published v0.3.18
# startup/WM_CLOSE on a fresh default profile. Never runs on a user machine.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows' -or
    $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_REPOSITORY -cne 'Lalalalendia/Akula' -or
    $env:AKULA_PUBLISHED_BASELINE_DISPOSABLE -cne '1' -or
    $env:GITHUB_EVENT_NAME -cnotin @('pull_request', 'workflow_dispatch')) {
  throw 'AKULA_PUBLISHED_BASELINE_HOLD: disposable GitHub-hosted Windows only'
}
. (Join-Path $PSScriptRoot 'akula-published-baseline-profile.ps1')
$root = Join-Path $env:RUNNER_TEMP ('akula-published-018-qa-' + $env:GITHUB_RUN_ID + '-' + $env:GITHUB_RUN_ATTEMPT)
if (Test-Path -LiteralPath $root) { throw 'AKULA_PUBLISHED_BASELINE_HOLD: fixture already exists' }
New-Item -ItemType Directory -Path $root | Out-Null

# Functional, synthetic manifest/custody tests; no real application state.
$fixture = Join-Path $root 'synthetic-newer-profile'
New-Item -ItemType Directory -Path (Join-Path $fixture 'nested') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $fixture 'newer-state.txt'), 'AKULA-NEWER-PROFILE')
[IO.File]::WriteAllText((Join-Path $fixture 'nested\file.txt'), 'KEEP-NESTED-STATE')
$backup = Join-Path $root 'preserved-newer-profile'
$witness = Reserve-AkulaPriorSmokeProfile $fixture $backup
if (Test-Path -LiteralPath $fixture) { throw 'Profile swap left newer state in default directory' }
Assert-AkulaPriorSmokeProfile $witness
New-Item -ItemType Directory -Path $fixture | Out-Null
[IO.File]::WriteAllText((Join-Path $fixture 'older-state.txt'), 'AKULA-OLDER-PROFILE')
Assert-AkulaPriorSmokeProfile $witness
$changed = Join-Path $backup 'newer-state.txt'
[IO.File]::WriteAllText($changed, 'CORRUPTED')
$refused = $false
try { Assert-AkulaPriorSmokeProfile $witness } catch {
  if ($_.Exception.Message -cnotmatch 'prior installed smoke AppData changed') { throw }
  $refused = $true
}
if (!$refused) { throw 'Profile witness did not detect corruption' }
[IO.File]::WriteAllText($changed, 'AKULA-NEWER-PROFILE')
Assert-AkulaPriorSmokeProfile $witness
Write-Host 'AKULA_PROFILE_HANDOFF_TEST_PASS old_fresh=1 new_preserved=1 sha256=1 tamper_refused=1'

$install = Join-Path $env:LOCALAPPDATA 'Akula'
$exe = Join-Path $install 'akula-app.exe'
$state = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$reg = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Akula'
if ((Test-Path -LiteralPath $install) -or (Test-Path -LiteralPath $state) -or
    (Test-Path $reg) -or
    @(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue).Count -ne 0) {
  throw 'AKULA_PUBLISHED_BASELINE_HOLD: runner already contains Akula'
}
$release = Invoke-RestMethod -Uri 'https://api.github.com/repos/Lalalalendia/Akula/releases/tags/v0.3.18' -Headers @{
  Accept = 'application/vnd.github+json'
  'User-Agent' = 'Akula-Published-018-Cold-Baseline-QA'
}
$name = 'Akula_0.3.18_x64-setup.exe'
$sha = 'b8efff995fdcce4950bb02e47b29ba37c2018579bd0b2911daa938b592434cf4'
$assets = @($release.assets | Where-Object { $_.name -ceq $name })
$signatures = @($release.assets | Where-Object { $_.name -ceq ($name + '.sig') })
if ($release.tag_name -cne 'v0.3.18' -or $release.draft -or $release.prerelease -or
    $release.target_commitish -cne 'c65bcd6bc2eeb6ec61684b3c1613def17a695df4' -or
    $assets.Count -ne 1 -or $signatures.Count -ne 1 -or $signatures[0].size -le 0 -or
    $assets[0].size -ne 22362932 -or $assets[0].digest -cne ('sha256:' + $sha) -or
    $assets[0].browser_download_url -cne
      ('https://github.com/Lalalalendia/Akula/releases/download/v0.3.18/' + $name)) {
  throw 'AKULA_PUBLISHED_BASELINE_HOLD: published 0.3.18 identity or signature metadata changed'
}
$setup = Join-Path $root $name
Invoke-WebRequest -Uri $assets[0].browser_download_url -OutFile $setup -MaximumRetryCount 3 -RetryIntervalSec 3
if ((Get-Item -LiteralPath $setup).Length -ne 22362932 -or
    (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant() -cne $sha) {
  throw 'AKULA_PUBLISHED_BASELINE_HOLD: downloaded 0.3.18 installer differs'
}
$installer = Start-Process -FilePath $setup -ArgumentList '/S' -PassThru
if (!$installer.WaitForExit(120000)) { throw 'Published 0.3.18 installer timed out' }
if ($installer.ExitCode -ne 0 -or !(Test-Path -LiteralPath $exe -PathType Leaf) -or
    (Get-ItemProperty $reg -Name DisplayVersion).DisplayVersion -cne '0.3.18') {
  throw 'Published 0.3.18 was not installed with expected version'
}
$vault = Join-Path $root 'vault'
New-Item -ItemType Directory -Path $vault | Out-Null
$note = Join-Path $vault 'Keep.md'
[IO.File]::WriteAllText($note, '# Original published baseline')
$noteHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash
$arguments = '--vault "' + $vault + '" --open "' + $note + '"'
$old = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
$visible = $false
for ($i=0; $i -lt 70; $i++) {
  $old.Refresh()
  if ($old.HasExited) { throw 'Published 0.3.18 exited before showing GUI' }
  if ($old.MainWindowHandle -ne 0) { $visible = $true; break }
  Start-Sleep -Seconds 1
}
if (!$visible -or !(Test-Path -LiteralPath $state -PathType Container)) {
  throw 'Published 0.3.18 did not open a GUI and initialize default AppData'
}
$profileNote = Join-Path $state 'published-018-profile-witness.txt'
[IO.File]::WriteAllText($profileNote, 'OLD-GUI-USER-STATE')
$profileHash = (Get-FileHash -LiteralPath $profileNote -Algorithm SHA256).Hash
if (!$old.CloseMainWindow()) { throw "Published 0.3.18 rejected WM_CLOSE (pid=$($old.Id))" }
if (!$old.WaitForExit(25000)) {
  $old.Refresh()
  throw "Published 0.3.18 GUI did not exit after WM_CLOSE with CLEAN old profile (pid=$($old.Id), hwnd=$($old.MainWindowHandle))"
}
if (@(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue).Count -ne 0 -or
    (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteHash -or
    (Get-FileHash -LiteralPath $profileNote -Algorithm SHA256).Hash -cne $profileHash) {
  throw 'Published 0.3.18 close modified Markdown/state or left a process'
}
Write-Host 'AKULA_PUBLISHED_018_COLD_GUI_PASS version=0.3.18 fresh_appdata=1 wm_close=1 hashes=1'

$uninstall = Join-Path $install 'uninstall.exe'
$p = Start-Process -FilePath $uninstall -ArgumentList '/S' -PassThru
if (!$p.WaitForExit(120000) -or $p.ExitCode -ne 0) {
  throw 'Published 0.3.18 uninstall failed'
}
# NSIS can return before the uninstaller child finishes removing its files.
# The required installed beta smoke uses the same bounded asynchronous fence.
for ($i = 0; $i -lt 40 -and ((Test-Path -LiteralPath $exe) -or (Test-Path $reg)); $i++) {
  Start-Sleep -Seconds 1
}
if ((Test-Path -LiteralPath $exe) -or (Test-Path $reg) -or
    (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -cne $noteHash -or
    (Get-FileHash -LiteralPath $profileNote -Algorithm SHA256).Hash -cne $profileHash) {
  throw 'Published 0.3.18 uninstall modified user data'
}
Write-Host 'AKULA_PUBLISHED_018_COLD_UNINSTALL_PASS appdata=1 note_sha256=1'
