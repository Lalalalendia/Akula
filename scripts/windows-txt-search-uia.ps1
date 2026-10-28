# DESK-10: search for a preexisting UTF-8 TXT in an installed Akula using
# real Win32 Ctrl+O / ASCII keyboard input, UI Automation and Ctrl+A/C.
# Only for disposable GitHub-hosted Windows beta runners; no user vaults.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Installed TXT search acceptance is restricted to GitHub-hosted Windows'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'Installed TXT UIA test needs STA Windows PowerShell 5.1'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([IO.Path]::GetFullPath($ExePath) -ne [IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed -PathType Leaf)) {
    throw 'Refusing an executable other than installed Akula'
}
$runnerRoot = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
if (![IO.Path]::GetFullPath($VaultPath).StartsWith(
    $runnerRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Refusing TXT fixture outside the disposable runner directory'
}
$baseline = Join-Path $VaultPath 'First note.md'
if (!(Test-Path -LiteralPath $baseline -PathType Leaf)) {
    throw 'Installed TXT search needs the existing synthetic beta note'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

# CJK codepoints avoid Windows PowerShell 5.1 non-BOM script decoding issues.
$stem = -join @([char]0x6771, [char]0x4eac, [char]0x65e5, [char]0x8a18)
$txt = Join-Path $VaultPath ($stem + '.TXT')
if (Test-Path -LiteralPath $txt) { throw 'Refusing to replace existing TXT fixture' }
$needle = 'txtuiasearch' + [guid]::NewGuid().ToString('N')
$body = ('CJK-title={0}{1}text={2}{1}original=unchanged' -f
    $stem, [Environment]::NewLine, $needle)
$baselineHash = (Get-FileHash -LiteralPath $baseline -Algorithm SHA256).Hash
$process = $null

function Focus-InstalledAkula {
    param([System.Diagnostics.Process]$Process)
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero -or
        $Process.MainWindowTitle -notlike '*Akula*') {
        throw 'Cannot focus an absent or unexpected installed Akula window'
    }
    $hwnd = [IntPtr]$Process.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    for ($i = 0; $i -lt 15; $i++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 200
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) { return $hwnd }
    }
    throw 'Cannot dispatch OS keys without foreground Akula'
}

function Focus-TxtBodyEditor {
    param(
        [System.Diagnostics.Process]$Process,
        [string]$ExpectedNeedle
    )
    # Ctrl+O can leave the keyboard on the dismissed search field even after
    # the window title changes. Resolve the installed body using native UIA.
    $hwnd = Focus-InstalledAkula -Process $Process
    $bounds = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($null -eq $bounds) { throw 'Installed TXT has no UIA root after search' }
    $condition = [System.Windows.Automation.OrCondition]::new(
        [System.Windows.Automation.Condition[]]@(
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                [System.Windows.Automation.ControlType]::Edit),
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
                [System.Windows.Automation.ControlType]::Document)))
    $selected = $null
    $observed = 0
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        Start-Sleep -Milliseconds 100
        try {
            $items = $bounds.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
            $matching = @()
            $candidates = @()
            $win = $bounds.Current.BoundingRectangle
            for ($i = 0; $i -lt $items.Count; $i++) {
                $item = $items.Item($i)
                $rect = $item.Current.BoundingRectangle
                if (!$item.Current.IsEnabled -or !$item.Current.IsKeyboardFocusable -or
                    $item.Current.IsOffscreen -or $rect.IsEmpty -or
                    $rect.Width -lt 160 -or $rect.Height -lt 10 -or
                    $rect.Left -lt $win.Left -or $rect.Right -gt $win.Right -or
                    $rect.Top -lt $win.Top -or $rect.Bottom -gt $win.Bottom) { continue }
                $candidates += $item
                $pattern = $null
                if ($item.TryGetCurrentPattern(
                    [System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)) {
                    $value = ([System.Windows.Automation.ValuePattern]$pattern).Current.Value
                    if ($null -ne $value -and $value.Contains($ExpectedNeedle)) {
                        $matching += $item
                    }
                }
            }
            if ($matching.Count -gt 1) {
                throw 'Multiple installed TXT editors match the unique synthetic body marker'
            }
            if ($matching.Count -eq 1) {
                $selected = $matching[0]
            } elseif ($candidates.Count -ge 2) {
                # Only as a fallback if the platform does not expose UIA Value:
                # final clipboard comparison still requires the exact TXT body.
                $ordered = @($candidates | Sort-Object { $_.Current.BoundingRectangle.Top })
                $selected = $ordered[$ordered.Count - 1]
            }
            $observed = $candidates.Count
            if ($null -ne $selected) { break }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            $selected = $null
        }
    }
    if ($null -eq $selected) {
        throw "Installed TXT body not identified by Windows UIA: visible_editors=$observed"
    }
    $selected.SetFocus()
    for ($i = 0; $i -lt 25; $i++) {
        Start-Sleep -Milliseconds 100
        try {
            if ($selected.Current.HasKeyboardFocus) {
                if ([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd) {
                    throw 'TXT UIA focus moved outside installed Akula'
                }
                Write-Host 'AKULA_WINDOWS_TXT_EDITOR_FOCUS_PASS uia=1 keyboard_focus=1'
                return
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            throw 'TXT body UIA element disappeared after attempting to focus'
        }
    }
    throw 'Installed TXT body UIA element did not gain keyboard focus'
}

function Normalize-GuiText {
    param([string]$Value)
    $cr = [string][char]13
    $lf = [string][char]10
    return $Value.Replace($cr + $lf, $lf).Replace($cr, $lf).TrimEnd([char]10)
}

try {
    [IO.File]::WriteAllText($txt, $body, ([System.Text.UTF8Encoding]::new($false)))
    $originalHash = (Get-FileHash -LiteralPath $txt -Algorithm SHA256).Hash
    $args = ('--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f
        $VaultPath, $DataDir, $baseline)
    $process = Start-Process -FilePath $installed -ArgumentList $args -PassThru
    $ready = $false
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) { throw 'Akula exited before TXT search started' }
        if ($process.MainWindowHandle -ne [IntPtr]::Zero -and
            $process.MainWindowTitle -like '*First note*' -and
            $process.MainWindowTitle -like '*Akula*') {
            $ready = $true
            break
        }
    }
    if (!$ready) { throw 'Installed Akula did not open the synthetic baseline note' }
    $hwnd = Focus-InstalledAkula -Process $process
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($null -eq $root) { throw 'Installed Akula has no Windows UI Automation root' }

    # A user shortcut, not the native test harness open_search() method.
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $process.Id, [uint16]0x4F)
    Start-Sleep -Milliseconds 350
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd) {
        throw 'Search popup changed foreground process'
    }
    [AkulaBetaGuiInput]::TypeAsciiKeys($hwnd, $process.Id, $needle)

    # Search ResultCard is an AccessKit ListBoxOption, distinct from the
    # always-visible file-tree TreeItem with the same CJK filename.
    $byName = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty, $stem)
    $found = $false
    for ($i = 0; $i -lt 180; $i++) {
        Start-Sleep -Milliseconds 250
        $process.Refresh()
        if ($process.HasExited) { throw 'Akula exited during TXT indexing/search' }
        try {
            $hits = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $byName)
            for ($j = 0; $j -lt $hits.Count; $j++) {
                $hit = $hits.Item($j)
                $kind = $hit.Current.ControlType
                if (!$hit.Current.IsOffscreen -and
                    ($kind -eq [System.Windows.Automation.ControlType]::ListItem -or
                     $kind -eq [System.Windows.Automation.ControlType]::DataItem)) {
                    $found = $true
                    break
                }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            # A replaced AccessKit tree is transient; retry its current nodes.
        }
        if ($found) { break }
    }
    if (!$found) { throw 'Ctrl+O did not expose the CJK TXT as a visible search result' }
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd) {
        throw 'Refusing Enter when Akula is not foreground'
    }
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $process.Id, [uint16]0x0D, $false)
    $opened = $false
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 200
        $process.Refresh()
        if ($process.HasExited) { throw 'Akula exited while opening the TXT search result' }
        if ($process.MainWindowTitle.Contains($stem) -and
            $process.MainWindowTitle -like '*Akula*') {
            $opened = $true
            break
        }
    }
    if (!$opened) { throw 'Enter did not open the exact CJK TXT search result' }

    # Explicitly focus the verified installed TXT body before Ctrl+A/C.
    # A changed title alone does not prove editor focus after closing Ctrl+O.
    Focus-TxtBodyEditor -Process $process -ExpectedNeedle $needle
    $hwnd = Focus-InstalledAkula -Process $process
    $sentinel = 'TXT_UIA_CLIPBOARD_UNCHANGED_' + [guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $sentinel -ErrorAction Stop
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $process.Id, [uint16]0x41)
    Start-Sleep -Milliseconds 150
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $process.Id, [uint16]0x43)
    $copied = $null
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 150
        try {
            $candidate = Get-Clipboard -Raw -ErrorAction Stop
            if ($candidate -ne $sentinel) { $copied = $candidate; break }
        } catch {
            # A temporary Windows clipboard lock never counts as success.
        }
    }
    if ($null -eq $copied -or (Normalize-GuiText $copied) -cne (Normalize-GuiText $body)) {
        # Only synthetic fixture metadata; never print clipboard contents.
        $observedLength = if ($null -eq $copied) { -1 } else { (Normalize-GuiText $copied).Length }
        $markerPresent = ($null -ne $copied -and $copied.Contains($needle))
        throw "Installed TXT editor clipboard differs from fixture: expected_length=$((Normalize-GuiText $body).Length) observed_length=$observedLength unique_marker_present=$markerPresent"
    }
    if ((Get-FileHash -LiteralPath $txt -Algorithm SHA256).Hash -ne $originalHash -or
        (Get-FileHash -LiteralPath $baseline -Algorithm SHA256).Hash -ne $baselineHash) {
        throw 'Global TXT search/open modified a source document'
    }
    Write-Host 'AKULA_WINDOWS_TXT_SEARCH_UIA_PASS installed=1 ctrl_o=1 cjk_title=1 body_query=1 result_uia=1 enter=1 gui_readback=1 source_sha256_unchanged=1'
} finally {
    if ($null -ne $process) {
        $process.Refresh()
        if (!$process.HasExited) {
            if (!$process.CloseMainWindow() -or !$process.WaitForExit(10000)) {
                $process.Kill()
                $process.WaitForExit()
            }
        }
        $process.Dispose()
    }
    if (Test-Path -LiteralPath $txt) {
        Remove-Item -LiteralPath $txt -Force -ErrorAction Stop
    }
}
