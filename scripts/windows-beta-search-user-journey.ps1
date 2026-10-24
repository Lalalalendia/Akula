# Real installed Akula search journey on a disposable GitHub-hosted Windows runner.
# Native Ctrl+O, physical-key typed query, UIA result, Enter and OS Ctrl+A/C
# prove the result is opened in the installed editor. No user files are read.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir,
    [Parameter(Mandatory = $true)][string]$OriginalNote,
    [Parameter(Mandatory = $true)][string]$TargetNote,
    [Parameter(Mandatory = $true)][string]$Marker
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Installed search QA requires an ephemeral GitHub-hosted Windows runner'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'Installed search QA requires STA Windows PowerShell 5.1'
}
$actualExe = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([System.IO.Path]::GetFullPath($ExePath) -ne [System.IO.Path]::GetFullPath($actualExe)) {
    throw 'Refusing to drive an executable other than installed Akula'
}
if ([System.IO.Path]::GetFullPath($OriginalNote) -ne
      [System.IO.Path]::GetFullPath((Join-Path $VaultPath 'First note.md')) -or
    [System.IO.Path]::GetFullPath($TargetNote) -ne
      [System.IO.Path]::GetFullPath((Join-Path $VaultPath 'Search target.md'))) {
    throw 'Refusing search QA on anything other than its fixed synthetic fixtures'
}
if ($Marker -cnotmatch '^AKULA_GUI_SEARCH_[0-9a-f]{32}$') {
    throw 'Search QA marker is not a disposable, ASCII-only synthetic token'
}
$beforeFirst = (Get-FileHash -LiteralPath $OriginalNote -Algorithm SHA256).Hash
$beforeTarget = (Get-FileHash -LiteralPath $TargetNote -Algorithm SHA256).Hash
$expectedTarget = [System.IO.File]::ReadAllText($TargetNote, [System.Text.Encoding]::UTF8)
if (!$expectedTarget.Contains($Marker) -or !$expectedTarget.Contains('# Search target')) {
    throw 'Search target fixture is not the expected synthetic note'
}
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')

function Focus-VerifiedWindow {
    param([System.Diagnostics.Process]$Process)
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero) {
        throw 'Installed Akula search target has no running window'
    }
    $hwnd = [IntPtr]$Process.MainWindowHandle
    [AkulaBetaGuiInput]::ShowWindow($hwnd, 9) | Out-Null
    for ($attempt = 0; $attempt -lt 15; $attempt++) {
        [AkulaBetaGuiInput]::SetForegroundWindow($hwnd) | Out-Null
        Start-Sleep -Milliseconds 200
        if ([AkulaBetaGuiInput]::GetForegroundWindow() -eq $hwnd) {
            return $hwnd
        }
    }
    throw 'Installed Akula could not be focused for the OS-only search path'
}

$proc = $null
try {
    $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $OriginalNote
    $proc = Start-Process -FilePath $ExePath -ArgumentList $arguments -PassThru
    $ready = $false
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 250
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Installed Akula exited before synthetic search' }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like 'First note*' -and
            $proc.MainWindowTitle -like '*Akula*') {
            $ready = $true
            break
        }
    }
    if (!$ready) { throw 'Search QA did not start from the expected original note' }
    $hwnd = Focus-VerifiedWindow -Process $proc
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $proc.Id, [System.UInt16]0x4f) # actual Ctrl+O
    Start-Sleep -Milliseconds 350
    Send-AkulaBetaText -Process $proc -Text $Marker

    # A genuine search result is a selected, visible UIA ListItem (AccessKit
    # Role::ListBoxOption), not merely a matching file in the vault/sidebar.
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($hwnd)
    if ($null -eq $root) { throw 'Search UI did not expose the installed AccessKit tree' }
    $byName = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        'Search target'
    )
    $listKind = [System.Windows.Automation.ControlType]::ListItem
    $found = $false
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Akula exited while indexing search fixture' }
        try {
            $matches = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $byName)
            for ($i = 0; $i -lt $matches.Count; $i++) {
                $item = $matches.Item($i)
                if ($item.Current.ControlType -eq $listKind -and !$item.Current.IsOffscreen) {
                    $found = $true
                    break
                }
            }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            # The result list can recompose during an index update.
        }
        if ($found) { break }
        Start-Sleep -Milliseconds 200
    }
    if (!$found) {
        throw 'Physical Ctrl+O / typed query did not create a visible semantic search result'
    }
    if ([AkulaBetaGuiInput]::GetForegroundWindow() -ne $hwnd) {
        throw 'Search result activation target lost foreground identity'
    }
    [AkulaBetaGuiInput]::SendNavigationKey($hwnd, $proc.Id, [System.UInt16]0x0d, $false) # Enter
    $opened = $false
    for ($i = 0; $i -lt 70; $i++) {
        Start-Sleep -Milliseconds 200
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Akula exited when opening search result' }
        if ($proc.MainWindowTitle -like 'Search target*' -and
            $proc.MainWindowTitle -like '*Akula*') {
            $opened = $true
            break
        }
    }
    if (!$opened) { throw 'Search Enter did not open the intended synthetic note' }

    $sentinel = 'AKULA_SEARCH_CLIPBOARD_POISON_' + [Guid]::NewGuid().ToString('N')
    Set-Clipboard -Value $sentinel -ErrorAction Stop
    $hwnd = Focus-VerifiedWindow -Process $proc
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $proc.Id, [System.UInt16]0x41) # Ctrl+A
    Start-Sleep -Milliseconds 300
    [AkulaBetaGuiInput]::PressCtrlShortcut($hwnd, $proc.Id, [System.UInt16]0x43) # Ctrl+C
    $copied = $null
    for ($attempt = 0; $attempt -lt 35; $attempt++) {
        Start-Sleep -Milliseconds 200
        try { $candidate = Get-Clipboard -Raw -ErrorAction Stop } catch { $candidate = $null }
        if (![string]::IsNullOrEmpty($candidate) -and $candidate -ne $sentinel) {
            $copied = $candidate
            break
        }
    }
    if ($null -eq $copied) { throw 'Opened search result did not copy its editor text' }
    $normalize = {
        param([string]$Value)
        $cr = [string][char]13
        $lf = [string][char]10
        $Value.Replace(($cr + $lf), $lf).Replace($cr, $lf).TrimEnd([char]10)
    }
    $seen = & $normalize $copied
    $expected = & $normalize $expectedTarget
    if (![string]::Equals($seen, $expected, [StringComparison]::Ordinal) -or
        !$seen.Contains($Marker)) {
        throw 'Native editor readback does not match the searched and opened Markdown note'
    }
    if ((Get-FileHash -LiteralPath $OriginalNote -Algorithm SHA256).Hash -ne $beforeFirst -or
        (Get-FileHash -LiteralPath $TargetNote -Algorithm SHA256).Hash -ne $beforeTarget) {
        throw 'Searching or opening modified one of the original Markdown files'
    }
    if (!$proc.CloseMainWindow() -or !$proc.WaitForExit(20000)) {
        throw 'Installed Akula would not close after the search journey'
    }
    Write-Host 'AKULA_INSTALLED_SEARCH_JOURNEY_PASS=ctrl_o,physical_query,uia_result,enter,editor_clipboard;source_unchanged=true'
} finally {
    if ($null -ne $proc) {
        $proc.Refresh()
        if (!$proc.HasExited) {
            # The runner is disposable; do not leave Akula running on failure.
            Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        }
    }
}
