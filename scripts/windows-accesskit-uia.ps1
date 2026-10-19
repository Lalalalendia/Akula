# Real Windows UI Automation smoke for the INSTALLED Akula editor.
# STA Windows PowerShell 5.1, only on a clean GitHub-hosted runner.
# This proves Windows UIA role/name/bounds/Invoke-to-clipboard, NOT Narrator usability.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'

if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'AccessKit UIA smoke is restricted to GitHub-hosted Windows runners'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'AccessKit UIA smoke requires STA Windows PowerShell 5.1'
}
$installedExe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([System.IO.Path]::GetFullPath($ExePath) -ne [System.IO.Path]::GetFullPath($installedExe) -or
    !(Test-Path -LiteralPath $installedExe)) {
    throw 'Refusing to test a non-installed or unexpected executable'
}

Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes, System.Windows.Forms -ErrorAction Stop
# Use the same vetted, HWND/PID/foreground-bound x64 SendInput helper as the
# existing installed Windows beta GUI smoke, in this separate STA process.
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

$labels = @()
foreach ($lang in @('en', 'ru')) {
    $locale = Join-Path $env:GITHUB_WORKSPACE "aquilum-app\native\assets\locales\$lang.json"
    $config = [System.IO.File]::ReadAllText($locale, [System.Text.Encoding]::UTF8) | ConvertFrom-Json
    $label = [string]$config.settings.shortcuts.copyCodeBlock
    if ([string]::IsNullOrWhiteSpace($label)) { throw "Missing $lang accessible Copy label" }
    $labels += $label
}
if ($labels.Count -ne 2 -or $labels[0] -eq $labels[1]) {
    throw 'Localized accessible Copy labels are missing or duplicated'
}

$note = Join-Path $VaultPath 'AccessKit UIA.md'
if (Test-Path -LiteralPath $note) { throw 'Not a clean UIA fixture' }
$markdown = @'
# AccessKit UIA

~~~rust
let first = 111;
~~~

Ordinary prose (no Copy button).

$$
x^2 + y^2
$$

~~~python
print('SECOND_UIA')
~~~
'@
[System.IO.File]::WriteAllText($note, $markdown, ([System.Text.UTF8Encoding]::new($false)))
$initialHash = (Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash

function Find-CodeCopyButtons {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string[]]$LocalizedNames
    )
    $condition = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button
    )
    $all = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
    $found = @()
    for ($n = 0; $n -lt $all.Count; $n++) {
        $item = $all.Item($n)
        if ($LocalizedNames -contains [string]$item.Current.Name) {
            $found += $item
        }
    }
    return $found
}

$proc = $null
try {
    $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $note
    $proc = Start-Process -FilePath $installedExe -ArgumentList $arguments -PassThru
    $ready = $false
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        Start-Sleep -Milliseconds 250
        $proc.Refresh()
        if ($proc.HasExited) { throw "Installed Akula exited before UIA activation: $($proc.ExitCode)" }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like '*AccessKit UIA*' -and
            $proc.MainWindowTitle -like '*Akula*') {
            $ready = $true
            break
        }
    }
    if (!$ready) { throw 'Installed Akula did not expose the expected fixture window' }

    # FromHandle sends actual OS UIA requests to the AccessKit Windows adapter.
    # The app may deliver its first tree asynchronously after EnableAccessTree.
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
    if ($null -eq $root) { throw 'UI Automation did not expose the installed Akula window' }

    $buttons = @()
    $lastCount = -1
    for ($attempt = 0; $attempt -lt 80; $attempt++) {
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Akula exited during Windows UI Automation enumeration' }
        try {
            $buttons = @(Find-CodeCopyButtons -Root $root -LocalizedNames $labels)
            $lastCount = $buttons.Count
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            $buttons = @()
        }
        if ($buttons.Count -eq 2) { break }
        Start-Sleep -Milliseconds 250
    }
    if ($buttons.Count -ne 2) {
        throw "Expected exactly two semantic UIA Copy buttons; observed $lastCount"
    }
    $buttons = @($buttons | Sort-Object { $_.Current.BoundingRectangle.Top })
    $windowBounds = $root.Current.BoundingRectangle
    if ($windowBounds.IsEmpty -or $windowBounds.Width -le 0 -or $windowBounds.Height -le 0) {
        throw 'UI Automation window bounds are empty'
    }
    foreach ($button in $buttons) {
        $current = $button.Current
        $rect = $current.BoundingRectangle
        if ($current.ControlType -ne [System.Windows.Automation.ControlType]::Button -or
            !$current.IsEnabled -or !$current.IsKeyboardFocusable -or $current.IsOffscreen -or
            $rect.IsEmpty -or $rect.Width -le 0 -or $rect.Height -le 0 -or
            $rect.Right -le $windowBounds.Left -or $rect.Left -ge $windowBounds.Right -or
            $rect.Bottom -le $windowBounds.Top -or $rect.Top -ge $windowBounds.Bottom) {
            throw 'UIA Copy button is not a visible, enabled, focusable button inside the app window'
        }
    }
    if ($buttons[0].Current.BoundingRectangle.Top -ge $buttons[1].Current.BoundingRectangle.Top) {
        throw 'UIA Copy button order is not the painted document order'
    }

    $expectedBodies = @('let first = 111;', "print('SECOND_UIA')")
    for ($i = 0; $i -lt $buttons.Count; $i++) {
        # A unique clipboard sentinel proves each OS UIA Invoke produced a fresh copy.
        [System.Windows.Forms.Clipboard]::SetText("AKULA_UIA_BEFORE_$i")
        $pattern = $buttons[$i].GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern)
        if ($null -eq $pattern) { throw "UIA Copy button $i has no InvokePattern" }
        ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
        $copied = $false
        $observedLength = -1
        for ($poll = 0; $poll -lt 60; $poll++) {
            Start-Sleep -Milliseconds 100
            try {
                $actual = [System.Windows.Forms.Clipboard]::GetText()
                $observedLength = $actual.Length
                if ($actual -ceq $expectedBodies[$i]) {
                    $copied = $true
                    break
                }
            } catch [System.Runtime.InteropServices.ExternalException] {
                # Transient clipboard locks are retried, not accepted as success.
            }
        }
        if (!$copied) {
            throw "UIA Invoke $i did not copy the exact fenced body (clipboard length $observedLength)"
        }
        if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $initialHash) {
            throw 'UIA Invoke modified Markdown source bytes'
        }
    }
    Write-Host 'AKULA_WINDOWS_ACCESSKIT_UIA_PASS buttons=2 patterns=2 exact_clipboard=2 source_unchanged=True'
    # Real OS keyboard path, distinct from UIA InvokePattern:
    # UIA Focus -> queued AccessKit Action::Focus -> Windows SendInput
    # Enter, Tab, Space, and Shift+Tab, against the installed foreground app.
    $hwnd = [IntPtr]$proc.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    $foreground = $false
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 150
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) {
            $foreground = $true
            break
        }
    }
    if (!$foreground) { throw 'Cannot focus verified Akula window for real keyboard events' }

    function Wait-ButtonFocus {
        param(
            [System.Windows.Automation.AutomationElement]$Target,
            [string]$Phase
        )
        for ($poll = 0; $poll -lt 60; $poll++) {
            Start-Sleep -Milliseconds 100
            if ($Target.Current.HasKeyboardFocus) { return }
        }
        throw "Actual Windows UIA focus did not reach $Phase"
    }

    function Wait-KeyboardClipboard {
        param([string]$Expected, [string]$Phase)
        for ($poll = 0; $poll -lt 60; $poll++) {
            Start-Sleep -Milliseconds 100
            try {
                if ([System.Windows.Forms.Clipboard]::GetText() -ceq $Expected) { return }
            } catch [System.Runtime.InteropServices.ExternalException] {
                # Clipboard is temporarily locked. The test does not accept it as a pass.
            }
        }
        throw "Actual Windows keyboard $Phase did not copy the expected code body"
    }

    $buttons[0].SetFocus()
    Wait-ButtonFocus -Target $buttons[0] -Phase 'first Copy button'
    [System.Windows.Forms.Clipboard]::SetText('AKULA_WINDOWS_KEYBOARD_ENTER_PENDING')
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [ushort]0x0d, $false)
    Wait-KeyboardClipboard -Expected $expectedBodies[0] -Phase 'Enter'

    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [ushort]0x09, $false)
    Wait-ButtonFocus -Target $buttons[1] -Phase 'second Copy button after Tab'
    [System.Windows.Forms.Clipboard]::SetText('AKULA_WINDOWS_KEYBOARD_SPACE_PENDING')
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [ushort]0x20, $false)
    Wait-KeyboardClipboard -Expected $expectedBodies[1] -Phase 'Space'

    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [ushort]0x09, $true)
    Wait-ButtonFocus -Target $buttons[0] -Phase 'first Copy button after Shift+Tab'
    if ((Get-FileHash -LiteralPath $note -Algorithm SHA256).Hash -ne $initialHash) {
        throw 'Real Windows keyboard navigation/activation edited the Markdown source'
    }
    Write-Host 'AKULA_WINDOWS_ACCESSKIT_KEYS_PASS enter=1 space=1 tab_forward=1 shift_tab_reverse=1 source_unchanged=True'

} finally {
    if ($null -ne $proc) {
        $proc.Refresh()
        if (!$proc.HasExited) {
            if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(10000)) {
                $proc.Kill()
                $proc.WaitForExit()
            }
        }
        $proc.Dispose()
    }
}
