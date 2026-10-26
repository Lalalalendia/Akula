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

# TreeItem exposes AccessKit Click, but Windows UIA does not expose InvokePattern
# for this role. Exercise the real installed pointer path, not a model-only hook.
. (Join-Path $PSScriptRoot 'windows-beta-gui-input.ps1')
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AkulaDotPointer {
    [StructLayout(LayoutKind.Explicit, Size = 40)]
    public struct Input {
        [FieldOffset(0)] public uint Type;
        [FieldOffset(20)] public uint Flags;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct Point {
        public int X;
        public int Y;
    }
    [DllImport("user32.dll", SetLastError = true)]
    static extern uint SendInput(uint count, [In] Input[] inputs, int size);
    [DllImport("user32.dll", SetLastError = true)]
    static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")]
    static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")]
    static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
    [DllImport("user32.dll")]
    static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", SetLastError = true)]
    static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);

    public static void Click(IntPtr hwnd, int expectedPid, int x, int y) {
        if (IntPtr.Size != 8 || Marshal.SizeOf(typeof(Input)) != 40 || hwnd == IntPtr.Zero)
            throw new InvalidOperationException("Invalid Windows x64 pointer input layout/window");
        uint owner;
        if (GetWindowThreadProcessId(hwnd, out owner) == 0 || owner != (uint)expectedPid)
            throw new InvalidOperationException("Refusing pointer input: Akula window PID mismatch");
        if (GetForegroundWindow() != hwnd)
            throw new InvalidOperationException("Refusing pointer input: Akula is not foreground");
        if (!SetCursorPos(x, y))
            throw new InvalidOperationException("Cannot position mouse on verified Akula TreeItem");
        IntPtr underPointer = WindowFromPoint(new Point { X = x, Y = y });
        if (underPointer == IntPtr.Zero || GetAncestor(underPointer, 2) != hwnd)
            throw new InvalidOperationException("Refusing to click outside the installed Akula window");
        var inputs = new Input[] {
            new Input { Type = 0, Flags = 0x0002 },
            new Input { Type = 0, Flags = 0x0004 }
        };
        uint sent = SendInput(2, inputs, Marshal.SizeOf(typeof(Input)));
        if (sent != 2)
            throw new InvalidOperationException("OS mouse click injected " + sent +
                "/2 events; Win32 error " + Marshal.GetLastWin32Error());
    }
}
'@ -ErrorAction Stop

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
    param(
        [System.Windows.Automation.AutomationElement]$Item,
        [System.Windows.Automation.AutomationElement]$Root,
        [System.Diagnostics.Process]$Process
    )
    $Process.Refresh()
    if ($Process.HasExited -or $Process.MainWindowHandle -eq [IntPtr]::Zero -or
        $Process.MainWindowTitle -notlike '*Akula*') {
        throw 'Cannot click an absent or unexpected Akula window'
    }
    $bounds = $Item.Current.BoundingRectangle
    $windowBounds = $Root.Current.BoundingRectangle
    if ($Item.Current.IsOffscreen -or $bounds.IsEmpty -or
        $bounds.Width -le 0 -or $bounds.Height -le 0) {
        throw "Cannot physically click an invisible TreeItem: $($Item.Current.Name)"
    }
    $x = [int][Math]::Floor($bounds.Left + $bounds.Width / 2)
    $y = [int][Math]::Floor($bounds.Top + $bounds.Height / 2)
    if ($x -lt $windowBounds.Left -or $x -ge $windowBounds.Right -or
        $y -lt $windowBounds.Top -or $y -ge $windowBounds.Bottom) {
        throw "TreeItem center outside verified Akula window: $($Item.Current.Name)"
    }
    $hwnd = [IntPtr]$Process.MainWindowHandle
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
    if (!$foreground) { throw 'Refusing pointer click without foreground Akula' }
    [AkulaDotPointer]::Click($hwnd, $Process.Id, $x, $y)
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
    Activate-TreeItem -Item $user -Root $root -Process $proc
    $noteRow = $null
    for ($i = 0; $i -lt 80; $i++) {
        Start-Sleep -Milliseconds 200
        $noteRow = Find-TreeItem -Root $root -Name 'Идея'
        if ($null -ne $noteRow -and !$noteRow.Current.IsOffscreen) { break }
    }
    if ($null -eq $noteRow -or $noteRow.Current.IsOffscreen) {
        throw 'Expanding the real .заметки TreeItem did not expose its note'
    }
    Activate-TreeItem -Item $noteRow -Root $root -Process $proc
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
