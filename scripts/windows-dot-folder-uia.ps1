# Real installed Akula UIA verification, isolated to disposable GitHub-hosted Windows.
param(
    [Parameter(Mandatory = $true)][string]$ExePath,
    [Parameter(Mandatory = $true)][string]$VaultPath,
    [Parameter(Mandatory = $true)][string]$DataDir
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Dot-folder UIA acceptance is limited to hosted Windows CI'
}
if ($PSVersionTable.PSEdition -ne 'Desktop' -or
    [System.Threading.Thread]::CurrentThread.GetApartmentState() -ne [System.Threading.ApartmentState]::STA) {
    throw 'UIA requires STA Windows PowerShell 5.1'
}
$installed = Join-Path $env:LOCALAPPDATA 'Akula\akula-app.exe'
if ([System.IO.Path]::GetFullPath($ExePath) -ne [System.IO.Path]::GetFullPath($installed) -or
    !(Test-Path -LiteralPath $installed)) { throw 'Unexpected executable, not installed Akula' }
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes -ErrorAction Stop
$baseline = Join-Path $VaultPath 'First note.md'
if (!(Test-Path -LiteralPath $baseline)) { throw 'Missing existing Windows beta note' }
$userDir = Join-Path $VaultPath '.заметки'
$archiveDir = Join-Path $VaultPath '.архив'
$internalDir = Join-Path $VaultPath '.git'
$trashDir = Join-Path $VaultPath '.trash'
foreach ($dir in @($userDir, $archiveDir, $internalDir, $trashDir)) {
    if (Test-Path -LiteralPath $dir) { throw "Not a clean isolated dot-folder fixture: $dir" }
    New-Item -ItemType Directory -Path $dir -ErrorAction Stop | Out-Null
}
$idea = Join-Path $userDir 'Идея.md'
[System.IO.File]::WriteAllText($idea, ('# Идея' + [Environment]::NewLine + 'DOT_FOLDER_USER_CONTENT_627'))
[System.IO.File]::WriteAllText((Join-Path $archiveDir 'Старая.md'), '# User archive')
[System.IO.File]::WriteAllText((Join-Path $internalDir 'SECRET.md'), '# Must remain excluded')
[System.IO.File]::WriteAllText((Join-Path $trashDir 'DELETED.md'), '# Must remain excluded')
$hash = (Get-FileHash -LiteralPath $idea -Algorithm SHA256).Hash

function Find-TreeItem {
    param(
        [System.Windows.Automation.AutomationElement]$Root,
        [string]$Name
    )
    $type = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::TreeItem
    )
    $items = $Root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $type)
    for ($i = 0; $i -lt $items.Count; $i++) {
        $item = $items.Item($i)
        if ([string]$item.Current.Name -ceq $Name) { return $item }
    }
    return $null
}
function Activate-TreeItem {
    param([System.Windows.Automation.AutomationElement]$Item)
    $pattern = $null
    if ($Item.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$pattern)) {
        ([System.Windows.Automation.InvokePattern]$pattern).Invoke()
        return
    }
    if ($Item.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$pattern)) {
        ([System.Windows.Automation.ExpandCollapsePattern]$pattern).Expand()
        return
    }
    throw "No UIA activation available for TreeItem '$($Item.Current.Name)'"
}
$proc = $null
try {
    $arguments = '--vault "{0}" --data-dir "{1}" --open "{2}" --focus' -f $VaultPath, $DataDir, $baseline
    $proc = Start-Process -FilePath $installed -ArgumentList $arguments -PassThru
    $ready = $false
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 250
        $proc.Refresh()
        if ($proc.HasExited) { throw "Installed Akula exited: $($proc.ExitCode)" }
        if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and
            $proc.MainWindowTitle -like '*First note*' -and $proc.MainWindowTitle -like '*Akula*') {
            $ready = $true
            break
        }
    }
    if (!$ready) { throw 'Installed Akula did not show the baseline note' }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle([IntPtr]$proc.MainWindowHandle)
    if ($null -eq $root) { throw 'Akula did not expose a UI Automation root' }
    $user = $null
    $archive = $null
    for ($i = 0; $i -lt 100; $i++) {
        Start-Sleep -Milliseconds 200
        try {
            $user = Find-TreeItem -Root $root -Name '.заметки'
            $archive = Find-TreeItem -Root $root -Name '.архив'
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            $user = $null
            $archive = $null
        }
        if ($null -ne $user -and $null -ne $archive) { break }
    }
    if ($null -eq $user -or $null -eq $archive) {
        throw 'Installed file tree did not expose both user dot folders'
    }
    foreach ($folder in @($user, $archive)) {
        $bounds = $folder.Current.BoundingRectangle
        if ($folder.Current.IsOffscreen -or $bounds.IsEmpty -or $bounds.Width -le 0 -or $bounds.Height -le 0) {
            throw "Installed tree folder not visible: $($folder.Current.Name)"
        }
    }
    foreach ($name in @('.git', '.trash', 'SECRET', 'DELETED')) {
        if ($null -ne (Find-TreeItem -Root $root -Name $name)) { throw "Internal UIA TreeItem exposed: $name" }
    }
    Activate-TreeItem -Item $user
    $noteRow = $null
    for ($i = 0; $i -lt 80; $i++) {
        Start-Sleep -Milliseconds 200
        $noteRow = Find-TreeItem -Root $root -Name 'Идея'
        if ($null -ne $noteRow -and !$noteRow.Current.IsOffscreen) { break }
    }
    if ($null -eq $noteRow -or $noteRow.Current.IsOffscreen) {
        throw 'Expanding the real .заметки TreeItem did not expose its note'
    }
    Activate-TreeItem -Item $noteRow
    $opened = $false
    for ($i = 0; $i -lt 80; $i++) {
        Start-Sleep -Milliseconds 200
        $proc.Refresh()
        if ($proc.HasExited) { throw 'Akula exited while navigating the user dot note' }
        if ($proc.MainWindowTitle -like '*Идея*' -and $proc.MainWindowTitle -like '*Akula*') {
            $opened = $true
            break
        }
    }
    if (!$opened) { throw 'UIA activation did not open the dot-folder note in installed Akula' }
    if ((Get-FileHash -LiteralPath $idea -Algorithm SHA256).Hash -ne $hash) {
        throw 'Tree navigation changed original note bytes'
    }
    Write-Host 'AKULA_WINDOWS_DOT_FOLDER_UIA_PASS folders=2 service_hidden=True opened_note=True source_unchanged=True'
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
