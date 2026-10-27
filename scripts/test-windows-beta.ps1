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

# Verify the *installed* companion is the exact compiled pinned Shiva binary;
# this is an integration test, not a downloadable or user-run converter script.
$sidecar = Join-Path $akulaInstall 'shiva-pdf-preview.exe'
$compiledSidecar = Join-Path $env:GITHUB_WORKSPACE '.artifacts\cargo-native\release\shiva-pdf-preview.exe'
if (!(Test-Path -LiteralPath $sidecar -PathType Leaf)) {
  throw 'Installed Akula is missing the required Shiva PDF sidecar'
}
if ((Get-FileHash -LiteralPath $sidecar).Hash -ne (Get-FileHash -LiteralPath $compiledSidecar).Hash) {
  throw 'Installed Shiva PDF sidecar differs from compiled pinned source'
}
$probeDir = Join-Path $env:RUNNER_TEMP 'akula-pdf-smoke'
New-Item -ItemType Directory -Force -Path $probeDir | Out-Null
$sourcePdf = Join-Path $probeDir 'real-text.pdf'
$extractedMd = Join-Path $probeDir 'text.md'
python scripts/pdf-preview/generate-fixture.py --out $sourcePdf
if ($LASTEXITCODE -ne 0) { throw 'Synthetic PDF generation failed' }
& $sidecar $sourcePdf $extractedMd
if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $extractedMd -PathType Leaf)) {
  throw 'Installed Shiva PDF sidecar rejected valid digital-text PDF'
}
$extracted = Get-Content -LiteralPath $extractedMd -Raw -Encoding UTF8
if (-not $extracted.Contains('AKULANATIVEPDFNEEDLE')) {
  throw 'Installed Shiva PDF extraction lost native PDF text'
}
Write-Host 'AKULA_PDF_SIDECAR_INSTALLED=PASS;source=pinned-shiva;real_pdf=1;external_setup=0'
# Run the full native application integration path against this *installed*
# sidecar: File Gate, idempotence, Tantivy search UI cards and result opening.
$env:AKULA_PDF_INSTALLED_SIDECAR = $sidecar
$env:AKULA_PDF_FIXTURE_PATH = $sourcePdf
Push-Location (Join-Path $env:GITHUB_WORKSPACE 'aquilum-app')
try {
  cargo test --locked -p aquilum-native --bin aquilum-native app::tests::pdf_preview::installed_shiva_pdf_is_searchable_and_opens_from_native_result -- --exact --ignored --nocapture
  if ($LASTEXITCODE -ne 0) { throw 'Installed Shiva to native Akula search E2E failed' }
} finally {
  Pop-Location
  Remove-Item Env:\AKULA_PDF_INSTALLED_SIDECAR -ErrorAction SilentlyContinue
  Remove-Item Env:\AKULA_PDF_FIXTURE_PATH -ErrorAction SilentlyContinue
}
Write-Host 'AKULA_PDF_NATIVE_SEARCH_E2E=PASS;source=installed;real_ui_state=1'


$vault = Join-Path $env:RUNNER_TEMP 'akula-beta-vault'
New-Item -ItemType Directory -Force -Path $vault | Out-Null
$note = Join-Path $vault 'First note.md'
Set-Content -LiteralPath $note -Value "# Akula Beta`n`nExisting local document"
$original = (Get-FileHash -LiteralPath $note).Hash
$syntheticEdit = 'AKULA_GUI_EDIT_' + [Guid]::NewGuid().ToString('N')

function Start-And-Close {
  param([switch]$EditNote, [switch]$VerifyRestoredContent)
  if ($EditNote -and $VerifyRestoredContent) { throw 'Only one Windows GUI acceptance mode per launch' }
  $arguments = "--vault `"$vault`" --data-dir `"$akulaData`" --open `"$note`""
  if ($EditNote -or $VerifyRestoredContent) { $arguments += ' --focus' }
  if ($EditNote) { $arguments += ' --trace' }
  # Diagnostics remain on the disposable runner only. Never upload raw GUI
  # event traces, which can contain typed text.
  $traceFile = if ($EditNote) { Join-Path $env:RUNNER_TEMP 'akula-beta-gui-events.log' } else { $null }
  if ($EditNote) {
    $app = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru -RedirectStandardError $traceFile
  } else {
    $app = Start-Process -FilePath $exe -ArgumentList $arguments -PassThru
  }
  $visible = $false
  for ($i = 0; $i -lt 30; $i++) {
    Start-Sleep -Seconds 1
    $app.Refresh()
    if ($app.HasExited) { throw "Akula exited unexpectedly: $($app.ExitCode)" }
    if ($app.MainWindowHandle -ne 0) { $visible = $true; break }
  }
  if (!$visible) { throw 'Akula window did not appear' }
  if ($EditNote -or $VerifyRestoredContent) {
    # Guard document identity before typing or UI readback. A process/HWND-only check cannot
    # distinguish an editor from an Akula home page or an open-error pane.
    $noteWindowReady = $false
    for ($i = 0; $i -lt 15; $i++) {
      $app.Refresh()
      if ($app.HasExited) { throw 'Akula exited before opening the requested note' }
      if ($app.MainWindowTitle -like 'First note*' -and $app.MainWindowTitle -like '*Akula*') {
        $noteWindowReady = $true
        break
      }
      Start-Sleep -Milliseconds 250
    }
    if (!$noteWindowReady) {
      throw 'Installed Akula did not show the requested note title before OS input'
    }
  }
  if ($EditNote) {
    # This must be genuine OS input into the installed foreground editor.
    # After initial fixture creation this script never writes to $note.
    $noteWindowTitle = $app.MainWindowTitle -like '*First note*'
    Send-AkulaBetaText -Process $app -Text $syntheticEdit
    $published = $false
    for ($i = 0; $i -lt 30; $i++) {
      Start-Sleep -Seconds 1
      $app.Refresh()
      if ($app.HasExited) { throw 'Akula exited during the GUI edit' }
      $onDisk = [System.IO.File]::ReadAllText($note)
      if ($onDisk.Contains($syntheticEdit)) { $published = $true; break }
    }
  }
  if ($VerifyRestoredContent) {
    # This is an OS-level GUI readback, not a file read masquerading as UI.
    # Poison the clipboard before launch; only an actual editor Copy command
    # can replace it with the note's contents. Never print clipboard data.
    Send-AkulaBetaShortcut -Process $app -Action SelectAll
    Start-Sleep -Milliseconds 350
    Send-AkulaBetaShortcut -Process $app -Action Copy
    $copied = $null
    for ($i = 0; $i -lt 20; $i++) {
      Start-Sleep -Milliseconds 300
      try {
        $candidate = Get-Clipboard -Raw -ErrorAction Stop
      } catch {
        $candidate = $null
      }
      if (![string]::IsNullOrEmpty($candidate) -and $candidate -ne $clipboardSentinel) {
        $copied = $candidate
        break
      }
    }
    if ($null -eq $copied) { throw 'Reopened editor did not place selected note into Windows clipboard' }
    $normalizeText = {
      param([string]$Value)
      $Value.Replace("`r`n", "`n").Replace("`r", "`n").TrimEnd([char]10)
    }
    $actual = & $normalizeText $copied
    $expected = & $normalizeText ([System.IO.File]::ReadAllText($note))
    if (![string]::Equals($actual, $expected, [StringComparison]::Ordinal)) {
      throw 'Reopened editor copy differs from the exact saved Markdown note'
    }
    if (!$actual.Contains($syntheticEdit) -or !$actual.Contains('# Akula Beta') -or
        !$actual.Contains('Existing local document')) {
      throw 'Reopened editor copy omitted either synthetic edit or original body'
    }
    Write-Host "AKULA_WINDOWS_BETA_REOPEN_GUI_READBACK_PASS: selected=True clipboard_replaced=True saved_note_match=True chars=$($actual.Length)"
  }
  # Even after a failed autosave, close normally to distinguish no-typing,
  # delayed-on-close saves, and rejected writes. Failure still remains red.
  if (!$app.CloseMainWindow()) { throw 'Akula window refused close request' }
  if (!$app.WaitForExit(20000)) { throw 'Akula did not exit within 20s' }
  if ($EditNote) {
    $afterClose = [System.IO.File]::ReadAllText($note)
    $postClosePublished = $afterClose.Contains($syntheticEdit)
    $traceText = if (Test-Path -LiteralPath $traceFile) {
      [System.IO.File]::ReadAllText($traceFile)
    } else { '' }
    $keyboardEvents = [regex]::Matches($traceText, 'KeyboardInput').Count
    $imeCommits = [regex]::Matches($traceText, 'Ime\(Commit').Count
    $focusedEvents = [regex]::Matches($traceText, 'Focused\(true\)').Count
    $saveErrors = [regex]::Matches($traceText, 'не сохранён|NeedsReview|SaveError|write refused|HOLD').Count
    # Exact note/title/error contents and raw winit event traces stay private.
    Write-Host "AKULA_WINDOWS_GUI_DIAGNOSTIC note_title=$noteWindowTitle keyboard_events=$keyboardEvents ime_commits=$imeCommits focus_events=$focusedEvents save_error_hints=$saveErrors trace_bytes=$($traceText.Length) saved_before_close=$published saved_after_close=$postClosePublished"
    if (!$published) {
      throw 'OS GUI input did not autosave within 30 seconds; see bounded diagnostic counts (no release approval)'
    }
    if (!$afterClose.Contains('# Akula Beta') -or !$afterClose.Contains('Existing local document')) {
      throw 'GUI edit removed original note content'
    }
  }
}
# Test the real Windows UI Automation adapter in a separate STA client before
# the regular GUI edit/restart/uninstall checks. Never run on a workstation.
$uiaScript = Join-Path $PSScriptRoot 'windows-accesskit-uia.ps1'
$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
& $windowsPowerShell -NoProfile -NonInteractive -Sta -ExecutionPolicy Bypass -File $uiaScript -ExePath $exe -VaultPath $vault -DataDir $akulaData
if ($LASTEXITCODE -ne 0) {
  throw "Installed Akula Windows UIA acceptance failed: PowerShell exit $LASTEXITCODE"
}

# Installed real Windows PDF pointer path; separate from native Rust harness.
$canvasScript = Join-Path $PSScriptRoot 'windows-canvas-installed-uia.ps1'
& $windowsPowerShell -NoProfile -NonInteractive -Sta -ExecutionPolicy Bypass -File $canvasScript -ExePath $exe -VaultPath $vault -DataDir $akulaData -Editable -IncludeDelete -IncludeResize -IncludeConnect
if ($LASTEXITCODE -ne 0) { throw "Installed Akula Canvas UIA smoke failed: PowerShell exit $LASTEXITCODE" }

$pdfPointerScript = Join-Path $PSScriptRoot 'windows-pdf-os-pointer.ps1'
& $windowsPowerShell -NoProfile -NonInteractive -Sta -ExecutionPolicy Bypass -File $pdfPointerScript -ExePath $exe -VaultPath $vault -DataDir $akulaData -FixturePdf $sourcePdf
if ($LASTEXITCODE -ne 0) {
  throw "Installed Akula PDF OS-pointer acceptance failed: PowerShell exit $LASTEXITCODE"
}

& $windowsPowerShell -NoProfile -NonInteractive -Sta -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'windows-dot-folder-uia.ps1') -ExePath $exe -VaultPath $vault -DataDir $akulaData; if ($LASTEXITCODE -ne 0) { throw "Installed Akula dot-folder UIA failed: PowerShell exit $LASTEXITCODE" }; Start-And-Close -EditNote
$state = Join-Path $akulaData 'window-state.json'
if (!(Test-Path $state)) { throw 'Akula window state not persisted to its own AppData' }
$savedText = [System.IO.File]::ReadAllText($note)
if (!$savedText.Contains($syntheticEdit) -or !$savedText.Contains('# Akula Beta') -or !$savedText.Contains('Existing local document')) {
  throw 'Installed GUI edit was not preserved after graceful shutdown'
}
if ($savedText.IndexOf($syntheticEdit) -ne $savedText.LastIndexOf($syntheticEdit)) {
  throw 'Expected exactly one GUI-injected edit marker'
}
$savedHash = (Get-FileHash -LiteralPath $note).Hash
if ($savedHash -eq $original) { throw 'GUI input had no observable file effect' }
# The second launch reads the *installed GUI editor model* back via OS
# Ctrl+A/Ctrl+C, not just the file on disk. Only synthetic CI clipboard bytes.
$clipboardSentinel = 'AKULA_CLIPBOARD_POISON_' + [Guid]::NewGuid().ToString('N')
Set-Clipboard -Value $clipboardSentinel -ErrorAction Stop
Start-And-Close -VerifyRestoredContent
if ((Get-FileHash -LiteralPath $note).Hash -ne $savedHash) { throw 'Restart changed the previously edited note' }
Assert-LegacyUntouched

# Installed NSIS + foreground OS keyboard regression for the 30k and 100k
# whole-note replacement path. Test-only vault, never developer/user files.
. (Join-Path $PSScriptRoot 'windows-beta-replace-all.ps1')
foreach ($lines in @(30000,100000)) {
  Test-AkulaBetaReplaceAll -Executable $exe -Vault $vault -DataDir $akulaData -LineCount $lines
}
if ((Get-FileHash -LiteralPath $note).Hash -ne $savedHash) {
  throw 'Large-document acceptance changed the original smoke-test note'
}
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
