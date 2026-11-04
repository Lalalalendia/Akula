# QA-only installed Akula filename acceptance after native rename safety (#913).
# Run ONLY on disposable GitHub-hosted Windows PowerShell STA; never on a user vault.
param(
  [Parameter(Mandatory = $true)][string]$ExePath,
  [Parameter(Mandatory = $true)][string]$VaultPath,
  [Parameter(Mandatory = $true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:RUNNER_OS -cne 'Windows' -or
    $PSVersionTable.PSEdition -cne 'Desktop' -or
    [Threading.Thread]::CurrentThread.GetApartmentState() -ne [Threading.ApartmentState]::STA -or
    [IntPtr]::Size -ne 8) {
  throw 'AKULA_RENAME_UIA_HOLD: disposable GitHub-hosted Windows x64 STA only'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
$dataExpected = Join-Path $env:APPDATA 'io.github.lalalendia.akula'
$runner = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
$vault = [IO.Path]::GetFullPath($VaultPath)
if ([IO.Path]::GetFullPath($ExePath) -cne [IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed -PathType Leaf) -or
    [IO.Path]::GetFullPath($DataDir) -cne [IO.Path]::GetFullPath($dataExpected) -or
    !$vault.StartsWith($runner, [StringComparison]::OrdinalIgnoreCase) -or
    !(Test-Path -LiteralPath $vault -PathType Container) -or
    ((Get-Item -LiteralPath $vault).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
  throw 'AKULA_RENAME_UIA_HOLD: installed executable and disposable vault identity required'
}
if (@(Get-Process -Name 'akula-app' -ErrorAction SilentlyContinue).Count -ne 0) {
  throw 'AKULA_RENAME_UIA_HOLD: existing Akula process must not be disturbed'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

function Focus-Akula([Diagnostics.Process]$process) {
  $process.Refresh()
  if ($process.HasExited -or $process.MainWindowHandle -eq [IntPtr]::Zero -or
      $process.MainWindowTitle -notlike '*Akula*') {
    throw 'Installed rename fixture has no active Akula window'
  }
  $hwnd = [IntPtr]$process.MainWindowHandle
  [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
  for ($attempt = 0; $attempt -lt 25; $attempt++) {
    [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
    Start-Sleep -Milliseconds 120
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return $hwnd }
  }
  throw 'Installed Akula could not be focused for OS keyboard input'
}

function Wait-Window([Diagnostics.Process]$process, [string]$stem) {
  for ($attempt = 0; $attempt -lt 120; $attempt++) {
    Start-Sleep -Milliseconds 250
    $process.Refresh()
    if ($process.HasExited) { throw 'Installed Akula terminated before rename acceptance' }
    if ($process.MainWindowHandle -ne [IntPtr]::Zero -and
        $process.MainWindowTitle.StartsWith($stem, [StringComparison]::Ordinal) -and
        $process.MainWindowTitle -like '*Akula*') { return }
  }
  throw "Installed Akula did not show the expected note: $stem"
}

function Visible-Editors([IntPtr]$hwnd) {
  $root = [Windows.Automation.AutomationElement]::FromHandle($hwnd)
  if ($null -eq $root) { throw 'No installed Akula UI Automation root' }
  $condition = [Windows.Automation.OrCondition]::new([Windows.Automation.Condition[]]@(
    [Windows.Automation.PropertyCondition]::new(
      [Windows.Automation.AutomationElement]::ControlTypeProperty,
      [Windows.Automation.ControlType]::Edit),
    [Windows.Automation.PropertyCondition]::new(
      [Windows.Automation.AutomationElement]::ControlTypeProperty,
      [Windows.Automation.ControlType]::Document)))
  $nodes = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
  $bounds = $root.Current.BoundingRectangle
  $editors = @()
  for ($j = 0; $j -lt $nodes.Count; $j++) {
    $item = $nodes.Item($j)
    $rect = $item.Current.BoundingRectangle
    if ($item.Current.IsEnabled -and $item.Current.IsKeyboardFocusable -and
        !$item.Current.IsOffscreen -and !$rect.IsEmpty -and $rect.Width -gt 80 -and
        $rect.Height -gt 10 -and $rect.Left -ge $bounds.Left -and
        $rect.Right -le $bounds.Right -and $rect.Top -ge $bounds.Top -and
        $rect.Bottom -le $bounds.Bottom) {
      $editors += $item
    }
  }
  return $editors
}

function Focus-Title([Diagnostics.Process]$process, [string]$stem) {
  $hwnd = Focus-Akula $process
  for ($attempt = 0; $attempt -lt 40; $attempt++) {
    try {
      $matching = @()
      foreach ($item in @(Visible-Editors $hwnd)) {
        $pattern = $null
        if ($item.TryGetCurrentPattern([Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) {
          $value = ([Windows.Automation.ValuePattern]$pattern).Current.Value
          if ($value -ceq $stem) { $matching += $item }
        }
      }
      if ($matching.Count -gt 1) { throw 'Multiple installed title editors have the same exact value' }
      if ($matching.Count -eq 1) {
        $matching[0].SetFocus()
        Start-Sleep -Milliseconds 160
        if ($matching[0].Current.HasKeyboardFocus -and
            [AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) {
          return $hwnd
        }
      }
    } catch [Windows.Automation.ElementNotAvailableException] {
      # AccessKit can replace the tree while focus or layout changes.
    }
    Start-Sleep -Milliseconds 200
  }
  throw "Cannot identify and focus the real installed title editor for $stem"
}

function Focus-Body([Diagnostics.Process]$process) {
  $hwnd = Focus-Akula $process
  for ($attempt = 0; $attempt -lt 35; $attempt++) {
    try {
      $editors = @(Visible-Editors $hwnd | Sort-Object { $_.Current.BoundingRectangle.Top })
      if ($editors.Count -ge 2) {
        $body = $editors[-1]
        $body.SetFocus()
        Start-Sleep -Milliseconds 150
        if ($body.Current.HasKeyboardFocus -and
            [AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return $hwnd }
      }
    } catch [Windows.Automation.ElementNotAvailableException] {}
    Start-Sleep -Milliseconds 200
  }
  throw 'Could not focus the installed note body to commit title blur'
}

function Wait-FileRename([Diagnostics.Process]$process, [string]$from, [string]$to) {
  for ($attempt = 0; $attempt -lt 100; $attempt++) {
    Start-Sleep -Milliseconds 200
    $process.Refresh()
    if ($process.HasExited) { throw 'Installed Akula exited during filename commit' }
    if (!(Test-Path -LiteralPath $from) -and
        (Test-Path -LiteralPath $to -PathType Leaf)) { return }
  }
  throw 'Installed title commit did not persist the renamed file on disk'
}

function Normalize-Text([string]$value) {
  $cr = [string][char]13
  $lf = [string][char]10
  return $value.Replace($cr + $lf, $lf).Replace($cr, $lf)
}

foreach ($mode in @('Enter', 'Blur')) {
  $originalStem = 'AkulaRename' + $mode
  $targetStem = 'AkulaRenamed' + $mode
  $from = Join-Path $vault ($originalStem + '.md')
  $to = Join-Path $vault ($targetStem + '.md')
  if ((Test-Path -LiteralPath $from) -or (Test-Path -LiteralPath $to)) {
    throw "AKULA_RENAME_UIA_HOLD: nonempty synthetic fixture for $mode"
  }
  $bodyText = 'AKULA_RENAME_BODY_' + $mode + '_' + [guid]::NewGuid().ToString('N')
  $source = '# ' + $originalStem + [char]10 + $bodyText + [char]10
  [IO.File]::WriteAllText($from, $source, [Text.UTF8Encoding]::new($false))
  $originalHash = (Get-FileHash -LiteralPath $from -Algorithm SHA256).Hash
  $process = $null
  try {
    $argsForApp = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault, $DataDir, $from
    $process = Start-Process -FilePath $installed -ArgumentList $argsForApp -PassThru
    Wait-Window $process $originalStem
    $hwnd = Focus-Title $process $originalStem
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $process.Id, [uint16]0x41)
    [AkulaBetaGuiInput]::TypeAsciiKeys($hwnd, $process.Id, $targetStem)
    if (!(Test-Path -LiteralPath $from -PathType Leaf) -or (Test-Path -LiteralPath $to)) {
      throw 'Filename was changed before an explicit Enter or blur commit'
    }

    if ($mode -ceq 'Enter') {
      [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $process.Id, [uint16]0x0d, $false)
    } else {
      $null = Focus-Body $process
    }
    Wait-FileRename $process $from $to
    Wait-Window $process $targetStem
    if ((Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash -cne $originalHash) {
      throw 'Renaming changed the exact synthetic Markdown bytes'
    }
    if (!$process.CloseMainWindow() -or !$process.WaitForExit(25000)) {
      throw 'Akula failed graceful close after installed rename'
    }
    $process.Dispose()
    $process = $null

    $after = (Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash
    $reopenArgs = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $vault, $DataDir, $to
    $process = Start-Process -FilePath $installed -ArgumentList $reopenArgs -PassThru
    Wait-Window $process $targetStem
    $hwnd = Focus-Body $process
    $sentinel = 'AKULA_RENAME_UIA_SENTINEL_' + [guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $sentinel -ErrorAction Stop
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $process.Id, [uint16]0x41)
    Start-Sleep -Milliseconds 200
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $process.Id, [uint16]0x43)
    $copied = $null
    for ($attempt = 0; $attempt -lt 55; $attempt++) {
      Start-Sleep -Milliseconds 150
      try {
        $candidate = Get-Clipboard -Raw -ErrorAction Stop
        if ($null -ne $candidate -and $candidate -cne $sentinel) {
          $copied = $candidate
          break
        }
      } catch {}
    }
    if ($null -eq $copied -or
        ![string]::Equals((Normalize-Text $copied), (Normalize-Text $source), [StringComparison]::Ordinal)) {
      throw 'Installed editor clipboard readback differs from the original exact body'
    }
    if (!$process.CloseMainWindow() -or !$process.WaitForExit(25000)) {
      throw 'Reopened Akula did not close normally'
    }
    $process.Dispose()
    $process = $null
    if ((Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash -cne $after -or
        (Test-Path -LiteralPath $from)) {
      throw 'Restart altered renamed Markdown or recreated old filename'
    }
    Write-Host "AKULA_RENAME_INSTALLED_UIA_PASS mode=$mode os_keys=1 file_gate=1 restart=1 gui_readback=1 sha256=1"
  } finally {
    if ($null -ne $process) {
      $process.Refresh()
      if (!$process.HasExited) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
      }
      $process.Dispose()
    }
  }
}
