# Windows-only smoke: actual DICOM folder button and system picker, no vault.
param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$FixtureRoot
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class DicomPickerInput {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    public static void Click(int x, int y) {
        if (!SetCursorPos(x, y)) throw new InvalidOperationException("Cannot position cursor");
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
        mouse_event(4, 0, 0, 0, UIntPtr.Zero);
    }
}
'@
function Get-Descendants([System.Windows.Automation.AutomationElement]$element) {
    return @($element.FindAll(
        [System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition))
}
function Click-Control([System.Windows.Automation.AutomationElement]$element) {
    if (!$element.Current.IsEnabled -or $element.Current.IsOffscreen) {
        throw 'DICOM GUI control is disabled or offscreen'
    }
    $rect = $element.Current.BoundingRectangle
    if ($rect.Width -lt 3 -or $rect.Height -lt 3) { throw 'DICOM control has no click area' }
    [DicomPickerInput]::Click([int]($rect.Left + $rect.Width / 2), [int]($rect.Top + $rect.Height / 2))
}
function Wait-FileDialog([int]$targetPid, [int]$mainHandle) {
    $desktop = [System.Windows.Automation.AutomationElement]::RootElement
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        Start-Sleep -Milliseconds 200
        $children = @($desktop.FindAll(
            [System.Windows.Automation.TreeScope]::Children,
            [System.Windows.Automation.Condition]::TrueCondition))
        foreach ($window in $children) {
            try {
                if ($window.Current.ProcessId -eq $targetPid -and
                    $window.Current.NativeWindowHandle -ne $mainHandle -and
                    !$window.Current.IsOffscreen -and
                    $window.Current.ControlType.ProgrammaticName -eq 'ControlType.Window') {
                    return $window
                }
            } catch [System.Windows.Automation.ElementNotAvailableException] {}
        }
    }
    throw 'Native Windows folder picker did not appear after clicking DICOM button'
}
function Wait-DicomFolder([IntPtr]$handle) {
    $last = @()
    for ($attempt = 0; $attempt -lt 100; $attempt++) {
        Start-Sleep -Milliseconds 200
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($handle)
        $last = @(Get-Descendants $root | ForEach-Object { $_.Current.Name } |
            Where-Object { ![string]::IsNullOrWhiteSpace($_) })
        if (@($last | Where-Object { $_ -like '*DICOM*серия 1/2*срез 1/3*' -and $_ -notlike '*ошибка чтения*' }).Count -gt 0) {
            return
        }
    }
    $last | Out-File -FilePath 'shots/dicom-picker-failure.txt' -Encoding utf8
    throw 'DICOM picker never showed series 1/2, slice 1/3 in native GUI'
}
$exe = (Resolve-Path -LiteralPath $Executable).Path
$folder = (Resolve-Path -LiteralPath (Join-Path $FixtureRoot 'multiseries_root')).Path
if (!(Test-Path -LiteralPath (Join-Path $folder 'DICOMDIR') -PathType Leaf)) {
    throw 'Synthetic two-series DICOMDIR fixture is missing'
}
$before = @(Get-ChildItem -LiteralPath $folder -File -Recurse |
    Sort-Object FullName | ForEach-Object { $_.FullName.Substring($folder.Length) + ':' +
        (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash })
$data = Join-Path $env:RUNNER_TEMP 'akula-dicom-picker-no-vault'
if (Test-Path -LiteralPath $data) { throw 'Picker test data directory unexpectedly exists' }
New-Item -Path $data -ItemType Directory | Out-Null
$process = Start-Process -FilePath $exe -ArgumentList @('--data-dir', ('"' + $data + '"')) -PassThru
try {
    $mainWindow = [IntPtr]::Zero
    for ($i = 0; $i -lt 60; $i++) {
        Start-Sleep -Milliseconds 500
        $process.Refresh()
        if ($process.HasExited) { throw 'Windows GUI closed before picker test' }
        if ($process.MainWindowHandle -ne 0) { $mainWindow = $process.MainWindowHandle; break }
    }
    if ($mainWindow -eq [IntPtr]::Zero) { throw 'No Akula main window' }
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($mainWindow)
    $buttons = @(Get-Descendants $root | Where-Object {
        $_.Current.ControlType.ProgrammaticName -eq 'ControlType.Button' -and
        ($_.Current.Name -like '*DICOM*папку*' -or $_.Current.Name -like '*DICOM*folder*')
    })
    if ($buttons.Count -ne 1) { throw "Expected one no-vault DICOM folder button; found $($buttons.Count)" }
    $null = [DicomPickerInput]::SetForegroundWindow($mainWindow)
    Start-Sleep -Milliseconds 300
    Click-Control $buttons[0]
    $dialog = Wait-FileDialog $process.Id ([int]$mainWindow.ToInt64())
    Write-Host 'AKULA_DICOM_FOLDER_PICKER_OPEN_PASS native_dialog=1 button_click=1 no_vault=1'
    $dialogHandle = [IntPtr]::new([int64]$dialog.Current.NativeWindowHandle)
    $null = [DicomPickerInput]::SetForegroundWindow($dialogHandle)
    Start-Sleep -Milliseconds 250
    [System.Windows.Forms.SendKeys]::SendWait('%d')
    [System.Windows.Forms.SendKeys]::SendWait($folder)
    [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
    Start-Sleep -Milliseconds 350
    # Windows Common Item Dialog may expose the "Select Folder" caption as
    # a text/control descendant, not strictly ControlType.Button. Click its
    # visible on-screen bounds, which is what the user does with a mouse.
    $confirm = $null
    for ($attempt = 0; $attempt -lt 40; $attempt++) {
        Start-Sleep -Milliseconds 250
        $dialog = [System.Windows.Automation.AutomationElement]::FromHandle($dialogHandle)
        $candidates = @(Get-Descendants $dialog | Where-Object {
            $_.Current.Name.Trim() -in @('Select Folder', 'Choose Folder', 'Выбрать папку', 'Выбор папки')
        })
        $usable = @($candidates | Where-Object { $_.Current.IsEnabled -and !$_.Current.IsOffscreen })
        if ($usable.Count -gt 0) { $confirm = $usable[0]; break }
    }
    if ($null -eq $confirm) {
        $snapshot = @(Get-Descendants $dialog | Where-Object {
            $_.Current.Name -match 'Folder|папк|Open|Открыть'
        } | ForEach-Object {
            "$($_.Current.Name) type=$($_.Current.ControlType.ProgrammaticName) enabled=$($_.Current.IsEnabled) offscreen=$($_.Current.IsOffscreen)"
        })
        $snapshot | Out-File -FilePath 'shots/dicom-picker-controls.txt' -Encoding utf8
        throw 'Windows folder dialog did not expose an enabled Select Folder control'
    }
    Write-Host "AKULA_DICOM_FOLDER_PICKER_CONFIRM_FOUND type=$($confirm.Current.ControlType.ProgrammaticName)"
    Click-Control $confirm
    Wait-DicomFolder $mainWindow
    Write-Host 'AKULA_DICOM_FOLDER_PICKER_OPENED_PASS source=multiseries_root series=2 slices=3'
    $after = @(Get-ChildItem -LiteralPath $folder -File -Recurse |
        Sort-Object FullName | ForEach-Object { $_.FullName.Substring($folder.Length) + ':' +
            (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash })
    if (($before -join [Environment]::NewLine) -ne ($after -join [Environment]::NewLine)) {
        throw 'DICOM picker mutated its source'
    }
    Write-Host "AKULA_DICOM_FOLDER_PICKER_READONLY_PASS file_count=$($after.Count)"
} finally {
    $process.Refresh()
    if (!$process.HasExited) { Stop-Process -Id $process.Id -Force }
}
